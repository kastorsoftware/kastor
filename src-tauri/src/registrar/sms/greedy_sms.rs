use serde::Deserialize;
use serde_json::{json, Value};

use super::provider::*;
use super::sms_activate::SmsActivateProvider;

const NAME: &str = "Greedy SMS";
const API: &str = "https://api.greedy-sms.com";
const PAGE_SIZE: u32 = 100;
const MAX_PAGES: u32 = 100;

pub struct GreedySmsProvider {
    api_key: String,
    agent: ureq::Agent,
    compatible: SmsActivateProvider,
}

impl GreedySmsProvider {
    pub fn new(api_key: String) -> Result<Self, SmsError> {
        let compatible = SmsActivateProvider::greedy_sms(&api_key)?;
        Ok(Self {
            api_key,
            compatible,
            agent: ureq::Agent::config_builder()
                .https_only(true)
                .http_status_as_error(false)
                .timeout_global(Some(std::time::Duration::from_secs(30)))
                .build()
                .new_agent(),
        })
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, SmsError> {
        let response = self
            .agent
            .post(format!("{API}{path}"))
            .header("x-api-key", &self.api_key)
            .send_json(body)
            .map_err(|_| error(SmsErrorKind::Transport, "catalog connection failed"))?;
        let status = response.status().as_u16();
        // Never surface remote bodies or HTTP errors containing credentials.
        if !(200..300).contains(&status) {
            return Err(error(
                match status {
                    401 | 403 => SmsErrorKind::Authentication,
                    429 => SmsErrorKind::RateLimited,
                    400 | 422 => SmsErrorKind::InvalidRequest,
                    _ => SmsErrorKind::Provider,
                },
                &format!("catalog request failed (HTTP {status})"),
            ));
        }
        response
            .into_body()
            .read_json()
            .map_err(|_| error(SmsErrorKind::InvalidResponse, "invalid catalog JSON"))
    }

    fn pages(&self, path: &str, body: Value) -> Result<Vec<Value>, SmsError> {
        collect_pages(body, |body| self.post(path, body))
    }
}

fn error(kind: SmsErrorKind, message: &str) -> SmsError {
    SmsError::new(NAME, kind, message)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    countries: Vec<Value>,
    total_pages: u32,
}

fn collect_pages(
    mut body: Value,
    mut request: impl FnMut(&Value) -> Result<Value, SmsError>,
) -> Result<Vec<Value>, SmsError> {
    let mut rows = Vec::new();
    for page in 1..=MAX_PAGES {
        body["page"] = json!(page);
        body["pageSize"] = json!(PAGE_SIZE);
        let result: Page = serde_json::from_value(request(&body)?)
            .map_err(|_| error(SmsErrorKind::InvalidResponse, "invalid catalog page"))?;
        if result.total_pages > MAX_PAGES {
            return Err(error(
                SmsErrorKind::InvalidResponse,
                "catalog exceeds page limit",
            ));
        }
        rows.extend(result.countries);
        if page >= result.total_pages {
            return Ok(rows);
        }
    }
    Err(error(
        SmsErrorKind::InvalidResponse,
        "catalog page limit reached",
    ))
}

#[derive(Deserialize)]
struct Country {
    id: u32,
    title: CountryTitle,
}

#[derive(Deserialize)]
struct CountryTitle {
    eng: String,
    rus: String,
}

fn parse_countries(rows: Vec<Value>) -> Result<Vec<SmsCountry>, SmsError> {
    rows.into_iter()
        .map(|row| {
            let country: Country = serde_json::from_value(row)
                .map_err(|_| error(SmsErrorKind::InvalidResponse, "invalid country"))?;
            let name = if country.title.eng.trim().is_empty() {
                country.title.rus
            } else {
                country.title.eng
            };
            Ok(SmsCountry {
                id: country.id.to_string(),
                name,
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct CountryPrices {
    country: u32,
    services: Vec<ServicePrice>,
}

#[derive(Deserialize)]
struct ServicePrice {
    price: f64,
    count: u64,
}

fn parse_prices(rows: Vec<Value>, service: &str) -> Result<Vec<SmsPrice>, SmsError> {
    let mut prices = Vec::new();
    for row in rows {
        let country: CountryPrices = serde_json::from_value(row)
            .map_err(|_| error(SmsErrorKind::InvalidResponse, "invalid country prices"))?;
        for offer in country.services {
            if !offer.price.is_finite() || offer.price < 0.0 {
                return Err(error(
                    SmsErrorKind::InvalidResponse,
                    "invalid service price",
                ));
            }
            prices.push(SmsPrice {
                country: SmsCountry {
                    id: country.country.to_string(),
                    name: country.country.to_string(),
                },
                service: service.to_string(),
                operator: None,
                price: offer.price,
                available: Some(offer.count),
            });
        }
    }
    Ok(prices)
}

impl SmsProvider for GreedySmsProvider {
    fn name(&self) -> &str {
        NAME
    }
    fn set_http_proxy(&mut self, proxy: &str) -> Result<(), SmsError> {
        self.agent = proxy_agent(NAME, proxy)?;
        self.compatible.set_http_proxy(proxy)
    }
    fn balance(&self) -> Result<f64, SmsError> {
        self.compatible.balance()
    }
    fn countries(&self) -> Result<Vec<SmsCountry>, SmsError> {
        parse_countries(self.pages("/activations/getCountries", json!({}))?)
    }
    fn prices(&self, service: &str, country: Option<&str>) -> Result<Vec<SmsPrice>, SmsError> {
        if service.trim().is_empty() {
            return Err(error(SmsErrorKind::InvalidRequest, "service is required"));
        }
        let mut body = json!({"service": super::sms_activate::compatible_service_name(service)});
        if let Some(country) = country {
            body["country"] = json!(country
                .parse::<u32>()
                .map_err(|_| error(SmsErrorKind::InvalidRequest, "invalid country ID"))?);
        }
        parse_prices(self.pages("/activations/getPrices", body)?, service)
    }
    fn operators(&self, country: &str) -> Result<Vec<SmsOperator>, SmsError> {
        let country_id = country
            .parse::<u32>()
            .map_err(|_| error(SmsErrorKind::InvalidRequest, "invalid country ID"))?;
        #[derive(Deserialize)]
        struct Operators {
            country: String,
            operators: Vec<String>,
        }
        let response: Vec<Operators> = serde_json::from_value(
            self.post("/activations/getOperators", &json!({"country":country_id}))?,
        )
        .map_err(|_| error(SmsErrorKind::InvalidResponse, "invalid operators"))?;
        Ok(response
            .into_iter()
            .filter(|entry| entry.country == country_id.to_string())
            .flat_map(|entry| {
                entry.operators.into_iter().map(|id| SmsOperator {
                    name: id.clone(),
                    id,
                })
            })
            .collect())
    }
    // Greedy explicitly documents the compatible API for the activation lifecycle.
    fn acquire_number(&self, request: &NumberRequest) -> Result<Activation, SmsError> {
        self.compatible.acquire_number(request)
    }
    fn activation_status(&self, id: &str) -> Result<ActivationStatus, SmsError> {
        self.compatible.activation_status(id)
    }
    fn update_activation(
        &self,
        id: &str,
        action: ActivationAction,
    ) -> Result<ActionResult, SmsError> {
        self.compatible.update_activation(id, action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loads_all_catalog_pages_and_preserves_filters() {
        let mut calls = 0;
        let rows = collect_pages(json!({"service":"tg","country":0}), |body| {
            calls += 1;
            assert_eq!(body["page"], calls);
            assert_eq!(body["service"], "tg");
            assert_eq!(body["country"], 0);
            Ok(json!({"countries":[{"id":calls}],"totalPages":2}))
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(rows.len(), 2);
    }
    #[test]
    fn parses_documented_country_and_price_shapes() {
        let countries = parse_countries(vec![
            json!({"id":0,"title":{"rus":"Россия","eng":"Russia"}}),
        ])
        .unwrap();
        assert_eq!(countries[0].id, "0");
        assert_eq!(countries[0].name, "Russia");
        let prices = parse_prices(vec![json!({"country":0,"services":[{"name":"Telegram","price":12.5,"count":100,"priceRange":[]} ]})],"telegram").unwrap();
        assert_eq!(prices[0].price, 12.5);
        assert_eq!(prices[0].available, Some(100));
        assert_eq!(prices[0].country.id, "0");
    }
    #[test]
    fn rejects_bad_pages_and_never_returns_partial_catalog() {
        assert!(
            collect_pages(json!({}), |_| Ok(json!({"countries":[],"totalPages":1000}))).is_err()
        );
        assert!(collect_pages(json!({}), |_| Ok(json!({"error":"BAD_KEY"}))).is_err());
        assert!(parse_prices(
            vec![json!({"country":0,"services":[{"price":-1,"count":2}]})],
            "tg"
        )
        .is_err());
        assert!(collect_pages(json!({}), |body| if body["page"] == 1 {
            Ok(json!({"countries":[{}],"totalPages":2}))
        } else {
            Err(error(SmsErrorKind::Transport, "test"))
        })
        .is_err());
    }
}
