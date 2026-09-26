use std::time::Duration;

use serde::Deserialize;

use super::provider::{
    ActionResult, Activation, ActivationAction, ActivationStatus, NumberRequest, SmsError,
    SmsErrorKind, SmsProvider,
};

const PROVIDER_NAME: &str = "5SIM";
const BASE_URL: &str = "https://5sim.com";

#[derive(Clone)]
pub struct FiveSimProvider {
    token: String,
    agent: ureq::Agent,
}

impl FiveSimProvider {
    pub fn new(token: impl Into<String>) -> Result<Self, SmsError> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(SmsError::new(
                PROVIDER_NAME,
                SmsErrorKind::Authentication,
                "API token is empty",
            ));
        }

        let config = ureq::Agent::config_builder()
            .https_only(true)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        Ok(Self {
            token,
            agent: config.new_agent(),
        })
    }

    fn get(&self, path: &str, max_price: Option<f64>) -> Result<String, SmsError> {
        let url = format!("{BASE_URL}{path}");
        let authorization = format!("Bearer {}", self.token);
        let mut request = self
            .agent
            .get(&url)
            .header("Authorization", &authorization)
            .header("Accept", "application/json");
        if let Some(price) = max_price {
            request = request.query("maxPrice", price.to_string());
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
            return Err(parse_http_error(status, &body));
        }
        Ok(body)
    }
}

impl SmsProvider for FiveSimProvider {
    fn set_http_proxy(&mut self, proxy: &str) -> Result<(), SmsError> {
        self.agent = super::provider::proxy_agent(PROVIDER_NAME, proxy)?;
        Ok(())
    }
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn countries(&self) -> Result<Vec<super::SmsCountry>, SmsError> {
        let value: serde_json::Value = parse_json(&self.get("/v1/guest/countries", None)?)?;
        let countries = value.as_object().ok_or_else(|| SmsError::new(PROVIDER_NAME, SmsErrorKind::InvalidResponse, "invalid countries"))?;
        Ok(countries.iter().map(|(id, country)| super::SmsCountry {
            id: id.clone(), name: country.get("text_en").and_then(serde_json::Value::as_str).unwrap_or(id).to_string(),
        }).collect())
    }

    fn prices(&self, service: &str, country: Option<&str>) -> Result<Vec<super::SmsPrice>, SmsError> {
        let service = five_sim_service_name(service);
        validate_path_segment(service, "service")?;
        let mut path = format!("/v1/guest/prices?product={service}");
        if let Some(country) = country {
            validate_path_segment(country, "country")?;
            path.push_str(&format!("&country={country}"));
        }
        let value: serde_json::Value = parse_json(&self.get(&path, None)?)?;
        let countries = value.as_object().ok_or_else(|| SmsError::new(PROVIDER_NAME, SmsErrorKind::InvalidResponse, "invalid prices"))?;
        let mut result = Vec::new();
        for (country, products) in countries {
            if let Some(operators) = products.get(service).and_then(serde_json::Value::as_object) {
                for (operator, details) in operators {
                    if let Some(price) = details.get("cost").and_then(serde_json::Value::as_f64) {
                        result.push(super::SmsPrice {
                            country: super::SmsCountry { id: country.clone(), name: country.clone() },
                            service: service.to_string(), operator: Some(operator.clone()), price,
                            available: details.get("count").and_then(serde_json::Value::as_u64),
                        });
                    }
                }
            }
        }
        Ok(result)
    }

    fn balance(&self) -> Result<f64, SmsError> {
        let body = self.get("/v1/user/profile", None)?;
        let profile: ProfileResponse = parse_json(&body)?;
        Ok(profile.balance)
    }

    fn acquire_number(&self, request: &NumberRequest) -> Result<Activation, SmsError> {
        validate_request(request)?;
        let operator = request.operator.as_deref().unwrap_or("any");
        let service = five_sim_service_name(&request.service);
        validate_path_segment(&request.country, "country")?;
        validate_path_segment(operator, "operator")?;
        validate_path_segment(service, "service")?;

        let path = format!(
            "/v1/user/buy/activation/{}/{}/{}",
            request.country, operator, service
        );
        let body = self.get(&path, request.max_price)?;
        let order: OrderResponse = parse_json(&body)?;
        Ok(order.into_activation())
    }

    fn activation_status(&self, activation_id: &str) -> Result<ActivationStatus, SmsError> {
        validate_id(activation_id)?;
        let body = self.get(&format!("/v1/user/check/{activation_id}"), None)?;
        let order: OrderResponse = parse_json(&body)?;
        Ok(order.into_status())
    }

    fn update_activation(
        &self,
        activation_id: &str,
        action: ActivationAction,
    ) -> Result<ActionResult, SmsError> {
        validate_id(activation_id)?;
        let (path, result) = match action {
            ActivationAction::Finish => ("finish", ActionResult::Finished),
            ActivationAction::Cancel => ("cancel", ActionResult::Cancelled),
            ActivationAction::Ban => ("ban", ActionResult::Cancelled),
            ActivationAction::MarkReady | ActivationAction::RequestAnotherCode => {
                return Err(SmsError::new(
                    PROVIDER_NAME,
                    SmsErrorKind::Unsupported,
                    "5SIM does not support this activation action",
                ));
            }
        };
        let body = self.get(&format!("/v1/user/{path}/{activation_id}"), None)?;
        let _: OrderResponse = parse_json(&body)?;
        Ok(result)
    }
}

