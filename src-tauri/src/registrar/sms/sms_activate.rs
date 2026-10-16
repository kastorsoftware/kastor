use std::time::Duration;

use super::provider::{
    ActionResult, Activation, ActivationAction, ActivationStatus, NumberRequest, SmsError,
    SmsCountry, SmsErrorKind, SmsOperator, SmsPrice, SmsProvider,
};

pub const GRIZZLY_SMS_ENDPOINT: &str = "https://api.grizzlysms.com/stubs/handler_api.php";
pub const SMS_BOWER_ENDPOINT: &str = "https://smsbower.page/stubs/handler_api.php";
pub const SMS_MAN_ENDPOINT: &str = "https://api.sms-man.com/stubs/handler_api.php";
pub const SMS_365_ENDPOINT: &str = "https://365api.net/stubs/handler_api.php";
pub const SMS_24_7_ENDPOINT: &str = "https://24sms7.com/stubs/handler_api.php";
pub const GREEDY_SMS_ENDPOINT: &str = "https://api.greedy-sms.com/stubs/handler_api.php";
pub const BRING_SMS_ENDPOINT: &str = "https://api.bring-sms.store/stubs/handler_api.php";

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

    pub fn bring_sms(api_key: impl Into<String>) -> Result<Self, SmsError> {
        Self::new("BRINGSMS", BRING_SMS_ENDPOINT, api_key)
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
    fn set_http_proxy(&mut self, proxy: &str) -> Result<(), SmsError> {
        self.agent = super::provider::proxy_agent(&self.name, proxy)?;
        Ok(())
    }
    fn name(&self) -> &str {
        &self.name
    }

    fn balance(&self) -> Result<f64, SmsError> {
        let body = self.request(&[("action", "getBalance".to_string())])?;
        parse_balance(&self.name, &body)
    }

    fn countries(&self) -> Result<Vec<SmsCountry>, SmsError> {
        let body = self.request(&[("action", "getCountries".to_string())])?;
        parse_countries(&self.name, &body)
    }

    fn operators(&self, country: &str) -> Result<Vec<SmsOperator>, SmsError> {
        if country.trim().is_empty() {
            return Err(SmsError::new(
                &self.name,
                SmsErrorKind::InvalidRequest,
                "country is required",
            ));
        }
        let body = self.request(&[
            ("action", "getOperators".to_string()),
            ("country", country.to_string()),
        ])?;
        parse_operators(&self.name, &body)
    }

    fn prices(
        &self,
        service: &str,
        country: Option<&str>,
    ) -> Result<Vec<SmsPrice>, SmsError> {
        if service.trim().is_empty() {
            return Err(SmsError::new(
                &self.name,
                SmsErrorKind::InvalidRequest,
                "service is required",
            ));
        }
        let mut params = vec![
            ("action", "getPrices".to_string()),
            ("service", compatible_service_name(service).to_string()),
        ];
        if let Some(country) = country.filter(|country| !country.trim().is_empty()) {
            params.push(("country", country.to_string()));
        }
        let body = self.request(&params)?;
        parse_prices(&self.name, service, &body)
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
        // Greedy starts waiting immediately and only documents statuses 3, 6 and 8.
        if self.endpoint == GREEDY_SMS_ENDPOINT && action == ActivationAction::MarkReady {
            return Ok(ActionResult::Ready);
        }
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

pub(super) fn compatible_service_name(service: &str) -> &str {
    if service.eq_ignore_ascii_case("telegram") {
        "tg"
    } else {
        service
    }
}

fn parse_balance(provider: &str, body: &str) -> Result<f64, SmsError> {
    if let Some(value) = body.strip_prefix("ACCESS_BALANCE:") {
        let invalid = || {
            SmsError::new(
                provider,
                SmsErrorKind::InvalidResponse,
                "invalid balance response",
            )
        };
        // Compatible providers can append a second amount after the balance.
        let mut amounts = value.split(':');
        let balance: f64 = amounts.next().ok_or_else(invalid)?.trim().parse()
            .map_err(|_| invalid())?;
        if !balance.is_finite() || balance < 0.0 {
            return Err(invalid());
        }
        if let Some(extra) = amounts.next() {
            let extra: f64 = extra.trim().parse().map_err(|_| invalid())?;
            if !extra.is_finite() || extra < 0.0 || amounts.next().is_some() {
                return Err(invalid());
            }
        }
        return Ok(balance);
    }
    Err(parse_provider_error(provider, body))
}

fn parse_number(provider: &str, body: &str) -> Result<Activation, SmsError> {
    let number = body.strip_prefix("ACCESS_NUMBER:").or_else(|| {
        (provider == "Greedy-SMS").then(|| body.strip_prefix("STATUS_OK:")).flatten()
    });
    if let Some(value) = number {
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

fn parse_countries(provider: &str, body: &str) -> Result<Vec<SmsCountry>, SmsError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| parse_provider_error(provider, body))?;
    let countries = value.as_object().ok_or_else(|| {
        SmsError::new(provider, SmsErrorKind::InvalidResponse, "invalid countries response")
    })?;
    let mut result: Vec<_> = countries
        .iter()
        .filter_map(|(id, name)| {
            name.as_str().map(|name| SmsCountry {
                id: id.to_string(),
                name: name.to_string(),
            })
        })
        .collect();
    result.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(result)
}

fn parse_operators(provider: &str, body: &str) -> Result<Vec<SmsOperator>, SmsError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| parse_provider_error(provider, body))?;
    let object = value.as_object().ok_or_else(|| {
        SmsError::new(provider, SmsErrorKind::InvalidResponse, "invalid operators response")
    })?;
    let mut result: Vec<_> = object
        .iter()
        .filter_map(|(id, name)| {
            name.as_str().map(|name| SmsOperator {
                id: id.to_string(),
                name: name.to_string(),
            })
        })
        .collect();
    result.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(result)
}

