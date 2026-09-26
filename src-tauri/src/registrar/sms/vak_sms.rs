use std::time::Duration;

use serde::Deserialize;

use super::provider::{
    ActionResult, Activation, ActivationAction, ActivationStatus, NumberRequest, SmsError,
    SmsErrorKind, SmsProvider,
};

const PROVIDER_NAME: &str = "VAK-SMS";
const BASE_URL: &str = "https://vak-sms.com/api";

#[derive(Clone)]
pub struct VakSmsProvider {
    api_key: String,
    agent: ureq::Agent,
}

impl VakSmsProvider {
    pub fn new(api_key: impl Into<String>) -> Result<Self, SmsError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(SmsError::new(
                PROVIDER_NAME,
                SmsErrorKind::Authentication,
                "API key is empty",
            ));
        }

        let config = ureq::Agent::config_builder()
            .https_only(true)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        Ok(Self {
            api_key,
            agent: config.new_agent(),
        })
    }

    fn request(&self, method: &str, params: &[(&str, String)]) -> Result<String, SmsError> {
        let url = format!("{BASE_URL}/{method}/");
        let mut request = self.agent.get(&url).query("apiKey", &self.api_key);
        for (key, value) in params {
            request = request.query(*key, value);
        }

        let response = request.call().map_err(|error| {
            SmsError::new(PROVIDER_NAME, SmsErrorKind::Transport, error.to_string())
        })?;
        let status = response.status().as_u16();
        let body = response.into_body().read_to_string().map_err(|error| {
            SmsError::new(
                PROVIDER_NAME,
                SmsErrorKind::InvalidResponse,
                format!("response body: {error}"),
            )
        })?;
        if !(200..300).contains(&status) {
            return Err(parse_error(&body));
        }
        Ok(body)
    }
}

