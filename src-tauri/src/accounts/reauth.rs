// bulk re-authorization: login via code from service messages (777000)

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::commands::{get_storage_pub, invalidate_accounts_cache};
use super::devices;
use super::session::{AccountJson, TelethonSession};
use crate::mtproto::auth::{compute_srp, perform_dh, Srp};
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::mtproto::transport::MtpTransport;

#[derive(Serialize, Clone, Default)]
pub struct ReauthResults {
    pub success: u32,
    pub unknown_2fa: u32,
    pub failed: u32,
    pub errors: Vec<String>,
}

#[tauri::command]
pub async fn reauth_accounts(
    ids: Vec<String>,
    threads: Option<usize>,
    terminate_others: Option<bool>,
) -> Result<ReauthResults, String> {
    let concurrency = threads.unwrap_or(5).max(1).min(1000);
    let do_terminate = terminate_others.unwrap_or(false);
    let storage = get_storage_pub();

    // filter eligible accounts
    let mut tasks: Vec<(String, TelethonSession, AccountJson)> = Vec::new();
    let mut results = ReauthResults::default();

    for id in &ids {
        let session_path = storage.session_path(id);
        let json_path = storage.json_path(id);
        if !session_path.exists() {
            continue;
        }

        let session = match TelethonSession::from_file(&session_path) {
            Ok(s) => s,
            Err(_) => {
                results.failed += 1;
                results.errors.push("session_read_error".into());
                continue;
            }
        };
        let json = if json_path.exists() {
            AccountJson::from_file(&json_path).unwrap_or_default()
        } else {
            AccountJson::default()
        };

        // skip frozen
        if json.status == crate::i18n::t("status_frozen")
            || json.status == crate::i18n::t("status_perm_spam")
        {
            results.failed += 1;
            results.errors.push("frozen".into());
            continue;
        }

        // skip unknown 2fa
        if json.two_fa.starts_with(&crate::i18n::t("two_fa_unknown"))
            || json.two_fa == crate::i18n::t("two_fa_unknown_set")
        {
            results.unknown_2fa += 1;
            continue;
        }

        tasks.push((id.clone(), session, json));
    }

    // immediately set status so frontend sees it right away
    for (id, _, _) in &tasks {
        update_status(id, &crate::i18n::t("reauth_status"));
    }

    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let results_lock = Arc::new(tokio::sync::Mutex::new(results));

    let mut handles = Vec::new();
    for (id, session, json) in tasks {
        let sem = sem.clone();
        let results_lock = results_lock.clone();
        let terminate = do_terminate;
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let outcome = reauth_single(&id, &session, &json, terminate).await;
            let mut r = results_lock.lock().await;
            match outcome {
                Ok(()) => r.success += 1,
                Err(e) => {
                    if e.contains("SESSION_PASSWORD_NEEDED") || e.contains("unknown_2fa") {
                        r.unknown_2fa += 1;
                    } else {
                        r.failed += 1;
                        r.errors.push(e);
                    }
                }
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    invalidate_accounts_cache();
    let final_results = results_lock.lock().await.clone();
    Ok(final_results)
}

async fn reauth_single(
    id: &str,
    session: &TelethonSession,
    json: &AccountJson,
    terminate_others: bool,
) -> Result<(), String> {
    if session.auth_key.len() != 256 {
        return Err("invalid_auth_key".into());
    }

    let storage = get_storage_pub();
    let json_path = storage.json_path(id);

    let addr = format!("{}:{}", session.server_address, session.port);
    let proxy = crate::proxy::select_proxy_for_account(json.proxy.as_deref())?;

    let mut old_key = [0u8; 256];
    old_key.copy_from_slice(&session.auth_key);

    // connect with old key to read code later
    let mut old_client = MtpClient::connect(&addr, &old_key, proxy.as_ref())
        .await
        .map_err(|e| format!("connect_old: {e}"))?;

    // use existing device info from json, or generate random if missing
    let dev = if !json.device.is_empty() && !json.sdk.is_empty() {
        devices::DeviceInfo {
            device: json.device.clone(),
            sdk: json.sdk.clone(),
            app_version: if json.app_version.is_empty() {
                "10.14.5".to_string()
            } else {
                json.app_version.clone()
            },
        }
    } else {
        devices::generate_random_device()
    };
    let app_id = if json.app_id == 0 {
        crate::get_app_config().app_id
    } else {
        json.app_id
    };

    // init connection on old client to verify session is alive
    let get_me_req =
        tl::build_get_me_request(app_id, &dev.device, &dev.sdk, &dev.app_version, "en", "en");
    let me_resp = old_client.invoke(&get_me_req).await.map_err(|e| {
        // if the session is already dead/banned/frozen at the start, mark it in json
        super::commands::check_and_mark_dead_session(&e, id);
        format!("get_me: {e}")
    })?;
    let me = tl::parse_users_response(&me_resp).map_err(|e| format!("parse_me: {e}"))?;

    let phone = if !me.phone.is_empty() {
        me.phone.clone()
    } else {
        json.phone.clone()
    };
    if phone.is_empty() {
        return Err("no_phone".into());
    }

    // terminate all other sessions if requested
    if terminate_others {
        let term_req = tl::build_auth_reset_authorizations();
        let _ = old_client.invoke(&term_req).await; // ignore errors (FRESH_RESET_AUTHORISATION_FORBIDDEN etc)
    }

    // update status
    update_status(id, &crate::i18n::t_with("reauth_step", &[("step", "1")]));

    // request new auth on the same DC via DH with fresh random device fingerprint
    let new_dev = devices::generate_random_device();
    let dc_addr = &addr;

    let mut transport = MtpTransport::connect(dc_addr, proxy.as_ref())
        .await
        .map_err(|e| format!("connect_new: {e}"))?;
    let dh = perform_dh(&mut transport)
        .await
        .map_err(|e| format!("dh: {e}"))?;
    let mut new_client = MtpClient::from_transport(transport, dh.auth_key, dh.server_salt, dc_addr);

    // send code
    let app_hash = if json.app_hash.is_empty() {
        "b18441a1ff607e10a989891a5462e627".to_string()
    } else {
        json.app_hash.clone()
    };
    let inner = tl::build_auth_send_code(&phone, app_id, &app_hash);
    let request = tl::wrap_init_connection(
        &inner,
        app_id,
        &new_dev.device,
        &new_dev.sdk,
        &new_dev.app_version,
        "en",
        "en",
    );
    let send_resp = new_client
        .invoke(&request)
        .await
        .map_err(|e| format!("send_code: {e}"))?;
    let sent = tl::parse_auth_sent_code(&send_resp).map_err(|e| format!("parse_sent_code: {e}"))?;

    let request_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i32;

    // read code from service messages (777000) via old client, up to 3 attempts
    let mut code: Option<String> = None;
    for attempt in 1..=3 {
        update_status(
            id,
            &crate::i18n::t_with("reauth_step", &[("step", &attempt.to_string())]),
        );
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        let hist_req = tl::build_get_history_service(3);
        if let Ok(hist_data) = old_client.invoke(&hist_req).await {
            if let Ok(messages) = tl::parse_messages_history(&hist_data) {
                for msg in &messages {
                    if let Some(c) = extract_code(msg, request_time) {
                        code = Some(c);
                        break;
                    }
                }
            }
        }
        if code.is_some() {
            break;
        }
    }

    let code = code.ok_or_else(|| "code_not_received".to_string())?;

    update_status(id, &crate::i18n::t("reauth_signing_in"));

    // sign in with new client
    let sign_in_req = tl::build_auth_sign_in(&phone, &sent.phone_code_hash, &code);
    let sign_resp = new_client.invoke(&sign_in_req).await;

    let auth_result: Result<tl::AuthorizedUser, String> = match sign_resp {
        Ok(data) => tl::parse_auth_authorization(&data),
        Err(e) => Err(e),
    };

    match auth_result {
        Ok(_user) => {}
        Err(e) => {
            if e.contains("SESSION_PASSWORD_NEEDED") {
                // try 2fa if password is known
                let two_fa = &json.two_fa;
                if two_fa.is_empty()
                    || two_fa.starts_with(&crate::i18n::t("two_fa_unknown"))
                    || *two_fa == crate::i18n::t("two_fa_unknown_set")
                {
                    // mark as unknown 2fa
                    let mut updated = json.clone();
                    updated.two_fa = crate::i18n::t("two_fa_unknown");
                    let _ = updated.to_file(&json_path);
                    return Err("unknown_2fa".into());
                }
                // attempt 2fa
                let pw_req = tl::build_account_get_password();
                let pw_data = new_client
                    .invoke(&pw_req)
                    .await
                    .map_err(|e| format!("get_password: {e}"))?;
                let pw = tl::parse_account_password(&pw_data)
                    .map_err(|e| format!("parse_password: {e}"))?;

                let srp = Srp {
                    g: pw.g,
                    p: pw.p,
                    salt1: pw.salt1,
                    salt2: pw.salt2,
                    srp_id: pw.srp_id,
                    srp_b: pw.srp_b,
                };
                let proof = compute_srp(&srp, two_fa).map_err(|e| format!("srp: {e}"))?;
                let check_req = tl::build_auth_check_password(srp.srp_id, &proof.a, &proof.m1);
                let check_resp = new_client
                    .invoke(&check_req)
                    .await
                    .map_err(|e| format!("check_password: {e}"))?;
                let _ = tl::parse_auth_authorization(&check_resp)
                    .map_err(|e| format!("auth_after_2fa: {e}"))?;
            } else {
                return Err(format!("sign_in: {e}"));
            }
        }
    }

    // save new auth_key first, before logging out old session
    let new_key = *new_client.auth_key();
    let new_session = TelethonSession {
        dc_id: session.dc_id,
        server_address: session.server_address.clone(),
        port: session.port,
        auth_key: new_key.to_vec(),
    };
    new_session
        .to_file(&storage.session_path(id))
        .map_err(|e| format!("save_session: {e}"))?;

    // only logout old session after new one is safely persisted
    let logout_req = tl::build_auth_log_out();
    let _ = old_client.invoke(&logout_req).await;

    // update json with new device info
    let mut updated = json.clone();
    updated.valid = true;
    updated.validated = true;
    updated.device = new_dev.device;
    updated.sdk = new_dev.sdk;
    updated.app_version = new_dev.app_version;
    let _ = updated.to_file(&json_path);

    // regenerate tdata
    let tdata = crate::converter::tdata::TDataAccount {
        dc_id: session.dc_id,
        user_id: json.user_id,
        auth_key: new_key.to_vec(),
    };
    let _ = crate::converter::tdata::write_tdata(&storage.tdata_dir(id), &tdata);

    Ok(())
}

fn extract_code(msg: &str, _request_time: i32) -> Option<String> {
    // look for 5-digit code in message text
    let text = msg;
    for i in 0..text.len().saturating_sub(4) {
        let slice = &text.as_bytes()[i..i + 5];
        if slice.iter().all(|b| b.is_ascii_digit()) {
            let before_ok = i == 0 || !text.as_bytes()[i - 1].is_ascii_digit();
            let after_ok = text
                .as_bytes()
                .get(i + 5)
                .map(|b| !b.is_ascii_digit())
                .unwrap_or(true);
            if before_ok && after_ok {
                return Some(String::from_utf8_lossy(slice).to_string());
            }
        }
    }
    None
}

fn update_status(id: &str, status: &str) {
    let storage = get_storage_pub();
    let json_path = storage.json_path(id);
    let mut json = AccountJson::from_file(&json_path).unwrap_or_default();
    json.status = status.to_string();
    let _ = json.to_file(&json_path);
    invalidate_accounts_cache();
}