#[derive(Deserialize)]
struct ProfileResponse {
    balance: f64,
}

#[derive(Deserialize)]
struct OrderResponse {
    id: u64,
    phone: String,
    #[serde(default)]
    operator: Option<String>,
    #[serde(default)]
    price: Option<f64>,
    status: String,
    #[serde(default)]
    expires: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    sms: Option<Vec<SmsMessage>>,
}

#[derive(Deserialize)]
struct SmsMessage {
    #[serde(default)]
    code: String,
    #[serde(default)]
    text: Option<String>,
}

impl OrderResponse {
    fn into_activation(self) -> Activation {
        Activation {
            id: self.id.to_string(),
            phone: self.phone,
            cost: self.price,
            country: self.country,
            operator: self.operator,
            expires_at: self.expires,
        }
    }

    fn into_status(self) -> ActivationStatus {
        if let Some(message) = self
            .sms
            .unwrap_or_default()
            .into_iter()
            .rev()
            .find(|message| !message.code.trim().is_empty())
        {
            return ActivationStatus::CodeReceived {
                code: message.code,
                text: message.text,
            };
        }

        match self.status.as_str() {
            "PENDING" | "RECEIVED" => ActivationStatus::Waiting,
            "CANCELED" => ActivationStatus::Cancelled,
            "TIMEOUT" => ActivationStatus::Expired,
            "FINISHED" => ActivationStatus::Finished,
            "BANNED" => ActivationStatus::Banned,
            status => ActivationStatus::Provider(status.to_string()),
        }
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

fn parse_http_error(status: u16, body: &str) -> SmsError {
    let normalized = body.trim().trim_matches('"').to_ascii_lowercase();
    let kind = if status == 401 || status == 403 {
        SmsErrorKind::Authentication
    } else if normalized.contains("no free phones") {
        SmsErrorKind::NoNumbers
    } else if normalized.contains("not enough user balance") {
        SmsErrorKind::InsufficientBalance
    } else if normalized.contains("order not found") {
        SmsErrorKind::ActivationNotFound
    } else if status == 429 {
        SmsErrorKind::RateLimited
    } else if status == 400 {
        SmsErrorKind::InvalidRequest
    } else {
        SmsErrorKind::Provider
    };
    SmsError::new(
        PROVIDER_NAME,
        kind,
        format!("HTTP {status}: {}", body.trim()),
    )
}

fn validate_request(request: &NumberRequest) -> Result<(), SmsError> {
    if request.country.trim().is_empty() || request.service.trim().is_empty() {
        return Err(SmsError::new(
            PROVIDER_NAME,
            SmsErrorKind::InvalidRequest,
            "country and service are required",
        ));
    }
    if request
        .max_price
        .is_some_and(|price| !price.is_finite() || price <= 0.0)
    {
        return Err(SmsError::new(
            PROVIDER_NAME,
            SmsErrorKind::InvalidRequest,
            "max price must be positive and finite",
        ));
    }
    Ok(())
}

fn validate_path_segment(value: &str, name: &str) -> Result<(), SmsError> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(SmsError::new(
            PROVIDER_NAME,
            SmsErrorKind::InvalidRequest,
            format!("{name} contains invalid characters"),
        ));
    }
    Ok(())
}

fn validate_id(activation_id: &str) -> Result<(), SmsError> {
    validate_path_segment(activation_id, "activation id")?;
    if !activation_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(SmsError::new(
            PROVIDER_NAME,
            SmsErrorKind::InvalidRequest,
            "activation id must contain only digits",
        ));
    }
    Ok(())
}

fn five_sim_service_name(service: &str) -> &str {
    if service.eq_ignore_ascii_case("tg") {
        "telegram"
    } else {
        service
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_activation() {
        let order: OrderResponse = parse_json(
            r#"{"id":123,"phone":"+79990001122","operator":"any","price":12.5,"status":"PENDING","expires":"2026-08-26T12:00:00Z","country":"russia","sms":null}"#,
        )
        .unwrap();
        let activation = order.into_activation();
        assert_eq!(activation.id, "123");
        assert_eq!(activation.phone, "+79990001122");
        assert_eq!(activation.cost, Some(12.5));
    }

    #[test]
    fn extracts_latest_sms_code() {
        let order: OrderResponse = parse_json(
            r#"{"id":123,"phone":"+79990001122","status":"RECEIVED","sms":[{"code":"11111","text":"old"},{"code":"22222","text":"new"}]}"#,
        )
        .unwrap();
        assert_eq!(
            order.into_status(),
            ActivationStatus::CodeReceived {
                code: "22222".to_string(),
                text: Some("new".to_string()),
            }
        );
    }

    #[test]
    fn maps_http_errors() {
        assert_eq!(
            parse_http_error(400, "no free phones").kind,
            SmsErrorKind::NoNumbers
        );
        assert_eq!(
            parse_http_error(401, "unauthorized").kind,
            SmsErrorKind::Authentication
        );
    }
}
