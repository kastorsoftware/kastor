// account metadata (.json companion file) + re-export TelethonSession from converter

use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;

// re-export session format from converter for backward compatibility
pub use crate::converter::telethon::TelethonSession;

fn deserialize_nullable_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where D: Deserializer<'de> {
    let opt = Option::<String>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

fn deserialize_nullable_i32<'de, D>(deserializer: D) -> Result<i32, D::Error>
where D: Deserializer<'de> {
    let opt = Option::<i32>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

// companion .json file with api credentials and validation state
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountJson {
    #[serde(default, deserialize_with = "deserialize_nullable_i32")]
    pub app_id: i32,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub app_hash: String,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub sdk: String,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub device: String,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub app_version: String,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub lang_pack: String,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub system_lang_pack: String,
    #[serde(default, alias = "2fa", deserialize_with = "deserialize_nullable_string")]
    #[serde(rename = "twoFA")]
    pub two_fa: String,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub authkey: String,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub mail: String,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub mail2fa: String,
    #[serde(default)]
    pub validated: bool,
    #[serde(default)]
    pub valid: bool,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub phone: String,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub first_name: String,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub last_name: String,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub username: String,
    #[serde(default, alias = "id")]
    pub user_id: i64,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub status: String,
    #[serde(default)]
    pub last_check_time: i64,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub last_connect_date: String,
    #[serde(default)]
    pub proxy: Option<String>,
    #[serde(default)]
    pub register_time: i64,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub spamblock: String,
    #[serde(default)]
    pub is_premium: bool,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub premium_expiry: String,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub role: String,
}

impl AccountJson {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        dbg_log!("AccountJson::from_file {:?}", path);
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::i18n::t_with("session_read_json_error", &[("error", &e.to_string())]))?;
        let json: Self = serde_json::from_str(&content)
            .map_err(|e| crate::i18n::t_with("session_parse_json_error", &[("error", &e.to_string())]))?;
        dbg_log!("AccountJson: app_id={} phone='{}' status='{}'", json.app_id, json.phone, json.status);
        Ok(json)
    }

    pub fn to_file(&self, path: &Path) -> Result<(), String> {
        dbg_log!("AccountJson::to_file {:?} status='{}'", path, self.status);
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| crate::i18n::t_with("session_serialize_error", &[("error", &e.to_string())]))?;
        std::fs::write(path, content)
            .map_err(|e| crate::i18n::t_with("session_write_json_error", &[("error", &e.to_string())]))?;
        if let Some(handle) = crate::get_app_handle() {
            use tauri::Emitter;
            let _ = handle.emit("accounts-changed", ());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_metadata_is_backward_compatible_and_roundtrips_credentials() {
        let legacy: AccountJson = serde_json::from_str("{}").unwrap();
        assert!(legacy.authkey.is_empty() && legacy.mail.is_empty() && legacy.mail2fa.is_empty());
        let nullable: AccountJson = serde_json::from_str(r#"{"authkey":null,"mail":null,"mail2fa":null}"#).unwrap();
        assert!(nullable.authkey.is_empty() && nullable.mail.is_empty() && nullable.mail2fa.is_empty());
        let json = AccountJson {
            mail: "login@example.test:dummy:password".into(),
            mail2fa: "recovery@example.test:dummy".into(),
            ..Default::default()
        };
        let serialized = serde_json::to_string(&json).unwrap();
        let restored: AccountJson = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.mail, json.mail);
        assert_eq!(restored.mail2fa, json.mail2fa);
        assert!(restored.authkey.is_empty());
        assert!(serialized.contains("\"authkey\":\"\""));
    }
}