impl SmsProvider for VakSmsProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn balance(&self) -> Result<f64, SmsError> {
        let response: VakResponse = parse_json(&self.request("getBalance", &[])?)?;
        ensure_success(&response)?;
        response.balance.ok_or_else(|| {
            SmsError::new(
                PROVIDER_NAME,
                SmsErrorKind::InvalidResponse,
                "balance is missing from response",
            )
        })
    }

    fn acquire_number(&self, request: &NumberRequest) -> Result<Activation, SmsError> {
        validate_request(request)?;
        let mut params = vec![
            ("service", service_name(&request.service).to_string()),
            ("country", request.country.to_ascii_uppercase()),
        ];
        if let Some(operator) = request.operator.as_deref() {
            params.push(("operator", operator.to_string()));
        }
        let response: VakResponse = parse_json(&self.request("getNumber", &params)?)?;
        ensure_success(&response)?;
        let id = response.id_num.ok_or_else(|| missing_field("idNum"))?;
        let phone = response.tel.ok_or_else(|| missing_field("tel"))?;
        Ok(Activation {
            id,
            phone,
            cost: response.cost,
            country: Some(request.country.clone()),
            operator: request.operator.clone(),
            expires_at: None,
        })
    }

    fn activation_status(&self, activation_id: &str) -> Result<ActivationStatus, SmsError> {
        validate_id(activation_id)?;
        let response: VakResponse =
            parse_json(&self.request("getSmsCode", &[("idNum", activation_id.to_string())])?)?;
        ensure_success(&response)?;
        match response.status.as_deref() {
            Some("WAIT_CODE") | Some("SMS_WAIT") => Ok(ActivationStatus::Waiting),
            Some("SMS_CODE") => response
                .sms_code
                .filter(|code| !code.trim().is_empty())
                .map(|code| ActivationStatus::CodeReceived { code, text: None })
                .ok_or_else(|| missing_field("smsCode")),
            Some("SMS_CANCEL") => Ok(ActivationStatus::Cancelled),
            Some(status) => Ok(ActivationStatus::Provider(status.to_string())),
            None => Err(missing_field("status")),
        }
    }

    fn update_activation(
        &self,
        activation_id: &str,
        action: ActivationAction,
    ) -> Result<ActionResult, SmsError> {
        validate_id(activation_id)?;
        if action == ActivationAction::MarkReady {
            return Ok(ActionResult::Ready);
        }
        if action == ActivationAction::RequestAnotherCode {
            return Err(SmsError::new(
                PROVIDER_NAME,
                SmsErrorKind::Unsupported,
                "VAK-SMS does not document requesting another SMS",
            ));
        }

        let status = if action == ActivationAction::Finish {
            "3"
        } else {
            "6"
        };
        let response: VakResponse = parse_json(&self.request(
            "setStatus",
            &[
                ("idNum", activation_id.to_string()),
                ("status", status.to_string()),
            ],
        )?)?;
        ensure_success(&response)?;
        Ok(if action == ActivationAction::Finish {
            ActionResult::Finished
        } else {
            ActionResult::Cancelled
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VakResponse {
    response: Option<serde_json::Value>,
    balance: Option<f64>,
    id_num: Option<String>,
    tel: Option<String>,
    cost: Option<f64>,
    status: Option<String>,
    sms_code: Option<String>,
    #[serde(alias = "error_msg")]
    error_msg: Option<String>,
    message: Option<String>,
    code: Option<String>,
}

fn parse_json(body: &str) -> Result<VakResponse, SmsError> {
    serde_json::from_str(body).map_err(|error| {
        SmsError::new(
            PROVIDER_NAME,
            SmsErrorKind::InvalidResponse,
            format!("invalid JSON response: {error}"),
        )
    })
}

fn ensure_success(response: &VakResponse) -> Result<(), SmsError> {
    if response
        .response
        .as_ref()
        .is_some_and(|value| value == 1 || value == "1")
    {
        return Ok(());
    }
    Err(SmsError::new(
        PROVIDER_NAME,
        error_kind(response),
        response
            .error_msg
            .as_deref()
            .or(response.message.as_deref())
            .or(response.code.as_deref())
            .unwrap_or("VAK-SMS rejected the request"),
    ))
}

fn parse_error(body: &str) -> SmsError {
    match parse_json(body) {
        Ok(response) => ensure_success(&response).unwrap_err(),
        Err(_) => SmsError::new(
            PROVIDER_NAME,
            SmsErrorKind::Provider,
            format!("unexpected API response: {}", body.trim()),
        ),
    }
}

fn error_kind(response: &VakResponse) -> SmsErrorKind {
    let text = response
        .error_msg
        .as_deref()
        .or(response.message.as_deref())
        .or(response.code.as_deref())
        .unwrap_or_default()
        .to_lowercase();
    if text.contains("пользователь не найден") || text.contains("key") || text.contains("auth")
    {
        SmsErrorKind::Authentication
    } else if text.contains("no_number") || text.contains("нет номеров") {
        SmsErrorKind::NoNumbers
    } else if text.contains("balance") || text.contains("баланс") {
        SmsErrorKind::InsufficientBalance
    } else {
        SmsErrorKind::Provider
    }
}

fn validate_request(request: &NumberRequest) -> Result<(), SmsError> {
    if request.country.trim().is_empty() || request.service.trim().is_empty() {
        return Err(SmsError::new(
            PROVIDER_NAME,
            SmsErrorKind::InvalidRequest,
            "country and service are required",
        ));
    }
    Ok(())
}

fn validate_id(activation_id: &str) -> Result<(), SmsError> {
    if activation_id.is_empty() || !activation_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(SmsError::new(
            PROVIDER_NAME,
            SmsErrorKind::InvalidRequest,
            "activation id must contain only digits",
        ));
    }
    Ok(())
}

fn service_name(service: &str) -> &str {
    if service.eq_ignore_ascii_case("telegram") {
        "tg"
    } else {
        service
    }
}

fn missing_field(name: &str) -> SmsError {
    SmsError::new(
        PROVIDER_NAME,
        SmsErrorKind::InvalidResponse,
        format!("{name} is missing from response"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_number_response() {
        let response: VakResponse =
            parse_json(r#"{"response":1,"idNum":"42","tel":"79990001122"}"#).unwrap();
        ensure_success(&response).unwrap();
        assert_eq!(response.id_num.as_deref(), Some("42"));
    }

    #[test]
    fn maps_authentication_error() {
        let response: VakResponse =
            parse_json(r#"{"response":0,"error_msg":"Пользователь не найден"}"#).unwrap();
        assert_eq!(
            ensure_success(&response).unwrap_err().kind,
            SmsErrorKind::Authentication
        );
    }
}
