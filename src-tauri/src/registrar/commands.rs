use crate::registrar::sms::{
    create_provider, SmsCountry, SmsOperator, SmsPrice, SmsProviderConfig, SmsProviderKind,
};
use crate::registrar::profile::{ProfileGeneratorConfig, RegistrationProfile};
use crate::registrar::db::{RegistrarDatabase, StoredRegistrationRecord};
use crate::registrar::antisafety::{
    AntiSafetyBalance, AntiSafetyClient, AntiSafetyConfig, AntiSafetyEmailTask,
    AntiSafetyGoogleAuthTask, AntiSafetyLoginTask, AntiSafetyPushToken, AntiSafetyTask,
    AntiSafetyToken,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SmsProviderDescriptor {
    pub kind: SmsProviderKind,
    pub label: String,
    pub requires_endpoint: bool,
}

#[tauri::command]
pub fn registrar_sms_providers() -> Vec<SmsProviderDescriptor> {
    SmsProviderKind::all()
        .iter()
        .copied()
        .map(|kind| SmsProviderDescriptor {
            kind,
            label: kind.label().to_string(),
            requires_endpoint: kind.requires_endpoint(),
        })
        .collect()
}

#[tauri::command]
pub fn registrar_generate_profiles(
    config: ProfileGeneratorConfig,
    count: usize,
) -> Result<Vec<RegistrationProfile>, String> {
    config.generate(count)
}

#[tauri::command]
pub async fn registrar_results(task_id: String) -> Result<Vec<StoredRegistrationRecord>, String> {
    tokio::task::spawn_blocking(move || RegistrarDatabase::open_task(&task_id)?.records())
        .await
        .map_err(|error| format!("registrar database task failed: {error}"))?
}

#[tauri::command]
pub async fn registrar_antisafety_balance(config: AntiSafetyConfig) -> Result<AntiSafetyBalance, String> {
    antisafety_call(config, |client| client.balance()).await
}

#[tauri::command]
pub async fn registrar_antisafety_create_play_integrity(
    config: AntiSafetyConfig,
    standard: bool,
    nonce: String,
    version: i32,
) -> Result<AntiSafetyTask, String> {
    antisafety_call(config, move |client| client.create_play_integrity(standard, &nonce, version)).await
}

#[tauri::command]
pub async fn registrar_antisafety_get_play_integrity(
    config: AntiSafetyConfig,
    id: String,
) -> Result<AntiSafetyToken, String> {
    antisafety_call(config, move |client| client.get_play_integrity(&id)).await
}

#[tauri::command]
pub async fn registrar_antisafety_create_android_push(
    config: AntiSafetyConfig,
) -> Result<AntiSafetyTask, String> {
    antisafety_call(config, |client| client.create_android_push()).await
}

#[tauri::command]
pub async fn registrar_antisafety_get_android_push(
    config: AntiSafetyConfig,
    id: String,
) -> Result<AntiSafetyPushToken, String> {
    antisafety_call(config, move |client| client.get_android_push(&id)).await
}

#[tauri::command]
pub async fn registrar_antisafety_create_email(
    config: AntiSafetyConfig,
    premium: bool,
) -> Result<AntiSafetyEmailTask, String> {
    antisafety_call(config, move |client| client.create_email(premium)).await
}

#[tauri::command]
pub async fn registrar_antisafety_get_email(
    config: AntiSafetyConfig,
    id: String,
    premium: bool,
) -> Result<String, String> {
    antisafety_call(config, move |client| client.get_email(&id, premium)).await
}

#[tauri::command]
pub async fn registrar_antisafety_create_google_auth(
    config: AntiSafetyConfig,
) -> Result<AntiSafetyGoogleAuthTask, String> {
    antisafety_call(config, |client| client.create_google_auth()).await
}

#[tauri::command]
pub async fn registrar_antisafety_create_login(
    config: AntiSafetyConfig,
) -> Result<AntiSafetyLoginTask, String> {
    antisafety_call(config, |client| client.create_login()).await
}

#[tauri::command]
pub async fn registrar_antisafety_create_recaptcha(
    config: AntiSafetyConfig,
    action: String,
    app_key: String,
    high_score: bool,
) -> Result<AntiSafetyTask, String> {
    antisafety_call(config, move |client| client.create_recaptcha(&action, &app_key, high_score)).await
}

#[tauri::command]
pub async fn registrar_antisafety_get_recaptcha(
    config: AntiSafetyConfig,
    id: String,
) -> Result<AntiSafetyToken, String> {
    antisafety_call(config, move |client| client.get_recaptcha(&id)).await
}

async fn antisafety_call<T, F>(config: AntiSafetyConfig, operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(AntiSafetyClient) -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(move || operation(AntiSafetyClient::new(config)?))
        .await
        .map_err(|error| format!("AntiSafety task failed: {error}"))?
}

#[tauri::command]
pub async fn registrar_sms_balance(config: SmsProviderConfig) -> Result<f64, String> {
    sms_catalog_call(config, "balance", |provider| provider.balance()).await
}

#[tauri::command]
pub async fn registrar_sms_countries(config: SmsProviderConfig) -> Result<Vec<SmsCountry>, String> {
    sms_catalog_call(config, "countries", |provider| provider.countries()).await
}

#[tauri::command]
pub async fn registrar_sms_operators(
    config: SmsProviderConfig,
    country: String,
) -> Result<Vec<SmsOperator>, String> {
    sms_catalog_call(config, "operators", move |provider| provider.operators(&country)).await
}

#[tauri::command]
pub async fn registrar_sms_prices(
    config: SmsProviderConfig,
    service: String,
    country: Option<String>,
) -> Result<Vec<SmsPrice>, String> {
    sms_catalog_call(config, "prices", move |provider| {
        provider.prices(&service, country.as_deref())
    }).await
}

async fn sms_catalog_call<T, F>(
    config: SmsProviderConfig,
    operation_name: &'static str,
    operation: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&dyn crate::registrar::sms::SmsProvider)
            -> Result<T, crate::registrar::sms::SmsError>
        + Send + 'static,
{
    let kind = config.kind;
    dbg_log!("registrar::catalog provider={:?} operation={} started", kind, operation_name);
    tokio::task::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let result = create_provider(config).and_then(|provider| operation(provider.as_ref()));
        // Provider errors may contain request URLs, so only their category is logged.
        match &result {
            Ok(_) => dbg_log!(
                "registrar::catalog provider={:?} operation={} completed duration_ms={}",
                kind, operation_name, started.elapsed().as_millis()
            ),
            Err(error) => dbg_log!(
                "registrar::catalog provider={:?} operation={} failed kind={:?} duration_ms={}",
                kind, operation_name, error.kind, started.elapsed().as_millis()
            ),
        }
        result.map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "SMS catalog worker failed".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> SmsProviderConfig {
        SmsProviderConfig {
            kind: SmsProviderKind::CustomSmsActivate,
            api_key: "test-key".to_string(),
            endpoint: Some("https://example.com/stubs/handler_api.php".to_string()),
        }
    }

    #[tokio::test]
    async fn catalog_worker_returns_operation_result_without_network() {
        let result = sms_catalog_call(config(), "test", |_| Ok(42_u32)).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn catalog_worker_preserves_operation_error() {
        let result = sms_catalog_call::<(), _>(config(), "test", |provider| {
            Err(crate::registrar::sms::SmsError::new(
                provider.name(),
                crate::registrar::sms::SmsErrorKind::Unsupported,
                "unsupported test operation",
            ))
        }).await;
        assert!(result.unwrap_err().contains("unsupported test operation"));
    }
}
