use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tauri::{Emitter, Manager};

use crate::proxy::ProxyConfig;
use crate::queue::TaskQueue;
use crate::registrar::db::{RegistrarDatabase, RegistrationRecord};
use crate::registrar::flow::{
    RegistrationCodeRequest, RegistrationIdentity, RegistrationOutcome, RegistrationSession,
};
use crate::registrar::profile::RegistrationProfile;
use crate::registrar::sms::{
    create_provider, ActivationAction, ActivationStatus, NumberRequest,
    SmsProviderConfig,
};

const MIN_POLL_INTERVAL_SECS: u64 = 2;
const MAX_POLL_INTERVAL_SECS: u64 = 60;
const MIN_CODE_TIMEOUT_SECS: u64 = 30;
const MAX_CODE_TIMEOUT_SECS: u64 = 900;
const MAX_CONCURRENCY: usize = 20;

#[derive(Debug, Clone, Deserialize)]
pub struct RegistrationAttempt {
    pub profile: RegistrationProfile,
    pub identity: RegistrationIdentity,
    #[serde(default)]
    pub proxy: Option<ProxyConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegistrarTaskConfig {
    pub sms_provider: SmsProviderConfig,
    pub number_request: NumberRequest,
    pub attempts: Vec<RegistrationAttempt>,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_code_timeout_secs")]
    pub code_timeout_secs: u64,
}

fn default_concurrency() -> usize {
    1
}

fn default_poll_interval_secs() -> u64 {
    5
}

fn default_code_timeout_secs() -> u64 {
    300
}

impl RegistrarTaskConfig {
    fn validate(&self) -> Result<(), String> {
        if self.sms_provider.api_key.trim().is_empty() {
            return Err("SMS provider API key is required".to_string());
        }
        if self.number_request.country.trim().is_empty() || self.number_request.service.trim().is_empty() {
            return Err("SMS country and service are required".to_string());
        }
        if self.attempts.is_empty() || self.attempts.len() > 100_000 {
            return Err("registration task must contain from 1 to 100000 attempts".to_string());
        }
        if !(1..=MAX_CONCURRENCY).contains(&self.concurrency) {
            return Err(format!("registration concurrency must be between 1 and {MAX_CONCURRENCY}"));
        }
        if !(MIN_POLL_INTERVAL_SECS..=MAX_POLL_INTERVAL_SECS).contains(&self.poll_interval_secs) {
            return Err(format!(
                "SMS polling interval must be between {MIN_POLL_INTERVAL_SECS} and {MAX_POLL_INTERVAL_SECS} seconds"
            ));
        }
        if !(MIN_CODE_TIMEOUT_SECS..=MAX_CODE_TIMEOUT_SECS).contains(&self.code_timeout_secs) {
            return Err(format!(
                "SMS code timeout must be between {MIN_CODE_TIMEOUT_SECS} and {MAX_CODE_TIMEOUT_SECS} seconds"
            ));
        }
        for attempt in &self.attempts {
            attempt.profile.validate()?;
            attempt.identity.validate()?;
        }
        create_provider(self.sms_provider.clone()).map_err(|error| error.to_string())?;
        Ok(())
    }
}

#[tauri::command]
pub async fn registrar_start(
    config: RegistrarTaskConfig,
    app: tauri::AppHandle,
) -> Result<String, String> {
    config.validate()?;

    let task_id = uuid::Uuid::new_v4().to_string();
    let (database, database_path) = RegistrarDatabase::create(&task_id)?;
    let database = Arc::new(tokio::sync::Mutex::new(database));
    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "registrar".to_string(),
            format!("Account registration: {} attempts", config.attempts.len()),
        )
        .await;
    let _ = app.emit("registrar-log", format!("DB: {}", database_path.display()));

    let running_task_id = task_id.clone();
    tokio::spawn(async move {
        let success = run_task(config, database, app.clone(), token).await.is_ok();
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&running_task_id, success).await;
    });

    Ok(task_id)
}

