use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::accounts::storage::AccountStorage;
use crate::mtproto::tl::AuthCodeSettings;
use crate::proxy::ProxyConfig;
use crate::queue::TaskQueue;
use crate::registrar::antisafety::{AntiSafetyClient, AntiSafetyConfig};
use crate::registrar::db::{registrar_directory, RegistrarDatabase, RegistrationRecord};
use crate::registrar::flow::{
    RegistrationCodeRequest, RegistrationIdentity, RegistrationOutcome, RegistrationSession,
};
use crate::registrar::profile::RegistrationProfile;
use crate::registrar::sms::{
    create_provider, ActivationAction, ActivationStatus, NumberRequest, SmsErrorKind,
    SmsProviderConfig,
};

const MAX_ATTEMPTS: usize = 100_000;
const CANCEL_RETRY_DELAY: Duration = Duration::from_secs(121);
const CANCEL_RETRIES: usize = 3;

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
    #[serde(default)]
    pub attempt_count: Option<usize>,
    #[serde(default)]
    pub random_names: bool,
    #[serde(default)]
    pub code_settings: AuthCodeSettings,
    #[serde(default = "one")]
    pub concurrency: usize,
    #[serde(default = "poll_interval")]
    pub poll_interval_secs: u64,
    #[serde(default = "code_timeout")]
    pub code_timeout_secs: u64,
    #[serde(default)]
    pub countries: Vec<String>,
    #[serde(default)]
    pub expected_prices: HashMap<String, f64>,
    #[serde(default)]
    pub use_full_balance: bool,
    #[serde(default)]
    pub min_price: Option<f64>,
    #[serde(default)]
    pub stop_on_price_change: bool,
    #[serde(default = "yes")]
    pub reject_app_code: bool,
    #[serde(default)]
    pub two_fa_password: Option<String>,
    #[serde(default)]
    pub antisafety: Option<AntiSafetyConfig>,
    #[serde(default = "yes")]
    pub send_push: bool,
    #[serde(default = "yes")]
    pub add_to_panel: bool,
    #[serde(default)]
    pub open_results: bool,
}

fn one() -> usize {
    1
}
fn yes() -> bool {
    true
}
fn poll_interval() -> u64 {
    5
}
fn code_timeout() -> u64 {
    300
}

