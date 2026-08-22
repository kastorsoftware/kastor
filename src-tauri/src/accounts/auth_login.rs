// phone+code login flow with full DH key exchange
// keeps intermediate state per session_id with 5-min timeout

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use serde::Serialize;

use crate::mtproto::auth::{compute_srp, perform_dh, Srp};
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::mtproto::transport::MtpTransport;
use crate::proxy::ProxyConfig;

use super::devices;
use super::session::{AccountJson, TelethonSession};

const DC2_ADDR: &str = "149.154.167.51:443";
const DC2_IP: &str = "149.154.167.51";
const DC2_PORT: i32 = 443;
const SESSION_TIMEOUT_SECS: u64 = 300;

// telegram production DC IPs (v4)
fn dc_addr(dc_id: i32) -> &'static str {
    match dc_id {
        1 => "149.154.175.53:443",
        2 => "149.154.167.51:443",
        3 => "149.154.175.100:443",
        4 => "149.154.167.91:443",
        5 => "91.108.56.130:443",
        _ => DC2_ADDR,
    }
}

fn dc_ip(dc_id: i32) -> String {
    dc_addr(dc_id).split(':').next().unwrap_or(DC2_IP).to_string()
}

pub struct AuthState {
    client: Option<MtpClient>,
    dc_id: i32,
    phone: String,
    phone_code_hash: String,
    proxy: Option<ProxyConfig>,
    device: devices::DeviceInfo,
    created_at: Instant,
}

#[derive(Default)]
pub struct AuthSessions {
    pub sessions: StdMutex<HashMap<String, AuthState>>,
}

impl AuthSessions {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn cleanup_expired(&self) {
        if let Ok(mut map) = self.sessions.lock() {
            map.retain(|_, s| s.created_at.elapsed().as_secs() < SESSION_TIMEOUT_SECS);
        }
    }
}

#[derive(Serialize)]
pub struct AuthSessionResp {
    pub session_id: String,
    pub code_type: String,
}

#[derive(Serialize)]
pub struct AuthResultResp {
    pub account_id: Option<String>,
    pub two_fa_required: bool,
    pub hint: String,
}

#[tauri::command]
pub async fn auth_send_code(
    phone: String,
    sessions: tauri::State<'_, Arc<AuthSessions>>,
) -> Result<AuthSessionResp, String> {
    sessions.cleanup_expired();
    let phone = normalize_phone(&phone);
    if phone.len() < 5 {
        return Err("PHONE_NUMBER_INVALID".into());
    }

    dbg_log!("auth_send_code phone={}", phone);

    let proxy = crate::proxy::select_proxy_for_account(None)?;
    let device = devices::generate_random_device();

    // try DC2 first; on PHONE_MIGRATE_X follow the redirect
    let mut current_dc = 2i32;
    let max_redirects = 4;
    let mut last_err = String::new();

    for _attempt in 0..=max_redirects {
        let addr = dc_addr(current_dc);
        let (client, sent) = match try_send_code_on_dc(addr, &phone, &device, proxy.as_ref()).await {
            Ok(ok) => ok,
            Err(e) => {
                if let Some(target_dc) = parse_phone_migrate(&e) {
                    dbg_log!("auth_send_code PHONE_MIGRATE_{} -> reconnecting", target_dc);
                    current_dc = target_dc;
                    continue;
                }
                last_err = e;
                break;
            }
        };

        let session_id = uuid::Uuid::new_v4().to_string();
        if let Ok(mut map) = sessions.sessions.lock() {
            map.insert(session_id.clone(), AuthState {
                client: Some(client),
                dc_id: current_dc,
                phone: phone.clone(),
                phone_code_hash: sent.phone_code_hash,
                proxy: proxy.clone(),
                device: device.clone(),
                created_at: Instant::now(),
            });
        }

        return Ok(AuthSessionResp { session_id, code_type: sent.code_type });
    }

    Err(map_rpc_error(&last_err))
}

async fn try_send_code_on_dc(
    addr: &str,
    phone: &str,
    device: &devices::DeviceInfo,
    proxy: Option<&ProxyConfig>,
) -> Result<(MtpClient, tl::SentCode), String> {
    let mut transport = MtpTransport::connect(addr, proxy)
        .await
        .map_err(|e| crate::i18n::t_with("auth_connect_error", &[("error", &e.to_string())]))?;

    let dh = perform_dh(&mut transport).await
        .map_err(|e| crate::i18n::t_with("auth_dh_error", &[("error", &e.to_string())]))?;

    let mut client = MtpClient::from_transport(transport, dh.auth_key, dh.server_salt, addr);

    let app_id = crate::get_app_config().app_id;
    let app_hash = crate::get_app_config().app_hash.clone();
    let inner = tl::build_auth_send_code(phone, app_id, &app_hash);
    let request = tl::wrap_init_connection(
        &inner,
        app_id,
        &device.device,
        &device.sdk,
        &device.app_version,
        "en",
        "en",
    );

    let response = client.invoke(&request).await?;
    let sent = tl::parse_auth_sent_code(&response)?;
    Ok((client, sent))
}

