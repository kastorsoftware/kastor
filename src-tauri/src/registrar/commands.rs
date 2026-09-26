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
    tokio::task::spawn_blocking(move || {
        create_provider(config)
            .map_err(|error| error.to_string())?
            .balance()
            .map_err(|error| error.to_string())
    })
        .await
        .map_err(|error| format!("SMS provider task failed: {error}"))?
}

#[tauri::command]
pub async fn registrar_sms_countries(config: SmsProviderConfig) -> Result<Vec<SmsCountry>, String> {
    tokio::task::spawn_blocking(move || {
        create_provider(config)
            .map_err(|error| error.to_string())?
            .countries()
            .map_err(|error| error.to_string())
    })
        .await
        .map_err(|error| format!("SMS provider task failed: {error}"))?
}

#[tauri::command]
pub async fn registrar_sms_operators(
    config: SmsProviderConfig,
    country: String,
) -> Result<Vec<SmsOperator>, String> {
    tokio::task::spawn_blocking(move || {
        create_provider(config)
            .map_err(|error| error.to_string())?
            .operators(&country)
            .map_err(|error| error.to_string())
    })
        .await
        .map_err(|error| format!("SMS provider task failed: {error}"))?
}

#[tauri::command]
pub async fn registrar_sms_prices(
    config: SmsProviderConfig,
    service: String,
    country: Option<String>,
) -> Result<Vec<SmsPrice>, String> {
    tokio::task::spawn_blocking(move || {
        create_provider(config)
            .map_err(|error| error.to_string())?
            .prices(&service, country.as_deref())
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("SMS provider task failed: {error}"))?
}
