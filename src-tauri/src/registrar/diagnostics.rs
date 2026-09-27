// Remote errors can contain URLs, credentials and verification challenges.
pub(crate) fn error_summary(error: &str) -> String {
    if let Some(rest) = error.strip_prefix("HTTP ") {
        if let Some((status, body)) = rest.split_once(": ") {
            if let Ok(status) = status.parse::<u16>() {
                return format!("http_status={status} code={}", remote_code(body));
            }
        }
    }
    if let Some(rest) = error.strip_prefix("RPC ") {
        if let Some((status, message)) = rest.split_once(": ") {
            if let Ok(status) = status.parse::<i32>() {
                return format!("rpc_status={status} code={}", remote_code(message));
            }
        }
    }
    let lower = error.to_ascii_lowercase();
    let category = if lower.contains("timed out") || lower.contains("timeout") {
        "timeout"
    } else if lower.contains("eof") {
        "eof"
    } else if lower.contains("unknown")
        || lower.contains("unexpected")
        || lower.contains("deserialize")
    {
        "decode"
    } else if lower.contains("proxy") {
        "proxy"
    } else if lower.contains("connect") {
        "connection"
    } else if lower.contains("write") || lower.contains("save") {
        "storage"
    } else {
        "other"
    };
    let mut summary = format!("category={category}");
    let remote = remote_code(error);
    if remote != "unrecognized" {
        summary.push_str(&format!(" code={remote}"));
    }
    if let Some(start) = error
        .find("0x")
        .filter(|_| lower.contains("constructor") || lower.contains("ctor"))
    {
        let hex: String = error[start + 2..]
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .take(8)
            .collect();
        if !hex.is_empty() {
            summary.push_str(&format!(" constructor=0x{hex}"));
        }
    }
    if let Some(start) = lower.find("os error ") {
        let code: String = lower[start + 9..]
            .chars()
            .take_while(char::is_ascii_digit)
            .take(6)
            .collect();
        if !code.is_empty() {
            summary.push_str(&format!(" os_error={code}"));
        }
    }
    summary
}

pub(crate) fn remote_code(message: &str) -> String {
    if let Some(rest) = message.strip_prefix("HTTP ") {
        if let Some((_, body)) = rest.split_once(": ") {
            return remote_code(body);
        }
    }
    if let Ok(serde_json::Value::Object(fields)) =
        serde_json::from_str::<serde_json::Value>(message)
    {
        for name in ["code", "error", "status"] {
            if let Some(serde_json::Value::String(code)) = fields.get(name) {
                if code.len() <= 96
                    && code.contains('_')
                    && code
                        .bytes()
                        .all(|byte| byte.is_ascii_alphabetic() || byte == b'_')
                {
                    return code.to_string();
                }
            }
        }
        return "unrecognized".into();
    }
    let code: String = message
        .split("__")
        .next()
        .unwrap_or_default()
        .chars()
        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
        .take(96)
        .collect();
    let code = code.trim_end_matches('_');
    if code.is_empty() || !code.contains('_') {
        "unrecognized".into()
    } else {
        code.into()
    }
}

pub(crate) fn log_failure(stage: &str, error: &str) {
    dbg_log!(
        "registrar::telegram stage={} failed {}",
        stage,
        error_summary(error)
    );
}

pub(crate) fn observe<T>(stage: &str, result: Result<T, String>) -> Result<T, String> {
    if let Err(error) = &result {
        log_failure(stage, error);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_diagnostics_strip_challenge_payloads() {
        assert_eq!(
            error_summary("RPC 400: RECAPTCHA_CHECK_login__secret-key"),
            "rpc_status=400 code=RECAPTCHA_CHECK"
        );
        assert_eq!(
            error_summary("RPC 303: PHONE_MIGRATE_5"),
            "rpc_status=303 code=PHONE_MIGRATE_5"
        );
        assert_eq!(remote_code("EARLY_CANCEL_DENIED"), "EARLY_CANCEL_DENIED");
        assert_eq!(
            error_summary("HTTP 429: EARLY_CANCEL_DENIED: secret"),
            "http_status=429 code=EARLY_CANCEL_DENIED"
        );
        assert_eq!(
            remote_code(r#"{"error":"cancel_not_allowed","api_key":"secret"}"#),
            "cancel_not_allowed"
        );
    }

    #[test]
    fn transport_and_decode_errors_do_not_leak_secrets() {
        assert_eq!(
            error_summary("connect https://example.com/?api_key=secret: os error 10053"),
            "category=connection os_error=10053"
        );
        assert_eq!(
            error_summary("unknown constructor 0x1234abcd with phone=123456789"),
            "category=decode constructor=0x1234abcd"
        );
        assert_eq!(
            error_summary("read length failed: early eof"),
            "category=eof"
        );
    }
}
