use std::time::Duration;

use serde::Deserialize;

use super::provider::{
    ActionResult, Activation, ActivationAction, ActivationStatus, NumberRequest, SmsError,
    SmsErrorKind, SmsProvider,
};

const PROVIDER_NAME: &str = "OnlineSIM";
const BASE_URL: &str = "https://onlinesim.io/api";

#[derive(Clone)]
pub struct OnlineSimProvider {
    api_key: String,
    agent: ureq::Agent,
}

impl OnlineSimProvider {
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
        let url = format!("{BASE_URL}/{method}.php");
        let mut request = self.agent.get(&url).query("apikey", &self.api_key);
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
            return Err(SmsError::new(
                PROVIDER_NAME,
                SmsErrorKind::Provider,
                format!("HTTP {status}: {}", body.trim()),
            ));
        }
        Ok(body)
    }

    fn state(&self, activation_id: &str) -> Result<OnlineSimOperation, SmsError> {
        let body = self.request(
            "getState",
            &[
                ("tzid", activation_id.to_string()),
                ("message_to_code", "1".to_string()),
                ("msg_list", "0".to_string()),
            ],
        )?;
        parse_operation(&body)
    }
}

impl SmsProvider for OnlineSimProvider {
    fn set_http_proxy(&mut self, proxy: &str) -> Result<(), SmsError> {
        self.agent = super::provider::proxy_agent(PROVIDER_NAME, proxy)?;
        Ok(())
    }
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn countries(&self) -> Result<Vec<super::SmsCountry>, SmsError> {
        let value: serde_json::Value =
            parse_json(&self.request("getTariffs", &[("lang", "en".into())])?)?;
        ensure_response_success(
            value
                .get("response")
                .ok_or_else(|| response_error("missing response status"))?,
        )?;
        let countries = value
            .get("countries")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| response_error("missing countries"))?;
        Ok(countries
            .values()
            .filter_map(|country| {
                Some(super::SmsCountry {
                    id: country.get("code")?.as_u64()?.to_string(),
                    name: country.get("name")?.as_str()?.to_string(),
                })
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
                "getTariffs",
                &[
                    ("country", country.id.clone()),
                    ("filter_service", service_name(service).into()),
                    ("lang", "en".into()),
                ],
            )?)?;
            ensure_response_success(
                value
                    .get("response")
                    .ok_or_else(|| response_error("missing response status"))?,
            )?;
            if let Some(services) = value.get("services").and_then(serde_json::Value::as_object) {
                for details in services.values() {
                    if details.get("slug").and_then(serde_json::Value::as_str)
                        != Some(service_name(service))
                    {
                        continue;
                    }
                    let price = details
                        .get("price")
                        .and_then(|price| price.as_f64().or_else(|| price.as_str()?.parse().ok()));
                    if let Some(price) = price {
                        result.push(super::SmsPrice {
                            country: country.clone(),
                            service: service.into(),
                            operator: None,
                            price,
                            available: details.get("count").and_then(serde_json::Value::as_u64),
                        });
                    }
                }
            }
        }
        Ok(result)
    }

    fn balance(&self) -> Result<f64, SmsError> {
        let response: BalanceResponse = parse_json(&self.request("getBalance", &[])?)?;
        ensure_response_success(&response.response)?;
        response.balance.parse().map_err(|_| {
            SmsError::new(
                PROVIDER_NAME,
                SmsErrorKind::InvalidResponse,
                "invalid balance response",
            )
        })
    }

    fn acquire_number(&self, request: &NumberRequest) -> Result<Activation, SmsError> {
        validate_request(request)?;
        let body = self.request(
            "getNum",
            &[
                ("service", service_name(&request.service).to_string()),
                ("country", request.country.clone()),
                ("number", "true".to_string()),
            ],
        )?;
        let response: BuyResponse = parse_json(&body)?;
        ensure_response_success(&response.response)?;
        let id = response.tzid.to_string();
        let operation = self.state(&id)?;
        let phone = operation.number.ok_or_else(|| {
            SmsError::new(
                PROVIDER_NAME,
                SmsErrorKind::InvalidResponse,
                "number is missing from getState response",
            )
        })?;
        Ok(Activation {
            id,
            phone,
            cost: operation.sum,
            country: operation.country.map(|country| country.to_string()),
            operator: None,
            expires_at: None,
        })
    }

    fn activation_status(&self, activation_id: &str) -> Result<ActivationStatus, SmsError> {
        validate_id(activation_id)?;
        let operation = self.state(activation_id)?;
        let status = operation.response.as_deref().unwrap_or_default();
        match status {
            "TZ_NUM_WAIT" | "TZ_INPOOL" => Ok(ActivationStatus::Waiting),
            "TZ_NUM_ANSWER" | "TZ_NUM_OK" => operation
                .msg
                .filter(|code| !code.trim().is_empty())
                .map(|code| ActivationStatus::CodeReceived { code, text: None })
                .ok_or_else(|| {
                    SmsError::new(
                        PROVIDER_NAME,
                        SmsErrorKind::InvalidResponse,
                        "SMS code is missing from getState response",
                    )
                }),
            "TZ_NUM_CANCEL" | "TZ_OVER" => Ok(ActivationStatus::Cancelled),
            "TZ_NUM_EXPIRED" => Ok(ActivationStatus::Expired),
            _ => Err(response_error(status)),
        }
    }

    fn update_activation(
        &self,
        activation_id: &str,
        action: ActivationAction,
    ) -> Result<ActionResult, SmsError> {
        validate_id(activation_id)?;
        match action {
            ActivationAction::MarkReady => Ok(ActionResult::Ready),
            ActivationAction::RequestAnotherCode => {
                let body =
                    self.request("setOperationRevise", &[("tzid", activation_id.to_string())])?;
                let response: ActionResponse = parse_json(&body)?;
                ensure_response_success(&response.response)?;
                Ok(ActionResult::WaitingForRetry)
            }
            ActivationAction::Finish | ActivationAction::Cancel | ActivationAction::Ban => {
                let body =
                    self.request("setOperationOk", &[("tzid", activation_id.to_string())])?;
                let response: ActionResponse = parse_json(&body)?;
                ensure_response_success(&response.response)?;
                Ok(if action == ActivationAction::Finish {
                    ActionResult::Finished
                } else {
                    ActionResult::Cancelled
                })
            }
        }
    }
}

