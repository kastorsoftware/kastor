use std::collections::HashSet;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use imap::Session;
use mailparse::{MailAddr, MailHeaderMap};
use native_tls::{TlsConnector, TlsStream};
use serde::Deserialize;

const IMAP_PORT: u16 = 993;
const SOCKET_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_ACCOUNTS: usize = 100_000;
const MAX_MESSAGE_BYTES: usize = 65_536;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmailSource {
    #[default]
    AntiSafety,
    Firstmail,
    Gmx,
}

impl EmailSource {
    fn host(self, address: &str) -> Option<&'static str> {
        match self {
            Self::AntiSafety => None,
            Self::Firstmail => Some("imap.firstmail.ltd"),
            Self::Gmx => gmx_host(address),
        }
    }
}

fn gmx_host(address: &str) -> Option<&'static str> {
    let address = address.to_ascii_lowercase();
    if address.ends_with("@gmx.de") {
        Some("imap.gmx.net")
    } else if address.ends_with("@gmx.com") {
        Some("imap.gmx.com")
    } else {
        None
    }
}

#[derive(Clone)]
pub struct MailboxAccount {
    pub address: String,
    password: String,
    source: EmailSource,
}

impl MailboxAccount {
    pub fn load_file(path: &Path, source: EmailSource) -> Result<Vec<Self>, String> {
        if source == EmailSource::AntiSafety {
            return Err("email file is not used with AntiSafety".into());
        }
        let metadata = std::fs::metadata(path).map_err(|_| "registrar.emailFileRequired")?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err("registrar.emailFileInvalid".into());
        }
        let content = std::fs::read_to_string(path).map_err(|_| "registrar.emailFileInvalid")?;
        parse_accounts(&content, source)
    }

    pub fn latest_uid(&self) -> Result<u32, String> {
        let mut session = self.connect()?;
        let result = session
            .uid_search("ALL")
            .map(|uids| uids.into_iter().max().unwrap_or(0))
            .map_err(|_| "IMAP mailbox search failed".to_string());
        let _ = session.logout();
        result
    }

    pub fn code_after(
        &self,
        baseline: u32,
        seen: &mut HashSet<u32>,
    ) -> Result<Option<String>, String> {
        let mut session = self.connect()?;
        let result = (|| {
            let mut uids: Vec<u32> = session
                .uid_search(format!("UID {}:*", baseline.saturating_add(1)))
                .map_err(|_| "IMAP mailbox search failed".to_string())?
                .into_iter()
                .filter(|uid| *uid > baseline && !seen.contains(uid))
                .collect();
            uids.sort_unstable_by(|left, right| right.cmp(left));
            for uid in uids.into_iter().take(20) {
                seen.insert(uid);
                let messages = session
                    .uid_fetch(
                        uid.to_string(),
                        format!("BODY.PEEK[]<0.{MAX_MESSAGE_BYTES}>"),
                    )
                    .map_err(|_| "IMAP message fetch failed".to_string())?;
                for message in messages.iter() {
                    if let Some(raw) = message.body() {
                        if let Some(code) = parse_telegram_code(raw) {
                            return Ok(Some(code));
                        }
                    }
                }
            }
            Ok(None)
        })();
        let _ = session.logout();
        result
    }

    fn connect(&self) -> Result<Session<TlsStream<TcpStream>>, String> {
        let host = self
            .source
            .host(&self.address)
            .ok_or("IMAP source is required")?;
        let addresses = (host, IMAP_PORT)
            .to_socket_addrs()
            .map_err(|_| "IMAP host lookup failed")?;
        let mut stream = None;
        for address in addresses {
            if let Ok(connected) = TcpStream::connect_timeout(&address, SOCKET_TIMEOUT) {
                stream = Some(connected);
                break;
            }
        }
        let stream = stream.ok_or("IMAP connection failed")?;
        stream
            .set_read_timeout(Some(SOCKET_TIMEOUT))
            .map_err(|_| "IMAP timeout setup failed")?;
        stream
            .set_write_timeout(Some(SOCKET_TIMEOUT))
            .map_err(|_| "IMAP timeout setup failed")?;
        let tls = TlsConnector::new().map_err(|_| "IMAP TLS setup failed")?;
        let stream = tls
            .connect(host, stream)
            .map_err(|_| "IMAP TLS connection failed")?;
        let mut client = imap::Client::new(stream);
        client.read_greeting().map_err(|_| "IMAP greeting failed")?;
        let mut session = client
            .login(&self.address, &self.password)
            .map_err(|_| "IMAP login failed")?;
        session
            .select("INBOX")
            .map_err(|_| "IMAP INBOX unavailable")?;
        Ok(session)
    }
}

