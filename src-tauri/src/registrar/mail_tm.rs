use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

const BASE_URL: &str = "https://api.mail.tm";
const REQUEST_INTERVAL: Duration = Duration::from_millis(150);
const RESPONSE_LIMIT: u64 = 262_144;

#[derive(Clone)]
pub(crate) struct MailTmClient {
    agent: ureq::Agent,
    next_request: Arc<Mutex<Instant>>,
}

#[derive(Clone)]
pub(crate) struct MailTmMailbox {
    pub address: String,
    password: String,
    token: String,
    client: MailTmClient,
}

#[derive(Deserialize)]
struct Collection<T> {
    #[serde(rename = "hydra:member")]
    members: Vec<T>,
}

#[derive(Deserialize)]
struct Domain {
    domain: String,
    #[serde(rename = "isActive")]
    active: bool,
    #[serde(rename = "isPrivate")]
    private: bool,
}

#[derive(Deserialize)]
struct Token {
    token: String,
}

#[derive(Deserialize)]
struct Sender {
    address: String,
}

#[derive(Deserialize)]
struct Message {
    id: String,
    from: Sender,
    subject: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    html: Vec<String>,
}

impl MailTmClient {
    pub fn new() -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .https_only(true)
                .http_status_as_error(false)
                .max_redirects(0)
                .timeout_global(Some(Duration::from_secs(15)))
                .build()
                .new_agent(),
            next_request: Arc::new(Mutex::new(Instant::now())),
        }
    }

    fn request<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        token: Option<&str>,
        payload: Option<&Value>,
    ) -> Result<T, String> {
        let mut next = self
            .next_request
            .lock()
            .map_err(|_| "mail_tm_unavailable")?;
        if let Some(delay) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(delay);
        }
        *next = Instant::now() + REQUEST_INTERVAL;
        drop(next);
        let url = format!("{BASE_URL}{path}");
        let response = if let Some(payload) = payload {
            self.agent
                .post(&url)
                .header("Accept", "application/ld+json")
                .send_json(payload)
        } else {
            let mut request = self.agent.get(&url).header("Accept", "application/ld+json");
            if let Some(token) = token {
                request = request.header("Authorization", &format!("Bearer {token}"));
            }
            request.call()
        };
        let mut response = response.map_err(|_| "mail_tm_unavailable")?;
        if response.status().as_u16() == 429 {
            return Err("mail_tm_rate_limited".into());
        }
        if !response.status().is_success() {
            return Err("mail_tm_unavailable".into());
        }
        response
            .body_mut()
            .with_config()
            .limit(RESPONSE_LIMIT)
            .read_json()
            .map_err(|_| "mail_tm_invalid_response".into())
    }

    pub fn create_mailbox(&self) -> Result<MailTmMailbox, String> {
        let domains: Collection<Domain> = self.request("/domains", None, None)?;
        let domain = domains
            .members
            .into_iter()
            .find(|domain| domain.active && !domain.private && valid_domain(&domain.domain))
            .ok_or("mail_tm_no_domains")?;
        let address = format!("{}@{}", uuid::Uuid::new_v4().simple(), domain.domain);
        let password = uuid::Uuid::new_v4().simple().to_string();
        let credentials = serde_json::json!({"address": address, "password": password});
        let _: Value = self.request("/accounts", None, Some(&credentials))?;
        let token: Token = self.request("/token", None, Some(&credentials))?;
        if token.token.is_empty() || token.token.contains(['\r', '\n']) {
            return Err("mail_tm_invalid_response".into());
        }
        Ok(MailTmMailbox {
            address,
            password,
            token: token.token,
            client: self.clone(),
        })
    }
}

impl MailTmMailbox {
    pub fn credentials(&self) -> String {
        format!("{}:{}", self.address, self.password)
    }

    pub fn code_after(&self, seen: &mut HashSet<String>) -> Result<Option<String>, String> {
        self.code_after_with_length(seen, None)
    }

    pub fn code_after_with_length(
        &self,
        seen: &mut HashSet<String>,
        code_length: Option<usize>,
    ) -> Result<Option<String>, String> {
        let messages: Collection<Message> =
            self.client.request("/messages", Some(&self.token), None)?;
        for message in messages.members.into_iter().take(20) {
            if seen.contains(&message.id) || !valid_id(&message.id) {
                continue;
            }
            if !message
                .from
                .address
                .to_ascii_lowercase()
                .ends_with("@telegram.org")
            {
                seen.insert(message.id);
                continue;
            }
            let full: Message = self.client.request(
                &format!("/messages/{}", message.id),
                Some(&self.token),
                None,
            )?;
            let code = message_code(&full, code_length);
            seen.insert(message.id);
            if code.is_some() {
                return Ok(code);
            }
        }
        Ok(None)
    }
}

fn message_code(message: &Message, code_length: Option<usize>) -> Option<String> {
    if !message
        .from
        .address
        .to_ascii_lowercase()
        .ends_with("@telegram.org")
    {
        return None;
    }
    let extract = |text: &str| {
        if code_length.is_some() {
            super::email::extract_code_with_length(text, code_length)
        } else {
            super::email::extract_code(text)
        }
    };
    extract(&message.subject)
        .or_else(|| extract(&message.text))
        .or_else(|| message.html.iter().find_map(|html| extract(html)))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn valid_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.contains('.')
        && domain
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_tm_message_parser_checks_sender_and_handles_text_and_html() {
        let mut message = Message {
            id: "abc123".into(),
            from: Sender {
                address: "recovery@telegram.org".into(),
            },
            subject: "Telegram".into(),
            text: "Ваш код: 598437. Подтвердите адрес электронной почты.".into(),
            html: Vec::new(),
        };
        assert_eq!(message_code(&message, Some(6)), Some("598437".into()));
        message.text.clear();
        message.html = vec!["<p>Your code: <b>835305</b></p>".into()];
        assert_eq!(message_code(&message, None), Some("835305".into()));
        message.from.address = "recovery@telegram.org.attacker.test".into();
        assert_eq!(message_code(&message, None), None);
    }

    #[test]
    fn mail_tm_ids_and_domains_cannot_inject_urls_or_headers() {
        for id in ["abc123", "a-b-123"] {
            assert!(valid_id(id));
        }
        for id in [
            "",
            "../token",
            "https://example.test",
            "id?token=x",
            "id\r\n",
        ] {
            assert!(!valid_id(id));
        }
        assert!(valid_domain("example.test"));
        for domain in [
            "",
            "example.test/path",
            "a@example.test",
            "example.test\r\n",
        ] {
            assert!(!valid_domain(domain));
        }
    }

    #[test]
    fn mail_tm_api_collections_are_decoded_from_documented_jsonld() {
        let domains: Collection<Domain> = serde_json::from_str(
            r#"{"hydra:member":[{"domain":"example.test","isActive":true,"isPrivate":false}]}"#,
        )
        .unwrap();
        assert!(domains.members[0].active && !domains.members[0].private);
        let messages: Collection<Message> = serde_json::from_str(r#"{"hydra:member":[{"id":"abc123","from":{"address":"noreply@telegram.org"},"subject":"Your code: 598437"}]}"#).unwrap();
        assert_eq!(
            message_code(&messages.members[0], Some(6)),
            Some("598437".into())
        );
    }
}