impl RegistrarTaskConfig {
    fn validate(&self) -> Result<(), String> {
        if self.attempts.is_empty() || self.attempts.len() > MAX_ATTEMPTS {
            return Err("registration task must contain from 1 to 100000 attempts".into());
        }
        if self
            .attempt_count
            .is_some_and(|count| count == 0 || count > MAX_ATTEMPTS)
        {
            return Err("invalid account count".into());
        }
        if !(1..=20).contains(&self.concurrency)
            || !(2..=60).contains(&self.poll_interval_secs)
            || !(30..=900).contains(&self.code_timeout_secs)
        {
            return Err("invalid concurrency, polling interval or SMS timeout".into());
        }
        if self.number_request.country.trim().is_empty()
            || !matches!(
                self.number_request.service.to_lowercase().as_str(),
                "tg" | "telegram"
            )
            || self
                .countries
                .iter()
                .any(|country| country.trim().is_empty())
        {
            return Err("Telegram country and service are required".into());
        }
        validate_prices(
            self.min_price,
            self.number_request.max_price,
            &self.expected_prices,
        )?;
        if self.stop_on_price_change && self.expected_prices.is_empty() {
            return Err(
                "load current Telegram prices before enabling price-change protection".into(),
            );
        }
        if self
            .two_fa_password
            .as_ref()
            .is_some_and(|password| password.is_empty() || password.len() > 256)
        {
            return Err("2FA password must contain from 1 to 256 bytes".into());
        }
        validate_required_push(self.send_push, self.antisafety.as_ref())?;
        for attempt in &self.attempts {
            attempt.profile.validate()?;
            attempt.identity.validate()?;
        }
        create_provider(self.sms_provider.clone()).map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn validate_required_push(enabled: bool, config: Option<&AntiSafetyConfig>) -> Result<(), String> {
    if !enabled {
        return Err("Push is required for registration".into());
    }
    let config = config.ok_or("AntiSafety credentials are required for Push")?;
    AntiSafetyClient::new(config.clone()).map(|_| ())
}

fn validate_prices(
    min: Option<f64>,
    max: Option<f64>,
    expected: &HashMap<String, f64>,
) -> Result<(), String> {
    if min
        .into_iter()
        .chain(max)
        .chain(expected.values().copied())
        .any(|price| !price.is_finite() || price < 0.0)
        || min.zip(max).is_some_and(|(min, max)| min > max)
    {
        return Err("invalid price range".into());
    }
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RegistrarProgress {
    pub task_id: String,
    pub total: Option<usize>,
    pub processed: usize,
    pub registered: usize,
    pub failed: usize,
    pub finished: bool,
    pub reason: String,
    pub results_directory: String,
}

struct Context {
    app: tauri::AppHandle,
    database: tokio::sync::Mutex<RegistrarDatabase>,
    config: RegistrarTaskConfig,
    token: Arc<AtomicBool>,
    progress: tokio::sync::Mutex<RegistrarProgress>,
    output: std::path::PathBuf,
}

impl Context {
    fn log(&self, index: usize, message: &str) {
        let task_id = self
            .output
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let _ = self
            .app
            .emit("registrar-log", format!("[{task_id}][{index}] {message}"));
        dbg_log!(
            "registrar::task task={} attempt={} {}",
            task_id,
            index,
            message
        );
    }

    async fn record(&self, id: i64, record: &RegistrationRecord) -> Result<(), String> {
        self.database.lock().await.update(id, record)
    }

    async fn completed(&self, success: bool) {
        let mut progress = self.progress.lock().await;
        progress.processed += 1;
        if success {
            progress.registered += 1;
        } else {
            progress.failed += 1;
        }
        if self.database.lock().await.save_progress(&progress).is_err() {
            self.log(0, "progress_save_failed");
        }
        let _ = self.app.emit("registrar-progress", progress.clone());
    }
}

#[tauri::command]
pub async fn registrar_start(
    config: RegistrarTaskConfig,
    app: tauri::AppHandle,
) -> Result<String, String> {
    config.validate()?;
    let task_id = uuid::Uuid::new_v4().to_string();
    let (database, _) = RegistrarDatabase::create(&task_id)?;
    let output = registrar_directory().join(&task_id);
    std::fs::create_dir_all(&output)
        .map_err(|error| format!("create registration output: {error}"))?;
    let storage = AccountStorage::new(&output);
    let progress = RegistrarProgress {
        task_id: task_id.clone(),
        total: (!config.use_full_balance)
            .then_some(config.attempt_count.unwrap_or(config.attempts.len())),
        processed: 0,
        registered: 0,
        failed: 0,
        finished: false,
        reason: "running".into(),
        results_directory: storage.tdatas_dir().to_string_lossy().into_owned(),
    };
    database.save_progress(&progress)?;
    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "registrar".into(),
            "Account registration".into(),
        )
        .await;
    let context = Arc::new(Context {
        app: app.clone(),
        database: tokio::sync::Mutex::new(database),
        config,
        token,
        progress: tokio::sync::Mutex::new(progress),
        output,
    });
    let response_id = task_id.clone();
    tokio::spawn(async move {
        let result = run_task(context.clone()).await;
        if let Err(error) = &result {
            context.log(0, &safe_error(error));
        }
        let mut progress = context.progress.lock().await;
        progress.finished = true;
        progress.reason = match result {
            Ok(reason) => reason.to_string(),
            Err(_) => "failed".into(),
        };
        if !context.token.load(Ordering::Relaxed) {
            progress.reason = "stopped".into();
        }
        if progress.registered == 0 && progress.failed > 0 && progress.reason == "completed" {
            progress.reason = "failed".into();
        }
        let success = progress.reason != "failed";
        let should_open = context.config.open_results && progress.registered > 0;
        let directory = progress.results_directory.clone();
        if context
            .database
            .lock()
            .await
            .save_progress(&progress)
            .is_err()
        {
            context.log(0, "progress_save_failed");
        }
        let _ = app.emit("registrar-progress", progress.clone());
        drop(progress);
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, success).await;
        if should_open {
            if open_directory(&directory).is_err() {
                context.log(0, "open_results_failed");
            }
        }
    });
    Ok(response_id)
}

#[tauri::command]
pub async fn registrar_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

pub(crate) fn open_directory(path: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path).spawn();
    result
        .map(|_| ())
        .map_err(|_| "could not open results directory".into())
}

