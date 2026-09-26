use std::time::Duration;

use super::provider::{
    ActionResult, Activation, ActivationAction, ActivationStatus, NumberRequest, SmsError,
    SmsErrorKind, SmsProvider,
};

pub const GRIZZLY_SMS_ENDPOINT: &str = "https://api.grizzlysms.com/stubs/handler_api.php";
pub const SMS_BOWER_ENDPOINT: &str = "https://smsbower.page/stubs/handler_api.php";
pub const SMS_MAN_ENDPOINT: &str = "https://api.sms-man.com/stubs/handler_api.php";
pub const SMS_365_ENDPOINT: &str = "https://365api.net/stubs/handler_api.php";
pub const SMS_24_7_ENDPOINT: &str = "https://24sms7.com/stubs/handler_api.php";
pub const GREEDY_SMS_ENDPOINT: &str = "https://api.greedy-sms.com/stubs/handler_api.php";

#[derive(Clone)]
pub struct SmsActivateProvider {
    name: String,
    endpoint: String,
    api_key: String,
    agent: ureq::Agent,
}

impl SmsActivateProvider {
    pub fn new(
        name: impl Into<String>,
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Result<Self, SmsError> {
        let name = name.into();
        let endpoint = endpoint.into();
        let api_key = api_key.into();
        validate_config(&name, &endpoint, &api_key)?;

        let config = ureq::Agent::config_builder()
            .https_only(true)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build();

        Ok(Self {
            name,
            endpoint,
            api_key,
            agent: config.new_agent(),
        })
    }

    pub fn grizzly(api_key: impl Into<String>) -> Result<Self, SmsError> {
        Self::new("GrizzlySMS", GRIZZLY_SMS_ENDPOINT, api_key)
    }

    pub fn sms_bower(api_key: impl Into<String>) -> Result<Self, SmsError> {
        Self::new("SMSBower", SMS_BOWER_ENDPOINT, api_key)
    }

    pub fn sms_man(api_key: impl Into<String>) -> Result<Self, SmsError> {
        Self::new("SMS-Man", SMS_MAN_ENDPOINT, api_key)
    }

    pub fn sms_365(api_key: impl Into<String>) -> Result<Self, SmsError> {
        Self::new("365SMS", SMS_365_ENDPOINT, api_key)
    }

    pub fn sms_24_7(api_key: impl Into<String>) -> Result<Self, SmsError> {
        Self::new("24SMS7", SMS_24_7_ENDPOINT, api_key)
    }

    pub fn greedy_sms(api_key: impl Into<String>) -> Result<Self, SmsError> {
        Self::new("Greedy-SMS", GREEDY_SMS_ENDPOINT, api_key)
    }

    fn request(&self, params: &[(&str, String)]) -> Result<String, SmsError> {
        let mut request = self
            .agent
            .get(&self.endpoint)
            .query("api_key", &self.api_key);
        for (key, value) in params {
            request = request.query(*key, value);
        }

        let response = request.call().map_err(|error| {
            SmsError::new(&self.name, SmsErrorKind::Transport, error.to_string())
        })?;
        let status = response.status().as_u16();
        let body = response.into_body().read_to_string().map_err(|error| {
            SmsError::new(
                &self.name,
                SmsErrorKind::InvalidResponse,
                format!("response body: {error}"),
            )
        })?;

        if !(200..300).contains(&status) {
            return Err(SmsError::new(
                &self.name,
                SmsErrorKind::Provider,
                format!("HTTP {status}: {}", body.trim()),
            ));
        }
        Ok(body.trim().to_string())
    }
}

impl SmsProvider for SmsActivateProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn balance(&self) -> Result<f64, SmsError> {
        let body = self.request(&[("action", "getBalance".to_string())])?;
        parse_balance(&self.name, &body)
    }

    fn acquire_number(&self, request: &NumberRequest) -> Result<Activation, SmsError> {
        validate_request(&self.name, request)?;
        let service = compatible_service_name(&request.service);
        let mut params = vec![
            ("action", "getNumber".to_string()),
            ("service", service.to_string()),
            ("country", request.country.clone()),
        ];
        if let Some(operator) = request.operator.as_deref() {
            params.push(("operator", operator.to_string()));
        }
        if let Some(max_price) = request.max_price {
            params.push(("maxPrice", max_price.to_string()));
        }

        let body = self.request(&params)?;
        parse_number(&self.name, &body)
    }

    fn activation_status(&self, activation_id: &str) -> Result<ActivationStatus, SmsError> {
        validate_id(&self.name, activation_id)?;
        let body = self.request(&[
            ("action", "getStatus".to_string()),
            ("id", activation_id.to_string()),
        ])?;
        parse_status(&self.name, &body)
    }

    fn update_activation(
        &self,
        activation_id: &str,
        action: ActivationAction,
    ) -> Result<ActionResult, SmsError> {
        validate_id(&self.name, activation_id)?;
        let status = match action {
            ActivationAction::MarkReady => "1",
            ActivationAction::RequestAnotherCode => "3",
            ActivationAction::Finish => "6",
            ActivationAction::Cancel | ActivationAction::Ban => "8",
        };
        let body = self.request(&[
            ("action", "setStatus".to_string()),
            ("id", activation_id.to_string()),
            ("status", status.to_string()),
        ])?;
        parse_action_result(&self.name, &body)
    }
}

fn validate_config(name: &str, endpoint: &str, api_key: &str) -> Result<(), SmsError> {
    if name.trim().is_empty() {
        return Err(SmsError::new(
            "SMS provider",
            SmsErrorKind::InvalidRequest,
            "provider name is empty",
        ));
    }
    if !endpoint.starts_with("https://") || endpoint.contains('?') || endpoint.contains('#') {
        return Err(SmsError::new(
            name,
            SmsErrorKind::InvalidRequest,
            "endpoint must be an HTTPS URL without query or fragment",
        ));
    }
    if api_key.trim().is_empty() {
        return Err(SmsError::new(
            name,
            SmsErrorKind::Authentication,
            "API key is empty",
        ));
    }
    Ok(())
}

