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
    fn set_http_proxy(&mut self, proxy: &str) -> Result<(), SmsError> {
        self.agent = super::provider::proxy_agent(PROVIDER_NAME, proxy)?;
        Ok(())
    }
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn countries(&self) -> Result<Vec<super::SmsCountry>, SmsError> {
        let countries: Vec<VakCountry> = parse_json(&self.request("getCountryList", &[])?)?;
        Ok(countries
            .into_iter()
            .map(|country| super::SmsCountry {
                id: country.country_code,
                name: country.country_name,
            })
            .collect())
    }

    fn prices(
        &self,
        service: &str,
        country: Option<&str>,
    ) -> Result<Vec<super::SmsPrice>, SmsError> {
        let countries = match country {
            Some(country) => vec![super::SmsCountry {
                id: country.to_string(),
                name: country.to_string(),
            }],
            None => self.countries()?,
        };
        let mut result = Vec::new();
        for country in countries {
            let value: serde_json::Value = parse_json(&self.request(
                "getCountNumber",
                &[
                    ("country", country.id.clone()),
                    ("service", service_name(service).to_string()),
                ],
            )?)?;
            if let Some(error) = value.get("error") {
                if error == "noService" || error == "noCountry" {
                    continue;
                }
                return Err(SmsError::new(
                    PROVIDER_NAME,
                    SmsErrorKind::Provider,
                    "catalog request rejected",
                ));
            }
            let price = value
                .get("price")
                .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()));
            if let Some(price) = price {
                result.push(super::SmsPrice {
                    country,
                    service: service.into(),
                    operator: None,
                    price,
                    available: value
                        .get(service_name(service))
                        .and_then(serde_json::Value::as_u64),
                });
            }
        }
        Ok(result)
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
            ("country", request.country.clone()),
        ];
        if let Some(operator) = request.operator.as_deref() {
            params.push(("operator", operator.to_string()));
        }
        if let Some(max_price) = request.max_price {
            params.push(("maxPrice", max_price.to_string()));
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
            Some("WAIT_CODE") | Some("SMS_WAIT") | Some("ready") | Some("waitSMS") => {
                Ok(ActivationStatus::Waiting)
            }
            Some("SMS_CODE") => response
                .sms_code
                .filter(|code| !code.trim().is_empty())
                .map(|code| ActivationStatus::CodeReceived { code, text: None })
                .ok_or_else(|| missing_field("smsCode")),
            Some("SMS_CANCEL") => Ok(ActivationStatus::Cancelled),
            Some(status) => Ok(ActivationStatus::Provider(status.to_string())),
            None => Ok(
                match response.sms_code.filter(|code| !code.trim().is_empty()) {
                    Some(code) => ActivationStatus::CodeReceived { code, text: None },
                    None => ActivationStatus::Waiting,
                },
            ),
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
        let status = match action {
            ActivationAction::RequestAnotherCode => "send",
            ActivationAction::Finish => "end",
            _ => "bad",
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
struct VakCountry {
    country_name: String,
    country_code: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VakResponse {
    response: Option<serde_json::Value>,
    balance: Option<f64>,
    id_num: Option<String>,
    #[serde(default, deserialize_with = "optional_phone")]
    tel: Option<String>,
    cost: Option<f64>,
    status: Option<String>,
    sms_code: Option<String>,
    #[serde(alias = "error_msg")]
    error_msg: Option<String>,
    message: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

fn optional_phone<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value)),
        Some(serde_json::Value::Number(value)) if value.as_u64().is_some() => {
            Ok(Some(value.to_string()))
        }
        _ => Err(serde::de::Error::custom("invalid phone number")),
    }
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, SmsError> {
    serde_json::from_str(body).map_err(|error| {
        SmsError::new(
            PROVIDER_NAME,
            SmsErrorKind::InvalidResponse,
            format!("invalid JSON response: {error}"),
        )
    })
}

fn ensure_success(response: &VakResponse) -> Result<(), SmsError> {
    if response.error.is_none()
        && (response.response.is_none()
            || response
                .response
                .as_ref()
                .is_some_and(|value| value == 1 || value == "1"))
    {
        return Ok(());
    }
    Err(SmsError::new(
        PROVIDER_NAME,
        error_kind(response),
        response
            .error_msg
            .as_deref()
            .or(response.error.as_deref())
            .or(response.message.as_deref())
            .or(response.code.as_deref())
            .unwrap_or("VAK-SMS rejected the request"),
    ))
}

fn parse_error(body: &str) -> SmsError {
    match parse_json(body) {
        Ok(response) => ensure_success(&response).err().unwrap_or_else(|| {
            SmsError::new(PROVIDER_NAME, SmsErrorKind::Provider, "HTTP request failed")
        }),
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
        .or(response.error.as_deref())
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
    if activation_id.is_empty()
        || activation_id.len() > 128
        || activation_id.chars().any(char::is_control)
    {
        return Err(SmsError::new(
            PROVIDER_NAME,
            SmsErrorKind::InvalidRequest,
            "invalid activation id",
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

    #[test]
    fn handles_official_number_shape_and_http_error_without_panic() {
        let response: VakResponse = parse_json(r#"{"idNum":"abc-123","tel":79990001122}"#).unwrap();
        ensure_success(&response).unwrap();
        assert_eq!(response.tel.as_deref(), Some("79990001122"));
        assert_eq!(
            parse_error(r#"{"balance":42}"#).kind,
            SmsErrorKind::Provider
        );
        assert_eq!(
            parse_error(r#"{"error":"apiKeyNotFound"}"#).kind,
            SmsErrorKind::Authentication
        );
        assert!(validate_id("abc-123").is_ok());
        assert!(validate_id("abc\n").is_err());
    }
}