fn parse_prices(provider: &str, service: &str, body: &str) -> Result<Vec<SmsPrice>, SmsError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| parse_provider_error(provider, body))?;
    let countries = value.as_object().ok_or_else(|| {
        SmsError::new(provider, SmsErrorKind::InvalidResponse, "invalid prices response")
    })?;
    let mut result = Vec::new();
    for (country_id, services) in countries {
        let Some(details) = services
            .as_object()
            .and_then(|services| services.get(compatible_service_name(service)))
            .and_then(serde_json::Value::as_object)
        else {
            continue;
        };
        let Some(price) = details.get("cost").and_then(serde_json::Value::as_f64) else {
            continue;
        };
        let available = details.get("count").and_then(serde_json::Value::as_u64);
        let name = details
            .get("country_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(country_id)
            .to_string();
        result.push(SmsPrice {
            country: SmsCountry {
                id: country_id.to_string(),
                name,
            },
            service: service.to_string(),
            operator: None,
            price,
            available,
        });
    }
    result.sort_by(|left, right| left.price.total_cmp(&right.price));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_and_extended_balances() {
        assert_eq!(parse_balance("test", "ACCESS_BALANCE:12.34").unwrap(), 12.34);
        assert_eq!(parse_balance("BRINGSMS", "ACCESS_BALANCE:12.34:0.00").unwrap(), 12.34);
        assert_eq!(parse_balance("test", "ACCESS_BALANCE:0:1.25").unwrap(), 0.0);
    }

    #[test]
    fn rejects_malformed_balances() {
        for body in [
            "ACCESS_BALANCE:", "ACCESS_BALANCE:NaN", "ACCESS_BALANCE:inf",
            "ACCESS_BALANCE:-1", "ACCESS_BALANCE:12:", "ACCESS_BALANCE:12:NaN",
            "ACCESS_BALANCE:12:-1", "ACCESS_BALANCE:12:0:1",
        ] {
            assert_eq!(parse_balance("test", body).unwrap_err().kind, SmsErrorKind::InvalidResponse);
        }
        assert_eq!(parse_balance("test", "BAD_KEY").unwrap_err().kind, SmsErrorKind::Authentication);
    }

    #[test]
    fn parses_number() {
        let activation = parse_number("test", "ACCESS_NUMBER:123:79990001122").unwrap();
        assert_eq!(activation.id, "123");
        assert_eq!(activation.phone, "79990001122");
    }

    #[test]
    fn greedy_accepts_documented_number_response_without_affecting_other_providers() {
        let activation = parse_number("Greedy-SMS", "STATUS_OK:123:79990001122").unwrap();
        assert_eq!(activation.id, "123");
        assert_eq!(activation.phone, "79990001122");
        assert!(parse_number("test", "STATUS_OK:123:79990001122").is_err());
        assert!(parse_number("Greedy-SMS", "STATUS_OK:123").is_err());
        assert!(parse_number("Greedy-SMS", "STATUS_OK:123:").is_err());
    }

    #[test]
    fn greedy_mark_ready_does_not_send_an_unsupported_status() {
        let provider = SmsActivateProvider::greedy_sms("test-key").unwrap();
        assert_eq!(provider.update_activation("123", ActivationAction::MarkReady).unwrap(), ActionResult::Ready);
        assert!(provider.update_activation("", ActivationAction::MarkReady).is_err());
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

    #[test]
    fn parses_price_list() {
        let prices = parse_prices(
            "test",
            "telegram",
            r#"{"31":{"tg":{"cost":9.93,"count":2575,"country_name":"South Africa"}}}"#,
        )
        .unwrap();
        assert_eq!(prices[0].country.id, "31");
        assert_eq!(prices[0].available, Some(2575));
    }
}