#[tauri::command]
pub async fn registrar_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn run_task(
    config: RegistrarTaskConfig,
    database: Arc<tokio::sync::Mutex<RegistrarDatabase>>,
    app: tauri::AppHandle,
    token: Arc<AtomicBool>,
) -> Result<(), String> {
    let total = config.attempts.len();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(config.concurrency));
    let mut handles = Vec::with_capacity(total);

    for (index, attempt) in config.attempts.into_iter().enumerate() {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| format!("acquire registrar worker: {error}"))?;
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let sms_provider = config.sms_provider.clone();
        let number_request = config.number_request.clone();
        let database = database.clone();
        let app = app.clone();
        let token = token.clone();
        let poll_interval_secs = config.poll_interval_secs;
        let code_timeout_secs = config.code_timeout_secs;
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            process_attempt(
                index + 1,
                total,
                attempt,
                sms_provider,
                number_request,
                database,
                app,
                token,
                poll_interval_secs,
                code_timeout_secs,
            )
            .await;
        }));
    }

    for handle in handles {
        handle
            .await
            .map_err(|error| format!("registrar worker failed: {error}"))?;
    }
    let _ = app.emit("registrar-log", "Completed");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn process_attempt(
    index: usize,
    total: usize,
    attempt: RegistrationAttempt,
    sms_provider: SmsProviderConfig,
    number_request: NumberRequest,
    database: Arc<tokio::sync::Mutex<RegistrarDatabase>>,
    app: tauri::AppHandle,
    token: Arc<AtomicBool>,
    poll_interval_secs: u64,
    code_timeout_secs: u64,
) {
    let prefix = format!("[{index}/{total}]");
    let provider_name = format!("{:?}", sms_provider.kind);
    let activation = match provider_call(sms_provider.clone(), move |provider| {
        provider.acquire_number(&number_request)
    })
    .await
    {
        Ok(activation) => activation,
        Err(error) => {
            let _ = app.emit("registrar-log", format!("{prefix} number: {error}"));
            return;
        }
    };

    let mut record = RegistrationRecord {
        phone: activation.phone.clone(),
        provider: provider_name,
        activation_id: Some(activation.id.clone()),
        status: "number_acquired".to_string(),
        account_id: None,
        error: None,
    };
    let record_id = match database.lock().await.insert(&record) {
        Ok(id) => id,
        Err(error) => {
            let _ = app.emit("registrar-log", format!("{prefix} database: {error}"));
            cancel_activation(sms_provider, activation.id).await;
            return;
        }
    };

    let mut session = match RegistrationSession::start(RegistrationCodeRequest {
        phone: activation.phone.clone(),
        profile: attempt.profile,
        proxy: attempt.proxy,
        code_settings: Default::default(),
    })
    .await
    {
        Ok((session, sent_code)) if sent_code.code_type == "sms" => session,
        Ok((mut session, sent_code)) => {
            let _ = session.cancel().await;
            finish_attempt(
                &database,
                record_id,
                &mut record,
                "unsupported_code_type",
                None,
                Some(format!("Telegram requested {} verification", sent_code.code_type)),
            )
            .await;
            cancel_activation(sms_provider, activation.id).await;
            return;
        }
        Err(error) => {
            finish_attempt(
                &database,
                record_id,
                &mut record,
                "send_code_failed",
                None,
                Some(error.clone()),
            )
            .await;
            let _ = app.emit("registrar-log", format!("{prefix} +{}: {error}", activation.phone));
            cancel_activation(sms_provider, activation.id).await;
            return;
        }
    };

    if let Err(error) = provider_call(sms_provider.clone(), {
        let activation_id = activation.id.clone();
        move |provider| provider.update_activation(&activation_id, ActivationAction::MarkReady)
    })
    .await
    {
        let _ = session.cancel().await;
        finish_attempt(
            &database,
            record_id,
            &mut record,
            "provider_ready_failed",
            None,
            Some(error.clone()),
        )
        .await;
        let _ = app.emit("registrar-log", format!("{prefix} +{}: {error}", activation.phone));
        cancel_activation(sms_provider, activation.id).await;
        return;
    }

    record.status = "waiting_code".to_string();
    let _ = database.lock().await.update(record_id, &record);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(code_timeout_secs);
    loop {
        if !token.load(Ordering::Relaxed) {
            let _ = session.cancel().await;
            finish_attempt(&database, record_id, &mut record, "stopped", None, None).await;
            cancel_activation(sms_provider, activation.id).await;
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = session.cancel().await;
            finish_attempt(
                &database,
                record_id,
                &mut record,
                "code_timeout",
                None,
                Some("SMS verification code did not arrive before the timeout".to_string()),
            )
            .await;
            cancel_activation(sms_provider, activation.id).await;
            return;
        }

        match provider_call(sms_provider.clone(), {
            let activation_id = activation.id.clone();
            move |provider| provider.activation_status(&activation_id)
        })
        .await
        {
            Ok(ActivationStatus::CodeReceived { code, .. }) => match session.complete(&code, &attempt.identity).await {
                Ok(RegistrationOutcome::Registered { account_id }) => {
                    let finish_error = provider_call(sms_provider.clone(), {
                        let activation_id = activation.id.clone();
                        move |provider| provider.update_activation(&activation_id, ActivationAction::Finish)
                    })
                    .await
                    .err()
                    .map(|error| error.to_string());
                    finish_attempt(
                        &database,
                        record_id,
                        &mut record,
                        "registered",
                        Some(account_id),
                        finish_error,
                    )
                    .await;
                    let _ = app.emit("registrar-log", format!("{prefix} +{}: registered", activation.phone));
                    return;
                }
                Ok(RegistrationOutcome::AlreadyRegistered) => {
                    finish_attempt(&database, record_id, &mut record, "already_registered", None, None).await;
                    cancel_activation(sms_provider, activation.id).await;
                    let _ = app.emit("registrar-log", format!("{prefix} +{}: already registered", activation.phone));
                    return;
                }
                Err(error) => {
                    finish_attempt(
                        &database,
                        record_id,
                        &mut record,
                        "registration_failed",
                        None,
                        Some(error.clone()),
                    )
                    .await;
                    cancel_activation(sms_provider, activation.id).await;
                    let _ = app.emit("registrar-log", format!("{prefix} +{}: {error}", activation.phone));
                    return;
                }
            },
            Ok(ActivationStatus::Cancelled | ActivationStatus::Banned | ActivationStatus::Expired) => {
                finish_attempt(&database, record_id, &mut record, "activation_unavailable", None, None).await;
                let _ = session.cancel().await;
                return;
            }
            Ok(_) => tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await,
            Err(error) => {
                finish_attempt(
                    &database,
                    record_id,
                    &mut record,
                    "provider_status_failed",
                    None,
                    Some(error.clone()),
                )
                .await;
                let _ = session.cancel().await;
                cancel_activation(sms_provider, activation.id).await;
                let _ = app.emit("registrar-log", format!("{prefix} +{}: {error}", activation.phone));
                return;
            }
        }
    }
}

