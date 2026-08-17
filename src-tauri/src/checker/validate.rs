// account validation via MTProto

use crate::mtproto::client::MtpClient;
use crate::proxy::ProxyConfig;
use crate::accounts::session::{AccountJson, TelethonSession};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ValidateResult {
    pub valid: bool,
    pub unreachable: bool,
    pub error: Option<String>,
    pub user_id: Option<i64>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub phone: Option<String>,
    pub username: Option<String>,
    pub premium: Option<bool>,
}

// returns (result, optional reusable client for further operations)
pub async fn validate_account(
    session: &TelethonSession,
    json: &AccountJson,
    proxy: Option<&ProxyConfig>,
) -> (ValidateResult, Option<MtpClient>) {
    let addr = format!("{}:{}", session.server_address, session.port);
    dbg_log!("validate: addr={} dc_id={}", addr, session.dc_id);

    if session.auth_key.len() != 256 {
        return (error_result(&crate::i18n::t("checker_validate_authkey_size"), false), None);
    }

    let mut key = [0u8; 256];
    key.copy_from_slice(&session.auth_key);

    let mut client = match MtpClient::connect(&addr, &key, proxy).await {
        Ok(c) => c,
        Err(e) => {
            return (error_result(&crate::i18n::t_with("checker_validate_connect", &[("error", &e)]), true), None);
        }
    };

    let app_id = if json.app_id == 0 { 2040 } else { json.app_id };
    let dev = if json.device.is_empty() || json.sdk.is_empty() {
        crate::accounts::devices::generate_random_device()
    } else {
        crate::accounts::devices::DeviceInfo {
            device: json.device.clone(),
            sdk: json.sdk.clone(),
            app_version: if json.app_version.is_empty() { "10.14.5".to_string() } else { json.app_version.clone() },
        }
    };
    let sys_lang = if json.system_lang_pack.is_empty() { "en" } else { &json.system_lang_pack };
    let lang = if json.lang_pack.is_empty() { "en" } else { &json.lang_pack };

    match client.get_me(app_id, &dev.device, &dev.sdk, &dev.app_version, sys_lang, lang).await {
        Ok(user) => {
            dbg_log!("validate: OK id={} phone='{}' premium={}", user.id, user.phone, user.premium);
            (ValidateResult {
                valid: true, unreachable: false, error: None,
                user_id: Some(user.id),
                first_name: Some(user.first_name),
                last_name: Some(user.last_name),
                phone: Some(user.phone),
                username: Some(user.username),
                premium: Some(user.premium),
            }, Some(client))
        }
        Err(e) => {
            dbg_log!("validate: error: {}", e);
            let invalid_patterns = [
                "-404", "AUTH_KEY_UNREGISTERED", "AUTH_KEY_INVALID",
                "AUTH_KEY_PERM_EMPTY", "SESSION_REVOKED", "SESSION_EXPIRED",
                "USER_DEACTIVATED", "USER_DEACTIVATED_BAN", "AUTH_KEY_DUPLICATED",
                "INPUT_USER_DEACTIVATED",
            ];
            let is_invalid = invalid_patterns.iter().any(|p| e.contains(p));
            if is_invalid {
                (error_result(&e, false), None)
            } else {
                (error_result(&e, true), None)
            }
        }
    }
}

fn error_result(msg: &str, unreachable: bool) -> ValidateResult {
    ValidateResult {
        valid: false, unreachable,
        error: Some(msg.to_string()),
        user_id: None, first_name: None,
        last_name: None, phone: None, username: None,
        premium: None,
    }
}