fn validate_request(provider: &str, request: &NumberRequest) -> Result<(), SmsError> {
    if request.country.trim().is_empty() || request.service.trim().is_empty() {
        return Err(SmsError::new(
            provider,
            SmsErrorKind::InvalidRequest,
            "country and service are required",
        ));
    }
    if request
        .max_price
        .is_some_and(|price| !price.is_finite() || price <= 0.0)
    {
        return Err(SmsError::new(
            provider,
            SmsErrorKind::InvalidRequest,
            "max price must be positive and finite",
        ));
    }
    Ok(())
}

fn validate_id(provider: &str, activation_id: &str) -> Result<(), SmsError> {
    if activation_id.is_empty() || !activation_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(SmsError::new(
            provider,
            SmsErrorKind::InvalidRequest,
            "activation id must contain only digits",
        ));
    }
    Ok(())
}

fn compatible_service_name(service: &str) -> &str {
    if service.eq_ignore_ascii_case("telegram") {
        "tg"
    } else {
        service
    }
}

fn parse_balance(provider: &str, body: &str) -> Result<f64, SmsError> {
    if let Some(value) = body.strip_prefix("ACCESS_BALANCE:") {
        return value.trim().parse().map_err(|_| {
            SmsError::new(
                provider,
                SmsErrorKind::InvalidResponse,
                "invalid balance response",
            )
        });
    }
    Err(parse_provider_error(provider, body))
}

fn parse_number(provider: &str, body: &str) -> Result<Activation, SmsError> {
    if let Some(value) = body.strip_prefix("ACCESS_NUMBER:") {
        let (id, phone) = value.split_once(':').ok_or_else(|| {
            SmsError::new(
                provider,
                SmsErrorKind::InvalidResponse,
                "invalid number response",
            )
        })?;
        if id.is_empty() || phone.is_empty() {
            return Err(SmsError::new(
                provider,
                SmsErrorKind::InvalidResponse,
                "number response has empty fields",
            ));
        }
        return Ok(Activation {
            id: id.to_string(),
            phone: phone.to_string(),
            cost: None,
            country: None,
            operator: None,
            expires_at: None,
        });
    }
    Err(parse_provider_error(provider, body))
}

fn parse_status(provider: &str, body: &str) -> Result<ActivationStatus, SmsError> {
    match body {
        "STATUS_WAIT_CODE" => Ok(ActivationStatus::Waiting),
        "STATUS_WAIT_RESEND" => Ok(ActivationStatus::WaitingForResend),
        "STATUS_CANCEL" => Ok(ActivationStatus::Cancelled),
        _ => {
            if let Some(code) = body.strip_prefix("STATUS_OK:") {
                return Ok(ActivationStatus::CodeReceived {
                    code: code.trim().to_string(),
                    text: None,
                });
            }
            if let Some(last_code) = body.strip_prefix("STATUS_WAIT_RETRY:") {
                return Ok(ActivationStatus::WaitingForRetry {
                    last_code: (!last_code.trim().is_empty()).then(|| last_code.trim().to_string()),
                });
            }
            Err(parse_provider_error(provider, body))
        }
    }
}

fn parse_action_result(provider: &str, body: &str) -> Result<ActionResult, SmsError> {
    match body {
        "ACCESS_READY" => Ok(ActionResult::Ready),
        "ACCESS_RETRY_GET" => Ok(ActionResult::WaitingForRetry),
        "ACCESS_ACTIVATION" => Ok(ActionResult::Finished),
        "ACCESS_CANCEL" => Ok(ActionResult::Cancelled),
        _ => Err(parse_provider_error(provider, body)),
    }
}

fn parse_provider_error(provider: &str, body: &str) -> SmsError {
    let code = body.split(':').next().unwrap_or(body).trim();
    let kind = match code {
        "BAD_KEY" => SmsErrorKind::Authentication,
        "NO_NUMBERS" => SmsErrorKind::NoNumbers,
        "NO_BALANCE" => SmsErrorKind::InsufficientBalance,
        "NO_ACTIVATION" => SmsErrorKind::ActivationNotFound,
        "EARLY_CANCEL_DENIED" => SmsErrorKind::TooEarlyToCancel,
        "BAD_ACTION" | "BAD_SERVICE" | "BAD_STATUS" | "BAD_COUNTRY" => SmsErrorKind::InvalidRequest,
        _ => SmsErrorKind::Provider,
    };
    SmsError::new(provider, kind, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_number() {
        let activation = parse_number("test", "ACCESS_NUMBER:123:79990001122").unwrap();
        assert_eq!(activation.id, "123");
        assert_eq!(activation.phone, "79990001122");
    }

    #[test]
    fn parses_statuses() {
        assert_eq!(
            parse_status("test", "STATUS_OK:12345").unwrap(),
            ActivationStatus::CodeReceived {
                code: "12345".to_string(),
                text: None,
            }
        );
        assert_eq!(
            parse_status("test", "STATUS_WAIT_RETRY:11111").unwrap(),
            ActivationStatus::WaitingForRetry {
                last_code: Some("11111".to_string()),
            }
        );
    }

    #[test]
    fn maps_provider_errors() {
        assert_eq!(
            parse_number("test", "NO_NUMBERS").unwrap_err().kind,
            SmsErrorKind::NoNumbers
        );
        assert_eq!(
            parse_status("test", "BAD_KEY").unwrap_err().kind,
            SmsErrorKind::Authentication
        );
    }
}
