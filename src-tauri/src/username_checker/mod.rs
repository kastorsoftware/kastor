// username_checker: check username availability via t.me + fragment.com
// multi-threaded with proxy rotation, optional auto-claim via MTProto

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex as TokioMutex;

use crate::i18n::{t, t_with};
use crate::proxy::{ProxyConfig, ProxyList, ProxyType};
use crate::queue::TaskQueue;

#[derive(Deserialize, Clone, Debug)]
pub struct UsernameCheckerConfig {
    pub input_path: String,
    pub output_path: String,
    pub auto_claim: bool,
}

#[tauri::command]
pub async fn username_checker_start(
    ids: Vec<String>,
    config: UsernameCheckerConfig,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();
    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "username_checker".to_string(),
            t("uchecker_task_name"),
        )
        .await;
    let cfg = Arc::new(config);
    tokio::spawn(async move {
        let result = run(ids, cfg.clone(), &app, token.clone()).await;
        match &result {
            Ok(_) => {
                emit(&app, t("done"));
            }
            Err(e) => {
                emit(&app, format!("{}: {e}", t("error")));
                emit(&app, t("done"));
            }
        }
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, result.is_ok()).await;
    });
    Ok(tid)
}

async fn interruptible_sleep(ms: u64, token: &Arc<AtomicBool>) {
    let mut remaining = ms;
    while remaining > 0 {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let chunk = remaining.min(200);
        tokio::time::sleep(Duration::from_millis(chunk)).await;
        remaining -= chunk;
    }
}

