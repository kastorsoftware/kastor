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

impl NumberRequest {
    pub fn telegram(country: impl Into<String>) -> Self {
        Self {
            country: country.into(),
            service: "telegram".to_string(),
            operator: None,
            max_price: None,
        }
    }
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
    fn balance(&self) -> Result<f64, SmsError>;
    fn acquire_number(&self, request: &NumberRequest) -> Result<Activation, SmsError>;
    fn activation_status(&self, activation_id: &str) -> Result<ActivationStatus, SmsError>;
    fn update_activation(
        &self,
        activation_id: &str,
        action: ActivationAction,
    ) -> Result<ActionResult, SmsError>;
}