async fn run_task(context: Arc<Context>) -> Result<&'static str, String> {
    let config = &context.config;
    let countries = if config.countries.is_empty() {
        vec![config.number_request.country.clone()]
    } else {
        config.countries.clone()
    };
    let target = (!config.use_full_balance)
        .then_some(config.attempt_count)
        .flatten();
    let limit = if config.use_full_balance || target.is_some() {
        MAX_ATTEMPTS
    } else {
        config.attempts.len()
    };
    // Price and balance checks are serialized with purchases to avoid competing reservations.
    let workers = if config.use_full_balance || config.stop_on_price_change {
        1
    } else {
        config.concurrency
    };
    let mut tasks = tokio::task::JoinSet::new();
    let mut reason = "completed";
    let mut first_error = None;
    let mut consecutive_failures = 0;
    for index in 0..limit {
        if !context.token.load(Ordering::Relaxed) {
            reason = "stopped";
            break;
        }
        let registered = context.progress.lock().await.registered;
        if target.is_some_and(|target| registered >= target) {
            break;
        }
        if tasks.len() >= workers
            || target.is_some_and(|target| tasks.len() >= target.saturating_sub(registered))
        {
            let success = match tasks.join_next().await {
                Some(Ok(Ok(success))) => success,
                Some(Ok(Err(error))) => {
                    reason = if error == "price_changed" {
                        "price_changed"
                    } else {
                        "failed"
                    };
                    if reason == "failed" {
                        first_error.get_or_insert(error);
                    }
                    false
                }
                _ => {
                    first_error.get_or_insert("registrar worker failed".to_string());
                    reason = "failed";
                    false
                }
            };
            context.completed(success).await;
            if reason != "completed" {
                break;
            }
            if success {
                consecutive_failures = 0;
            } else {
                consecutive_failures += 1;
            }
            if (config.use_full_balance || target.is_some()) && consecutive_failures >= 3 {
                reason = "repeated_failures";
                break;
            }
            let registered = context.progress.lock().await.registered;
            if target.is_some_and(|target| registered >= target) {
                break;
            }
        }
        let mut request = config.number_request.clone();
        request.country = countries[index % countries.len()].clone();
        let guarded = config.use_full_balance
            || config.stop_on_price_change
            || config.min_price.is_some()
            || request.max_price.is_some();
        if guarded {
            let mut selected = Ok(None);
            for offset in 0..countries.len() {
                request.country = countries[(index + offset) % countries.len()].clone();
                selected = checked_request(config, request.clone()).await;
                if !matches!(selected, Ok(None)) {
                    break;
                }
            }
            match selected {
                Ok(Some(value)) => request = value,
                Ok(None) => {
                    reason = "budget_or_price_limit";
                    break;
                }
                Err(error) => {
                    reason = if error == "price_changed" {
                        "price_changed"
                    } else {
                        "failed"
                    };
                    if reason == "failed" {
                        first_error.get_or_insert(error);
                    }
                    context.log(0, reason);
                    break;
                }
            }
        }
        if !context.token.load(Ordering::Relaxed) {
            reason = "stopped";
            break;
        }
        let mut attempt = config.attempts[index % config.attempts.len()].clone();
        if config.random_names {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            attempt.identity.first_name = (0..10)
                .map(|_| (b'a' + rng.gen_range(0..26)) as char)
                .collect();
            attempt.identity.last_name = (0..6)
                .map(|_| (b'a' + rng.gen_range(0..26)) as char)
                .collect();
        }
        let worker_context = context.clone();
        tasks.spawn(
            async move { process_attempt(worker_context, index + 1, attempt, request).await },
        );
    }
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(success)) => context.completed(success).await,
            Ok(Err(error)) => {
                first_error.get_or_insert(error);
                context.completed(false).await;
            }
            Err(_) => {
                first_error.get_or_insert("registrar worker panicked".into());
                context.completed(false).await;
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    if config.use_full_balance && reason == "completed" {
        reason = "attempt_limit";
    }
    let registered = context.progress.lock().await.registered;
    if target.is_some_and(|target| registered < target) && reason == "completed" {
        reason = "attempt_limit";
    }
    Ok(reason)
}

