use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NumberRequest {
    pub country: String,
    pub service: String,
    pub operator: Option<String>,
    pub max_price: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmsCountry {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmsOperator {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmsPrice {
    pub country: SmsCountry,
    pub service: String,
    pub operator: Option<String>,
    pub price: f64,
    pub available: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Activation {
    pub id: String,
    pub phone: String,
    pub cost: Option<f64>,
    pub country: Option<String>,
    pub operator: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum ActivationStatus {
    Waiting,
    WaitingForRetry { last_code: Option<String> },
    WaitingForResend,
    CodeReceived { code: String, text: Option<String> },
    Cancelled,
    Finished,
    Banned,
    Expired,
    Provider(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationAction {
    MarkReady,
    RequestAnotherCode,
    Finish,
    Cancel,
    Ban,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionResult {
    Ready,
    WaitingForRetry,
    Finished,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmsErrorKind {
    Authentication,
    NoNumbers,
    InsufficientBalance,
    InvalidRequest,
    ActivationNotFound,
    TooEarlyToCancel,
    RateLimited,
    Unsupported,
    Transport,
    InvalidResponse,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsError {
    pub provider: String,
    pub kind: SmsErrorKind,
    pub message: String,
}

impl SmsError {
    pub(crate) fn new(
        provider: impl Into<String>,
        kind: SmsErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for SmsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.provider, self.message)
    }
}

impl Error for SmsError {}

// Implementations perform blocking HTTP requests. Call them from spawn_blocking in async flows.
pub trait SmsProvider: Send + Sync {
    fn name(&self) -> &str;
    fn set_http_proxy(&mut self, proxy: &str) -> Result<(), SmsError>;
    fn balance(&self) -> Result<f64, SmsError>;
    fn countries(&self) -> Result<Vec<SmsCountry>, SmsError> {
        Err(SmsError::new(
            self.name(),
            SmsErrorKind::Unsupported,
            "listing countries is not supported",
        ))
    }
    fn operators(&self, _country: &str) -> Result<Vec<SmsOperator>, SmsError> {
        Err(SmsError::new(
            self.name(),
            SmsErrorKind::Unsupported,
            "listing operators is not supported",
        ))
    }
    fn prices(&self, _service: &str, _country: Option<&str>) -> Result<Vec<SmsPrice>, SmsError> {
        Err(SmsError::new(
            self.name(),
            SmsErrorKind::Unsupported,
            "listing prices is not supported",
        ))
    }
    fn acquire_number(&self, request: &NumberRequest) -> Result<Activation, SmsError>;
    fn activation_status(&self, activation_id: &str) -> Result<ActivationStatus, SmsError>;
    fn update_activation(
        &self,
        activation_id: &str,
        action: ActivationAction,
    ) -> Result<ActionResult, SmsError>;
}

pub(super) fn proxy_agent(provider: &str, value: &str) -> Result<ureq::Agent, SmsError> {
    let proxy = ureq::Proxy::new(value).map_err(|_| {
        SmsError::new(
            provider,
            SmsErrorKind::InvalidRequest,
            "invalid SMS HTTP proxy",
        )
    })?;
    Ok(ureq::Agent::config_builder()
        .https_only(true)
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .proxy(Some(proxy))
        .build()
        .new_agent())
}