async fn provider_call<T, F>(
    config: SmsProviderConfig,
    operation: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&dyn crate::registrar::sms::SmsProvider) -> Result<T, crate::registrar::sms::SmsError>
        + Send
        + 'static,
{
    tokio::task::spawn_blocking(move || {
        let provider = create_provider(config).map_err(|error| error.to_string())?;
        operation(provider.as_ref()).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("SMS provider task failed: {error}"))?
}

async fn cancel_activation(config: SmsProviderConfig, activation_id: String) {
    let _ = provider_call(config, move |provider| {
        provider.update_activation(&activation_id, ActivationAction::Cancel)
    })
    .await;
}

async fn finish_attempt(
    database: &tokio::sync::Mutex<RegistrarDatabase>,
    record_id: i64,
    record: &mut RegistrationRecord,
    status: &str,
    account_id: Option<String>,
    error: Option<String>,
) {
    record.status = status.to_string();
    record.account_id = account_id;
    record.error = error;
    let _ = database.lock().await.update(record_id, record);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_task_limits() {
        let config = RegistrarTaskConfig {
            sms_provider: SmsProviderConfig {
                kind: crate::registrar::sms::SmsProviderKind::FiveSim,
                api_key: "key".to_string(),
                endpoint: None,
            },
            number_request: NumberRequest::telegram("russia"),
            attempts: Vec::new(),
            concurrency: 1,
            poll_interval_secs: 5,
            code_timeout_secs: 300,
        };
        assert!(config.validate().is_err());
    }
}
