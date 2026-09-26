use serde::{Deserialize, Serialize};

use super::{FiveSimProvider, OnlineSimProvider, SmsActivateProvider, SmsError, SmsErrorKind, SmsProvider, VakSmsProvider};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmsProviderKind {
    GrizzlySms,
    SmsBower,
    SmsMan,
    Sms365,
    Sms24_7,
    GreedySms,
    BringSms,
    FiveSim,
    VakSms,
    OnlineSim,
    CustomSmsActivate,
}

impl SmsProviderKind {
    pub const fn all() -> &'static [Self] {
        &[
            Self::GrizzlySms,
            Self::SmsBower,
            Self::SmsMan,
            Self::Sms365,
            Self::Sms24_7,
            Self::GreedySms,
            Self::BringSms,
            Self::FiveSim,
            Self::VakSms,
            Self::OnlineSim,
            Self::CustomSmsActivate,
        ]
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::GrizzlySms => "Grizzly SMS",
            Self::SmsBower => "SMS Bower",
            Self::SmsMan => "SMS-Man",
            Self::Sms365 => "365SMS",
            Self::Sms24_7 => "24SMS7",
            Self::GreedySms => "Greedy SMS",
            Self::BringSms => "BRINGSMS",
            Self::FiveSim => "5SIM",
            Self::VakSms => "Vak-SMS",
            Self::OnlineSim => "OnlineSIM",
            Self::CustomSmsActivate => "Custom SMS-Activate",
        }
    }

    pub const fn requires_endpoint(self) -> bool {
        matches!(self, Self::CustomSmsActivate)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsProviderConfig {
    pub kind: SmsProviderKind,
    pub api_key: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub http_proxy: Option<String>,
}

pub fn create_provider(config: SmsProviderConfig) -> Result<Box<dyn SmsProvider>, SmsError> {
    let mut provider: Box<dyn SmsProvider> = match config.kind {
        SmsProviderKind::GrizzlySms => Box::new(SmsActivateProvider::grizzly(config.api_key)?),
        SmsProviderKind::SmsBower => Box::new(SmsActivateProvider::sms_bower(config.api_key)?),
        SmsProviderKind::SmsMan => Box::new(SmsActivateProvider::sms_man(config.api_key)?),
        SmsProviderKind::Sms365 => Box::new(SmsActivateProvider::sms_365(config.api_key)?),
        SmsProviderKind::Sms24_7 => Box::new(SmsActivateProvider::sms_24_7(config.api_key)?),
        SmsProviderKind::GreedySms => Box::new(SmsActivateProvider::greedy_sms(config.api_key)?),
        SmsProviderKind::BringSms => Box::new(SmsActivateProvider::bring_sms(config.api_key)?),
        SmsProviderKind::FiveSim => Box::new(FiveSimProvider::new(config.api_key)?),
        SmsProviderKind::VakSms => Box::new(VakSmsProvider::new(config.api_key)?),
        SmsProviderKind::OnlineSim => Box::new(OnlineSimProvider::new(config.api_key)?),
        SmsProviderKind::CustomSmsActivate => {
            let endpoint = config.endpoint.ok_or_else(|| {
                SmsError::new(
                    "Custom SMS provider",
                    SmsErrorKind::InvalidRequest,
                    "endpoint is required for a custom SMS-Activate provider",
                )
            })?;
            Box::new(SmsActivateProvider::new("Custom SMS provider", endpoint, config.api_key)?)
        }
    };
    if let Some(proxy) = config.http_proxy {
        provider.set_http_proxy(&proxy)?;
    }
    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_provider_requires_endpoint() {
        let result = create_provider(SmsProviderConfig {
            kind: SmsProviderKind::CustomSmsActivate,
            api_key: "key".to_string(),
            endpoint: None,
            http_proxy: None,
        });
        let error = match result {
            Ok(_) => panic!("custom provider without endpoint must fail"),
            Err(error) => error,
        };
        assert_eq!(error.kind, SmsErrorKind::InvalidRequest);
    }
}
