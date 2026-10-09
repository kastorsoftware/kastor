use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rand::{seq::SliceRandom, Rng};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::accounts::storage::AccountStorage;
use crate::mtproto::tl::{AuthCodeSettings, SentCode};
use crate::proxy::{ProxyConfig, ProxyList};
use crate::queue::TaskQueue;
use crate::registrar::antisafety::{AntiSafetyClient, AntiSafetyConfig};
use crate::registrar::db::{registrar_directory, RegistrarDatabase, RegistrationRecord};
use crate::registrar::email::{EmailSource, MailboxAccount, RecoveryEmailSource};
use crate::registrar::flow::{
    RegistrationCodeRequest, RegistrationIdentity, RegistrationOutcome, RegistrationSession,
    RECAPTCHA_ANTISAFETY_REQUIRED,
};
use crate::registrar::mail_tm::{MailTmClient, MailTmMailbox};
use crate::registrar::profile::{fetch_latest_android_versions, RegistrationProfile};
use crate::registrar::sms::{
    create_provider, Activation, ActivationAction, ActivationStatus, NumberRequest, SmsErrorKind,
    SmsProviderConfig, SmsProviderKind,
};

const MAX_ATTEMPTS: usize = 100_000;
const MAX_APP_SMS_FALLBACK_WAIT_SECS: u64 = 900;
const MAX_REREGISTRATION_RESTARTS: usize = 2;
const MAX_EXPIRED_CODE_RESTARTS: usize = 2;
const POST_DELETION_SMS_WAIT: Duration = Duration::from_secs(120);
const ACCOUNT_DELETION_PAUSE_MIN_SECS: u64 = 2;
const ACCOUNT_DELETION_PAUSE_MAX_SECS: u64 = 6;
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
    #[serde(default = "yes")]
    pub re_register_existing: bool,
    #[serde(default)]
    pub two_fa_password: Option<String>,
    #[serde(default)]
    pub set_recovery_email: bool,
    #[serde(default)]
    pub recovery_email_source: RecoveryEmailSource,
    #[serde(default)]
    pub recovery_email_file_path: Option<String>,
    #[serde(default)]
    pub antisafety: Option<AntiSafetyConfig>,
    #[serde(default)]
    pub email_premium: bool,
    #[serde(default)]
    pub allow_email_fallback: bool,
    #[serde(default)]
    pub email_source: EmailSource,
    #[serde(default)]
    pub email_file_path: Option<String>,
    #[serde(default)]
    pub max_consecutive_failures: Option<usize>,
    #[serde(default = "yes")]
    pub send_push: bool,
    #[serde(default = "yes")]
    pub add_to_panel: bool,
    #[serde(default)]
    pub open_results: bool,
    #[serde(default)]
    pub hide_phone_number: bool,
    #[serde(default)]
    pub hide_online_status: bool,
    #[serde(default)]
    pub set_avatar: bool,
    #[serde(default = "default_avatar_mode")]
    pub avatar_mode: String,
    #[serde(default)]
    pub avatar_path: String,
    #[serde(default)]
    pub set_username: bool,
    #[serde(default = "default_username_mode")]
    pub username_mode: String,
    #[serde(default)]
    pub username_prefix: String,
    #[serde(default)]
    pub username_list_path: String,
    #[serde(default)]
    pub manual_locale: bool,
    #[serde(default = "default_registration_language")]
    pub registration_language: String,
    #[serde(default)]
    pub timezone_offset_seconds: i32,
}

fn one() -> usize {
    1
}
fn yes() -> bool {
    true
}
fn poll_interval() -> u64 {
    3
}
fn code_timeout() -> u64 {
    300
}
fn default_avatar_mode() -> String {
    "emoji".to_string()
}
fn default_username_mode() -> String {
    "random".to_string()
}
fn default_registration_language() -> String {
    "en".to_string()
}