async fn checked_request(
    config: &RegistrarTaskConfig,
    mut request: NumberRequest,
) -> Result<Option<NumberRequest>, String> {
    let country = request.country.clone();
    let service = request.service.clone();
    let operator = request.operator.clone();
    let min = config.min_price.unwrap_or(0.0);
    let max = request.max_price.unwrap_or(f64::INFINITY);
    let prices = provider_call(config.sms_provider.clone(), "prices", move |provider| {
        provider.prices(&service, Some(&country))
    })
    .await?;
    let price = prices
        .into_iter()
        .filter(|price| {
            price.country.id == request.country
                && price.available != Some(0)
                && price.price.is_finite()
                && price.price >= min
                && price.price <= max
                && operator.as_ref().is_none_or(|operator| {
                    price.operator.is_none() || price.operator.as_ref() == Some(operator)
                })
        })
        .min_by(|left, right| left.price.total_cmp(&right.price));
    let Some(price) = price else {
        return Ok(None);
    };
    if config.stop_on_price_change {
        let expected = config
            .expected_prices
            .get(&request.country)
            .ok_or("missing initial country price")?;
        if (expected - price.price).abs() > 0.000001 {
            return Err("price_changed".into());
        }
        request.max_price = Some(price.price);
    }
    if price.operator.is_some() {
        request.operator = price.operator.clone();
    }
    if config.use_full_balance {
        let balance = provider_call(config.sms_provider.clone(), "balance", |provider| {
            provider.balance()
        })
        .await?;
        if !balance.is_finite() || balance < price.price {
            return Ok(None);
        }
        request.max_price = Some(request.max_price.unwrap_or(balance).min(balance));
    }
    Ok(Some(request))
}

async fn process_attempt(
    context: Arc<Context>,
    index: usize,
    attempt: RegistrationAttempt,
    request: NumberRequest,
) -> Result<bool, String> {
    if !context.token.load(Ordering::Relaxed) {
        return Ok(false);
    }
    let config = &context.config;
    let max_price = request.max_price;
    let requested_country = request.country.clone();
    let mut record = RegistrationRecord {
        phone: String::new(),
        provider: config.sms_provider.kind.label().into(),
        activation_id: None,
        status: "buying_number".into(),
        account_id: None,
        error: None,
    };
    let record_id = context.database.lock().await.insert(&record)?;
    context.log(index, "buying_number");
    let activation = match provider_call(
        config.sms_provider.clone(),
        "acquire_number",
        move |provider| provider.acquire_number(&request),
    )
    .await
    {
        Ok(value) => value,
        Err(_) => {
            record.status = "purchase_failed".into();
            record.error = Some("SMS number purchase failed".into());
            context.record(record_id, &record).await?;
            context.log(index, "purchase_failed");
            return Ok(false);
        }
    };
    record.phone = activation.phone.clone();
    record.activation_id = Some(activation.id.clone());
    record.status = "number_acquired".into();
    if let Err(error) = context.record(record_id, &record).await {
        cancel_activation(config.sms_provider.clone(), activation.id.clone());
        return Err(error);
    }
    if activation.cost.is_some_and(|price| {
        !price.is_finite()
            || price < config.min_price.unwrap_or(0.0)
            || price > max_price.unwrap_or(f64::INFINITY)
            || (config.stop_on_price_change
                && config
                    .expected_prices
                    .get(&requested_country)
                    .is_some_and(|expected| (expected - price).abs() > 0.000001))
    }) {
        cancel_activation(config.sms_provider.clone(), activation.id);
        record.status = "price_rejected".into();
        record.error = Some("Purchased number price is outside the accepted range".into());
        context.record(record_id, &record).await?;
        context.log(index, "price_rejected");
        if config.stop_on_price_change {
            return Err("price_changed".into());
        }
        return Ok(false);
    }
    let outcome = register_number(
        &context,
        index,
        &attempt,
        &activation,
        record_id,
        &mut record,
    )
    .await;
    if record.account_id.is_none() && record.status != "registration_storage_failed" {
        cancel_activation(config.sms_provider.clone(), activation.id);
    }
    match outcome {
        Ok(success) => Ok(success),
        Err(error) => {
            dbg_log!(
                "registrar::attempt task={} attempt={} failed {}",
                context
                    .output
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                index,
                super::diagnostics::error_summary(&error)
            );
            if record.status != "registration_storage_failed" {
                record.status = if record.account_id.is_some() {
                    "registered_with_warnings"
                } else {
                    "registration_failed"
                }
                .into();
            }
            record.error = Some(safe_error(&error));
            context.record(record_id, &record).await?;
            context.log(index, &record.status);
            context.log(index, record.error.as_deref().unwrap_or("unknown_error"));
            Ok(record.account_id.is_some())
        }
    }
}

