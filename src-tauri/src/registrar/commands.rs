use crate::registrar::sms::{
    create_provider, SmsCountry, SmsOperator, SmsPrice, SmsProviderConfig,
};
use crate::registrar::profile::{ProfileGeneratorConfig, RegistrationProfile};
use crate::registrar::db::{RegistrarDatabase, StoredRegistrationRecord};

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