#[derive(Clone)]
struct ProxyLocale {
    language: String,
    timezone_offset_seconds: i32,
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
        if !(1..=100).contains(&self.concurrency)
            || !(2..=60).contains(&self.poll_interval_secs)
            || !(30..=900).contains(&self.code_timeout_secs)
        {
            return Err("invalid concurrency, polling interval or SMS timeout".into());
        }
        if self
            .max_consecutive_failures
            .is_some_and(|count| count == 0 || count > MAX_ATTEMPTS)
        {
            return Err("invalid consecutive failure limit".into());
        }
        if self.number_request.country.trim().is_empty()
            || !matches!(
                self.number_request.service.to_lowercase().as_str(),
                "tg" | "tgs" | "telegram"
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
        if self.sms_provider.kind == SmsProviderKind::OnlineSim
            && (self.number_request.max_price.is_some() || self.stop_on_price_change)
        {
            return Err("registrar.onlineSimPriceLimitUnsupported".into());
        }
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
        validate_antisafety(self.send_push, self.email_source, self.antisafety.as_ref())?;
        if self.set_recovery_email {
            if self.two_fa_password.is_none() {
                return Err("registrar.passwordRequired".into());
            }
            if self.recovery_email_source.imap_source().is_some()
                && self
                    .recovery_email_file_path
                    .as_deref()
                    .is_none_or(str::is_empty)
            {
                return Err("registrar.recoveryEmailFileRequired".into());
            }
        }
        if self.email_source != EmailSource::AntiSafety
            && self.email_file_path.as_deref().is_none_or(str::is_empty)
        {
            return Err("registrar.emailFileRequired".into());
        }
        for attempt in &self.attempts {
            attempt.profile.validate_request()?;
            attempt.identity.validate()?;
        }
        create_provider(self.sms_provider.clone()).map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn validate_antisafety(
    send_push: bool,
    email_source: EmailSource,
    config: Option<&AntiSafetyConfig>,
) -> Result<(), String> {
    if send_push || email_source == EmailSource::AntiSafety {
        let config = config.ok_or("AntiSafety credentials are required")?;
        AntiSafetyClient::new(config.clone()).map(|_| ())?;
    }
    Ok(())
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

#[derive(Default)]
struct PendingCancellations {
    pending: AtomicUsize,
    refunds: AtomicUsize,
}

struct CancellationTicket(Arc<PendingCancellations>);

impl PendingCancellations {
    fn track(self: &Arc<Self>) -> CancellationTicket {
        self.pending.fetch_add(1, Ordering::SeqCst);
        CancellationTicket(self.clone())
    }
}

impl CancellationTicket {
    fn complete(self, refunded: bool) {
        if refunded {
            self.0.refunds.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl Drop for CancellationTicket {
    fn drop(&mut self) {
        self.0.pending.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Context {
    mailboxes: Vec<MailboxAccount>,
    mailbox_index: AtomicUsize,
    recovery_mailboxes: Vec<MailboxAccount>,
    recovery_mailbox_index: AtomicUsize,
    reserved_mailboxes: tokio::sync::Mutex<HashSet<String>>,
    mail_tm: MailTmClient,
    usernames: Vec<String>,
    username_index: AtomicUsize,
    proxy_locales: tokio::sync::Mutex<HashMap<String, ProxyLocale>>,
    purchase_gate: tokio::sync::Mutex<Option<String>>,
    cancellations: Arc<PendingCancellations>,
    app: tauri::AppHandle,
    database: tokio::sync::Mutex<RegistrarDatabase>,
    config: RegistrarTaskConfig,
    android_app_versions: Vec<String>,
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
    let mailboxes = if config.email_source == EmailSource::AntiSafety {
        Vec::new()
    } else {
        let path = config
            .email_file_path
            .as_deref()
            .ok_or("registrar.emailFileRequired")?;
        MailboxAccount::load_file(std::path::Path::new(path), config.email_source)?
    };
    let recovery_mailboxes = if config.set_recovery_email {
        if let Some(source) = config.recovery_email_source.imap_source() {
            let path = config
                .recovery_email_file_path
                .as_deref()
                .ok_or("registrar.recoveryEmailFileRequired")?;
            MailboxAccount::load_file(std::path::Path::new(path), source)?
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    if config.set_avatar
        && (!matches!(
            config.avatar_mode.as_str(),
            "emoji" | "folder" | "single" | "generated"
        ) || (!matches!(config.avatar_mode.as_str(), "emoji" | "generated")
            && config.avatar_path.trim().is_empty()))
    {
        return Err("registrar.avatarRequired".into());
    }
    if config.set_avatar
        && ((config.avatar_mode == "single"
            && !std::path::Path::new(&config.avatar_path).is_file())
            || (config.avatar_mode == "folder"
                && crate::accounts::actions::photos_from_folder(&config.avatar_path).is_empty()))
    {
        return Err("registrar.avatarRequired".into());
    }
    if config.set_username
        && !matches!(
            config.username_mode.as_str(),
            "random" | "from_list" | "prefix_random"
        )
    {
        return Err("invalid username mode".into());
    }
    if config.set_username
        && config.username_mode == "prefix_random"
        && (config.username_prefix.is_empty()
            || config.username_prefix.len() > 30
            || !config
                .username_prefix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
    {
        return Err("registrar.invalidUsernamePrefix".into());
    }
    let usernames = if config.set_username && config.username_mode == "from_list" {
        let text = std::fs::read_to_string(&config.username_list_path)
            .map_err(|_| "registrar.usernameFileRequired")?;
        let mut values = Vec::new();
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let value = line.trim_start_matches('@').to_string();
            if !(5..=32).contains(&value.len())
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err("registrar.invalidUsernameFile".into());
            }
            if !values
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&value))
            {
                values.push(value);
            }
        }
        if values.is_empty() {
            return Err("registrar.usernameFileRequired".into());
        }
        values
    } else {
        Vec::new()
    };
    let android_app_versions = tokio::task::spawn_blocking(fetch_latest_android_versions)
        .await
        .map_err(|_| "Telegram Android releases worker failed")??;
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
        mailboxes,
        mailbox_index: AtomicUsize::new(0),
        recovery_mailboxes,
        recovery_mailbox_index: AtomicUsize::new(0),
        reserved_mailboxes: tokio::sync::Mutex::new(HashSet::new()),
        mail_tm: MailTmClient::new(),
        usernames,
        username_index: AtomicUsize::new(0),
        proxy_locales: tokio::sync::Mutex::new(HashMap::new()),
        purchase_gate: tokio::sync::Mutex::new(None),
        cancellations: Arc::new(PendingCancellations::default()),
        app: app.clone(),
        database: tokio::sync::Mutex::new(database),
        config,
        android_app_versions,
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
    let mut countries = if config.countries.is_empty() {
        vec![config.number_request.country.clone()]
    } else {
        config.countries.clone()
    };
    countries.sort_by(|left, right| {
        config
            .expected_prices
            .get(left)
            .copied()
            .unwrap_or(f64::INFINITY)
            .total_cmp(
                &config
                    .expected_prices
                    .get(right)
                    .copied()
                    .unwrap_or(f64::INFINITY),
            )
    });
    let target = (!config.use_full_balance)
        .then_some(config.attempt_count)
        .flatten();
    let limit = if config.use_full_balance || target.is_some() {
        MAX_ATTEMPTS
    } else {
        config.attempts.len()
    };
    let workers = config.concurrency;
    let mut tasks: tokio::task::JoinSet<Result<bool, String>> = tokio::task::JoinSet::new();
    let sms_config = select_sms_proxy(&config.sms_provider).await?;
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
                    reason = match error.as_str() {
                        "price_changed" => "price_changed",
                        "budget_or_price_limit" => "budget_or_price_limit",
                        _ => "failed",
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
            if reason != "budget_or_price_limit" {
                context.completed(success).await;
            }
            if reason != "completed" {
                break;
            }
            if success {
                consecutive_failures = 0;
            } else {
                consecutive_failures += 1;
            }
            if config
                .max_consecutive_failures
                .is_some_and(|limit| consecutive_failures >= limit)
            {
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
            || request.max_price.is_some()
            || countries.len() > 1;
        if guarded && !config.use_full_balance && !config.stop_on_price_change {
            let mut selected = Ok(None);
            for country in &countries {
                request.country = country.clone();
                selected = checked_request(config, &sms_config, request.clone()).await;
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
        attempt
            .profile
            .assign_random_android_client(&context.android_app_versions)?;
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
        let worker_countries = countries.clone();
        let worker_sms_config = sms_config.clone();
        tasks.spawn(async move {
            process_attempt(
                worker_context,
                index + 1,
                attempt,
                request,
                worker_countries,
                worker_sms_config,
            )
            .await
        });
    }
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(success)) => context.completed(success).await,
            Ok(Err(error)) => {
                match error.as_str() {
                    "price_changed" => reason = "price_changed",
                    "budget_or_price_limit" => {
                        if reason == "completed" {
                            reason = "budget_or_price_limit";
                        }
                        continue;
                    }
                    _ => {
                        first_error.get_or_insert(error);
                    }
                }
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
    sms_config: &SmsProviderConfig,
    mut request: NumberRequest,
) -> Result<Option<NumberRequest>, String> {
    let country = request.country.clone();
    let service = request.service.clone();
    let operator = request.operator.clone();
    let min = config.min_price.unwrap_or(0.0);
    let max = request.max_price.unwrap_or(f64::INFINITY);
    let prices = provider_call(sms_config.clone(), "prices", move |provider| {
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
        let balance =
            provider_call(sms_config.clone(), "balance", |provider| provider.balance()).await?;
        if !balance.is_finite() || balance < price.price {
            return Err("sms_insufficientbalance".into());
        }
        if config.sms_provider.kind != SmsProviderKind::OnlineSim {
            request.max_price = Some(request.max_price.unwrap_or(balance).min(balance));
        }
    }
    Ok(Some(request))
}

async fn select_sms_proxy(config: &SmsProviderConfig) -> Result<SmsProviderConfig, String> {
    let mut candidates = ProxyList::load().proxies;
    candidates.shuffle(&mut rand::thread_rng());
    if candidates.is_empty() {
        if config.http_proxy.is_some() || crate::settings::AppSettings::load().allow_no_proxy {
            return Ok(config.clone());
        }
        return Err("sms_proxy_unavailable".to_string());
    }
    for proxy in candidates {
        let mut candidate = config.clone();
        candidate.http_proxy = Some(proxy.to_string_repr());
        match provider_call(candidate.clone(), "proxy_check", |provider| {
            provider.balance()
        })
        .await
        {
            Ok(_) => return Ok(candidate),
            Err(error) if error == "sms_transport" || error == "sms_invalidrequest" => continue,
            Err(error) => return Err(error),
        }
    }
    Err("sms_proxy_unavailable".to_string())
}

async fn purchase_for_attempt(
    context: &Context,
    index: usize,
    request: &mut NumberRequest,
    countries: &[String],
    sms_config: &SmsProviderConfig,
) -> Result<Option<Activation>, String> {
    let protected_purchase = context.config.use_full_balance || context.config.stop_on_price_change;
    let mut gate = if protected_purchase {
        Some(context.purchase_gate.lock().await)
    } else {
        None
    };
    if let Some(reason) = gate.as_ref().and_then(|guard| guard.as_ref()) {
        return Err(reason.clone());
    }
    loop {
        let refunds_before_purchase = context.cancellations.refunds.load(Ordering::SeqCst);
        if protected_purchase {
            let mut selected = Ok(None);
            let mut insufficient_balance = false;
            for country in countries {
                let mut candidate = request.clone();
                candidate.country = country.clone();
                selected = checked_request(&context.config, sms_config, candidate).await;
                if matches!(&selected, Err(error) if error == "sms_insufficientbalance") {
                    insufficient_balance = true;
                    selected = Ok(None);
                }
                if !matches!(selected, Ok(None)) {
                    break;
                }
            }
            match selected {
                Ok(Some(value)) => *request = value,
                Ok(None) => {
                    if insufficient_balance {
                        if wait_for_balance_refund(context, index, refunds_before_purchase).await? {
                            continue;
                        }
                        return Ok(None);
                    }
                    **gate.as_mut().expect("protected purchase holds gate") =
                        Some("budget_or_price_limit".into());
                    return Ok(None);
                }
                Err(error) => {
                    **gate.as_mut().expect("protected purchase holds gate") = Some(error.clone());
                    return Err(error);
                }
            }
        }
        if !context.token.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        let purchase_request = request.clone();
        let activation =
            match provider_call(sms_config.clone(), "acquire_number", move |provider| {
                acquire_telegram_number(provider, &purchase_request)
            })
            .await
            {
                Ok(activation) => activation,
                Err(error) if error == "sms_insufficientbalance" => {
                    if wait_for_balance_refund(context, index, refunds_before_purchase).await? {
                        continue;
                    }
                    return Ok(None);
                }
                Err(_) => return Err("purchase_failed".into()),
            };
        if context.config.stop_on_price_change
            && purchase_price_rejected(&context.config, request, &activation)
        {
            **gate.as_mut().expect("price protection holds gate") = Some("price_changed".into());
        }
        return Ok(Some(activation));
    }
}

async fn wait_for_cancellation_refund(
    queue: &PendingCancellations,
    token: &AtomicBool,
    refunds_before_purchase: usize,
) -> Result<bool, String> {
    loop {
        if !token.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        // Read pending first so the final successful cancellation cannot be missed.
        let pending = queue.pending.load(Ordering::SeqCst);
        if queue.refunds.load(Ordering::SeqCst) != refunds_before_purchase {
            return Ok(true);
        }
        if pending == 0 {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn wait_for_balance_refund(
    context: &Context,
    index: usize,
    refunds: usize,
) -> Result<bool, String> {
    if context.cancellations.pending.load(Ordering::SeqCst) > 0 {
        context.log(index, "waiting_for_cancellation_refund");
    }
    let retry =
        wait_for_cancellation_refund(&context.cancellations, &context.token, refunds).await?;
    if retry {
        context.log(index, "checking_balance_after_refund");
    }
    Ok(retry)
}

fn purchase_price_rejected(
    config: &RegistrarTaskConfig,
    request: &NumberRequest,
    activation: &Activation,
) -> bool {
    activation.cost.is_some_and(|price| {
        !price.is_finite()
            || price < config.min_price.unwrap_or(0.0)
            || price > request.max_price.unwrap_or(f64::INFINITY)
            || (config.stop_on_price_change
                && config
                    .expected_prices
                    .get(&request.country)
                    .is_some_and(|expected| (expected - price).abs() > 0.000001))
    })
}

async fn process_attempt(
    context: Arc<Context>,
    index: usize,
    mut attempt: RegistrationAttempt,
    mut request: NumberRequest,
    countries: Vec<String>,
    sms_config: SmsProviderConfig,
) -> Result<bool, String> {
    if !context.token.load(Ordering::Relaxed) {
        return Ok(false);
    }
    let config = &context.config;
    attempt.proxy = crate::proxy::select_proxy_for_account(None)?;
    if context.config.manual_locale {
        apply_locale(
            &mut attempt.profile,
            &context.config.registration_language,
            context.config.timezone_offset_seconds,
        );
    } else if let Some(proxy) = attempt.proxy.as_ref() {
        match proxy_locale(&context, proxy).await {
            Ok(locale) => apply_locale(
                &mut attempt.profile,
                &locale.language,
                locale.timezone_offset_seconds,
            ),
            Err(_) => context.log(index, "proxy_locale_unavailable"),
        }
    }
    context.log(
        index,
        if attempt.proxy.is_some() {
            "proxy_selected"
        } else {
            "proxy_unavailable"
        },
    );
    context.log(index, "buying_number");
    let purchase =
        purchase_for_attempt(&context, index, &mut request, &countries, &sms_config).await;
    let activation = match purchase {
        Ok(Some(activation)) => activation,
        Ok(None) => {
            context.log(index, "budget_or_price_limit");
            return Err("budget_or_price_limit".into());
        }
        Err(error) if error == "purchase_failed" => {
            let record = RegistrationRecord {
                phone: String::new(),
                provider: config.sms_provider.kind.label().into(),
                activation_id: None,
                status: "purchase_failed".into(),
                account_id: None,
                error: Some("SMS number purchase failed".into()),
            };
            context.database.lock().await.insert(&record)?;
            context.log(index, "purchase_failed");
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    context.log(index, "number_purchased");
    let mut record = RegistrationRecord {
        phone: activation.phone.clone(),
        provider: config.sms_provider.kind.label().into(),
        activation_id: Some(activation.id.clone()),
        status: "number_acquired".into(),
        account_id: None,
        error: None,
    };
    let record_id = match context.database.lock().await.insert(&record) {
        Ok(id) => id,
        Err(error) => {
            cancel_activation(&context, sms_config.clone(), activation.id);
            return Err(error);
        }
    };
    let phone = activation.phone.clone();
    let already_in_accounts = tokio::task::spawn_blocking(move || {
        crate::accounts::commands::get_storage_pub().contains_phone(&phone)
    })
    .await;
    match already_in_accounts {
        Ok(Ok(true)) => {
            cancel_activation(&context, sms_config, activation.id);
            record.status = "number_already_in_accounts".into();
            record.error = Some("Number already exists in Accounts".into());
            context.record(record_id, &record).await?;
            context.log(index, "number_already_in_accounts");
            return Ok(false);
        }
        Ok(Ok(false)) => {}
        _ => {
            cancel_activation(&context, sms_config, activation.id);
            record.status = "duplicate_check_failed".into();
            record.error = Some("Failed to inspect stored accounts".into());
            context.record(record_id, &record).await?;
            context.log(index, "duplicate_check_failed");
            return Err("Failed to inspect stored accounts".into());
        }
    }
    if purchase_price_rejected(config, &request, &activation) {
        cancel_activation(&context, sms_config.clone(), activation.id);
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
        &sms_config,
        record_id,
        &mut record,
    )
    .await;
    if record.account_id.is_none() && record.status != "registration_storage_failed" {
        cancel_activation(&context, sms_config, activation.id);
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
            if record.status != "registration_storage_failed" && error != "used_number" {
                let (status, message) = if record.account_id.is_some() {
                    ("registered_with_warnings", safe_error(&error))
                } else if error == "sms_timeout" {
                    ("sms_timeout", safe_error(&error))
                } else if telegram_rpc_code(&error) == Some("PHONE_NUMBER_BANNED") {
                    ("phone_number_banned", safe_error(&error))
                } else if telegram_rpc_code(&error).is_some_and(is_premium_required_code) {
                    ("premium_required", safe_error(&error))
                } else if telegram_rpc_code(&error)
                    .is_some_and(|code| code.starts_with("2FA_CONFIRM_WAIT_"))
                {
                    ("account_reset_pending", safe_error(&error))
                } else if error == "email_reset_unavailable"
                    || error == "email_reset_already_requested"
                {
                    ("login_email_reset_pending", safe_error(&error))
                } else {
                    ("registration_failed", safe_error(&error))
                };
                record.status = status.into();
                record.error = Some(message);
            } else {
                record.error = Some(safe_error(&error));
            }
            context.record(record_id, &record).await?;
            for message in failure_log_messages(
                &record.status,
                record.error.as_deref().unwrap_or("unknown_error"),
            ) {
                context.log(index, message);
            }
            Ok(record.account_id.is_some())
        }
    }
}

fn failure_log_messages<'a>(status: &'a str, error: &'a str) -> Vec<&'a str> {
    if status == "phone_number_banned" || status == error {
        vec![error]
    } else {
        vec![status, error]
    }
}

fn acquire_telegram_number(
    provider: &dyn crate::registrar::sms::SmsProvider,
    request: &NumberRequest,
) -> Result<crate::registrar::sms::Activation, crate::registrar::sms::SmsError> {
    let aliases = if matches!(
        request.service.to_ascii_lowercase().as_str(),
        "tg" | "tgs" | "telegram"
    ) {
        &["tg", "tgs", "telegram"][..]
    } else {
        &[request.service.as_str()][..]
    };
    let mut last_error = None;
    for service in aliases {
        let mut variant = request.clone();
        variant.service = (*service).to_string();
        match provider.acquire_number(&variant) {
            Ok(activation) => return Ok(activation),
            Err(error)
                if matches!(
                    error.kind,
                    SmsErrorKind::NoNumbers
                        | SmsErrorKind::InvalidRequest
                        | SmsErrorKind::Unsupported
                ) =>
            {
                last_error = Some(error)
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.expect("at least one Telegram service alias"))
}

fn safe_error(error: &str) -> String {
    if error == "registrar.emailAccountsExhausted" {
        return "email_accounts_exhausted".into();
    }
    if error
        .strip_prefix("email_reset_wait:")
        .and_then(|value| value.parse::<u64>().ok())
        .is_some()
    {
        return "email_reset_unavailable".into();
    }
    if error == "used_number" {
        return "telegram_existing_email_required".into();
    }
    if error.starts_with("sms_")
        && error.len() <= 128
        && error
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '_')
    {
        return error.to_string();
    }
    match error {
        "stopped"
        | "sms_timeout"
        | "unsupported_verification_type"
        | "app_code_rejected"
        | "activation_unavailable"
        | "sms_proxy_unavailable"
        | "email_verification_timeout"
        | "recovery_email_timeout"
        | "recovery_email_rejected_limit"
        | "mail_tm_unavailable"
        | "mail_tm_rate_limited"
        | "mail_tm_invalid_response"
        | "mail_tm_no_domains"
        | "email_reset_already_requested"
        | "email_reset_unavailable"
        | RECAPTCHA_ANTISAFETY_REQUIRED
        | "re_registration_retry_limit" => return error.to_string(),
        _ => {}
    }
    if let Some(code) = telegram_rpc_code(error) {
        if code
            .strip_prefix("2FA_CONFIRM_WAIT_")
            .and_then(|value| value.parse::<u64>().ok())
            .is_some()
        {
            return "account_reset_unavailable".into();
        }
        if let Some(days) = premium_days_from_code(code) {
            return format!("telegram_premium_required_days:{days}");
        }
        for prefix in ["FLOOD_WAIT_", "FLOOD_PREMIUM_WAIT_"] {
            if let Some(seconds) = code.strip_prefix(prefix) {
                if let Ok(seconds) = seconds.parse::<u64>() {
                    return format!("telegram_flood_wait:{seconds}");
                }
            }
        }
        let message = match code {
            "FLOOD_WAIT" | "FLOOD_PREMIUM_WAIT" => "telegram_flood_wait_unknown",
            "PHONE_NUMBER_BANNED" => "telegram_phone_banned",
            "PHONE_NUMBER_FLOOD" | "PHONE_PASSWORD_FLOOD" => "telegram_number_rate_limited",
            "PHONE_NUMBER_INVALID" => "telegram_phone_invalid",
            "PHONE_NUMBER_OCCUPIED" => "telegram_phone_occupied",
            "PHONE_CODE_INVALID" | "PHONE_CODE_EMPTY" => "telegram_code_invalid",
            "PHONE_CODE_EXPIRED" | "PHONE_CODE_HASH_EMPTY" | "PHONE_HASH_EXPIRED" => {
                "telegram_code_expired"
            }
            "API_ID_INVALID" | "API_ID_PUBLISHED_FLOOD" => "telegram_api_invalid",
            "SMS_CODE_CREATE_FAILED" | "SEND_CODE_UNAVAILABLE" => "telegram_sms_unavailable",
            "UPDATE_APP_TO_LOGIN" => "telegram_client_outdated",
            "AUTH_RESTART" | "SIGN_IN_FAILED" => "telegram_auth_retry",
            code if code.strip_prefix("AUTH_RESTART_").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            }) =>
            {
                "telegram_auth_retry"
            }
            "AUTH_PAYMENT_REQUIRED" => "telegram_premium_required",
            "2FA_RECENT_CONFIRM" => "account_reset_cancelled",
            "PHONE_NUMBER_APP_SIGNUP_FORBIDDEN" => "telegram_app_signup_forbidden",
            "EMAIL_NOT_ALLOWED" | "EMAIL_INVALID" => "telegram_email_rejected",
            "EMAIL_CODE_INVALID" | "CODE_INVALID" => "telegram_email_code_invalid",
            "EMAIL_VERIFY_EXPIRED" | "EMAIL_CODE_EXPIRED" | "EMAIL_HASH_EXPIRED" => {
                "telegram_email_code_expired"
            }
            "USERNAME_OCCUPIED" | "USERNAME_PURCHASE_AVAILABLE" => "telegram_username_occupied",
            "USERNAME_INVALID" => "telegram_username_invalid",
            "FIRSTNAME_INVALID" | "LASTNAME_INVALID" => "telegram_name_invalid",
            "SESSION_PASSWORD_NEEDED" => "telegram_2fa_required",
            "PASSWORD_HASH_INVALID" => "telegram_2fa_invalid",
            _ => "registration_step_failed",
        };
        if message != "registration_step_failed" {
            return message.to_string();
        }
        if is_safe_unknown_rpc_code(error, code) {
            return format!("telegram_unknown_rpc:{code}");
        }
    }
    "registration_step_failed".to_string()
}

fn is_premium_required_code(code: &str) -> bool {
    code == "AUTH_PAYMENT_REQUIRED" || premium_days_from_code(code).is_some()
}

fn premium_days_from_code(code: &str) -> Option<u32> {
    code.strip_prefix("AUTH_PAYMENT_REQUIRED_")?
        .strip_suffix("_DAYS")?
        .parse::<u32>()
        .ok()
        .filter(|days| *days > 0 && *days <= 3650)
}

fn telegram_rpc_code(error: &str) -> Option<&str> {
    let payload = if error.starts_with("RPC ") || error.starts_with("rpc error ") {
        error.split_once(": ")?.1
    } else {
        error
    };
    let code = payload
        .split(|character: char| {
            !character.is_ascii_uppercase() && !character.is_ascii_digit() && character != '_'
        })
        .next()?;
    (code.len() <= 128 && code.contains('_')).then_some(code)
}

fn is_safe_unknown_rpc_code(error: &str, code: &str) -> bool {
    let payload = if error.starts_with("RPC ") || error.starts_with("rpc error ") {
        error.split_once(": ").map(|(_, payload)| payload)
    } else {
        None
    };
    payload == Some(code)
        && code.len() <= 64
        && code.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        && !code.starts_with("RECAPTCHA_CHECK_")
        && code.split('_').count() >= 2
        && code.split('_').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        })
}

async fn open_registration_session(
    context: &Context,
    index: usize,
    attempt: &RegistrationAttempt,
    activation: &crate::registrar::sms::Activation,
    sms_config: &SmsProviderConfig,
    record_id: i64,
    record: &mut RegistrationRecord,
    previous_login_mail: &str,
) -> Result<RegistrationSession, String> {
    let config = &context.config;
    context.log(index, "requesting_telegram_code");
    let (mut session, sent_code) = RegistrationSession::start(RegistrationCodeRequest {
        phone: activation.phone.clone(),
        profile: attempt.profile.clone(),
        proxy: attempt.proxy.clone(),
        code_settings: config.code_settings.clone(),
        antisafety: config.antisafety.clone(),
        antisafety_proxy: sms_config.http_proxy.clone(),
        cancellation: Some(context.token.clone()),
        log_target: Some(context.app.clone()),
        log_prefix: format!(
            "[{}][{}]",
            context
                .output
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            index
        ),
    })
    .await?;
    session.set_output_directory(context.output.clone());
    let mut sent_code = if sent_code.code_type == "email_setup" {
        context.log(index, "setting_up_login_email");
        setup_login_email(
            context,
            index,
            &mut session,
            sms_config.http_proxy.as_deref(),
            &context.token,
        )
        .await?
    } else {
        sent_code
    };
    context.log(
        index,
        match sent_code.code_type.as_str() {
            "sms" | "sms_word" | "sms_phrase" => "code_delivery_sms",
            "app" => "code_delivery_app",
            "email" => "code_delivery_email",
            _ => "code_delivery_other",
        },
    );
    if sent_code.code_type == "email" {
        if config.re_register_existing {
            sent_code =
                reset_unavailable_login_email(context, index, &mut session, sent_code).await?;
            if sent_code.code_type == "email_setup" {
                context.log(index, "setting_up_login_email");
                sent_code = setup_login_email(
                    context,
                    index,
                    &mut session,
                    sms_config.http_proxy.as_deref(),
                    &context.token,
                )
                .await?;
            }
            context.log(
                index,
                match sent_code.code_type.as_str() {
                    "sms" | "sms_word" | "sms_phrase" => "code_delivery_sms",
                    "app" => "code_delivery_app",
                    "email" => "code_delivery_email",
                    _ => "code_delivery_other",
                },
            );
        }
    }
    if sent_code.code_type == "email" {
        let _ = session.cancel().await;
        record.status = "used_number".into();
        record.error = Some(safe_error("used_number"));
        context.record(record_id, record).await?;
        return Err("used_number".into());
    }
    if sent_code.code_type == "app" && config.reject_app_code {
        if let Some(wait_secs) = app_sms_fallback_delay(&sent_code, MAX_APP_SMS_FALLBACK_WAIT_SECS)
        {
            context.log(index, "waiting_for_sms_fallback");
            for _ in 0..wait_secs {
                if !context.token.load(Ordering::Relaxed) {
                    let _ = session.cancel().await;
                    return Err("stopped".into());
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            context.log(index, "requesting_sms_fallback");
            sent_code = match session.resend().await {
                Ok(value) => value,
                Err(error) => {
                    let _ = session.cancel().await;
                    return Err(error);
                }
            };
            context.log(
                index,
                if matches!(
                    sent_code.code_type.as_str(),
                    "sms" | "sms_word" | "sms_phrase"
                ) {
                    "code_delivery_sms"
                } else {
                    "code_delivery_other"
                },
            );
        }
        if sent_code.code_type == "app" {
            context.log(index, "sms_fallback_unavailable");
            let _ = session.cancel().await;
            return Err("app_code_rejected".into());
        }
    }
    if !matches!(
        sent_code.code_type.as_str(),
        "sms" | "sms_word" | "sms_phrase" | "app"
    ) {
        let _ = session.cancel().await;
        return Err("unsupported_verification_type".into());
    }
    if session.login_mail().is_empty() {
        session.set_login_mail(previous_login_mail.to_string());
    }
    Ok(session)
}

fn app_sms_fallback_delay(sent_code: &SentCode, max_wait_secs: u64) -> Option<u64> {
    if sent_code.code_type != "app" || sent_code.next_type.as_deref() != Some("sms") {
        return None;
    }
    let wait = u64::from(sent_code.timeout_secs?) + 1;
    (wait <= max_wait_secs).then_some(wait)
}

async fn reset_unavailable_login_email(
    context: &Context,
    index: usize,
    session: &mut RegistrationSession,
    sent: SentCode,
) -> Result<SentCode, String> {
    if !context.token.load(Ordering::Relaxed) {
        return Err("stopped".into());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "invalid system clock")?
        .as_secs();
    if !login_email_reset_available(&sent, now) {
        return Err("email_reset_unavailable".into());
    }
    context.log(index, "resetting_login_email");
    let sent = match session.reset_login_email().await {
        Ok(value) => value,
        Err(error) if telegram_rpc_code(&error) == Some("TASK_ALREADY_EXISTS") => {
            return Err("email_reset_unavailable".into());
        }
        Err(error) => return Err(error),
    };
    if sent.code_type == "email" {
        return Err("email_reset_unavailable".into());
    }
    context.log(index, "login_email_reset_completed");
    Ok(sent)
}

fn login_email_reset_available(sent: &SentCode, now: u64) -> bool {
    if let Some(date) = sent.email_reset_pending_date.filter(|date| *date > 0) {
        return u64::from(date) <= now;
    }
    sent.email_reset_available_secs == Some(0)
}

#[derive(Debug, PartialEq, Eq)]
enum SmsWaitResult {
    Code(String),
    RefreshSession,
}

fn sms_wait_boundary(
    now: tokio::time::Instant,
    deadline: tokio::time::Instant,
    refresh_at: Option<tokio::time::Instant>,
) -> Result<Option<SmsWaitResult>, String> {
    if now >= deadline {
        Err("sms_timeout".into())
    } else if refresh_at.is_some_and(|refresh| now >= refresh) {
        Ok(Some(SmsWaitResult::RefreshSession))
    } else {
        Ok(None)
    }
}

async fn wait_for_sms_code(
    context: &Context,
    activation: &crate::registrar::sms::Activation,
    sms_config: &SmsProviderConfig,
    used_codes: &HashSet<String>,
    deadline: tokio::time::Instant,
    refresh_at: Option<tokio::time::Instant>,
) -> Result<SmsWaitResult, String> {
    loop {
        if !context.token.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        if let Some(action) = sms_wait_boundary(tokio::time::Instant::now(), deadline, refresh_at)?
        {
            return Ok(action);
        }
        let id = activation.id.clone();
        let boundary = refresh_at.unwrap_or(deadline).min(deadline);
        let status = match tokio::time::timeout_at(
            boundary,
            provider_call(sms_config.clone(), "activation_status", move |provider| {
                provider.activation_status(&id)
            }),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => continue,
        };
        if !context.token.load(Ordering::Relaxed) || tokio::time::Instant::now() >= deadline {
            return Err(if context.token.load(Ordering::Relaxed) {
                "sms_timeout"
            } else {
                "stopped"
            }
            .into());
        }
        if let Some(code) = fresh_sms_code(&status, used_codes) {
            return Ok(SmsWaitResult::Code(code));
        }
        match status {
            ActivationStatus::Cancelled
            | ActivationStatus::Banned
            | ActivationStatus::Expired
            | ActivationStatus::Finished => {
                return Err("activation_unavailable".into());
            }
            _ => {}
        }
        // Cancellation is checked every second, even with a longer provider poll interval.
        for _ in 0..context.config.poll_interval_secs {
            if !context.token.load(Ordering::Relaxed) || tokio::time::Instant::now() >= boundary {
                break;
            }
            tokio::time::sleep_until(
                (tokio::time::Instant::now() + Duration::from_secs(1)).min(boundary),
            )
            .await;
        }
    }
}

fn fresh_sms_code(status: &ActivationStatus, used_codes: &HashSet<String>) -> Option<String> {
    match status {
        ActivationStatus::CodeReceived { code, .. }
            if !code.trim().is_empty() && !used_codes.contains(code.trim()) =>
        {
            Some(code.trim().to_string())
        }
        _ => None,
    }
}

fn retry_expired_code(error: &str, registered: bool, restarts: usize) -> bool {
    !registered
        && restarts < MAX_EXPIRED_CODE_RESTARTS
        && telegram_rpc_code(error) == Some("PHONE_CODE_EXPIRED")
}

fn activation_deadline(activation: &Activation) -> Option<tokio::time::Instant> {
    let expires = chrono::DateTime::parse_from_rfc3339(activation.expires_at.as_deref()?).ok()?;
    let remaining = (expires.with_timezone(&chrono::Utc) - chrono::Utc::now())
        .to_std()
        .unwrap_or(Duration::ZERO);
    tokio::time::Instant::now().checked_add(remaining)
}

fn post_deletion_wait_deadline(
    started: tokio::time::Instant,
    expires: Option<tokio::time::Instant>,
) -> tokio::time::Instant {
    let deadline = started + POST_DELETION_SMS_WAIT;
    expires.map_or(deadline, |expires| expires.min(deadline))
}

fn post_deletion_wait_result(
    result: Result<SmsWaitResult, String>,
    now: tokio::time::Instant,
    expires: Option<tokio::time::Instant>,
) -> Result<SmsWaitResult, String> {
    if expires.is_some_and(|expires| now >= expires) {
        return Err("activation_unavailable".into());
    }
    match result {
        Err(error) if error == "sms_timeout" => Ok(SmsWaitResult::RefreshSession),
        result => result,
    }
}

async fn prepare_next_sms(
    sms_config: &SmsProviderConfig,
    activation: &Activation,
) -> Result<(), String> {
    let id = activation.id.clone();
    provider_call(
        sms_config.clone(),
        "request_another_code",
        move |provider| {
            match provider.update_activation(&id, ActivationAction::RequestAnotherCode) {
                Ok(_) => Ok(()),
                // Providers such as 5SIM expose new messages without a separate retry action.
                Err(error) if error.kind == SmsErrorKind::Unsupported => Ok(()),
                Err(error) => Err(error),
            }
        },
    )
    .await
}

fn different_registration_proxy(
    pool: &[ProxyConfig],
    current: Option<&ProxyConfig>,
) -> Option<ProxyConfig> {
    let candidates: Vec<_> = pool
        .iter()
        .filter(|candidate| {
            !current.is_some_and(|current| {
                candidate.proxy_type == current.proxy_type
                    && candidate.host.eq_ignore_ascii_case(&current.host)
                    && candidate.port == current.port
                    && candidate.username == current.username
                    && candidate.password == current.password
            })
        })
        .collect();
    candidates
        .choose(&mut rand::thread_rng())
        .map(|proxy| (*proxy).clone())
}

async fn rotate_registration_proxy(
    context: &Context,
    index: usize,
    attempt: &mut RegistrationAttempt,
) {
    if let Some(proxy) =
        different_registration_proxy(&ProxyList::load().proxies, attempt.proxy.as_ref())
    {
        context.log(index, "registration_proxy_changed");
        if !context.config.manual_locale {
            match proxy_locale(context, &proxy).await {
                Ok(locale) => apply_locale(
                    &mut attempt.profile,
                    &locale.language,
                    locale.timezone_offset_seconds,
                ),
                Err(_) => context.log(index, "proxy_locale_unavailable"),
            }
        }
        attempt.proxy = Some(proxy);
    } else {
        context.log(index, "registration_proxy_unchanged");
    }
}

fn account_deletion_pause() -> Duration {
    Duration::from_secs(
        rand::thread_rng()
            .gen_range(ACCOUNT_DELETION_PAUSE_MIN_SECS..=ACCOUNT_DELETION_PAUSE_MAX_SECS),
    )
}

async fn wait_after_account_deletion(token: &AtomicBool) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + account_deletion_pause();
    loop {
        if !token.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(());
        }
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_secs(1)).min(deadline),
        )
        .await;
    }
}

async fn register_number(
    context: &Context,
    index: usize,
    attempt: &RegistrationAttempt,
    activation: &crate::registrar::sms::Activation,
    sms_config: &SmsProviderConfig,
    record_id: i64,
    record: &mut RegistrationRecord,
) -> Result<bool, String> {
    let mut session_attempt = attempt.clone();
    let attempt = &mut session_attempt;
    let config = &context.config;
    if !context.token.load(Ordering::Relaxed) {
        return Err("stopped".into());
    }
    let mut session = open_registration_session(
        context, index, attempt, activation, sms_config, record_id, record, "",
    )
    .await?;
    let id = activation.id.clone();
    let ready = provider_call(
        sms_config.clone(),
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
    let mut re_register_existing = config.re_register_existing;
    let mut re_registration_restarts = 0;
    let mut expired_code_restarts = 0;
    let mut used_codes = HashSet::new();
    let expires = activation_deadline(activation);
    let mut after_account_deletion = false;
    let account_id = loop {
        record.status = "waiting_code".into();
        if let Err(error) = context.record(record_id, record).await {
            let _ = session.cancel().await;
            return Err(error);
        }
        context.log(index, "waiting_code");
        let started = tokio::time::Instant::now();
        let timeout = Duration::from_secs(config.code_timeout_secs);
        let mut deadline = if after_account_deletion {
            post_deletion_wait_deadline(started, expires)
        } else {
            started + timeout
        };
        let mut refresh_at = if after_account_deletion {
            None
        } else {
            Some(started + timeout / 2)
        };
        let code = loop {
            let result = wait_for_sms_code(
                context,
                activation,
                sms_config,
                &used_codes,
                deadline,
                refresh_at,
            )
            .await;
            let result = if after_account_deletion {
                post_deletion_wait_result(result, tokio::time::Instant::now(), expires)
            } else {
                result
            };
            match result {
                Ok(SmsWaitResult::Code(code)) => break code,
                Ok(SmsWaitResult::RefreshSession) => {
                    refresh_at = None;
                    context.log(
                        index,
                        if after_account_deletion {
                            "re_registration_sms_retry_same_proxy"
                        } else {
                            "sms_wait_session_refresh"
                        },
                    );
                    let restart_deadline = if after_account_deletion {
                        post_deletion_wait_deadline(tokio::time::Instant::now(), expires)
                    } else {
                        deadline
                    };
                    session = tokio::time::timeout_at(restart_deadline, async {
                        let _ = session.cancel().await;
                        prepare_next_sms(sms_config, activation).await?;
                        if !after_account_deletion {
                            rotate_registration_proxy(context, index, attempt).await;
                        }
                        open_registration_session(
                            context,
                            index,
                            attempt,
                            activation,
                            sms_config,
                            record_id,
                            record,
                            session.login_mail(),
                        )
                        .await
                    })
                    .await
                    .map_err(|_| {
                        if after_account_deletion
                            && expires.is_some_and(|expires| tokio::time::Instant::now() >= expires)
                        {
                            "activation_unavailable"
                        } else {
                            "sms_timeout"
                        }
                        .to_string()
                    })??;
                    if after_account_deletion {
                        deadline =
                            post_deletion_wait_deadline(tokio::time::Instant::now(), expires);
                    }
                    context.log(index, "waiting_code");
                }
                Err(error) => {
                    let _ = session.cancel().await;
                    return Err(error);
                }
            }
        };
        used_codes.insert(code.clone());
        context.log(index, "completing_registration");
        let outcome = match session
            .complete(&code, &attempt.identity, re_register_existing)
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                if retry_expired_code(&error, session.is_registered(), expired_code_restarts) {
                    expired_code_restarts += 1;
                    context.log(index, "registration_code_expired_retry");
                    let _ = session.cancel().await;
                    prepare_next_sms(sms_config, activation).await?;
                    if !after_account_deletion {
                        rotate_registration_proxy(context, index, attempt).await;
                    }
                    session = open_registration_session(
                        context,
                        index,
                        attempt,
                        activation,
                        sms_config,
                        record_id,
                        record,
                        session.login_mail(),
                    )
                    .await?;
                    continue;
                }
                if session.is_registered() {
                    record.status = "registration_storage_failed".into();
                    let id = activation.id.clone();
                    if provider_call(sms_config.clone(), "finish", move |provider| {
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
        match outcome {
            RegistrationOutcome::Registered { account_id } => break account_id,
            RegistrationOutcome::AlreadyRegistered => {
                record.status = "already_registered".into();
                context.record(record_id, record).await?;
                context.log(index, "already_registered");
                return Ok(false);
            }
            RegistrationOutcome::ReRegistrationRequired => {
                re_register_existing = false;
                after_account_deletion = true;
                context.log(index, "existing_account_deleted");
                context.log(index, "re_registration_cooldown");
                wait_after_account_deletion(&context.token).await?;
            }
        }
        re_registration_restarts += 1;
        if re_registration_restarts > MAX_REREGISTRATION_RESTARTS {
            return Err("re_registration_retry_limit".into());
        }
        prepare_next_sms(sms_config, activation).await?;
        session = open_registration_session(
            context,
            index,
            attempt,
            activation,
            sms_config,
            record_id,
            record,
            session.login_mail(),
        )
        .await?;
    };
    record.account_id = Some(account_id.clone());
    record.status = "registered".into();
    let mut warnings = Vec::new();
    let id = activation.id.clone();
    if provider_call(sms_config.clone(), "finish", move |provider| {
        provider.update_activation(&id, ActivationAction::Finish)
    })
    .await
    .is_err()
    {
        warnings.push("provider_finish_failed");
    }
    context.record(record_id, record).await?;
    super::flow::pause_for_registration_transition().await;
    if let Some(password) = &config.two_fa_password {
        context.log(index, "setting_2fa");
        let result = if config.set_recovery_email {
            setup_two_fa_recovery_email(context, index, &mut session, &account_id, password).await
        } else {
            session.set_two_fa(&account_id, password).await
        };
        if let Err(error) = result {
            context.log(index, &safe_error(&error));
            warnings.push("two_fa_failed");
        }
    }
    if config.hide_phone_number {
        super::flow::pause_for_registration_transition().await;
        context.log(index, "hiding_phone_number");
        if let Err(error) = session.hide_phone_number().await {
            context.log(index, &safe_error(&error));
            warnings.push("phone_hide_failed");
        }
    }
    if config.hide_online_status {
        super::flow::pause_for_registration_transition().await;
        context.log(index, "hiding_online_status");
        if let Err(error) = session.hide_online_status().await {
            context.log(index, &safe_error(&error));
            warnings.push("online_hide_failed");
        }
    }
    if config.set_avatar {
        super::flow::pause_for_registration_transition().await;
        context.log(index, "setting_avatar");
        if let Err(error) = session
            .set_avatar(&config.avatar_mode, &config.avatar_path)
            .await
        {
            context.log(index, &safe_error(&error));
            warnings.push("avatar_set_failed");
        }
    }
    if config.set_username {
        super::flow::pause_for_registration_transition().await;
        context.log(index, "setting_username");
        let from_list = if config.username_mode == "from_list" {
            context
                .usernames
                .get(context.username_index.fetch_add(1, Ordering::Relaxed))
                .map(String::as_str)
        } else {
            None
        };
        if let Err(error) = session
            .set_username(
                &account_id,
                &config.username_mode,
                &config.username_prefix,
                from_list,
            )
            .await
        {
            context.log(index, &safe_error(&error));
            warnings.push("username_set_failed");
        }
    }
    if config.send_push {
        context.log(index, "registering_push");
        if let Err(error) = register_push(
            &mut session,
            config
                .antisafety
                .clone()
                .ok_or("missing Push configuration")?,
            sms_config.http_proxy.as_deref(),
            &context.token,
        )
        .await
        {
            context.log(index, &safe_error(&error));
            warnings.push("push_failed");
        }
    }
    super::flow::pause_for_registration_transition().await;
    context.log(index, "loading_initial_dialogs");
    if let Err(error) = session.load_initial_dialogs().await {
        dbg_log!(
            "registrar::initial_dialogs failed {}",
            super::diagnostics::error_summary(&error)
        );
        context.log(index, &safe_error(&error));
        warnings.push("initial_dialogs_failed");
    }
    super::flow::pause_for_registration_transition().await;
    context.log(index, "loading_initial_contacts");
    if let Err(error) = session.load_initial_contacts().await {
        dbg_log!(
            "registrar::initial_contacts failed {}",
            super::diagnostics::error_summary(&error)
        );
        context.log(index, &safe_error(&error));
        warnings.push("initial_contacts_failed");
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
    http_proxy: Option<&str>,
    token: &AtomicBool,
) -> Result<(), String> {
    if !token.load(Ordering::Relaxed) {
        return Err("stopped".into());
    }
    let client = AntiSafetyClient::new_with_proxy(config, http_proxy)?;
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

enum EmailCandidate {
    AntiSafety {
        client: AntiSafetyClient,
        id: String,
        premium: bool,
        address: String,
    },
    Imap {
        account: MailboxAccount,
        baseline: u32,
        seen: std::collections::HashSet<u32>,
    },
    MailTm {
        account: MailTmMailbox,
        seen: HashSet<String>,
    },
}

impl EmailCandidate {
    fn address(&self) -> &str {
        match self {
            Self::AntiSafety { address, .. } => address,
            Self::Imap { account, .. } => &account.address,
            Self::MailTm { account, .. } => &account.address,
        }
    }

    fn credentials(&self) -> String {
        match self {
            Self::AntiSafety { .. } => String::new(),
            Self::Imap { account, .. } => account.credentials(),
            Self::MailTm { account, .. } => account.credentials(),
        }
    }

    async fn poll_code(&mut self) -> Result<Option<String>, String> {
        self.poll_code_with_length(None).await
    }

    async fn poll_code_with_length(
        &mut self,
        code_length: Option<usize>,
    ) -> Result<Option<String>, String> {
        match self {
            Self::MailTm { account, seen } => {
                let account = account.clone();
                let mut polled = std::mem::take(seen);
                let (result, updated) = tokio::task::spawn_blocking(move || {
                    let result = if code_length.is_some() {
                        account.code_after_with_length(&mut polled, code_length)
                    } else {
                        account.code_after(&mut polled)
                    };
                    (result, polled)
                })
                .await
                .map_err(|_| "mail_tm_unavailable")?;
                *seen = updated;
                result
            }
            Self::AntiSafety {
                client,
                id,
                premium,
                ..
            } => {
                let client = client.clone();
                let id = id.clone();
                let premium = *premium;
                tokio::task::spawn_blocking(move || client.get_email(&id, premium))
                    .await
                    .map_err(|_| "Email worker failed".to_string())?
                    .map(Some)
                    .or(Ok(None))
            }
            Self::Imap {
                account,
                baseline,
                seen,
            } => {
                let account = account.clone();
                let baseline = *baseline;
                let mut polled = std::mem::take(seen);
                let result = tokio::task::spawn_blocking(move || {
                    let result = if code_length.is_some() {
                        account.code_after_with_length(baseline, &mut polled, code_length)
                    } else {
                        account.code_after(baseline, &mut polled)
                    };
                    (result, polled)
                })
                .await
                .map_err(|_| "IMAP worker failed".to_string())?;
                *seen = result.1;
                result.0
            }
        }
    }
}

async fn next_email_candidate(
    context: &Context,
    index: usize,
    http_proxy: Option<&str>,
) -> Result<EmailCandidate, String> {
    match context.config.email_source {
        EmailSource::AntiSafety => {
            let config = context
                .config
                .antisafety
                .clone()
                .ok_or("missing AntiSafety configuration")?;
            let client = AntiSafetyClient::new_with_proxy(config, http_proxy)?;
            let premium = context.config.email_premium;
            let task_client = client.clone();
            let result = tokio::task::spawn_blocking(move || task_client.create_email(premium))
                .await
                .map_err(|_| "Email worker failed")?;
            let (task, premium) = match result {
                Ok(task) => (task, premium),
                Err(_) if context.config.allow_email_fallback => {
                    let fallback_client = client.clone();
                    let task =
                        tokio::task::spawn_blocking(move || fallback_client.create_email(!premium))
                            .await
                            .map_err(|_| "Email worker failed")??;
                    context.log(index, "login_email_fallback_type_selected");
                    (task, !premium)
                }
                Err(error) => return Err(error),
            };
            Ok(EmailCandidate::AntiSafety {
                client,
                id: task.id,
                premium,
                address: task.email,
            })
        }
        EmailSource::Firstmail | EmailSource::Gmx => next_imap_candidate(context, false).await,
    }
}

async fn next_imap_candidate(context: &Context, recovery: bool) -> Result<EmailCandidate, String> {
    let (mailboxes, index) = if recovery {
        (&context.recovery_mailboxes, &context.recovery_mailbox_index)
    } else {
        (&context.mailboxes, &context.mailbox_index)
    };
    let account = loop {
        let position = index.fetch_add(1, Ordering::Relaxed);
        let account = mailboxes
            .get(position)
            .ok_or("registrar.emailAccountsExhausted")?
            .clone();
        if context
            .reserved_mailboxes
            .lock()
            .await
            .insert(account.address.to_ascii_lowercase())
        {
            break account;
        }
    };
    let baseline = tokio::task::spawn_blocking({
        let account = account.clone();
        move || account.latest_uid()
    })
    .await
    .map_err(|_| "IMAP worker failed")??;
    Ok(EmailCandidate::Imap {
        account,
        baseline,
        seen: HashSet::new(),
    })
}

async fn setup_two_fa_recovery_email(
    context: &Context,
    index: usize,
    session: &mut RegistrationSession,
    account_id: &str,
    password: &str,
) -> Result<(), String> {
    const MAX_CANDIDATES: usize = 10;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    tokio::time::timeout_at(deadline, async {
        for _ in 0..MAX_CANDIDATES {
            if !context.token.load(Ordering::Relaxed) {
                return Err("stopped".into());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("recovery_email_timeout".into());
            }
            context.log(index, "preparing_recovery_email");
            let candidate = if context.config.recovery_email_source == RecoveryEmailSource::MailTm {
                let client = context.mail_tm.clone();
                tokio::task::spawn_blocking(move || client.create_mailbox())
                    .await
                    .map_err(|_| "mail_tm_unavailable")?
                    .map(|account| EmailCandidate::MailTm {
                        account,
                        seen: HashSet::new(),
                    })
            } else {
                next_imap_candidate(context, true).await
            };
            let mut candidate = match candidate {
                Ok(candidate) => candidate,
                Err(error) if error.starts_with("IMAP ") => {
                    context.log(index, "recovery_email_mailbox_unavailable");
                    continue;
                }
                Err(error) => return Err(error),
            };
            let credentials = candidate.credentials();
            context.log(index, "setting_recovery_email");
            let code_length = match session
                .begin_two_fa_with_email(account_id, password, &credentials)
                .await
            {
                Ok(crate::accounts::actions::PasswordSetup::Applied) => {
                    context.log(index, "recovery_email_confirmed");
                    return Ok(());
                }
                Ok(crate::accounts::actions::PasswordSetup::EmailConfirmationRequired {
                    code_length,
                }) => code_length,
                Err(error) if is_email_rejected(&error) => {
                    context.log(index, "recovery_email_rejected");
                    continue;
                }
                Err(error) => return Err(error),
            };
            context.log(index, "waiting_recovery_email_code");
            loop {
                if !context.token.load(Ordering::Relaxed) {
                    return Err("stopped".into());
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err("recovery_email_timeout".into());
                }
                match candidate.poll_code_with_length(code_length).await {
                    Ok(Some(code)) => {
                        match session
                            .confirm_two_fa_email(account_id, password, &code)
                            .await
                        {
                            Ok(()) => {
                                context.log(index, "recovery_email_confirmed");
                                return Ok(());
                            }
                            Err(error) if telegram_rpc_code(&error) == Some("CODE_INVALID") => {}
                            Err(error) => return Err(error),
                        }
                    }
                    Ok(None) => {}
                    Err(error)
                        if error == "mail_tm_rate_limited"
                            || error == "mail_tm_unavailable"
                            || error.starts_with("IMAP ") => {}
                    Err(error) => return Err(error),
                }
                for _ in 0..5 {
                    if !context.token.load(Ordering::Relaxed) {
                        return Err("stopped".into());
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
        Err("recovery_email_rejected_limit".into())
    })
    .await
    .map_err(|_| "recovery_email_timeout".to_string())?
}

async fn setup_login_email(
    context: &Context,
    index: usize,
    session: &mut RegistrationSession,
    http_proxy: Option<&str>,
    token: &AtomicBool,
) -> Result<crate::mtproto::tl::SentCode, String> {
    const MAX_EMAIL_CANDIDATES: usize = 10;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    for candidate_index in 0..MAX_EMAIL_CANDIDATES {
        if !token.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("email_verification_timeout".into());
        }
        let mut candidate = match next_email_candidate(context, index, http_proxy).await {
            Ok(candidate) => candidate,
            Err(error) if error.starts_with("IMAP ") => {
                context.log(index, "login_email_mailbox_unavailable");
                continue;
            }
            Err(error) => return Err(error),
        };
        if candidate_index > 0 {
            context.log(index, "login_email_trying_another");
        }
        match session
            .request_login_email_verification(candidate.address())
            .await
        {
            Ok(()) => {}
            Err(error) if is_email_rejected(&error) => {
                context.log(index, "login_email_rejected");
                continue;
            }
            Err(error) => {
                context.log(index, &safe_error(&error));
                return Err(error);
            }
        }
        loop {
            if !token.load(Ordering::Relaxed) {
                return Err("stopped".into());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("email_verification_timeout".into());
            }
            match candidate.poll_code().await? {
                Some(code) => match session.confirm_login_email_verification(&code).await {
                    Ok(sent_code) => {
                        session.set_login_mail(candidate.credentials());
                        return Ok(sent_code);
                    }
                    Err(error) if is_email_rejected(&error) => {
                        context.log(index, "login_email_rejected");
                        break;
                    }
                    Err(error) => return Err(error),
                },
                None => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }
    Err("registrar.emailRejectedLimit".into())
}

fn is_email_rejected(error: &str) -> bool {
    error.contains("EMAIL_NOT_ALLOWED") || error.contains("EMAIL_INVALID")
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

fn apply_locale(profile: &mut RegistrationProfile, language: &str, timezone_offset_seconds: i32) {
    profile.lang_code = language.to_string();
    profile.system_lang_code = if language == "ru" {
        "ru-RU".to_string()
    } else {
        "en-US".to_string()
    };
    profile.timezone_offset_seconds = timezone_offset_seconds.clamp(-43_200, 50_400);
}

async fn proxy_locale(context: &Context, proxy: &ProxyConfig) -> Result<ProxyLocale, String> {
    let key = proxy.to_string_repr();
    if let Some(value) = context.proxy_locales.lock().await.get(&key).cloned() {
        return Ok(value);
    }
    let proxy_value = key.clone();
    let result = tokio::task::spawn_blocking(move || {
        let proxy = ureq::Proxy::new(&proxy_value).map_err(|_| "invalid proxy")?;
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(15)))
            .proxy(Some(proxy))
            .build()
            .new_agent();
        let response: serde_json::Value = agent
            .get("https://ipwho.is/")
            .call()
            .map_err(|_| "geo request failed")?
            .body_mut()
            .read_json()
            .map_err(|_| "invalid geo response")?;
        if response.get("success").and_then(|value| value.as_bool()) != Some(true) {
            return Err("geo lookup failed");
        }
        let country = response
            .get("country_code")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let offset = response
            .pointer("/timezone/offset")
            .and_then(|value| value.as_i64())
            .ok_or("timezone missing")? as i32;
        Ok(ProxyLocale {
            language: if country == "RU" {
                "ru".to_string()
            } else {
                "en".to_string()
            },
            timezone_offset_seconds: offset,
        })
    })
    .await
    .map_err(|_| "geo worker failed")??;
    context
        .proxy_locales
        .lock()
        .await
        .insert(key, result.clone());
    Ok(result)
}

fn cancel_activation(context: &Context, config: SmsProviderConfig, activation_id: String) {
    let ticket = context.cancellations.track();
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
        ticket.complete(result.is_ok());
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

    #[tokio::test]
    async fn balance_wait_resumes_after_successful_cancellation() {
        let queue = Arc::new(PendingCancellations::default());
        let ticket = queue.track();
        let completion = tokio::spawn(async move {
            tokio::task::yield_now().await;
            ticket.complete(true);
        });
        assert_eq!(
            wait_for_cancellation_refund(&queue, &AtomicBool::new(true), 0).await,
            Ok(true)
        );
        completion.await.unwrap();
        assert_eq!(queue.pending.load(Ordering::SeqCst), 0);
        assert_eq!(queue.refunds.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_cancellations_do_not_restart_purchases() {
        let queue = Arc::new(PendingCancellations::default());
        let ticket = queue.track();
        ticket.complete(false);
        assert_eq!(
            wait_for_cancellation_refund(&queue, &AtomicBool::new(true), 0).await,
            Ok(false)
        );
        assert_eq!(queue.pending.load(Ordering::SeqCst), 0);
        assert_eq!(queue.refunds.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn completed_refund_is_not_lost_before_the_wait_begins() {
        let queue = Arc::new(PendingCancellations::default());
        queue.track().complete(true);
        assert_eq!(
            wait_for_cancellation_refund(&queue, &AtomicBool::new(true), 0).await,
            Ok(true)
        );
        assert_eq!(
            wait_for_cancellation_refund(&queue, &AtomicBool::new(true), 1).await,
            Ok(false)
        );
    }

    #[tokio::test]
    async fn balance_wait_stops_but_keeps_cancellation_pending() {
        let queue = Arc::new(PendingCancellations::default());
        let ticket = queue.track();
        assert_eq!(
            wait_for_cancellation_refund(&queue, &AtomicBool::new(false), 0).await,
            Err("stopped".into())
        );
        assert_eq!(queue.pending.load(Ordering::SeqCst), 1);
        drop(ticket);
        assert_eq!(queue.pending.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn balance_wait_can_resume_without_waiting_for_every_cancellation() {
        let queue = Arc::new(PendingCancellations::default());
        let remaining = queue.track();
        queue.track().complete(true);
        assert_eq!(
            wait_for_cancellation_refund(&queue, &AtomicBool::new(true), 0).await,
            Ok(true)
        );
        assert_eq!(queue.pending.load(Ordering::SeqCst), 1);
        remaining.complete(false);
    }

    #[test]
    fn session_refresh_selects_a_different_proxy_not_a_duplicate_pool_entry() {
        let current = ProxyConfig::from_string("socks5://192.0.2.1:1080").unwrap();
        let duplicate = ProxyConfig::from_string("socks5://192.0.2.1:1080").unwrap();
        let alternative = ProxyConfig::from_string("socks5://192.0.2.2:1080").unwrap();
        assert!(different_registration_proxy(&[], Some(&current)).is_none());
        assert!(different_registration_proxy(&[duplicate.clone()], Some(&current)).is_none());
        for _ in 0..16 {
            let selected = different_registration_proxy(
                &[duplicate.clone(), alternative.clone()],
                Some(&current),
            )
            .unwrap();
            assert_eq!(selected.host, alternative.host);
        }
        assert!(different_registration_proxy(&[alternative], None).is_some());
    }

    #[test]
    fn phone_ban_and_identical_failure_messages_are_logged_only_once() {
        assert_eq!(
            failure_log_messages("phone_number_banned", "telegram_phone_banned"),
            vec!["telegram_phone_banned"]
        );
        assert_eq!(
            failure_log_messages("sms_timeout", "sms_timeout"),
            vec!["sms_timeout"]
        );
        assert_eq!(
            failure_log_messages("registration_failed", "telegram_code_invalid"),
            vec!["registration_failed", "telegram_code_invalid"]
        );
    }

    #[test]
    fn account_deletion_pause_is_between_two_and_six_seconds() {
        for _ in 0..32 {
            assert!((Duration::from_secs(2)..=Duration::from_secs(6))
                .contains(&account_deletion_pause()));
        }
    }

    #[tokio::test]
    async fn stopping_skips_the_post_deletion_pause() {
        assert_eq!(
            wait_after_account_deletion(&AtomicBool::new(false)).await,
            Err("stopped".into())
        );
    }

    #[test]
    fn post_deletion_sms_wait_repeats_every_two_minutes_without_a_retry_limit() {
        let started = tokio::time::Instant::now();
        for cycle in 0..10 {
            let now = started + POST_DELETION_SMS_WAIT * cycle;
            let deadline = post_deletion_wait_deadline(now, None);
            assert_eq!(deadline - now, Duration::from_secs(120));
            let result =
                sms_wait_boundary(deadline, deadline, None).map(|_| SmsWaitResult::RefreshSession);
            assert_eq!(
                post_deletion_wait_result(result, deadline, None),
                Ok(SmsWaitResult::RefreshSession)
            );
        }
    }

    #[test]
    fn post_deletion_sms_wait_stops_at_activation_expiry_and_preserves_other_errors() {
        let started = tokio::time::Instant::now();
        let expires = started + Duration::from_secs(250);
        let third_cycle = started + Duration::from_secs(240);
        assert_eq!(
            post_deletion_wait_deadline(third_cycle, Some(expires)),
            expires
        );
        assert_eq!(
            post_deletion_wait_result(Err("sms_timeout".into()), expires, Some(expires)),
            Err("activation_unavailable".into())
        );
        for error in ["stopped", "activation_unavailable", "sms_transport"] {
            assert_eq!(
                post_deletion_wait_result(Err(error.into()), started, Some(expires)),
                Err(error.into())
            );
        }
        assert_eq!(
            post_deletion_wait_result(
                Ok(SmsWaitResult::Code("22222".into())),
                started,
                Some(expires)
            ),
            Ok(SmsWaitResult::Code("22222".into()))
        );
    }

    #[test]
    fn sms_activation_expiry_uses_the_provider_timestamp() {
        let mut activation = Activation {
            id: "test".into(),
            phone: "+79990001122".into(),
            cost: None,
            country: None,
            operator: None,
            expires_at: None,
        };
        assert_eq!(activation_deadline(&activation), None);
        activation.expires_at = Some("invalid date".into());
        assert_eq!(activation_deadline(&activation), None);
        activation.expires_at = Some("2020-01-01T00:00:00Z".into());
        assert!(activation_deadline(&activation).unwrap() <= tokio::time::Instant::now());
        activation.expires_at =
            Some((chrono::Utc::now() + chrono::Duration::seconds(90)).to_rfc3339());
        let remaining = activation_deadline(&activation).unwrap() - tokio::time::Instant::now();
        assert!(remaining > Duration::from_secs(80) && remaining <= Duration::from_secs(90));
    }

    #[test]
    fn sms_session_refresh_happens_halfway_without_extending_the_deadline() {
        let started = tokio::time::Instant::now();
        for seconds in [30, 31, 300, 900] {
            let timeout = Duration::from_secs(seconds);
            let deadline = started + timeout;
            let refresh = started + timeout / 2;
            assert_eq!(
                sms_wait_boundary(started, deadline, Some(refresh)),
                Ok(None)
            );
            assert_eq!(
                sms_wait_boundary(refresh, deadline, Some(refresh)),
                Ok(Some(SmsWaitResult::RefreshSession))
            );
            assert_eq!(sms_wait_boundary(refresh, deadline, None), Ok(None));
            assert_eq!(
                sms_wait_boundary(deadline, deadline, None),
                Err("sms_timeout".into())
            );
            assert_eq!(
                sms_wait_boundary(deadline, deadline, Some(refresh)),
                Err("sms_timeout".into())
            );
        }
    }

    #[test]
    fn expired_code_restarts_are_bounded_and_never_retry_a_registered_account() {
        let error = "RPC 400: PHONE_CODE_EXPIRED";
        assert!(retry_expired_code(error, false, 0));
        assert!(retry_expired_code(error, false, 1));
        assert!(!retry_expired_code(error, false, MAX_EXPIRED_CODE_RESTARTS));
        assert!(!retry_expired_code(error, true, 0));
        for error in [
            "RPC 400: PHONE_CODE_INVALID",
            "RPC 420: FLOOD_WAIT_30",
            "RPC 400: SESSION_PASSWORD_NEEDED",
            "RPC 400: PHONE_CODE_EXPIRED_OTHER",
            "connection timed out",
        ] {
            assert!(!retry_expired_code(error, false, 0));
        }
    }

    #[test]
    fn restarted_registration_ignores_all_previously_consumed_sms_codes() {
        let mut used_codes = HashSet::new();
        let received = |code: &str| ActivationStatus::CodeReceived {
            code: code.into(),
            text: None,
        };
        let first = fresh_sms_code(&received("12345"), &used_codes).unwrap();
        used_codes.insert(first);
        assert_eq!(fresh_sms_code(&received("12345"), &used_codes), None);
        assert_eq!(fresh_sms_code(&received(" 12345 "), &used_codes), None);
        let next = fresh_sms_code(&received("67890"), &used_codes).unwrap();
        used_codes.insert(next);
        assert_eq!(fresh_sms_code(&received("12345"), &used_codes), None);
        assert_eq!(fresh_sms_code(&received("67890"), &used_codes), None);
        assert_eq!(fresh_sms_code(&received(" "), &used_codes), None);
        assert_eq!(
            fresh_sms_code(&ActivationStatus::Waiting, &used_codes),
            None
        );
        assert_eq!(
            fresh_sms_code(&ActivationStatus::Expired, &used_codes),
            None
        );
        assert_eq!(
            fresh_sms_code(&received("54321"), &used_codes),
            Some("54321".into())
        );
    }

    #[test]
    fn login_email_reset_requires_immediate_availability_or_expired_pending_reset() {
        let mut sent = SentCode {
            phone_code_hash: "hash".into(),
            code_type: "email".into(),
            next_type: None,
            timeout_secs: None,
            email_reset_available_secs: Some(604800),
            email_reset_pending_date: None,
        };
        assert!(!login_email_reset_available(&sent, 100));
        sent.email_reset_available_secs = None;
        assert!(!login_email_reset_available(&sent, 100));
        sent.email_reset_available_secs = Some(0);
        assert!(login_email_reset_available(&sent, 100));
        sent.email_reset_pending_date = Some(101);
        assert!(!login_email_reset_available(&sent, 100));
        sent.email_reset_available_secs = Some(604800);
        assert!(login_email_reset_available(&sent, 101));
        assert!(login_email_reset_available(&sent, 102));
    }

    #[test]
    fn app_code_resends_only_when_telegram_offers_sms_within_timeout() {
        let mut sent = SentCode {
            phone_code_hash: "hash".into(),
            code_type: "app".into(),
            next_type: Some("sms".into()),
            timeout_secs: Some(60),
            email_reset_available_secs: None,
            email_reset_pending_date: None,
        };
        assert_eq!(app_sms_fallback_delay(&sent, 120), Some(61));
        assert_eq!(app_sms_fallback_delay(&sent, 30), None);
        sent.next_type = Some("call".into());
        assert_eq!(app_sms_fallback_delay(&sent, 120), None);
        sent.next_type = None;
        assert_eq!(app_sms_fallback_delay(&sent, 120), None);
    }

    #[test]
    fn antisafety_credentials_are_required_only_for_selected_features() {
        let config = AntiSafetyConfig {
            api_key: "test-key".into(),
            aid: "test-aid".into(),
        };
        assert!(validate_antisafety(false, EmailSource::Gmx, None).is_ok());
        assert!(validate_antisafety(true, EmailSource::Gmx, None).is_err());
        assert!(validate_antisafety(false, EmailSource::AntiSafety, None).is_err());
        assert!(validate_antisafety(
            true,
            EmailSource::AntiSafety,
            Some(&AntiSafetyConfig {
                api_key: "".into(),
                ..config.clone()
            })
        )
        .is_err());
        assert!(validate_antisafety(true, EmailSource::Gmx, Some(&config)).is_ok());
    }

    #[test]
    fn only_address_rejections_switch_mailboxes() {
        assert!(is_email_rejected("RPC 400: EMAIL_NOT_ALLOWED"));
        assert!(is_email_rejected("RPC 400: EMAIL_INVALID"));
        assert!(!is_email_rejected("RPC 400: PHONE_HASH_EXPIRED"));
        assert!(!is_email_rejected("RPC 400: CODE_INVALID"));
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
            safe_error("used_number"),
            "telegram_existing_email_required"
        );
        assert_eq!(
            safe_error("request https://example.com/?api_key=secret failed"),
            "registration_step_failed"
        );
        assert_eq!(
            safe_error("rpc error 400: PHONE_CODE_INVALID"),
            "telegram_code_invalid"
        );
        assert_eq!(safe_error("sms_transport"), "sms_transport");
        assert_eq!(
            safe_error("registrar.emailAccountsExhausted"),
            "email_accounts_exhausted"
        );
        assert_eq!(
            safe_error(RECAPTCHA_ANTISAFETY_REQUIRED),
            RECAPTCHA_ANTISAFETY_REQUIRED
        );
    }

    #[test]
    fn telegram_errors_are_readable_without_exposing_remote_payloads() {
        assert_eq!(
            safe_error("RPC 420: 2FA_CONFIRM_WAIT_604800"),
            "account_reset_unavailable"
        );
        assert_eq!(
            safe_error("RPC 400: 2FA_RECENT_CONFIRM"),
            "account_reset_cancelled"
        );
        assert_eq!(
            safe_error("email_reset_wait:604800"),
            "email_reset_unavailable"
        );
        assert!(is_premium_required_code("AUTH_PAYMENT_REQUIRED_7_DAYS"));
        assert_eq!(
            safe_error("AUTH_PAYMENT_REQUIRED_7_DAYS"),
            "telegram_premium_required_days:7"
        );
        assert_eq!(safe_error("RPC 500: AUTH_RESTART_2"), "telegram_auth_retry");
        assert_eq!(
            safe_error("RPC 420: FLOOD_WAIT_123"),
            "telegram_flood_wait:123"
        );
        assert_eq!(
            safe_error("FLOOD_PREMIUM_WAIT_45"),
            "telegram_flood_wait:45"
        );
        assert_eq!(
            safe_error("RPC 400: PHONE_NUMBER_FLOOD"),
            "telegram_number_rate_limited"
        );
        assert_eq!(
            safe_error("RPC 400: USERNAME_OCCUPIED"),
            "telegram_username_occupied"
        );
        assert_eq!(
            safe_error("RPC 400: UNKNOWN_SECRET_value"),
            "registration_step_failed"
        );
        assert_eq!(
            safe_error("RPC 400: SOME_NEW_ERROR"),
            "telegram_unknown_rpc:SOME_NEW_ERROR"
        );
        assert_eq!(
            safe_error("RPC 400: RECAPTCHA_CHECK_LOGIN__SITEKEY"),
            "registration_step_failed"
        );
        assert_eq!(
            safe_error("RPC 420: FLOOD_WAIT_123_secret"),
            "registration_step_failed"
        );
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