fn parse_accounts(content: &str, source: EmailSource) -> Result<Vec<MailboxAccount>, String> {
    let mut accounts = Vec::new();
    let mut unique = HashSet::new();
    let mut firstmail_header = source == EmailSource::Firstmail
        && content
            .lines()
            .map(|line| line.trim().trim_start_matches('\u{feff}'))
            .find(|line| !line.is_empty())
            .is_some_and(|line| line.starts_with("Order #"));
    for line in content
        .lines()
        .map(|line| line.trim().trim_start_matches('\u{feff}'))
        .filter(|line| !line.is_empty())
    {
        if firstmail_header && is_firstmail_header_line(line) {
            continue;
        }
        firstmail_header = false;
        let (address, password) = line.split_once(':').ok_or("registrar.emailFileInvalid")?;
        let address = address.trim();
        if address.len() > 254
            || !address.contains('@')
            || address.contains(char::is_whitespace)
            || address.contains(['\r', '\n', '\0'])
            || password.is_empty()
            || password.len() > 1024
            || password.contains(['\r', '\n', '\0'])
        {
            return Err("registrar.emailFileInvalid".into());
        }
        if source == EmailSource::Gmx && gmx_host(address).is_none() {
            return Err("registrar.emailFileInvalid".into());
        }
        if unique.insert(address.to_ascii_lowercase()) {
            accounts.push(MailboxAccount {
                address: address.to_string(),
                password: password.to_string(),
                source,
            });
        }
        if accounts.len() > MAX_ACCOUNTS {
            return Err("registrar.emailFileInvalid".into());
        }
    }
    if accounts.is_empty() {
        return Err("registrar.emailFileRequired".into());
    }
    Ok(accounts)
}

fn is_firstmail_header_line(line: &str) -> bool {
    [
        "Order #",
        "Shop Order UID:",
        "Package:",
        "Quantity:",
        "Has Real Logins:",
        "Total:",
        "Date:",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
        || (line.len() >= 3 && line.bytes().all(|byte| byte == b'='))
}

fn parse_telegram_code(raw: &[u8]) -> Option<String> {
    let message = mailparse::parse_mail(raw).ok()?;
    let from = message.headers.get_first_header("From")?;
    let senders = mailparse::addrparse_header(from).ok()?;
    if !senders.iter().any(|sender| matches!(sender, MailAddr::Single(mailbox) if mailbox.addr.to_ascii_lowercase().ends_with("@telegram.org"))) {
        return None;
    }
    let subject = message
        .headers
        .get_first_value("Subject")
        .unwrap_or_default();
    extract_code(&subject).or_else(|| code_from_part(&message))
}

fn code_from_part(message: &mailparse::ParsedMail<'_>) -> Option<String> {
    if message.subparts.is_empty() {
        return message.get_body().ok().and_then(|body| extract_code(&body));
    }
    message.subparts.iter().find_map(code_from_part)
}

fn extract_code(text: &str) -> Option<String> {
    text.split(|character: char| !character.is_ascii_digit())
        .find(|part| (5..=8).contains(&part.len()))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_file_requires_credentials_and_deduplicates() {
        let accounts = parse_accounts(
            "a@gmx.de:pass:with:colon\na@gmx.de:other\n",
            EmailSource::Gmx,
        )
        .unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].password, "pass:with:colon");
        assert!(parse_accounts("a@example.com:pass", EmailSource::Gmx).is_err());
        assert!(parse_accounts("a@gmx.de", EmailSource::Gmx).is_err());
    }

    #[test]
    fn gmx_mailboxes_use_the_host_for_their_domain() {
        let accounts = parse_accounts("a@gmx.de:pass\nb@GMX.COM:pass\n", EmailSource::Gmx).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(
            accounts[0].source.host(&accounts[0].address),
            Some("imap.gmx.net")
        );
        assert_eq!(
            accounts[1].source.host(&accounts[1].address),
            Some("imap.gmx.com")
        );
        assert!(parse_accounts("a@gmx.com.example:pass", EmailSource::Gmx).is_err());
    }

    #[test]
    fn firstmail_export_header_is_optional_and_only_skipped_at_the_start() {
        let export = "Order #20d185e2-b361-4097-9f5e-e1ad1e22832e\r\n\
Shop Order UID: da20a6f49d8531a65799f52fb13beecf\r\n\
Package: forever_realnames\r\n\
Quantity: 1,000 accounts\r\n\
Has Real Logins: True\r\n\
Total: 1.70€\r\n\
Date: 2026-09-29 14:28:54\r\n\
==================================================\r\n\r\n\
richardthompkins1938@firstmailmorty.com:f.\r\n";
        let accounts = parse_accounts(export, EmailSource::Firstmail).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(
            accounts[0].address,
            "richardthompkins1938@firstmailmorty.com"
        );
        assert_eq!(accounts[0].password, "f.");
        assert_eq!(
            parse_accounts("a@firstmailmorty.com:pass", EmailSource::Firstmail)
                .unwrap()
                .len(),
            1
        );
        assert!(parse_accounts(export, EmailSource::Gmx).is_err());
        assert!(parse_accounts(
            "Order #1\nUnexpected: value\na@firstmailmorty.com:pass",
            EmailSource::Firstmail
        )
        .is_err());
        assert!(parse_accounts(
            "a@firstmailmorty.com:pass\nOrder #1",
            EmailSource::Firstmail
        )
        .is_err());
    }

    #[test]
    fn only_telegram_mail_supplies_verification_code() {
        let valid = b"From: Telegram <noreply@telegram.org>\r\nSubject: Your code: 123456\r\n\r\nUse 123456";
        let invalid = b"From: Other <sender@example.com>\r\nSubject: 123456\r\n\r\nUse 123456";
        let spoofed = b"From: Telegram @telegram.org <sender@example.com>\r\nSubject: 123456\r\n\r\nUse 123456";
        assert_eq!(parse_telegram_code(valid).as_deref(), Some("123456"));
        assert_eq!(parse_telegram_code(invalid), None);
        assert_eq!(parse_telegram_code(spoofed), None);
    }
}