#[tauri::command]
pub async fn username_checker_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn run(
    account_ids: Vec<String>,
    cfg: Arc<UsernameCheckerConfig>,
    app: &tauri::AppHandle,
    token: Arc<AtomicBool>,
) -> Result<(), String> {
    let input_path = cfg.input_path.trim();
    if input_path.is_empty() {
        return Err(t("uchecker_no_input_file"));
    }
    let lines = std::fs::read_to_string(input_path)
        .map_err(|e| t_with("uchecker_read_file_error", &[("error", &e.to_string())]))?;
    let raw_names: Vec<String> = lines
        .lines()
        .map(|l| l.trim().trim_start_matches('@').to_lowercase())
        .filter(|l| !l.is_empty())
        .collect();

    let mut usernames: Vec<String> = Vec::new();
    let mut skipped = 0u32;
    let mut seen = std::collections::HashSet::new();
    for name in &raw_names {
        if !(5..=32).contains(&name.len()) {
            emit(app, t_with("uchecker_invalid_short", &[("name", name)]));
            skipped += 1;
            continue;
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            emit(app, t_with("uchecker_invalid_chars", &[("name", name)]));
            skipped += 1;
            continue;
        }
        if !seen.insert(name.clone()) {
            skipped += 1;
            continue;
        }
        usernames.push(name.clone());
    }
    if skipped > 0 {
        emit(
            app,
            t_with(
                "uchecker_skipped_invalid",
                &[("count", &skipped.to_string())],
            ),
        );
    }
    if usernames.is_empty() {
        return Err(t("uchecker_file_empty"));
    }

    let proxy_list = ProxyList::load();
    let proxies = &proxy_list.proxies;
    if proxies.is_empty() {
        return Err(t("uchecker_no_proxies"));
    }

    let concurrency = proxies.len().min(usernames.len());
    emit(
        app,
        t_with(
            "uchecker_loaded",
            &[
                ("usernames", &usernames.len().to_string()),
                ("proxies", &proxies.len().to_string()),
                ("threads", &concurrency.to_string()),
            ],
        ),
    );

    let output_path = resolve_output_path(&cfg.output_path);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    let db = rusqlite::Connection::open(&output_path)
        .map_err(|e| t_with("uchecker_open_db_error", &[("error", &e.to_string())]))?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS username_check (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL,
            status TEXT NOT NULL,
            checked_at TEXT DEFAULT (datetime('now'))
        );",
    )
    .map_err(|e| t_with("uchecker_create_table_error", &[("error", &e.to_string())]))?;
    let writer = Arc::new(TokioMutex::new(db));

    let free_count = Arc::new(AtomicU32::new(0));
    let taken_count = Arc::new(AtomicU32::new(0));
    let mut batches: Vec<Vec<(usize, String)>> = vec![Vec::new(); concurrency];
    for (i, name) in usernames.iter().enumerate() {
        batches[i % concurrency].push((i, name.clone()));
    }
    let free_queue: Arc<TokioMutex<Vec<String>>> = Arc::new(TokioMutex::new(Vec::new()));
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut handles = Vec::new();
    let total = usernames.len();

    for (thread_idx, batch) in batches.into_iter().enumerate() {
        if batch.is_empty() {
            continue;
        }
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let proxy = proxies[thread_idx % proxies.len()].clone();
        let sem = sem.clone();
        let token_clone = token.clone();
        let app_clone = app.clone();
        let writer_clone = writer.clone();
        let free_count_clone = free_count.clone();
        let taken_count_clone = taken_count.clone();
        let free_queue_clone = free_queue.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            if !token_clone.load(Ordering::Relaxed) {
                return;
            }
            for (idx, name) in batch {
                if !token_clone.load(Ordering::Relaxed) {
                    break;
                }
                let result = tokio::task::spawn_blocking({
                    let proxy = proxy.clone();
                    let name = name.clone();
                    move || check_username(&name, &proxy)
                })
                .await
                .unwrap_or(Err("task panic".into()));
                let status = match result {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = app_clone.emit(
                            "username-checker-log",
                            t_with(
                                "uchecker_error_user",
                                &[
                                    ("idx", &(idx + 1).to_string()),
                                    ("total", &total.to_string()),
                                    ("name", &name),
                                    ("error", &e),
                                ],
                            ),
                        );
                        UsernameStatus::Error(e)
                    }
                };
                let _ = app_clone.emit(
                    "username-checker-log",
                    t_with(
                        "uchecker_progress",
                        &[
                            ("idx", &(idx + 1).to_string()),
                            ("total", &total.to_string()),
                            ("name", &name),
                            ("status", &status.label_i18n()),
                        ],
                    ),
                );
                match &status {
                    UsernameStatus::Free => {
                        free_count_clone.fetch_add(1, Ordering::Relaxed);
                        let w = writer_clone.lock().await;
                        w.execute(
                            "INSERT INTO username_check (username, status) VALUES (?1, ?2)",
                            rusqlite::params![&name, "free"],
                        )
                        .ok();
                        free_queue_clone.lock().await.push(name.clone());
                    }
                    UsernameStatus::ForSale => {
                        taken_count_clone.fetch_add(1, Ordering::Relaxed);
                        let w = writer_clone.lock().await;
                        w.execute(
                            "INSERT INTO username_check (username, status) VALUES (?1, ?2)",
                            rusqlite::params![&name, "fragment"],
                        )
                        .ok();
                    }
                    UsernameStatus::Sold => {
                        taken_count_clone.fetch_add(1, Ordering::Relaxed);
                        let w = writer_clone.lock().await;
                        w.execute(
                            "INSERT INTO username_check (username, status) VALUES (?1, ?2)",
                            rusqlite::params![&name, "fragment_sold"],
                        )
                        .ok();
                    }
                    UsernameStatus::Taken => {
                        taken_count_clone.fetch_add(1, Ordering::Relaxed);
                        let w = writer_clone.lock().await;
                        w.execute(
                            "INSERT INTO username_check (username, status) VALUES (?1, ?2)",
                            rusqlite::params![&name, "taken"],
                        )
                        .ok();
                    }
                    UsernameStatus::Error(_) => {
                        taken_count_clone.fetch_add(1, Ordering::Relaxed);
                    }
                }
                interruptible_sleep(300, &token_clone).await;
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }

    if cfg.auto_claim && !account_ids.is_empty() {
        let free_names = free_queue.lock().await.clone();
        if !free_names.is_empty() {
            emit(
                app,
                t_with(
                    "uchecker_autoclaim",
                    &[
                        ("free", &free_names.len().to_string()),
                        ("accounts", &account_ids.len().to_string()),
                    ],
                ),
            );
            auto_claim_usernames(&free_names, &account_ids, app, &token, &writer).await;
        }
    }

    let f = free_count.load(Ordering::Relaxed);
    let tk = taken_count.load(Ordering::Relaxed);
    emit(
        app,
        t_with(
            "uchecker_result",
            &[
                ("free", &f.to_string()),
                ("taken", &tk.to_string()),
                ("path", &output_path.display().to_string()),
            ],
        ),
    );
    Ok(())
}

#[derive(Debug)]
#[allow(dead_code)]
enum UsernameStatus {
    Free,
    Taken,
    ForSale,
    Sold,
    Error(String),
}
impl UsernameStatus {
    fn label_i18n(&self) -> String {
        match self {
            Self::Free => t("uchecker_status_free"),
            Self::Taken => t("uchecker_status_taken"),
            Self::ForSale => t("uchecker_status_for_sale"),
            Self::Sold => t("uchecker_status_sold"),
            Self::Error(_) => t("uchecker_status_error"),
        }
    }
}

