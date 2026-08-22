use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::params;
use serde::Deserialize;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex as TokioMutex;

use crate::accounts::commands::get_storage_pub;
use crate::accounts::session::AccountJson;
use crate::i18n::{t, t_with};
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::queue::TaskQueue;

const BOTFATHER_REQUEST_DELAY_MS: u64 = 350;
const ACCOUNT_TIMEOUT_SECS: u64 = 30 * 60;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BotParserConfig {
    pub output_path: String,
    #[serde(default)]
    pub regenerate_tokens: bool,
    #[serde(default = "default_threads")]
    pub threads: usize,
    #[serde(default)]
    pub max_flood_wait: u64,
}

fn default_threads() -> usize {
    3
}

#[tauri::command]
pub async fn bot_parser_start(
    ids: Vec<String>,
    config: BotParserConfig,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() {
        return Err(t("bot_parser_no_accounts"));
    }

    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();
    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "bot_parser".to_string(),
            t_with("bot_parser_task_name", &[("count", &ids.len().to_string())]),
        )
        .await;

    tokio::spawn(async move {
        let ok = run(ids, config, &app, token.clone()).await;
        match ok {
            Ok(_) => emit(&app, "Done"),
            Err(e) => {
                emit(&app, format!("Error: {e}"));
                emit(&app, "Done");
            }
        }
        emit(&app, "__FINISHED__");
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn bot_parser_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn run(
    ids: Vec<String>,
    config: BotParserConfig,
    app: &tauri::AppHandle,
    token: Arc<AtomicBool>,
) -> Result<(), String> {
    let output_path = resolve_output_path(&config.output_path);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    let db = init_db(&output_path)?;
    emit(
        app,
        t_with(
            "bot_parser_db_path",
            &[("path", &output_path.display().to_string())],
        ),
    );

    let writer = Arc::new(TokioMutex::new(db));
    let found_total = Arc::new(AtomicU32::new(0));
    let token_total = Arc::new(AtomicU32::new(0));
    let error_total = Arc::new(AtomicU32::new(0));
    let total_accounts = ids.len();
    let concurrency = config.threads.max(1).min(100).min(total_accounts);
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let cfg = Arc::new(config);
    let mut handles = Vec::new();

    for (idx, id) in ids.into_iter().enumerate() {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let sem = sem.clone();
        let cfg = cfg.clone();
        let writer = writer.clone();
        let app_clone = app.clone();
        let token_clone = token.clone();
        let found_total = found_total.clone();
        let token_total = token_total.clone();
        let error_total = error_total.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            if !token_clone.load(Ordering::Relaxed) {
                return;
            }
            let process_result = tokio::time::timeout(
                Duration::from_secs(ACCOUNT_TIMEOUT_SECS),
                process_account(
                    &id,
                    idx + 1,
                    total_accounts,
                    &cfg,
                    &writer,
                    &app_clone,
                    &token_clone,
                ),
            )
            .await;

            match process_result {
                Ok(Ok((bots, tokens))) => {
                    found_total.fetch_add(bots as u32, Ordering::Relaxed);
                    token_total.fetch_add(tokens as u32, Ordering::Relaxed);
                    emit(&app_clone, format!("__DONE__:{id}"));
                }
                Ok(Err(e)) => {
                    if token_clone.load(Ordering::Relaxed) {
                        error_total.fetch_add(1, Ordering::Relaxed);
                        crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                        emit(&app_clone, format!("__ERROR__:{id}"));
                        let prefix = format!("[{}/{}] {}", idx + 1, total_accounts, id);
                        emit(
                            &app_clone,
                            t_with(
                                "bot_parser_account_error",
                                &[("prefix", &prefix), ("error", &e)],
                            ),
                        );
                    }
                }
                Err(_) => {
                    if token_clone.load(Ordering::Relaxed) {
                        error_total.fetch_add(1, Ordering::Relaxed);
                        emit(&app_clone, format!("__ERROR__:{id}"));
                        let prefix = format!("[{}/{}] {}", idx + 1, total_accounts, id);
                        emit(
                            &app_clone,
                            format!("{prefix} account timeout after {ACCOUNT_TIMEOUT_SECS} sec"),
                        );
                    }
                }
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    emit(
        app,
        t_with(
            "bot_parser_result",
            &[
                ("prefix", "Bot parser"),
                ("bots", &found_total.load(Ordering::Relaxed).to_string()),
                ("tokens", &token_total.load(Ordering::Relaxed).to_string()),
                ("errors", &error_total.load(Ordering::Relaxed).to_string()),
                ("path", &output_path.display().to_string()),
            ],
        ),
    );
    Ok(())
}

async fn process_account(
    id: &str,
    idx: usize,
    total: usize,
    config: &BotParserConfig,
    writer: &Arc<TokioMutex<rusqlite::Connection>>,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(usize, usize), String> {
    let storage = get_storage_pub();
    let json_path = storage.json_path(id);
    let json = if json_path.exists() {
        AccountJson::from_file(&json_path).unwrap_or_default()
    } else {
        AccountJson::default()
    };
    let account_label = if json.phone.is_empty() {
        id.to_string()
    } else {
        format!("+{}", json.phone)
    };
    let prefix = format!("[{idx}/{total}] {account_label}");

    let mut client = crate::accounts::connect::connect_account(id).await?;
    client.set_max_flood_wait(config.max_flood_wait);
    client.set_log_target("bot-parser-log", app.clone());
    client.set_log_prefix(&prefix);

    let (bf_id, bf_hash) =
        resolve_botfather(&mut client, config.max_flood_wait, app, &prefix, token).await?;
    if bf_id == 0 && bf_hash == 0 {
        return Ok((0, 0));
    }

    let _ = invoke_botfather(&mut client, &tl::build_unblock_peer(bf_id, bf_hash), token).await;
    let _ = invoke_botfather(&mut client, &tl::build_mute_peer(bf_id, bf_hash), token).await;

    emit(app, t_with("bot_parser_collecting", &[("prefix", &prefix)]));
    let bot_usernames = collect_bot_usernames(&mut client, bf_id, bf_hash, token).await?;

    if bot_usernames.is_empty() {
        emit(app, t_with("bot_parser_no_bots", &[("prefix", &prefix)]));
        cleanup_botfather(&mut client, bf_id, bf_hash, token).await;
        return Ok((0, 0));
    }

    emit(
        app,
        t_with(
            "bot_parser_found",
            &[
                ("prefix", &prefix),
                ("count", &bot_usernames.len().to_string()),
            ],
        ),
    );

    {
        let db = writer.lock().await;
        let _ = db.execute(
            "INSERT INTO accounts (account_id, phone, bots_found, status) VALUES (?1, ?2, ?3, 'running')",
            params![id, json.phone, bot_usernames.len() as i64],
        );
    }

    let mut token_count = 0usize;
    for (bot_idx, username) in bot_usernames.iter().enumerate() {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let mut status = "pending";
        let mut token_value = String::new();

        if config.regenerate_tokens {
            emit(
                app,
                t_with(
                    "bot_parser_revoke",
                    &[
                        ("prefix", &prefix),
                        ("idx", &(bot_idx + 1).to_string()),
                        ("total", &bot_usernames.len().to_string()),
                        ("username", username),
                    ],
                ),
            );
            match request_token_command(&mut client, bf_id, bf_hash, "/revoke", username, token)
                .await
            {
                Ok(Some(tok)) => {
                    token_value = tok;
                    status = "revoked";
                    token_count += 1;
                }
                Ok(None) => {
                    status = "token_not_found";
                }
                Err(e) => {
                    status = "error";
                    emit(
                        app,
                        t_with(
                            "bot_parser_bot_error",
                            &[("prefix", &prefix), ("username", username), ("error", &e)],
                        ),
                    );
                }
            }
            pause_ms(500, token).await;
        }

        emit(
            app,
            t_with(
                "bot_parser_token",
                &[
                    ("prefix", &prefix),
                    ("idx", &(bot_idx + 1).to_string()),
                    ("total", &bot_usernames.len().to_string()),
                    ("username", username),
                ],
            ),
        );
        match request_token_command(&mut client, bf_id, bf_hash, "/token", username, token).await {
            Ok(Some(tok)) => {
                if token_value.is_empty() {
                    token_count += 1;
                }
                token_value = tok;
                status = if config.regenerate_tokens {
                    "regenerated"
                } else {
                    "done"
                };
            }
            Ok(None) => {
                if token_value.is_empty() {
                    status = "token_not_found";
                }
            }
            Err(e) => {
                if token_value.is_empty() {
                    status = "error";
                }
                emit(
                    app,
                    t_with(
                        "bot_parser_bot_error",
                        &[("prefix", &prefix), ("username", username), ("error", &e)],
                    ),
                );
            }
        }

        {
            let db = writer.lock().await;
            let _ = db.execute(
                "INSERT INTO bot_tokens (account_id, phone, username, token, regenerated, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    json.phone,
                    username,
                    token_value,
                    config.regenerate_tokens as i32,
                    status
                ],
            );
        }
        pause_ms(500, token).await;
    }

    {
        let db = writer.lock().await;
        let _ = db.execute(
            "UPDATE accounts SET tokens_found = ?1, status = 'done', finished_at = datetime('now') WHERE account_id = ?2",
            params![token_count as i64, id],
        );
    }

    cleanup_botfather(&mut client, bf_id, bf_hash, token).await;
    emit(app, t_with("bot_parser_cleanup", &[("prefix", &prefix)]));

    if let Some(fatal) = client.fatal_error() {
        return Err(fatal.to_string());
    }
    Ok((bot_usernames.len(), token_count))
}

async fn resolve_botfather(
    client: &mut MtpClient,
    max_flood_wait: u64,
    app: &tauri::AppHandle,
    prefix: &str,
    token: &Arc<AtomicBool>,
) -> Result<(i64, i64), String> {
    let req = tl::build_resolve_username("BotFather");
    for _ in 0..5 {
        if !token.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        pause_ms(BOTFATHER_REQUEST_DELAY_MS, token).await;
        match client.invoke(&req).await {
            Ok(data) => {
                return tl::parse_resolved_peer(&data).map_err(|e| format!("parse BotFather: {e}"))
            }
            Err(e) if e.contains("FLOOD_WAIT") => {
                let wait_secs = parse_flood_wait(&e).unwrap_or(30);
                if max_flood_wait > 0 && wait_secs > max_flood_wait {
                    emit(
                        app,
                        t_with(
                            "bot_parser_flood_wait",
                            &[
                                ("prefix", prefix),
                                ("seconds", &wait_secs.to_string()),
                                ("limit", &max_flood_wait.to_string()),
                            ],
                        ),
                    );
                    return Ok((0, 0));
                }
                emit(
                    app,
                    t_with(
                        "bot_parser_flood_wait_wait",
                        &[("prefix", prefix), ("seconds", &wait_secs.to_string())],
                    ),
                );
                pause_ms((wait_secs + 1) * 1000, token).await;
            }
            Err(e) => return Err(format!("resolve BotFather: {e}")),
        }
    }
    Err("resolve BotFather: max attempts".into())
}

async fn collect_bot_usernames(
    client: &mut MtpClient,
    bf_id: i64,
    bf_hash: i64,
    token: &Arc<AtomicBool>,
) -> Result<Vec<String>, String> {
    let before_max = max_history_id(client, bf_id, bf_hash, token).await;
    send_text(client, bf_id, bf_hash, "/mybots", token).await?;
    pause_ms(800, token).await;

    let mut usernames = Vec::new();
    let mut seen_usernames = HashSet::new();
    let mut seen_pages = HashSet::new();

    for _ in 0..20 {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let data =
            invoke_botfather(client, &tl::build_get_history(bf_id, bf_hash, 8), token).await?;
        let messages = tl::parse_messages_structured(&data)
            .map_err(|e| format!("parse /mybots history: {e}"))?;
        let Some(msg) = messages
            .iter()
            .filter(|m| m.id > before_max && !m.buttons.is_empty())
            .max_by_key(|m| m.id)
        else {
            break;
        };

        let signature = msg
            .buttons
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("|");
        if !seen_pages.insert(signature) {
            break;
        }

        for button in &msg.buttons {
            if let Some(username) = extract_bot_username(&button.text) {
                if seen_usernames.insert(username.clone()) {
                    usernames.push(username);
                }
            }
        }

        let next = msg
            .buttons
            .iter()
            .find(|b| is_next_button(&b.text) && b.data.is_some())
            .and_then(|b| b.data.clone());

        if let Some(callback_data) = next {
            let cb_req = tl::build_bot_callback_answer(bf_id, bf_hash, msg.id, &callback_data);
            let _ = invoke_botfather(client, &cb_req, token).await;
            pause_ms(800, token).await;
        } else {
            break;
        }
    }

    Ok(usernames)
}

async fn request_token_command(
    client: &mut MtpClient,
    bf_id: i64,
    bf_hash: i64,
    command: &str,
    username: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<Option<String>, String> {
    let before = max_history_id(client, bf_id, bf_hash, cancel).await;
    send_text(client, bf_id, bf_hash, command, cancel).await?;
    pause_ms(650, cancel).await;
    send_text(client, bf_id, bf_hash, &format!("@{username}"), cancel).await?;

    for _ in 0..20 {
        if !cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        pause_ms(500, cancel).await;
        let data =
            invoke_botfather(client, &tl::build_get_history(bf_id, bf_hash, 8), cancel).await?;
        let messages = tl::parse_messages_structured(&data)
            .map_err(|e| format!("parse token history: {e}"))?;
        for msg in messages.iter().filter(|m| m.id > before) {
            if let Some(tok) = extract_bot_token(&msg.text) {
                return Ok(Some(tok));
            }
        }
    }
    Ok(None)
}

async fn send_text(
    client: &mut MtpClient,
    peer_id: i64,
    access_hash: i64,
    text: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let req = tl::build_send_message(peer_id, access_hash, text, rand::random());
    invoke_botfather(client, &req, cancel)
        .await
        .map(|_| ())
        .map_err(|e| format!("send {text}: {e}"))
}

async fn max_history_id(
    client: &mut MtpClient,
    peer_id: i64,
    access_hash: i64,
    cancel: &Arc<AtomicBool>,
) -> i32 {
    let Ok(data) = invoke_botfather(
        client,
        &tl::build_get_history(peer_id, access_hash, 10),
        cancel,
    )
    .await
    else {
        return 0;
    };
    let Ok(messages) = tl::parse_messages_structured(&data) else {
        return 0;
    };
    messages.iter().map(|m| m.id).max().unwrap_or(0)
}

async fn cleanup_botfather(
    client: &mut MtpClient,
    bf_id: i64,
    bf_hash: i64,
    cancel: &Arc<AtomicBool>,
) {
    let _ = invoke_botfather(client, &tl::build_delete_history(bf_id, bf_hash), cancel).await;
    let _ = invoke_botfather(client, &tl::build_block_peer(bf_id, bf_hash), cancel).await;
}

async fn invoke_botfather(
    client: &mut MtpClient,
    request: &[u8],
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<u8>, String> {
    pause_ms(BOTFATHER_REQUEST_DELAY_MS, cancel).await;
    client.invoke(request).await
}

async fn pause_ms(ms: u64, token: &Arc<AtomicBool>) {
    let mut elapsed = 0;
    while elapsed < ms {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let chunk = (ms - elapsed).min(200);
        tokio::time::sleep(Duration::from_millis(chunk)).await;
        elapsed += chunk;
    }
}

fn extract_bot_username(text: &str) -> Option<String> {
    for raw in text.split_whitespace() {
        let cleaned = raw
            .trim()
            .trim_start_matches('@')
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .to_string();
        if cleaned.len() >= 5
            && cleaned.len() <= 32
            && cleaned.to_ascii_lowercase().ends_with("bot")
            && cleaned
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Some(cleaned);
        }
    }
    None
}

fn is_next_button(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.contains('»')
        || trimmed.contains('›')
        || trimmed == ">"
        || trimmed == "➡"
        || trimmed == "➡️"
}

fn extract_bot_token(text: &str) -> Option<String> {
    for word in text.split_whitespace() {
        let token = word
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != ':' && c != '-' && c != '_');
        let Some((left, right)) = token.split_once(':') else {
            continue;
        };
        if left.len() >= 5
            && left.chars().all(|c| c.is_ascii_digit())
            && right.len() >= 20
            && right
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Some(token.to_string());
        }
    }
    None
}

fn parse_flood_wait(error: &str) -> Option<u64> {
    error.split('_').last().and_then(|s| s.parse::<u64>().ok())
}

fn resolve_output_path(user_path: &str) -> PathBuf {
    let trimmed = user_path.trim();
    if !trimmed.is_empty() {
        let p = PathBuf::from(trimmed);
        return if p.extension().map(|e| e == "db").unwrap_or(false) {
            p
        } else {
            p.with_extension("db")
        };
    }
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kastor")
        .join("bot_parser");
    let now = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    base.join(format!("bot_tokens_{now}.db"))
}

fn init_db(path: &PathBuf) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(path)
        .map_err(|e| t_with("bot_parser_db_open_error", &[("error", &e.to_string())]))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;

         CREATE TABLE IF NOT EXISTS accounts (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             account_id TEXT NOT NULL,
             phone TEXT DEFAULT '',
             bots_found INTEGER DEFAULT 0,
             tokens_found INTEGER DEFAULT 0,
             status TEXT DEFAULT 'pending',
             started_at TEXT DEFAULT (datetime('now')),
             finished_at TEXT
         );

         CREATE TABLE IF NOT EXISTS bot_tokens (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             account_id TEXT NOT NULL,
             phone TEXT DEFAULT '',
             username TEXT NOT NULL,
             token TEXT DEFAULT '',
             regenerated INTEGER DEFAULT 0,
             status TEXT DEFAULT 'pending',
             created_at TEXT DEFAULT (datetime('now'))
         );

         CREATE INDEX IF NOT EXISTS idx_bot_tokens_account ON bot_tokens(account_id);
         CREATE INDEX IF NOT EXISTS idx_bot_tokens_username ON bot_tokens(username);
         CREATE INDEX IF NOT EXISTS idx_bot_tokens_status ON bot_tokens(status);",
    )
    .map_err(|e| t_with("bot_parser_db_tables_error", &[("error", &e.to_string())]))?;
    Ok(conn)
}

fn emit(app: &tauri::AppHandle, msg: impl AsRef<str>) {
    let _ = app.emit("bot-parser-log", msg.as_ref());
}
