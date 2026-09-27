use crate::accounts::devices::DeviceInfo;
use crate::accounts::session::AccountJson;
use crate::accounts::storage::AccountStorage;
use crate::mtproto::auth::perform_dh;
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl::{self, AuthCodeSettings, AuthorizedUser, SentCode};
use crate::mtproto::transport::MtpTransport;
use crate::proxy::ProxyConfig;
use crate::registrar::profile::RegistrationProfile;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::registrar::antisafety::{AntiSafetyClient, AntiSafetyConfig};

const RECAPTCHA_TIMEOUT: Duration = Duration::from_secs(120);
const RECAPTCHA_POLL_INTERVAL: Duration = Duration::from_secs(5);

pub struct RegistrationSession {
    client: MtpClient,
    dc_id: i32,
    phone: String,
    phone_code_hash: String,
    profile: RegistrationProfile,
    proxy: Option<ProxyConfig>,
    output_directory: Option<std::path::PathBuf>,
    server_registered: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegistrationCodeRequest {
    pub phone: String,
    pub profile: RegistrationProfile,
    pub proxy: Option<ProxyConfig>,
    pub code_settings: AuthCodeSettings,
    #[serde(default)]
    pub antisafety: Option<AntiSafetyConfig>,
    #[serde(skip)]
    pub antisafety_proxy: Option<String>,
    #[serde(skip)]
    pub cancellation: Option<Arc<AtomicBool>>,
    #[serde(skip)]
    pub log_target: Option<tauri::AppHandle>,
    #[serde(skip)]
    pub log_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecaptchaChallenge {
    action: String,
    app_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecaptchaState {
    Creating,
    Waiting,
    Ready,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationIdentity {
    pub first_name: String,
    pub last_name: String,
}

impl RegistrationIdentity {
    pub fn validate(&self) -> Result<(), String> {
        if self.first_name.trim().is_empty()
            || self.first_name.len() > 64
            || self.first_name.contains(['\r', '\n', '\0'])
            || self.last_name.len() > 64
            || self.last_name.contains(['\r', '\n', '\0'])
        {
            return Err("invalid registration name".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RegistrationOutcome {
    Registered { account_id: String },
    AlreadyRegistered,
}

impl RegistrationSession {
    pub async fn start(request: RegistrationCodeRequest) -> Result<(Self, SentCode), String> {
        let phone = normalize_phone(&request.phone)?;
        request.profile.validate()?;
        let mut dc_id = 2;
        let mut last_error = "Telegram data center migration limit exceeded".to_string();

        for _ in 0..=4 {
            match start_on_dc(
                dc_id,
                &phone,
                &request.profile,
                request.proxy.as_ref(),
                &request.code_settings,
                request.antisafety.as_ref(),
                request.antisafety_proxy.as_deref(),
                request.cancellation.as_deref(),
                request.log_target.as_ref(),
                &request.log_prefix,
            )
            .await
            {
                Ok((client, sent_code)) => {
                    return Ok((
                        Self {
                            client,
                            dc_id,
                            phone,
                            phone_code_hash: sent_code.phone_code_hash.clone(),
                            profile: request.profile,
                            proxy: request.proxy,
                            output_directory: None,
                            server_registered: false,
                        },
                        sent_code,
                    ));
                }
                Err(error) => {
                    dbg_log!(
                        "registrar::send_code dc={} failed {}",
                        dc_id,
                        super::diagnostics::error_summary(&error)
                    );
                    if let Some(migrated_dc) = phone_migrate_dc(&error) {
                        dbg_log!(
                            "registrar::send_code migrating from_dc={} to_dc={}",
                            dc_id,
                            migrated_dc
                        );
                        dc_id = migrated_dc;
                        continue;
                    }
                    last_error = error;
                    break;
                }
            }
        }
        Err(last_error)
    }

    pub fn set_output_directory(&mut self, directory: std::path::PathBuf) {
        self.output_directory = Some(directory);
    }

    pub fn is_registered(&self) -> bool {
        self.server_registered
    }

    fn storage(&self) -> AccountStorage {
        self.output_directory
            .as_ref()
            .map(|path| AccountStorage::new(path))
            .unwrap_or_else(crate::accounts::commands::get_storage_pub)
    }

    pub async fn set_two_fa(&mut self, account_id: &str, password: &str) -> Result<(), String> {
        dbg_log!("registrar::post_registration 2fa started");
        crate::accounts::actions::set_2fa_password(&mut self.client, password).await?;
        let path = self.storage().json_path(account_id);
        let mut json = AccountJson::from_file(&path)?;
        json.two_fa = password.to_string();
        json.to_file(&path)?;
        dbg_log!("registrar::post_registration 2fa completed");
        Ok(())
    }

    pub async fn register_push(&mut self, token: &str) -> Result<(), String> {
        if token.trim().is_empty() {
            return Err("invalid Push token".to_string());
        }
        // AntiSafety's token_secret is not documented as an MTProto encryption key.
        let request =
            crate::mtproto::tl_gen::build_account_registerDevice(false, 2, token, false, &[], &[]);
        let response = self.client.invoke(&request).await?;
        if !crate::mtproto::tl_gen::parse_account_registerDevice(&response)? {
            return Err("Telegram rejected Push registration".to_string());
        }
        Ok(())
    }

    pub async fn hide_phone_number(&mut self) -> Result<(), String> {
        self.set_privacy(crate::mtproto::tl_gen::INPUT_PRIVACY_KEY_PHONE_NUMBER)
            .await
    }

    pub async fn set_random_avatar(&mut self) -> Result<(), String> {
        crate::accounts::actions::set_random_emoji_avatar(&mut self.client).await
    }

    pub async fn set_avatar(&mut self, mode: &str, path: &str) -> Result<(), String> {
        if mode == "emoji" { return self.set_random_avatar().await; }
        if mode == "generated" { return crate::accounts::actions::set_generated_face_avatar(&mut self.client).await; }
        let photo = if mode == "folder" {
            let photos = crate::accounts::actions::photos_from_folder(path);
            photos.get(rand::random::<usize>() % photos.len().max(1)).ok_or("avatar folder is empty")?.clone()
        } else { path.to_string() };
        crate::accounts::actions::set_photo_from_path(&mut self.client, &photo).await
    }

    pub async fn set_username(&mut self, account_id: &str, mode: &str, prefix: &str, value: Option<&str>) -> Result<(), String> {
        let username = crate::accounts::actions::set_username(&mut self.client, mode, prefix, value).await?;
        let path = self.storage().json_path(account_id);
        let mut json = AccountJson::from_file(&path)?;
        json.username = username;
        json.to_file(&path)
    }

    pub async fn hide_online_status(&mut self) -> Result<(), String> {
        self.set_privacy(crate::mtproto::tl_gen::INPUT_PRIVACY_KEY_STATUS_TIMESTAMP)
            .await
    }

    async fn set_privacy(&mut self, key_constructor: u32) -> Result<(), String> {
        let key = crate::mtproto::tl_gen::serialize_bare_ctor(key_constructor);
        let rule = crate::mtproto::tl_gen::serialize_bare_ctor(
            crate::mtproto::tl_gen::INPUT_PRIVACY_VALUE_DISALLOW_ALL,
        );
        self.client
            .invoke(&crate::mtproto::tl_gen::build_account_setPrivacy(&key, &[&rule]))
            .await
            .map(|_| ())
    }

    pub async fn complete(
        &mut self,
        code: &str,
        identity: &RegistrationIdentity,
    ) -> Result<RegistrationOutcome, String> {
        validate_code(code)?;
        identity.validate()?;
        let sign_in = tl::build_auth_sign_in(&self.phone, &self.phone_code_hash, code);
        match super::diagnostics::observe("sign_in_invoke", self.client.invoke(&sign_in).await) {
            Ok(response) => match super::diagnostics::observe(
                "sign_in_parse",
                tl::parse_auth_authorization(&response),
            ) {
                Ok(_) => Ok(RegistrationOutcome::AlreadyRegistered),
                Err(error) if is_unoccupied(&error) => self.sign_up(identity).await,
                Err(error) => Err(error),
            },
            Err(error) if is_unoccupied(&error) => self.sign_up(identity).await,
            Err(error) => Err(error),
        }
    }

    pub async fn cancel(&mut self) -> Result<(), String> {
        let request = tl::build_auth_cancel_code(&self.phone, &self.phone_code_hash);
        let result = super::diagnostics::observe("cancel_code", self.client.invoke(&request).await);
        if let Ok(response) = &result {
            if let Ok(accepted) = super::diagnostics::observe(
                "cancel_code_parse",
                crate::mtproto::tl_gen::parse_auth_cancelCode(response),
            ) {
                dbg_log!(
                    "registrar::telegram stage=cancel_code dc={} accepted={}",
                    self.dc_id,
                    accepted
                );
            }
        }
        dbg_log!(
            "registrar::telegram stage=cancel_code dc={} transport_success={}",
            self.dc_id,
            result.is_ok()
        );
        result.map(|_| ())
    }

    pub async fn resend(&mut self) -> Result<SentCode, String> {
        let request = tl::build_auth_resend_code(&self.phone, &self.phone_code_hash);
        let response =
            super::diagnostics::observe("resend_code_invoke", self.client.invoke(&request).await)?;
        let sent_code =
            super::diagnostics::observe("resend_code_parse", tl::parse_auth_sent_code(&response))?;
        self.phone_code_hash = sent_code.phone_code_hash.clone();
        Ok(sent_code)
    }

    pub async fn request_login_email_verification(&mut self, email: &str) -> Result<(), String> {
        if email.trim().is_empty() || email.len() > 254 || email.contains(['\r', '\n', '\0']) {
            return Err("invalid login email".to_string());
        }
        let purpose = crate::mtproto::tl_gen::serialize_emailVerifyPurposeLoginSetup(
            &self.phone,
            &self.phone_code_hash,
        );
        let request = crate::mtproto::tl_gen::build_account_sendVerifyEmailCode(&purpose, email);
        let response = super::diagnostics::observe(
            "send_verify_email_code",
            self.client.invoke(&request).await,
        )?;
        super::diagnostics::observe(
            "send_verify_email_code_parse",
            crate::mtproto::tl_gen::parse_account_sendVerifyEmailCode(&response),
        )?;
        Ok(())
    }

    pub async fn confirm_login_email_verification(&mut self, code: &str) -> Result<SentCode, String> {
        if code.trim().is_empty() || code.len() > 64 || code.contains(['\r', '\n', '\0']) {
            return Err("invalid email verification code".to_string());
        }
        let purpose = crate::mtproto::tl_gen::serialize_emailVerifyPurposeLoginSetup(
            &self.phone,
            &self.phone_code_hash,
        );
        let verification = crate::mtproto::tl_gen::serialize_emailVerificationCode(code);
        let request = crate::mtproto::tl_gen::build_account_verifyEmail(&purpose, &verification);
        let response = super::diagnostics::observe("verify_email", self.client.invoke(&request).await)?;
        match super::diagnostics::observe(
            "verify_email_parse",
            crate::mtproto::tl_gen::parse_account_verifyEmail(&response),
        )? {
            crate::mtproto::tl_gen::TlAccountEmailVerified::Login { sent_code, .. } => {
                tl::parse_auth_sent_code_inner(&sent_code)
            }
            crate::mtproto::tl_gen::TlAccountEmailVerified::EmailVerified { .. } => {
                Err("Telegram did not return a login verification code".to_string())
            }
        }
    }

    async fn sign_up(
        &mut self,
        identity: &RegistrationIdentity,
    ) -> Result<RegistrationOutcome, String> {
        let request = tl::build_auth_sign_up(
            &self.phone,
            &self.phone_code_hash,
            &identity.first_name,
            &identity.last_name,
        );
        let result = match super::diagnostics::observe("sign_up_invoke", self.client.invoke(&request).await) {
            Ok(response) => super::diagnostics::observe(
                "sign_up_parse",
                tl::parse_auth_authorization(&response),
            ),
            Err(error) => Err(error),
        };
        let user = match result {
            Ok(user) => user,
            Err(error) => self.recover_registered_user(&error).await?,
        };
        self.server_registered = true;
        super::diagnostics::observe("save_account", self.save(&user))
            .map(|account_id| RegistrationOutcome::Registered { account_id })
    }

    async fn recover_registered_user(&mut self, original_error: &str) -> Result<AuthorizedUser, String> {
        for _ in 0..3 {
            let result = self.client.get_me(
                self.profile.api_id,
                &self.profile.device_model,
                &self.profile.system_version,
                &self.profile.app_version,
                &self.profile.system_lang_code,
                &self.profile.lang_code,
            ).await;
            match result {
                Ok(user) if is_matching_self_user(user.id, &user.phone, &self.phone) => {
                        dbg_log!("registrar::sign_up recovered authorized session after ambiguous response");
                        return Ok(AuthorizedUser {
                            user_id: user.id,
                            phone: user.phone,
                            first_name: user.first_name,
                            last_name: user.last_name,
                            username: user.username,
                            is_premium: user.premium,
                        });
                }
                Ok(_) => break,
                Err(error) if !crate::mtproto::client::is_network_error(&error) => break,
                Err(_) => tokio::time::sleep(Duration::from_secs(2)).await,
            }
        }
        Err(original_error.to_string())
    }

    fn save(&self, user: &AuthorizedUser) -> Result<String, String> {
        let device = DeviceInfo {
            device: self.profile.device_model.clone(),
            sdk: self.profile.system_version.clone(),
            app_version: self.profile.app_version.clone(),
        };
        let storage = self.storage();
        let account_id = crate::accounts::auth_login::save_authorized_account_to(
            &storage,
            user,
            self.dc_id,
            &self.phone,
            self.proxy.as_ref(),
            &device,
            self.profile.api_id,
            &self.profile.api_hash,
            self.client.auth_key(),
            "",
        )?;
        let path = storage.json_path(&account_id);
        let mut json = AccountJson::from_file(&path)?;
        json.lang_pack = self.profile.lang_code.clone();
        json.system_lang_pack = self.profile.system_lang_code.clone();
        json.to_file(&path)?;
        Ok(account_id)
    }
}

async fn start_on_dc(
    dc_id: i32,
    phone: &str,
    profile: &RegistrationProfile,
    proxy: Option<&ProxyConfig>,
    settings: &AuthCodeSettings,
    antisafety: Option<&AntiSafetyConfig>,
    antisafety_proxy: Option<&str>,
    cancellation: Option<&AtomicBool>,
    log_target: Option<&tauri::AppHandle>,
    log_prefix: &str,
) -> Result<(MtpClient, SentCode), String> {
    let address = dc_address(dc_id)?;
    dbg_log!(
        "registrar::send_code dc={} api_id={} proxy={} started",
        dc_id,
        profile.api_id,
        proxy.is_some()
    );
    dbg_log!("registrar::send_code settings allow_app_hash={} allow_missed_call={} firebase_token_present={} sandbox={:?}", settings.allow_app_hash, settings.allow_missed_call, settings.firebase_token.as_ref().is_some_and(|token| !token.is_empty()), settings.firebase_app_sandbox);
    let (transport, dh) = connect_and_auth(address, proxy, cancellation).await?;
    let mut client = MtpClient::from_transport(transport, dh.auth_key, dh.server_salt, address);
    client.set_proxy(proxy.cloned());
    if let Some(handle) = log_target {
        client.set_log_target("registrar-log", handle.clone());
        client.set_log_prefix(log_prefix);
    }
    let inner =
        tl::build_auth_send_code_with_settings(phone, profile.api_id, &profile.api_hash, settings);
    let response = match invoke_send_code(&mut client, &inner, profile).await {
        Ok(response) => response,
        Err(error) => {
            let Some(challenge) = parse_recaptcha_challenge(&error) else {
                return Err(error);
            };
            let config = antisafety.ok_or("AntiSafety credentials are required for reCAPTCHA")?;
            dbg_log!(
                "registrar::send_code dc={} recaptcha_challenge_received",
                dc_id
            );
            let token = solve_recaptcha(config, challenge, antisafety_proxy, cancellation).await?;
            let protected_inner = tl::wrap_with_recaptcha(&token, &inner);
            super::diagnostics::observe(
                "send_code_recaptcha_invoke",
                invoke_send_code(&mut client, &protected_inner, profile).await,
            )?
        }
    };
    if let Some(bytes) = response.get(..4) {
        dbg_log!(
            "registrar::send_code dc={} response_bytes={} constructor={:#x}",
            dc_id,
            response.len(),
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        );
    }
    let sent_code =
        super::diagnostics::observe("send_code_parse", tl::parse_auth_sent_code(&response))?;
    dbg_log!(
        "registrar::send_code dc={} completed delivery={}",
        dc_id,
        sent_code.code_type
    );
    Ok((client, sent_code))
}

async fn connect_and_auth(
    address: &str,
    proxy: Option<&ProxyConfig>,
    cancellation: Option<&AtomicBool>,
) -> Result<(MtpTransport, crate::mtproto::auth::DhResult), String> {
    for attempt in 0..3 {
        if is_cancelled(cancellation) {
            return Err("stopped".to_string());
        }
        let result = async {
            let mut transport = super::diagnostics::observe(
                "connect",
                MtpTransport::connect(address, proxy).await,
            )?;
            let dh = super::diagnostics::observe("dh", perform_dh(&mut transport).await)?;
            Ok::<_, String>((transport, dh))
        }
        .await;
        match result {
            Ok(value) => return Ok(value),
            Err(error) if attempt < 2 && crate::mtproto::is_network_error(&error) => {
                tokio::time::sleep(Duration::from_secs(attempt + 1)).await;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("bounded connection attempts return a result")
}

async fn invoke_send_code(
    client: &mut MtpClient,
    inner: &[u8],
    profile: &RegistrationProfile,
) -> Result<Vec<u8>, String> {
    let request = tl::wrap_init_connection(
        inner,
        profile.api_id,
        &profile.device_model,
        &profile.system_version,
        &profile.app_version,
        &profile.system_lang_code,
        &profile.lang_code,
    );
    super::diagnostics::observe("send_code_invoke", client.invoke(&request).await)
}

async fn solve_recaptcha(
    config: &AntiSafetyConfig,
    challenge: RecaptchaChallenge,
    http_proxy: Option<&str>,
    cancellation: Option<&AtomicBool>,
) -> Result<String, String> {
    if is_cancelled(cancellation) {
        return Err("stopped".to_string());
    }
    let client = AntiSafetyClient::new_with_proxy(config.clone(), http_proxy)?;
    let mut state = RecaptchaState::Creating;
    dbg_log!("registrar::recaptcha state={state:?}");
    let task_client = client.clone();
    let task = tokio::task::spawn_blocking(move || {
        task_client.create_recaptcha(&challenge.action, &challenge.app_key, true)
    })
    .await
    .map_err(|_| "reCAPTCHA worker failed".to_string())??;

    state = RecaptchaState::Waiting;
    dbg_log!("registrar::recaptcha state={state:?}");
    let deadline = tokio::time::Instant::now() + RECAPTCHA_TIMEOUT;
    loop {
        if is_cancelled(cancellation) {
            state = RecaptchaState::Cancelled;
            dbg_log!("registrar::recaptcha state={state:?}");
            return Err("stopped".to_string());
        }
        let task_client = client.clone();
        let id = task.id.clone();
        let result = tokio::task::spawn_blocking(move || task_client.get_recaptcha(&id))
            .await
            .map_err(|_| "reCAPTCHA worker failed".to_string())?;
        if let Ok(token) = result {
            state = RecaptchaState::Ready;
            dbg_log!("registrar::recaptcha state={state:?}");
            return Ok(token.token);
        }
        if tokio::time::Instant::now() >= deadline {
            state = RecaptchaState::TimedOut;
            dbg_log!("registrar::recaptcha state={state:?}");
            return Err("reCAPTCHA token timeout".to_string());
        }
        wait_for_recaptcha_poll(cancellation).await?;
    }
}

async fn wait_for_recaptcha_poll(cancellation: Option<&AtomicBool>) -> Result<(), String> {
    for _ in 0..RECAPTCHA_POLL_INTERVAL.as_secs() {
        if is_cancelled(cancellation) {
            return Err("stopped".to_string());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Ok(())
}

fn is_cancelled(cancellation: Option<&AtomicBool>) -> bool {
    cancellation.is_some_and(|token| !token.load(Ordering::Relaxed))
}

fn parse_recaptcha_challenge(error: &str) -> Option<RecaptchaChallenge> {
    let payload = error.split_once("RECAPTCHA_CHECK_")?.1;
    let (action, app_key) = payload.split_once("__")?;
    let valid = |value: &str, max_length| {
        !value.is_empty() && value.len() <= max_length && !value.contains(['\r', '\n', '\0'])
    };
    (valid(action, 256) && valid(app_key, 512)).then(|| RecaptchaChallenge {
        action: action.to_string(),
        app_key: app_key.to_string(),
    })
}

fn dc_address(dc_id: i32) -> Result<&'static str, String> {
    match dc_id {
        1 => Ok("149.154.175.53:443"),
        2 => Ok("149.154.167.51:443"),
        3 => Ok("149.154.175.100:443"),
        4 => Ok("149.154.167.91:443"),
        5 => Ok("91.108.56.130:443"),
        _ => Err(format!("unsupported Telegram data center: {dc_id}")),
    }
}

fn phone_migrate_dc(error: &str) -> Option<i32> {
    let marker = if error.contains("PHONE_MIGRATE_") {
        "PHONE_MIGRATE_"
    } else {
        "NETWORK_MIGRATE_"
    };
    let start = error.find(marker)? + marker.len();
    let dc_id: i32 = error[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    (1..=5).contains(&dc_id).then_some(dc_id)
}

fn normalize_phone(phone: &str) -> Result<String, String> {
    let phone: String = phone.chars().filter(char::is_ascii_digit).collect();
    if !(5..=15).contains(&phone.len()) {
        return Err("phone number must contain 5 to 15 digits".to_string());
    }
    Ok(phone)
}

fn is_matching_self_user(user_id: i64, phone: &str, expected_phone: &str) -> bool {
    user_id > 0 && normalize_phone(phone).ok().as_deref() == Some(expected_phone)
}

fn validate_code(code: &str) -> Result<(), String> {
    if code.trim().is_empty() || code.len() > 256 || code.chars().any(char::is_control) {
        return Err("invalid verification code".to_string());
    }
    Ok(())
}

fn is_unoccupied(error: &str) -> bool {
    error.contains("PHONE_NUMBER_UNOCCUPIED") || error == "SIGN_UP_REQUIRED"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_phone_migration_target() {
        assert_eq!(phone_migrate_dc("rpc error 303: PHONE_MIGRATE_4"), Some(4));
        assert_eq!(phone_migrate_dc("PHONE_MIGRATE_9"), None);
    }

    #[test]
    fn validates_phone_and_code() {
        assert_eq!(normalize_phone("+1 (202) 555-0100").unwrap(), "12025550100");
        assert!(validate_code("12\n34").is_err());
    }

    #[test]
    fn accepts_signup_required_and_network_migration() {
        assert!(is_unoccupied("SIGN_UP_REQUIRED"));
        assert!(!is_unoccupied("PHONE_CODE_INVALID"));
        assert_eq!(phone_migrate_dc("NETWORK_MIGRATE_3"), Some(3));
    }

    #[test]
    fn only_recovers_registration_for_the_expected_phone() {
        assert!(is_matching_self_user(42, "+1 (202) 555-0100", "12025550100"));
        assert!(!is_matching_self_user(0, "+1 (202) 555-0100", "12025550100"));
        assert!(!is_matching_self_user(42, "+1 (202) 555-0101", "12025550100"));
    }

    #[test]
    fn parses_recaptcha_challenge_without_retaining_rpc_prefix() {
        assert_eq!(
            parse_recaptcha_challenge("RPC 400: RECAPTCHA_CHECK_login__site-key"),
            Some(RecaptchaChallenge {
                action: "login".to_string(),
                app_key: "site-key".to_string(),
            })
        );
        assert_eq!(parse_recaptcha_challenge("RECAPTCHA_CHECK_login"), None);
        assert_eq!(
            parse_recaptcha_challenge("RECAPTCHA_CHECK___site-key"),
            None
        );
    }
}
