use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const BASE_URL: &str = "https://antisafety.net/api/tasks";

#[derive(Debug, Clone, Deserialize)]
pub struct AntiSafetyConfig {
    pub api_key: String,
    pub aid: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AntiSafetyBalance {
    pub balance: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AntiSafetyTask {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AntiSafetyToken {
    pub token: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AntiSafetyPushToken {
    pub token: String,
    pub token_secret: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AntiSafetyEmailTask {
    pub id: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AntiSafetyGoogleAuthTask {
    pub id: String,
    pub email: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AntiSafetyLoginTask {
    GoogleAuth {
        id: String,
        email: String,
        token: String,
    },
    Email {
        id: String,
        email: String,
    },
}

#[derive(Clone)]
pub struct AntiSafetyClient {
    api_key: String,
    aid: String,
    agent: ureq::Agent,
}

impl AntiSafetyClient {
    pub fn new(config: AntiSafetyConfig) -> Result<Self, String> {
        Self::new_with_proxy(config, None)
    }

    pub fn new_with_proxy(config: AntiSafetyConfig, http_proxy: Option<&str>) -> Result<Self, String> {
        validate_secret(&config.api_key, "API key")?;
        validate_value(&config.aid, "AID", 128)?;
        let proxy = http_proxy
            .map(|value| ureq::Proxy::new(value).map_err(|_| "invalid AntiSafety proxy".to_string()))
            .transpose()?;
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .proxy(proxy)
            .build()
            .new_agent();
        Ok(Self {
            api_key: config.api_key,
            aid: config.aid,
            agent,
        })
    }

    pub fn balance(&self) -> Result<AntiSafetyBalance, String> {
        let response = self.request("balance", &[])?;
        Ok(AntiSafetyBalance {
            balance: response.required_i64("balance")?,
            currency: response.required_string("currency")?,
        })
    }

    pub fn create_play_integrity(
        &self,
        standard: bool,
        nonce: &str,
        version: i32,
    ) -> Result<AntiSafetyTask, String> {
        validate_value(nonce, "nonce", 32_768)?;
        if version <= 0 {
            return Err("version code must be positive".to_string());
        }
        self.create_task(
            "play-integrity/create",
            &[
                ("type", if standard { "standard" } else { "classic" }.to_string()),
                ("nonce", nonce.to_string()),
                ("version", version.to_string()),
            ],
        )
    }

    pub fn get_play_integrity(&self, id: &str) -> Result<AntiSafetyToken, String> {
        self.get_token("play-integrity/get", &[("id", id.to_string())])
    }

    pub fn create_android_push(&self) -> Result<AntiSafetyTask, String> {
        self.create_task("android-push/create", &[])
    }

    pub fn get_android_push(&self, id: &str) -> Result<AntiSafetyPushToken, String> {
        let response = self.request("android-push/get", &[("id", id.to_string())])?;
        Ok(AntiSafetyPushToken {
            token: response.required_string("token")?,
            token_secret: response.required_string("token_secret")?,
        })
    }

    pub fn create_email(&self, premium: bool) -> Result<AntiSafetyEmailTask, String> {
        let parameters = if premium {
            vec![("type", "premium".to_string())]
        } else {
            Vec::new()
        };
        let response = self.request("email/create", &parameters)?;
        Ok(AntiSafetyEmailTask {
            id: response.required_string("id")?,
            email: response.required_string("email")?,
        })
    }

    pub fn get_email(&self, id: &str, premium: bool) -> Result<String, String> {
        let mut parameters = vec![("id", id.to_string())];
        if premium {
            parameters.push(("type", "premium".to_string()));
        }
        self.request("email/get", &parameters)?.required_code("result")
    }

    pub fn create_google_auth(&self) -> Result<AntiSafetyGoogleAuthTask, String> {
        let response = self.request("gauth/create", &[])?;
        Ok(AntiSafetyGoogleAuthTask {
            id: response.required_string("id")?,
            email: response.required_string("email")?,
            token: response.required_string("token")?,
        })
    }

    pub fn create_login(&self) -> Result<AntiSafetyLoginTask, String> {
        let response = self.request("login/create", &[])?;
        let id = response.required_string("id")?;
        let email = response.required_string("email")?;
        match response.required_string("type")?.as_str() {
            "gauth" => Ok(AntiSafetyLoginTask::GoogleAuth {
                id,
                email,
                token: response.required_string("token")?,
            }),
            "email" => Ok(AntiSafetyLoginTask::Email { id, email }),
            _ => Err("AntiSafety returned an unknown login task type".to_string()),
        }
    }

    pub fn create_recaptcha(
        &self,
        action: &str,
        app_key: &str,
        high_score: bool,
    ) -> Result<AntiSafetyTask, String> {
        validate_value(action, "action", 256)?;
        validate_value(app_key, "app key", 512)?;
        self.create_task(
            "recaptcha/create",
            &[
                ("action", action.to_string()),
                ("app_key", app_key.to_string()),
                ("score", if high_score { "high" } else { "low" }.to_string()),
            ],
        )
    }

    pub fn get_recaptcha(&self, id: &str) -> Result<AntiSafetyToken, String> {
        self.get_token("recaptcha/get", &[("id", id.to_string())])
    }

    fn create_task(&self, path: &str, parameters: &[(&str, String)]) -> Result<AntiSafetyTask, String> {
        Ok(AntiSafetyTask {
            id: self.request(path, parameters)?.required_string("id")?,
        })
    }

    fn get_token(&self, path: &str, parameters: &[(&str, String)]) -> Result<AntiSafetyToken, String> {
        Ok(AntiSafetyToken {
            token: self.request(path, parameters)?.required_string("token")?,
        })
    }

    fn request(&self, path: &str, parameters: &[(&str, String)]) -> Result<ApiResponse, String> {
        let url = format!("{BASE_URL}/{path}");
        let mut request = self
            .agent
            .get(&url)
            .query("api_key", &self.api_key)
            .query("aid", &self.aid)
            .header("Accept", "application/json");
        for (key, value) in parameters {
            request = request.query(key, value);
        }
        let response = request
            .call()
            .map_err(|error| format!("AntiSafety request failed: {error}"))?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_string()
            .map_err(|error| format!("read AntiSafety response: {error}"))?;
        let response: ApiResponse = serde_json::from_str(&body)
            .map_err(|error| format!("parse AntiSafety response: {error}"))?;
        if !(200..300).contains(&status) {
            return Err(response
                .error_code()
                .map(|code| format!("ANTI_SAFETY_HTTP_{status}_{code}"))
                .unwrap_or_else(|| format!("ANTI_SAFETY_HTTP_{status}")));
        }
        if response.status != "ok" {
            return Err(response
                .error_code()
                .map(|code| format!("ANTI_SAFETY_{code}"))
                .unwrap_or_else(|| "AntiSafety rejected the request".to_string()));
        }
        Ok(response)
    }
}

#[derive(Deserialize)]
struct ApiResponse {
    status: String,
    #[serde(default)]
    balance: Option<i64>,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    token_secret: Option<String>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    detail: Option<String>,
}

impl ApiResponse {
    fn error_code(&self) -> Option<String> {
        self.error
            .as_deref()
            .or(self.message.as_deref())
            .or(self.detail.as_deref())
            .map(str::trim)
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 64
                    && value
                        .bytes()
                        .all(|byte| {
                            byte.is_ascii_alphanumeric()
                                || byte == b'_'
                                || byte == b'-'
                                || byte == b' '
                        })
            })
            .map(|value| {
                value
                    .split(|character: char| character == '_' || character == '-' || character.is_ascii_whitespace())
                    .filter(|part| !part.is_empty())
                    .map(str::to_ascii_uppercase)
                    .collect::<Vec<_>>()
                    .join("_")
            })
    }
}

impl ApiResponse {
    fn required_i64(&self, name: &str) -> Result<i64, String> {
        self.balance
            .ok_or_else(|| format!("AntiSafety response is missing {name}"))
    }

    fn required_string(&self, name: &str) -> Result<String, String> {
        let value = match name {
            "currency" => self.currency.clone(),
            "id" => self.id.as_ref().and_then(value_to_string),
            "email" => self.email.clone(),
            "token" => self.token.clone(),
            "token_secret" => self.token_secret.clone(),
            "type" => self.r#type.clone(),
            _ => None,
        };
        value.ok_or_else(|| format!("AntiSafety response is missing {name}"))
    }

    fn required_code(&self, name: &str) -> Result<String, String> {
        if name != "result" {
            return Err(format!("unsupported AntiSafety result field: {name}"));
        }
        self.result
            .as_ref()
            .and_then(value_to_string)
            .ok_or_else(|| "AntiSafety response is missing result".to_string())
    }
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn validate_secret(value: &str, name: &str) -> Result<(), String> {
    validate_value(value, name, 512)
}

fn validate_value(value: &str, name: &str, max_length: usize) -> Result<(), String> {
    if value.trim().is_empty()
        || value.len() > max_length
        || value.contains(['\r', '\n', '\0'])
    {
        return Err(format!("invalid AntiSafety {name}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_numeric_verification_code() {
        assert_eq!(value_to_string(&serde_json::json!(12345)), Some("12345".to_string()));
    }

    #[test]
    fn rejects_unsafe_config_values() {
        assert!(AntiSafetyClient::new(AntiSafetyConfig {
            api_key: "key\nvalue".to_string(),
            aid: "aid".to_string(),
        })
        .is_err());
    }

    #[test]
    fn exposes_only_safe_error_codes() {
        let safe: ApiResponse = serde_json::from_str(r#"{"status":"error","error":"email_out_of_stock"}"#).unwrap();
        assert_eq!(safe.error_code().as_deref(), Some("EMAIL_OUT_OF_STOCK"));

        let unsafe_value: ApiResponse = serde_json::from_str(
            r#"{"status":"error","message":"request failed: api_key=secret"}"#,
        )
        .unwrap();
        assert_eq!(unsafe_value.error_code(), None);
    }
}