#[derive(Deserialize)]
struct BalanceResponse {
    response: serde_json::Value,
    balance: String,
}

#[derive(Deserialize)]
struct BuyResponse {
    response: serde_json::Value,
    tzid: u64,
}

#[derive(Deserialize)]
struct ActionResponse {
    response: serde_json::Value,
}

#[derive(Deserialize)]
struct OnlineSimOperation {
    response: Option<String>,
    number: Option<String>,
    country: Option<serde_json::Value>,
    msg: Option<String>,
    sum: Option<f64>,
}

fn parse_operation(body: &str) -> Result<OnlineSimOperation, SmsError> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|error| {
        SmsError::new(
            PROVIDER_NAME,
            SmsErrorKind::InvalidResponse,
            format!("invalid JSON response: {error}"),
        )
    })?;
    if let Some(items) = value.as_array() {
        return items
            .first()
            .ok_or_else(|| response_error("empty getState response"))
            .and_then(|item| parse_json(&item.to_string()));
    }
    parse_json(body)
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

fn ensure_response_success(response: &serde_json::Value) -> Result<(), SmsError> {
    if response == 1 || response == "1" {
        return Ok(());
    }
    Err(response_error(
        response.as_str().unwrap_or("request rejected"),
    ))
}

fn response_error(code: &str) -> SmsError {
    let kind = match code {
        "ERROR_WRONG_KEY" => SmsErrorKind::Authentication,
        "ERROR_NO_SERVICE" | "ERROR_NO_TZID" => SmsErrorKind::InvalidRequest,
        "ERROR_NO_NUMBERS" | "TZ_INPOOL" => SmsErrorKind::NoNumbers,
        "ERROR_NO_BALANCE" => SmsErrorKind::InsufficientBalance,
        "ERROR_NO_OPERATION" => SmsErrorKind::ActivationNotFound,
        "ERROR_WAIT" => SmsErrorKind::RateLimited,
        _ => SmsErrorKind::Provider,
    };
    SmsError::new(PROVIDER_NAME, kind, code)
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
        "telegram"
    } else {
        service
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_operation_array() {
        let operation = parse_operation(
            r#"[{"tzid":10000,"response":"TZ_NUM_ANSWER","number":"+37360000000","msg":"123456"}]"#,
        )
        .unwrap();
        assert_eq!(operation.msg.as_deref(), Some("123456"));
    }

    #[test]
    fn maps_known_errors() {
        assert_eq!(
            response_error("ERROR_WRONG_KEY").kind,
            SmsErrorKind::Authentication
        );
        assert_eq!(
            response_error("ERROR_NO_NUMBERS").kind,
            SmsErrorKind::NoNumbers
        );
    }
}