fn safe_error(error: &str) -> String {
    if error.starts_with("sms_")
        && error.len() <= 128
        && error
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '_')
    {
        return error.to_string();
    }
    if let Some(code) = error.split_whitespace().find(|part| {
        part.len() <= 128
            && part.contains('_')
            && part.chars().all(|character| {
                character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
            })
    }) {
        return code.to_string();
    }
    match error {
        "stopped"
        | "sms_timeout"
        | "unsupported_verification_type"
        | "app_code_rejected"
        | "activation_unavailable" => error.to_string(),
        _ => "registration_step_failed".to_string(),
    }
}

async fn register_number(
    context: &Context,
    index: usize,
    attempt: &RegistrationAttempt,
    activation: &crate::registrar::sms::Activation,
    record_id: i64,
    record: &mut RegistrationRecord,
) -> Result<bool, String> {
    let config = &context.config;
    if !context.token.load(Ordering::Relaxed) {
        return Err("stopped".into());
    }
    let (mut session, sent_code) = RegistrationSession::start(RegistrationCodeRequest {
        phone: activation.phone.clone(),
        profile: attempt.profile.clone(),
        proxy: attempt.proxy.clone(),
        code_settings: config.code_settings.clone(),
        antisafety: config.antisafety.clone(),
        cancellation: Some(context.token.clone()),
    })
    .await?;
    session.set_output_directory(context.output.clone());
    context.log(index, &format!("code_delivery={}", sent_code.code_type));
    if sent_code.code_type == "app" && config.reject_app_code {
        let _ = session.cancel().await;
        return Err("app_code_rejected".into());
    }
    if !matches!(
        sent_code.code_type.as_str(),
        "sms" | "sms_word" | "sms_phrase" | "app"
    ) {
        let _ = session.cancel().await;
        return Err("unsupported_verification_type".into());
    }
    let id = activation.id.clone();
    let ready =
        provider_call(
            config.sms_provider.clone(),
            "mark_ready",
            move |provider| match provider.update_activation(&id, ActivationAction::MarkReady) {
                Ok(_) => Ok(()),
                Err(error) if error.kind == SmsErrorKind::Unsupported => Ok(()),
                Err(error) => Err(error),
            },
        )
        .await;
    if let Err(error) = ready {
        let _ = session.cancel().await;
        return Err(error);
    }
    record.status = "waiting_code".into();
    if let Err(error) = context.record(record_id, record).await {
        let _ = session.cancel().await;
        return Err(error);
    }
    context.log(index, "waiting_code");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(config.code_timeout_secs);
    let code = loop {
        if !context.token.load(Ordering::Relaxed) {
            let _ = session.cancel().await;
            return Err("stopped".into());
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = session.cancel().await;
            return Err("sms_timeout".into());
        }
        let id = activation.id.clone();
        let status = match provider_call(
            config.sms_provider.clone(),
            "activation_status",
            move |provider| provider.activation_status(&id),
        )
        .await
        {
            Ok(status) => status,
            Err(error) => {
                let _ = session.cancel().await;
                return Err(error);
            }
        };
        if !context.token.load(Ordering::Relaxed) || tokio::time::Instant::now() >= deadline {
            let _ = session.cancel().await;
            return Err(if context.token.load(Ordering::Relaxed) {
                "sms_timeout"
            } else {
                "stopped"
            }
            .into());
        }
        match status {
            ActivationStatus::CodeReceived { code, .. } => break code,
            ActivationStatus::Cancelled
            | ActivationStatus::Banned
            | ActivationStatus::Expired
            | ActivationStatus::Finished => {
                let _ = session.cancel().await;
                return Err("activation_unavailable".into());
            }
            _ => tokio::time::sleep(Duration::from_secs(1)).await,
        }
        // Cancellation is checked every second, even with a longer provider poll interval.
        for _ in 1..config.poll_interval_secs {
            if !context.token.load(Ordering::Relaxed) {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    };
    context.log(index, "completing_registration");
    let outcome = match session.complete(&code, &attempt.identity).await {
        Ok(outcome) => outcome,
        Err(error) => {
            if session.is_registered() {
                record.status = "registration_storage_failed".into();
                let id = activation.id.clone();
                if provider_call(config.sms_provider.clone(), "finish", move |provider| {
                    provider.update_activation(&id, ActivationAction::Finish)
                })
                .await
                .is_err()
                {
                    context.log(index, "provider_finish_failed");
                }
            } else {
                let _ = session.cancel().await;
            }
            return Err(error);
        }
    };
    let account_id = match outcome {
        RegistrationOutcome::AlreadyRegistered => {
            record.status = "already_registered".into();
            context.record(record_id, record).await?;
            context.log(index, "already_registered");
            return Ok(false);
        }
        RegistrationOutcome::Registered { account_id } => account_id,
    };
    record.account_id = Some(account_id.clone());
    record.status = "registered".into();
    let mut warnings = Vec::new();
    let id = activation.id.clone();
    if provider_call(config.sms_provider.clone(), "finish", move |provider| {
        provider.update_activation(&id, ActivationAction::Finish)
    })
    .await
    .is_err()
    {
        warnings.push("provider_finish_failed");
    }
    context.record(record_id, record).await?;
    if let Some(password) = &config.two_fa_password {
        context.log(index, "setting_2fa");
        if session.set_two_fa(&account_id, password).await.is_err() {
            warnings.push("two_fa_failed");
        }
    }
    if config.send_push {
        context.log(index, "registering_push");
        if register_push(
            &mut session,
            config
                .antisafety
                .clone()
                .ok_or("missing Push configuration")?,
            &context.token,
        )
        .await
        .is_err()
        {
            warnings.push("push_failed");
        }
    }
    if config.add_to_panel {
        context.log(index, "adding_to_panel");
        let source = AccountStorage::new(&context.output);
        let id = account_id.clone();
        if tokio::task::spawn_blocking(move || copy_to_panel(&source, &id))
            .await
            .map_err(|_| "panel export worker failed")?
            .is_err()
        {
            warnings.push("panel_export_failed");
        }
    }
    if !warnings.is_empty() {
        record.status = "registered_with_warnings".into();
        record.error = Some(warnings.join(", "));
        for warning in &warnings {
            context.log(index, warning);
        }
    }
    context.record(record_id, record).await?;
    context.log(index, &record.status);
    Ok(true)
}

async fn register_push(
    session: &mut RegistrationSession,
    config: AntiSafetyConfig,
    token: &AtomicBool,
) -> Result<(), String> {
    if !token.load(Ordering::Relaxed) {
        return Err("stopped".into());
    }
    let client = AntiSafetyClient::new(config)?;
    let task_client = client.clone();
    let task = tokio::task::spawn_blocking(move || task_client.create_android_push())
        .await
        .map_err(|_| "Push worker failed")??;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        if !token.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        let task_client = client.clone();
        let id = task.id.clone();
        let result = tokio::task::spawn_blocking(move || task_client.get_android_push(&id))
            .await
            .map_err(|_| "Push worker failed")?;
        match result {
            Ok(token) => return session.register_push(&token.token).await,
            Err(_) if tokio::time::Instant::now() < deadline => {
                for _ in 0..5 {
                    if !token.load(Ordering::Relaxed) {
                        return Err("stopped".into());
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
            Err(_) => return Err("Push token timeout".into()),
        }
    }
}

fn copy_to_panel(source: &AccountStorage, account_id: &str) -> Result<(), String> {
    let target = crate::accounts::commands::get_storage_pub();
    publish_account(source, &target, account_id)?;
    crate::accounts::commands::invalidate_accounts_cache();
    Ok(())
}

fn publish_account(
    source: &AccountStorage,
    target: &AccountStorage,
    account_id: &str,
) -> Result<(), String> {
    uuid::Uuid::parse_str(account_id).map_err(|_| "invalid account id")?;
    let session = target.session_path(account_id);
    let json = target.json_path(account_id);
    let tdata = target.tdata_dir(account_id);
    if [&session, &json, &tdata].iter().any(|path| path.exists()) {
        return Err("account output already exists".into());
    }
    let staging = target
        .session_json_dir()
        .join(format!(".registrar-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&staging).map_err(|_| "create panel staging directory failed")?;
    let mut published_tdata = false;
    let mut published_json = false;
    let result = (|| {
        std::fs::copy(
            source.session_path(account_id),
            staging.join("account.session"),
        )
        .map_err(|_| "stage account session failed")?;
        std::fs::copy(source.json_path(account_id), staging.join("account.json"))
            .map_err(|_| "stage account JSON failed")?;
        crate::accounts::commands::copy_dir_recursive(
            &source.tdata_dir(account_id),
            &staging.join("tdata"),
        )
        .map_err(|_| "stage account TData failed")?;
        std::fs::rename(staging.join("tdata"), &tdata)
            .map_err(|_| "publish account TData failed")?;
        published_tdata = true;
        std::fs::hard_link(staging.join("account.json"), &json)
            .map_err(|_| "publish account JSON failed")?;
        published_json = true;
        // Session is the discovery marker: publish it only after all other files are ready.
        std::fs::hard_link(staging.join("account.session"), &session)
            .map_err(|_| "publish account session failed")?;
        Ok(())
    })();
    if result.is_err() {
        if published_json {
            let _ = std::fs::remove_file(&json);
        }
        if published_tdata {
            let _ = std::fs::remove_dir_all(&tdata);
        }
    }
    if std::fs::remove_dir_all(&staging).is_err() {
        dbg_log!("registrar::panel staging cleanup failed");
    }
    result
}

async fn provider_call<T, F>(
    config: SmsProviderConfig,
    operation_name: &'static str,
    operation: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(
            &dyn crate::registrar::sms::SmsProvider,
        ) -> Result<T, crate::registrar::sms::SmsError>
        + Send
        + 'static,
{
    tokio::task::spawn_blocking(move || {
        let kind = config.kind;
        let started = std::time::Instant::now();
        dbg_log!("registrar::sms provider={:?} operation={} started", kind, operation_name);
        let result = create_provider(config).and_then(|provider| operation(provider.as_ref()));
        match &result {
            Ok(_) => dbg_log!("registrar::sms provider={:?} operation={} completed duration_ms={}", kind, operation_name, started.elapsed().as_millis()),
            Err(error) => dbg_log!("registrar::sms provider={:?} operation={} failed kind={:?} code={} {} duration_ms={}",
                kind, operation_name, error.kind, super::diagnostics::remote_code(&error.message),
                super::diagnostics::error_summary(&error.message), started.elapsed().as_millis()),
        }
        result.map_err(|error| format!("sms_{:?}", error.kind).to_lowercase())
    })
    .await
    .map_err(|_| "SMS worker failed".to_string())?
}

fn cancel_activation(config: SmsProviderConfig, activation_id: String) {
    // Providers may reject early cancellation. Keep their retry window out of the registration task.
    let _ = tokio::spawn(async move {
        let result = cancel_with_retry(
            || {
                let id = activation_id.clone();
                provider_call(config.clone(), "cancel", move |provider| {
                    provider
                        .update_activation(&id, ActivationAction::Cancel)
                        .map(|_| ())
                })
            },
            || tokio::time::sleep(CANCEL_RETRY_DELAY),
        )
        .await;
        if let Err(error) = result {
            dbg_log!(
                "registrar::activation cancellation failed category={}",
                error
            );
        }
    });
}

async fn cancel_with_retry<F, Fut, W, Wait>(mut cancel: F, mut wait: W) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
    W: FnMut() -> Wait,
    Wait: std::future::Future<Output = ()>,
{
    let mut retries = 0;
    loop {
        match cancel().await {
            Err(error) if error == "sms_tooearlytocancel" && retries < CANCEL_RETRIES => {
                retries += 1;
                dbg_log!(
                    "registrar::activation cancellation deferred retry={}/{} retry_after_secs={}",
                    retries,
                    CANCEL_RETRIES,
                    CANCEL_RETRY_DELAY.as_secs()
                );
                // Cleanup remains active after Stop: the reservation still needs cancellation.
                wait().await;
                dbg_log!(
                    "registrar::activation cancellation retry={} started",
                    retries
                );
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_requires_push_and_antisafety_credentials() {
        let config = AntiSafetyConfig {
            api_key: "test-key".into(),
            aid: "test-aid".into(),
        };
        assert!(validate_required_push(false, Some(&config)).is_err());
        assert!(validate_required_push(true, None).is_err());
        assert!(validate_required_push(
            true,
            Some(&AntiSafetyConfig {
                api_key: "".into(),
                ..config.clone()
            })
        )
        .is_err());
        assert!(validate_required_push(true, Some(&config)).is_ok());
    }

    #[tokio::test]
    async fn early_cancellation_stops_retrying_after_success() {
        let attempts = std::cell::Cell::new(0);
        let waited = std::cell::Cell::new(false);
        let result = cancel_with_retry(
            || {
                let attempt = attempts.get();
                attempts.set(attempt + 1);
                std::future::ready(if attempt == 0 {
                    Err("sms_tooearlytocancel".into())
                } else {
                    assert!(waited.get());
                    Ok(())
                })
            },
            || {
                waited.set(true);
                std::future::ready(())
            },
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(attempts.get(), 2);
    }

    #[tokio::test]
    async fn cancellation_does_not_retry_other_errors_or_retry_forever() {
        for error in ["sms_transport", "sms_tooearlytocancel"] {
            let attempts = std::cell::Cell::new(0);
            let waits = std::cell::Cell::new(0);
            let result = cancel_with_retry(
                || {
                    attempts.set(attempts.get() + 1);
                    std::future::ready(Err(error.to_string()))
                },
                || {
                    waits.set(waits.get() + 1);
                    std::future::ready(())
                },
            )
            .await;
            assert_eq!(result.unwrap_err(), error);
            assert_eq!(
                attempts.get(),
                if error == "sms_tooearlytocancel" {
                    4
                } else {
                    1
                }
            );
            assert_eq!(
                waits.get(),
                if error == "sms_tooearlytocancel" {
                    3
                } else {
                    0
                }
            );
        }
    }

    #[tokio::test]
    async fn cancellation_waits_before_each_of_three_retries() {
        let attempts = std::cell::Cell::new(0);
        let waits = std::cell::Cell::new(0);
        assert_eq!(CANCEL_RETRY_DELAY.as_secs(), 121);
        let result = cancel_with_retry(
            || {
                assert_eq!(attempts.get(), waits.get());
                let attempt = attempts.get();
                attempts.set(attempt + 1);
                std::future::ready(if attempt < 3 {
                    Err("sms_tooearlytocancel".into())
                } else {
                    Ok(())
                })
            },
            || {
                waits.set(waits.get() + 1);
                std::future::ready(())
            },
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(attempts.get(), 4);
        assert_eq!(waits.get(), 3);
    }

    #[test]
    fn price_ranges_reject_nonfinite_and_reversed_bounds() {
        assert!(validate_prices(Some(f64::NAN), None, &HashMap::new()).is_err());
        assert!(validate_prices(Some(2.0), Some(1.0), &HashMap::new()).is_err());
        assert!(validate_prices(Some(0.0), Some(1.0), &HashMap::new()).is_ok());
    }

    #[test]
    fn diagnostic_errors_never_include_request_urls_or_credentials() {
        assert_eq!(
            safe_error("request https://example.com/?api_key=secret failed"),
            "registration_step_failed"
        );
        assert_eq!(
            safe_error("rpc error 400: PHONE_CODE_INVALID"),
            "PHONE_CODE_INVALID"
        );
        assert_eq!(safe_error("sms_transport"), "sms_transport");
    }

    #[test]
    fn panel_publication_does_not_expose_partial_accounts() {
        let directory =
            std::env::temp_dir().join(format!("registrar-test-{}", uuid::Uuid::new_v4()));
        let source = AccountStorage::new(&directory.join("source"));
        let target = AccountStorage::new(&directory.join("target"));
        let id = uuid::Uuid::new_v4().to_string();
        std::fs::write(source.session_path(&id), b"session fixture").unwrap();
        assert!(publish_account(&source, &target, &id).is_err());
        assert!(!target.session_path(&id).exists());
        assert!(!target.json_path(&id).exists());
        assert!(!target.tdata_dir(&id).exists());
        std::fs::write(source.json_path(&id), b"{}").unwrap();
        std::fs::create_dir(source.tdata_dir(&id)).unwrap();
        std::fs::write(source.tdata_dir(&id).join("key_data"), b"tdata fixture").unwrap();
        publish_account(&source, &target, &id).unwrap();
        assert_eq!(
            std::fs::read(target.session_path(&id)).unwrap(),
            b"session fixture"
        );
        assert!(publish_account(&source, &target, &id).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