// extract X from "PHONE_MIGRATE_X" rpc error message
fn parse_phone_migrate(err: &str) -> Option<i32> {
    // matches "rpc error 303: PHONE_MIGRATE_4" or "PHONE_MIGRATE_4" or "NETWORK_MIGRATE_4" or "USER_MIGRATE_4"
    for prefix in ["PHONE_MIGRATE_", "NETWORK_MIGRATE_", "USER_MIGRATE_"] {
        if let Some(idx) = err.find(prefix) {
            let tail = &err[idx + prefix.len()..];
            let num: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = num.parse::<i32>() {
                if (1..=5).contains(&n) {
                    return Some(n);
                }
            }
        }
    }
    None
}

#[tauri::command]
pub async fn auth_sign_in(
    session_id: String,
    code: String,
    sessions: tauri::State<'_, Arc<AuthSessions>>,
) -> Result<AuthResultResp, String> {
    sessions.cleanup_expired();
    dbg_log!("auth_sign_in session_id={} code_len={}", session_id, code.len());

    let mut state = take_state(&sessions, &session_id)?;
    let mut client = state.client.take().ok_or("session lost client")?;

    let inner = tl::build_auth_sign_in(&state.phone, &state.phone_code_hash, &code);
    let resp = client.invoke(&inner).await;

    // unify both error sources: client.invoke -> RPC error, parse_auth_authorization -> rpc_error inside body
    let result: Result<tl::AuthorizedUser, String> = match resp {
        Ok(data) => tl::parse_auth_authorization(&data),
        Err(e) => Err(e),
    };

    match result {
        Ok(user) => {
            let auth_key = *client.auth_key();
            let account_id = save_account(&user, &state, &auth_key, "")?;
            Ok(AuthResultResp { account_id: Some(account_id), two_fa_required: false, hint: String::new() })
        }
        Err(e) => {
            let mapped = map_rpc_error(&e);
            if mapped == "SESSION_PASSWORD_NEEDED" {
                // restore client into state so check_password can reuse the same connection
                state.client = Some(client);
                put_state(&sessions, &session_id, state);
                return Ok(AuthResultResp { account_id: None, two_fa_required: true, hint: String::new() });
            }
            if mapped == "PHONE_CODE_INVALID" {
                // A mistyped code is retryable; keep the authorization session alive.
                state.client = Some(client);
                put_state(&sessions, &session_id, state);
            }
            Err(mapped)
        }
    }
}

#[tauri::command]
pub async fn auth_check_password(
    session_id: String,
    password: String,
    sessions: tauri::State<'_, Arc<AuthSessions>>,
) -> Result<String, String> {
    sessions.cleanup_expired();
    dbg_log!("auth_check_password session_id={}", session_id);

    let mut state = take_state(&sessions, &session_id)?;
    let mut client = state.client.take().ok_or("session lost client")?;

    // refetch password to get fresh srp_b/srp_id
    let pw_req = tl::build_account_get_password();
    let pw_data = client.invoke(&pw_req).await
        .map_err(|e| map_rpc_error(&e))?;
    let pw = tl::parse_account_password(&pw_data)
        .map_err(|e| map_rpc_error(&e))?;

    if !pw.has_password {
        return Err(crate::i18n::t("auth_2fa_not_set"));
    }

    let srp = Srp {
        g: pw.g,
        p: pw.p,
        salt1: pw.salt1,
        salt2: pw.salt2,
        srp_id: pw.srp_id,
        srp_b: pw.srp_b,
    };

    let proof = compute_srp(&srp, &password)
        .map_err(|e| crate::i18n::t_with("auth_srp_error", &[("error", &e.to_string())]))?;

    let req = tl::build_auth_check_password(srp.srp_id, &proof.a, &proof.m1);
    let resp = client.invoke(&req).await
        .map_err(|e| map_rpc_error(&e))?;
    let user = tl::parse_auth_authorization(&resp)
        .map_err(|e| map_rpc_error(&e))?;

    let auth_key = *client.auth_key();
    save_account(&user, &state, &auth_key, &password)
}