fn check_username(name: &str, proxy: &ProxyConfig) -> Result<UsernameStatus, String> {
    let agent = build_ureq_agent(proxy)?;
    let tme_body = http_get_with_retries(&agent, &format!("https://t.me/{name}"), 3)?;
    if !tme_body.contains("tgme_icon_user") {
        return Ok(UsernameStatus::Taken);
    }
    let frag_body =
        http_get_with_retries(&agent, &format!("https://fragment.com/username/{name}"), 3)?;
    if frag_body.contains("tm-section-header-status") {
        let lower = frag_body.to_lowercase();
        if lower.contains(">for sale<") || lower.contains("\">for sale</") {
            return Ok(UsernameStatus::ForSale);
        }
        if lower.contains(">taken<") || lower.contains("\">taken</") {
            return Ok(UsernameStatus::Taken);
        }
        if lower.contains(">sold<") || lower.contains("\">sold</") {
            return Ok(UsernameStatus::Sold);
        }
    }
    Ok(UsernameStatus::Free)
}

fn build_ureq_agent(proxy: &ProxyConfig) -> Result<ureq::Agent, String> {
    let proxy_url = match proxy.proxy_type {
        ProxyType::Socks5 => {
            let auth = match (&proxy.username, &proxy.password) {
                (Some(u), Some(p)) => format!("{u}:{p}@"),
                (Some(u), None) => format!("{u}@"),
                _ => String::new(),
            };
            format!("socks5://{auth}{}:{}", proxy.host, proxy.port)
        }
        ProxyType::Https => {
            let auth = match (&proxy.username, &proxy.password) {
                (Some(u), Some(p)) => format!("{u}:{p}@"),
                (Some(u), None) => format!("{u}@"),
                _ => String::new(),
            };
            format!("http://{auth}{}:{}", proxy.host, proxy.port)
        }
        ProxyType::Socks4 => {
            format!("socks4://{}:{}", proxy.host, proxy.port)
        }
    };
    let proxy = ureq::Proxy::new(&proxy_url).map_err(|e| format!("proxy: {e}"))?;
    let config = ureq::Agent::config_builder()
        .proxy(Some(proxy))
        .timeout_global(Some(Duration::from_secs(10)))
        .build();
    Ok(config.new_agent())
}

fn http_get_with_retries(
    agent: &ureq::Agent,
    url: &str,
    max_retries: u32,
) -> Result<String, String> {
    let attempts = max_retries.max(5);
    for attempt in 0..attempts {
        match agent.get(url).call() {
            Ok(mut resp) => {
                return resp
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| format!("read body: {e}"));
            }
            Err(e) => {
                if attempt + 1 >= attempts {
                    return Err(format!("HTTP failed after {attempts} attempts: {e}"));
                }
                std::thread::sleep(Duration::from_millis(500 * (attempt as u64 + 1)));
            }
        }
    }
    Err("unreachable".into())
}

async fn auto_claim_usernames(
    names: &[String],
    account_ids: &[String],
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
    writer: &Arc<TokioMutex<rusqlite::Connection>>,
) {
    use crate::accounts::commands::get_storage_pub;
    use crate::accounts::connect::connect_account;
    let storage = get_storage_pub();
    for (name, id) in names.iter().zip(account_ids) {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let json = if let Some(p) = storage
            .json_path(id)
            .exists()
            .then(|| storage.json_path(id))
        {
            crate::accounts::session::AccountJson::from_file(&p).unwrap_or_default()
        } else {
            crate::accounts::session::AccountJson::default()
        };
        let mut client = match connect_account(id).await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let req = crate::mtproto::tl::build_account_update_username(name);
        match client.invoke(&req).await {
            Ok(_) => {
                emit(
                    app,
                    t_with(
                        "uchecker_claimed",
                        &[("name", name), ("phone", &json.phone)],
                    ),
                );
                let w = writer.lock().await;
                w.execute(
                    "INSERT INTO username_check (username, status) VALUES (?1, ?2)",
                    rusqlite::params![name, format!("claimed:+{}", json.phone)],
                )
                .ok();
            }
            Err(e) => {
                emit(
                    app,
                    t_with("uchecker_claim_failed", &[("name", name), ("error", &e)]),
                );
                let w = writer.lock().await;
                w.execute(
                    "INSERT INTO username_check (username, status) VALUES (?1, ?2)",
                    rusqlite::params![name, format!("claim_failed:{e}")],
                )
                .ok();
            }
        }
        interruptible_sleep(1000, token).await;
    }

    if names.len() > account_ids.len() {
        emit(
            app,
            t_with(
                "uchecker_claim_unassigned",
                &[("count", &(names.len() - account_ids.len()).to_string())],
            ),
        );
    }
}

fn resolve_output_path(user_path: &str) -> PathBuf {
    let trimmed = user_path.trim();
    if !trimmed.is_empty() {
        return PathBuf::from(trimmed);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kastor")
        .join("username_check.db")
}

fn emit(app: &tauri::AppHandle, msg: String) {
    let _ = app.emit("username-checker-log", msg);
}