fn take_state(sessions: &AuthSessions, session_id: &str) -> Result<AuthState, String> {
    let mut map = sessions.sessions.lock().map_err(|_| "session lock poisoned")?;
    map.remove(session_id).ok_or_else(|| crate::i18n::t("auth_session_expired"))
}

fn put_state(sessions: &AuthSessions, session_id: &str, state: AuthState) {
    if let Ok(mut map) = sessions.sessions.lock() {
        map.insert(session_id.to_string(), state);
    }
}

fn save_account(
    user: &tl::AuthorizedUser,
    state: &AuthState,
    auth_key: &[u8; 256],
    two_fa_password: &str,
) -> Result<String, String> {
    let storage = super::commands::get_storage_pub();

    // dedup by auth_key
    let new_key_vec = auth_key.to_vec();
    if let Ok(entries) = std::fs::read_dir(storage.session_json_dir()) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().map(|e| e == "session").unwrap_or(false) {
                if let Ok(s) = TelethonSession::from_file(&p) {
                    if s.auth_key == new_key_vec {
                        return Err("duplicate".into());
                    }
                }
            }
        }
    }

    let id = uuid::Uuid::new_v4().to_string();

    let session = TelethonSession {
        dc_id: state.dc_id,
        server_address: dc_ip(state.dc_id),
        port: DC2_PORT,
        auth_key: new_key_vec.clone(),
    };
    session.to_file(&storage.session_path(&id))
        .map_err(|e| crate::i18n::t_with("auth_session_write_error", &[("error", &e.to_string())]))?;

    let proxy_repr = state.proxy.as_ref().map(|p| p.to_string_repr());
    let config = crate::get_app_config();
    let json = AccountJson {
        app_id: config.app_id,
        app_hash: config.app_hash.clone(),
        sdk: state.device.sdk.clone(),
        device: state.device.device.clone(),
        app_version: state.device.app_version.clone(),
        lang_pack: "en".to_string(),
        system_lang_pack: "en-US".to_string(),
        proxy: proxy_repr,
        two_fa: two_fa_password.to_string(),
        phone: if !user.phone.is_empty() {
            user.phone.clone()
        } else {
            state.phone.clone()
        },
        first_name: user.first_name.clone(),
        last_name: user.last_name.clone(),
        username: user.username.clone(),
        user_id: user.user_id,
        is_premium: user.is_premium,
        validated: true,
        valid: true,
        status: crate::i18n::t("status_clean"),
        register_time: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
        ..Default::default()
    };
    json.to_file(&storage.json_path(&id))?;

    // generate tdata for telegram desktop launch
    let tdata = crate::converter::tdata::TDataAccount {
        dc_id: state.dc_id,
        user_id: user.user_id,
        auth_key: new_key_vec,
    };
    let _ = crate::converter::tdata::write_tdata(&storage.tdata_dir(&id), &tdata);

    super::commands::invalidate_accounts_cache();
    Ok(id)
}

fn normalize_phone(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

fn map_rpc_error(e: &str) -> String {
    // strip "rpc error 400: " or similar prefix and translate well-known codes
    let core = if let Some(idx) = e.find(": ") {
        &e[idx + 2..]
    } else {
        e
    };
    let core_upper: String = core.split_whitespace().next().unwrap_or(core).to_string();
    match core_upper.as_str() {
        "PHONE_NUMBER_INVALID" => "PHONE_NUMBER_INVALID".to_string(),
        "PHONE_CODE_INVALID" => "PHONE_CODE_INVALID".to_string(),
        "PHONE_CODE_EXPIRED" => "PHONE_CODE_EXPIRED".to_string(),
        "PHONE_CODE_EMPTY" => "PHONE_CODE_INVALID".to_string(),
        "PASSWORD_HASH_INVALID" => "PASSWORD_HASH_INVALID".to_string(),
        "SESSION_PASSWORD_NEEDED" => "SESSION_PASSWORD_NEEDED".to_string(),
        "PHONE_NUMBER_BANNED" => "PHONE_NUMBER_BANNED".to_string(),
        "PHONE_NUMBER_FLOOD" => "PHONE_NUMBER_FLOOD".to_string(),
        "FLOOD_WAIT_X" => "FLOOD_WAIT".to_string(),
        _ => {
            if core.starts_with("FLOOD_WAIT_") {
                return format!("FLOOD_WAIT:{}", core.trim_start_matches("FLOOD_WAIT_"));
            }
            if core.starts_with("PHONE_NUMBER") {
                return "PHONE_NUMBER_INVALID".to_string();
            }
            e.to_string()
        }
    }
}
