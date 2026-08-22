// parser: pulls channel/group members via alphabet-based search or message
// history iteration. Persists results to a SQLite .db file.
// Modes:
//   - group: alphabet search on participants (bypasses ~10k limit)
//   - channel_admin: same but requires admin rights
//   - messages: iterates message history, collects senders (works on channels too)

pub mod user_lookup;

use rusqlite::params;
use serde::Deserialize;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};

use crate::accounts::connect::connect_account;
use crate::i18n::{t, t_with};
use crate::mtproto::client::MtpClient;
use crate::mtproto::invite::resolve_channel_link;
use crate::mtproto::tl::{
    self, OnlineBucket, ParticipantUser, ParticipantsBatch, ParticipantsFilter,
};
use crate::mtproto::tl_gen;
use crate::queue::TaskQueue;

async fn interruptible_sleep(ms: u64, token: &Arc<AtomicBool>) {
    let mut remaining = ms;
    while remaining > 0 {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let chunk = remaining.min(200);
        tokio::time::sleep(std::time::Duration::from_millis(chunk)).await;
        remaining -= chunk;
    }
}

const PAGE_SIZE: i32 = 200;
const MAX_OFFSET: i32 = 200_000;
const MAX_CONSECUTIVE_ERRORS: u32 = 3;
const PAGE_DELAY_MS: u64 = 300;
const CHAR_DELAY_MS: u64 = 500;
// messages mode: how many messages to fetch per page
const MSG_PAGE_SIZE: i32 = 100;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ParserMode {
    Group,
    ChannelAdmin,
    Messages,
    Comments,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ParserConfig {
    pub mode: ParserMode,
    pub targets: Vec<String>,

    // participant filters (for group/channel_admin modes)
    pub parse_deleted: bool,
    pub parse_recent: bool,
    pub parse_week: bool,
    pub parse_month: bool,
    pub parse_long: bool,

    pub premium_only: bool,
    pub parse_admins: bool,
    pub parse_bots: bool,
    pub exclude_admins: bool,
    pub exclude_no_username: bool,

    // alphabet (for group/channel_admin modes)
    pub chars_en: bool,
    pub chars_ru: bool,
    pub chars_cn: bool,
    pub chars_ar: bool,
    pub chars_he: bool,
    pub chars_fa: bool,
    pub chars_emoji: bool,

    // messages mode settings
    pub parsing_days: u32, // 0 = unlimited (all history)

    pub leave_after: bool,

    pub output_path: String,
    #[serde(default)]
    pub create_txt: bool,
    #[serde(default)]
    pub max_flood_wait: u64,
}

#[tauri::command]
pub async fn parser_start(
    ids: Vec<String>,
    config: ParserConfig,
    threads: Option<usize>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() {
        return Err(t("parser_no_accounts"));
    }
    let targets: Vec<String> = config
        .targets
        .iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if targets.is_empty() {
        return Err(t("parser_no_targets"));
    }

    let concurrency = threads.unwrap_or(ids.len()).max(1).min(100);
    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "parser".to_string(),
            t_with(
                "parser_task_name",
                &[
                    ("groups", &targets.len().to_string()),
                    ("accounts", &ids.len().to_string()),
                ],
            ),
        )
        .await;

    let cfg = Arc::new(config);
    let max_flood_wait = cfg.max_flood_wait;

    tokio::spawn(async move {
        let result = run_batch(
            ids,
            targets,
            cfg,
            concurrency,
            max_flood_wait,
            &app,
            token.clone(),
        )
        .await;
        match &result {
            Ok(_) => {
                let _ = app.emit("parser-log", t("done"));
            }
            Err(e) => {
                let _ = app.emit("parser-log", format!("{}: {e}", t("error")));
                let _ = app.emit("parser-log", t("done"));
            }
        }
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn parser_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

fn init_db(path: &PathBuf) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(path)
        .map_err(|e| t_with("db_open_error", &[("error", &e.to_string())]))?;
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS users (
            user_id INTEGER PRIMARY KEY,
            access_hash INTEGER DEFAULT 0,
            username TEXT DEFAULT '',
            phone TEXT DEFAULT '',
            first_name TEXT DEFAULT '',
            last_name TEXT DEFAULT '',
            is_bot INTEGER DEFAULT 0,
            is_admin INTEGER DEFAULT 0,
            is_deleted INTEGER DEFAULT 0,
            premium INTEGER DEFAULT 0,
            status TEXT DEFAULT 'unknown',
            source TEXT DEFAULT 'participants',
            source_group INTEGER DEFAULT 0,
            msg_id INTEGER DEFAULT 0,
            parsed_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS group_info (
            channel_id INTEGER PRIMARY KEY,
            link TEXT DEFAULT '',
            title TEXT DEFAULT '',
            username TEXT DEFAULT '',
            participants_count INTEGER DEFAULT 0,
            is_broadcast INTEGER DEFAULT 0,
            has_photo INTEGER DEFAULT 0,
            invite_link TEXT DEFAULT '',
            slow_mode INTEGER DEFAULT 0,
            scam INTEGER DEFAULT 0,
            status TEXT DEFAULT 'pending',
            users_collected INTEGER DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_users_status ON users(status);
        CREATE INDEX IF NOT EXISTS idx_users_premium ON users(premium);
        CREATE INDEX IF NOT EXISTS idx_users_source ON users(source);
        CREATE INDEX IF NOT EXISTS idx_users_source_group ON users(source_group);
    ",
    )
    .map_err(|e| t_with("db_create_tables_error", &[("error", &e.to_string())]))?;
    Ok(conn)
}

async fn run_batch(
    account_ids: Vec<String>,
    targets: Vec<String>,
    cfg: Arc<ParserConfig>,
    concurrency: usize,
    max_flood_wait: u64,
    app: &tauri::AppHandle,
    token: Arc<AtomicBool>,
) -> Result<(), String> {
    let total_targets = targets.len();
    emit(
        app,
        t_with(
            "parser_start",
            &[
                ("groups", &total_targets.to_string()),
                ("accounts", &account_ids.len().to_string()),
                ("threads", &concurrency.to_string()),
            ],
        ),
    );

    // open shared output DB
    let output_path = resolve_output_path(&cfg.output_path, &0);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    let db = Arc::new(std::sync::Mutex::new(init_db(&output_path)?));
    emit(
        app,
        t_with(
            "parser_db_path",
            &[("path", &output_path.display().to_string())],
        ),
    );

    // shared target queue
    let target_idx = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let groups_done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let total_users = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // emit initial progress
    let _ = app.emit(
        "parser-progress",
        serde_json::json!({
            "groups_total": total_targets,
            "groups_done": 0,
            "users_total": 0,
        })
        .to_string(),
    );

    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut handles = Vec::new();

    for (i, account_id) in account_ids.into_iter().enumerate() {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let sem = sem.clone();
        let cfg = cfg.clone();
        let db = db.clone();
        let target_idx = target_idx.clone();
        let targets = targets.clone();
        let groups_done = groups_done.clone();
        let total_users = total_users.clone();
        let token_clone = token.clone();
        let app_clone = app.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();

            loop {
                if !token_clone.load(Ordering::Relaxed) {
                    break;
                }

                let idx = target_idx.fetch_add(1, Ordering::Relaxed);
                if idx >= targets.len() {
                    break;
                }

                let target = &targets[idx];
                let prefix = t_with(
                    "parser_prefix",
                    &[
                        ("acc", &(i + 1).to_string()),
                        ("group", &(idx + 1).to_string()),
                        ("total", &targets.len().to_string()),
                    ],
                );
                let _ = app_clone.emit(
                    "parser-log",
                    format!(
                        "{} {}",
                        prefix,
                        t_with("parser_thread_start", &[("target", target)])
                    ),
                );

                let result = run_single_target(
                    &account_id,
                    target,
                    &cfg,
                    max_flood_wait,
                    &db,
                    &app_clone,
                    &token_clone,
                    &prefix,
                )
                .await;

                let done = groups_done.fetch_add(1, Ordering::Relaxed) + 1;

                match result {
                    Ok(collected) => {
                        let total = total_users.fetch_add(collected as usize, Ordering::Relaxed)
                            + collected as usize;
                        let _ = app_clone.emit(
                            "parser-log",
                            format!(
                                "{} {}",
                                prefix,
                                t_with("parser_thread_done", &[("count", &collected.to_string())])
                            ),
                        );
                        let _ = app_clone.emit(
                            "parser-progress",
                            serde_json::json!({
                                "groups_total": total_targets,
                                "groups_done": done,
                                "users_total": total,
                            })
                            .to_string(),
                        );
                    }
                    Err(e) => {
                        crate::accounts::commands::check_and_mark_dead_session(&e, &account_id);
                        let _ = app_clone
                            .emit("parser-log", format!("{} {}: {}", prefix, t("error"), e));
                        // update group status
                        if let Ok(db) = db.lock() {
                            db.execute(
                                "UPDATE group_info SET status = ?1 WHERE link = ?2",
                                params![&format!("error: {}", e), target],
                            )
                            .ok();
                        }
                        let _ = app_clone.emit(
                            "parser-progress",
                            serde_json::json!({
                                "groups_total": total_targets,
                                "groups_done": done,
                                "users_total": total_users.load(Ordering::Relaxed),
                            })
                            .to_string(),
                        );
                        // if fatal session error, this account is done
                        if crate::mtproto::is_fatal_session_error(&e) {
                            break;
                        }
                    }
                }
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    // TXT export
    if cfg.create_txt {
        let txt_path = output_path.with_extension("txt");
        emit(
            app,
            t_with(
                "parser_export_txt",
                &[("path", &txt_path.display().to_string())],
            ),
        );
        if let Ok(db) = db.lock() {
            export_txt(&db, &txt_path);
        }
        emit(
            app,
            t_with(
                "parser_txt_exported",
                &[("path", &txt_path.display().to_string())],
            ),
        );
    }

    let total = total_users.load(Ordering::Relaxed);
    let done = groups_done.load(Ordering::Relaxed);
    emit(
        app,
        t_with(
            "parser_total",
            &[("done", &done.to_string()), ("total", &total.to_string())],
        ),
    );
    Ok(())
}

fn export_txt(db: &rusqlite::Connection, path: &PathBuf) {
    use std::io::Write;
    let mut stmt =
        match db.prepare("SELECT username FROM users WHERE username != '' ORDER BY username") {
            Ok(s) => s,
            Err(_) => return,
        };
    let mut file = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut seen = std::collections::HashSet::new();
    for row in rows.flatten() {
        if seen.insert(row.clone()) {
            let _ = writeln!(file, "@{}", row);
        }
    }
}

/// Run parser for a single target group. Returns number of users collected.
async fn run_single_target(
    account_id: &str,
    target: &str,
    cfg: &ParserConfig,
    max_flood_wait: u64,
    db: &Arc<std::sync::Mutex<rusqlite::Connection>>,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
    prefix: &str,
) -> Result<u32, String> {
    // for messages/comments mode we don't need status filters
    if !matches!(cfg.mode, ParserMode::Messages | ParserMode::Comments) && !any_status_filter(cfg) {
        return Err(t("parser_no_filter"));
    }

    let mut client = connect_account(account_id).await?;
    client.set_log_target("parser-log", app.clone());
    client.set_max_flood_wait(max_flood_wait);

    let resolved = resolve_channel_link(&mut client, target).await?;

    let hint = if resolved.title_hint.is_empty() {
        resolved
            .username_hint
            .clone()
            .map(|u| format!("(@{u})"))
            .unwrap_or_else(|| t("parser_target_private"))
    } else {
        format!("({})", resolved.title_hint)
    };
    let _ = app.emit(
        "parser-log",
        format!(
            "{} {}",
            prefix,
            t_with(
                "parser_target",
                &[("id", &resolved.channel_id.to_string()), ("hint", &hint)]
            )
        ),
    );

    // mode sanity checks
    match cfg.mode {
        ParserMode::Group => {
            if resolved.is_broadcast {
                // update status and return error
                if let Ok(db) = db.lock() {
                    db.execute(
                        "UPDATE group_info SET status = 'not_group' WHERE link = ?1",
                        params![target],
                    )
                    .ok();
                }
                return Err(t("parser_broadcast_not_group"));
            }
        }
        ParserMode::ChannelAdmin => {
            let probe_req = tl::build_channels_get_participants_search(
                resolved.channel_id,
                resolved.access_hash,
                "",
                0,
                1,
            );
            if let Err(e) = client.invoke(&probe_req).await {
                if e.contains("PARTICIPANTS_HIDDEN") || e.contains("CHAT_ADMIN_REQUIRED") {
                    if let Ok(db) = db.lock() {
                        db.execute(
                            "UPDATE group_info SET status = 'no_access' WHERE link = ?1",
                            params![target],
                        )
                        .ok();
                    }
                    return Err(t("parser_no_admin_rights"));
                }
                return Err(t_with("parser_access_check", &[("error", &e)]));
            }
        }
        ParserMode::Messages | ParserMode::Comments => {}
    }

    // insert group_info
    {
        let db = db.lock().unwrap();
        db.execute(
            "INSERT OR REPLACE INTO group_info (channel_id, link, title, username, participants_count, is_broadcast, status) VALUES (?1, ?2, ?3, ?4, 0, ?5, 'parsing')",
            params![
                resolved.channel_id,
                target,
                resolved.title_hint,
                resolved.username_hint.as_deref().unwrap_or(""),
                resolved.is_broadcast as i32,
            ],
        ).ok();
    }

    // dispatch to the appropriate method
    let (collected, _total_walked) = match cfg.mode {
        ParserMode::Group | ParserMode::ChannelAdmin => {
            run_participants_mode(
                account_id,
                &mut client,
                &resolved,
                cfg,
                max_flood_wait,
                db,
                app,
                token,
            )
            .await?
        }
        ParserMode::Messages => {
            run_messages_mode(
                account_id,
                &mut client,
                &resolved,
                cfg,
                max_flood_wait,
                db,
                app,
                token,
            )
            .await?
        }
        ParserMode::Comments => {
            run_comments_mode(
                account_id,
                &mut client,
                &resolved,
                cfg,
                max_flood_wait,
                db,
                app,
                token,
            )
            .await?
        }
    };

    // update group status
    {
        let db = db.lock().unwrap();
        db.execute(
            "UPDATE group_info SET status = 'done', users_collected = ?1 WHERE channel_id = ?2",
            params![collected as i64, resolved.channel_id],
        )
        .ok();
    }

    // leave group if configured
    if cfg.leave_after && resolved.joined_now {
        let leave_req = tl::build_leave_channel(resolved.channel_id, resolved.access_hash);
        let _ = client.invoke(&leave_req).await;
    }

    Ok(collected)
}

// ─── Participants mode (alphabet search) ───────────────────────────────────

async fn run_participants_mode(
    account_id: &str,
    client: &mut MtpClient,
    resolved: &crate::mtproto::invite::ResolvedChannel,
    cfg: &ParserConfig,
    max_flood_wait: u64,
    db: &Arc<std::sync::Mutex<rusqlite::Connection>>,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(u32, u32), String> {
    let mut admin_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    if cfg.parse_admins || cfg.exclude_admins {
        match fetch_all_admins(
            client,
            resolved.channel_id,
            resolved.access_hash,
            app,
            token,
        )
        .await
        {
            Ok(ids) => {
                emit(
                    app,
                    t_with("parser_admins_loaded", &[("count", &ids.len().to_string())]),
                );
                admin_ids = ids;
            }
            Err(e) => {
                emit(app, t_with("parser_admins_error", &[("error", &e)]));
            }
        }
    }

    let search_chars = build_search_alphabet(cfg);
    let use_alphabet_search = !search_chars.is_empty();

    let method = if use_alphabet_search {
        t_with(
            "parser_method_alphabet",
            &[("count", &search_chars.len().to_string())],
        )
    } else {
        t("parser_method_pagination")
    };
    emit(app, t_with("parser_method", &[("method", &method)]));

    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut collected = 0u32;
    let mut total_walked = 0u32;

    if use_alphabet_search {
        let total_chars = search_chars.len();
        for (char_idx, search_char) in search_chars.iter().enumerate() {
            if !token.load(Ordering::Relaxed) {
                emit(app, t("stopped_by_user"));
                break;
            }

            let mut offset = 0i32;
            let mut consecutive_errors = 0u32;
            let mut char_collected = 0u32;

            loop {
                if !token.load(Ordering::Relaxed) {
                    break;
                }
                if offset >= MAX_OFFSET {
                    break;
                }

                let req = tl::build_channels_get_participants_search(
                    resolved.channel_id,
                    resolved.access_hash,
                    search_char,
                    offset,
                    PAGE_SIZE,
                );

                let batch: ParticipantsBatch = match client.invoke(&req).await {
                    Ok(data) => match tl::parse_channel_participants(&data) {
                        Ok(b) => b,
                        Err(e) => {
                            emit(
                                app,
                                t_with(
                                    "parser_parse_error",
                                    &[
                                        ("char", search_char),
                                        ("offset", &offset.to_string()),
                                        ("error", &e),
                                    ],
                                ),
                            );
                            break;
                        }
                    },
                    Err(e) => {
                        if crate::mtproto::is_fatal_session_error(&e) {
                            return Err(e);
                        }
                        if e.contains("CHANNEL_PRIVATE") {
                            consecutive_errors += 1;
                            if consecutive_errors > MAX_CONSECUTIVE_ERRORS {
                                break;
                            }
                            emit(app, t("parser_channel_private_wait"));
                            interruptible_sleep(45_000, token).await;
                            let _ = reconnect_client(client, account_id).await;
                            continue;
                        }
                        if let Some(wait_secs) = parse_flood_wait_secs(&e) {
                            if max_flood_wait > 0 && wait_secs > max_flood_wait {
                                break;
                            }
                            emit(
                                app,
                                t_with(
                                    "parser_flood_wait_short",
                                    &[("seconds", &wait_secs.to_string())],
                                ),
                            );
                            interruptible_sleep((wait_secs + 10) * 1000, token).await;
                            let _ = reconnect_client(client, account_id).await;
                            continue;
                        }
                        if e.contains("PARTICIPANTS_HIDDEN") || e.contains("CHAT_ADMIN_REQUIRED") {
                            return Err(t("parser_members_hidden"));
                        }
                        if crate::mtproto::is_network_error(&e) {
                            let _ = reconnect_client(client, account_id).await;
                            consecutive_errors += 1;
                            if consecutive_errors > MAX_CONSECUTIVE_ERRORS {
                                break;
                            }
                            continue;
                        }
                        emit(app, t_with("parser_error", &[("error", &e)]));
                        break;
                    }
                };

                consecutive_errors = 0;
                if batch.users.is_empty() {
                    break;
                }
                let users_count = batch.users.len() as u32;

                for mut u in batch.users {
                    if !seen.insert(u.id) {
                        continue;
                    }
                    total_walked += 1;
                    if admin_ids.contains(&u.id) {
                        u.is_admin = true;
                    }
                    if cfg.exclude_admins && u.is_admin {
                        continue;
                    }
                    if cfg.exclude_no_username && u.username.is_empty() {
                        continue;
                    }
                    if !passes_filters(&u, cfg) {
                        continue;
                    }
                    insert_user_participant(db, &u, resolved.channel_id);
                    collected += 1;
                    char_collected += 1;
                }

                let advance = if batch.participants_count > 0 {
                    batch.participants_count
                } else {
                    users_count
                };
                offset += advance as i32;
                interruptible_sleep(PAGE_DELAY_MS, token).await;
            }

            emit(
                app,
                t_with(
                    "parser_char_progress",
                    &[
                        ("done", &(char_idx + 1).to_string()),
                        ("total", &total_chars.to_string()),
                        ("char", search_char),
                        ("added", &char_collected.to_string()),
                        ("collected", &collected.to_string()),
                        ("viewed", &total_walked.to_string()),
                    ],
                ),
            );
            if char_idx + 1 < total_chars {
                interruptible_sleep(CHAR_DELAY_MS, token).await;
            }
        }
    } else {
        // empty-query fallback
        let mut offset = 0i32;
        loop {
            if !token.load(Ordering::Relaxed) {
                break;
            }
            if offset >= MAX_OFFSET {
                break;
            }
            let req = tl::build_channels_get_participants_search(
                resolved.channel_id,
                resolved.access_hash,
                "",
                offset,
                PAGE_SIZE,
            );
            let batch: ParticipantsBatch = match client.invoke(&req).await {
                Ok(data) => match tl::parse_channel_participants(&data) {
                    Ok(b) => b,
                    Err(e) => {
                        emit(app, t_with("parser_error", &[("error", &e)]));
                        break;
                    }
                },
                Err(e) => {
                    if crate::mtproto::is_fatal_session_error(&e) {
                        return Err(e);
                    }
                    emit(app, t_with("parser_error", &[("error", &e)]));
                    break;
                }
            };
            if batch.users.is_empty() {
                break;
            }
            for mut u in batch.users {
                if !seen.insert(u.id) {
                    continue;
                }
                total_walked += 1;
                if admin_ids.contains(&u.id) {
                    u.is_admin = true;
                }
                if cfg.exclude_admins && u.is_admin {
                    continue;
                }
                if cfg.exclude_no_username && u.username.is_empty() {
                    continue;
                }
                if !passes_filters(&u, cfg) {
                    continue;
                }
                insert_user_participant(db, &u, resolved.channel_id);
                collected += 1;
                if collected % 100 == 0 {
                    emit(
                        app,
                        t_with(
                            "parser_collected_progress",
                            &[("count", &collected.to_string())],
                        ),
                    );
                }
            }
            let advance = if batch.participants_count > 0 {
                batch.participants_count
            } else {
                PAGE_SIZE as u32
            };
            offset += advance as i32;
            interruptible_sleep(PAGE_DELAY_MS, token).await;
        }
    }

    Ok((collected, total_walked))
}

// ─── Messages mode (iterate history, collect senders) ──────────────────────

async fn run_messages_mode(
    account_id: &str,
    client: &mut MtpClient,
    resolved: &crate::mtproto::invite::ResolvedChannel,
    cfg: &ParserConfig,
    max_flood_wait: u64,
    db: &Arc<std::sync::Mutex<rusqlite::Connection>>,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(u32, u32), String> {
    // compute cutoff timestamp
    let cutoff_ts: i32 = if cfg.parsing_days > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as i64;
        let cutoff = now - (cfg.parsing_days as i64 * 86400);
        cutoff as i32
    } else {
        0 // no cutoff — scan all history
    };

    if cfg.parsing_days > 0 {
        emit(
            app,
            t_with(
                "parser_msg_mode_days",
                &[("days", &cfg.parsing_days.to_string())],
            ),
        );
    } else {
        emit(app, t("parser_msg_mode_all"));
    }

    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut collected = 0u32;
    let mut total_messages = 0u32;
    let mut offset_id = 0i32;

    loop {
        if !token.load(Ordering::Relaxed) {
            emit(app, t("stopped_by_user"));
            break;
        }

        let req = tl::build_get_history_channel_paged(
            resolved.channel_id,
            resolved.access_hash,
            offset_id,
            0,
            MSG_PAGE_SIZE,
            0,
            0,
        );

        let data = match client.invoke(&req).await {
            Ok(d) => d,
            Err(e) => {
                if crate::mtproto::is_fatal_session_error(&e) {
                    return Err(e);
                }
                if let Some(wait_secs) = parse_flood_wait_secs(&e) {
                    if max_flood_wait > 0 && wait_secs > max_flood_wait {
                        emit(
                            app,
                            t_with(
                                "parser_flood_over_limit",
                                &[("seconds", &wait_secs.to_string())],
                            ),
                        );
                        break;
                    }
                    emit(
                        app,
                        t_with(
                            "parser_flood_wait_short",
                            &[("seconds", &wait_secs.to_string())],
                        ),
                    );
                    interruptible_sleep((wait_secs + 10) * 1000, token).await;
                    let _ = reconnect_client(client, account_id).await;
                    continue;
                }
                if crate::mtproto::is_network_error(&e) {
                    let _ = reconnect_client(client, account_id).await;
                    continue;
                }
                emit(app, t_with("parser_get_history_error", &[("error", &e)]));
                break;
            }
        };

        // parse the messages response to extract senders
        let (messages_batch, reached_cutoff) = parse_messages_senders(
            &data,
            cutoff_ts,
            cfg,
            &mut seen,
            db,
            &mut collected,
            resolved.channel_id,
        );
        total_messages += messages_batch.message_count;

        if messages_batch.message_count == 0 {
            emit(app, t("parser_msg_history_end"));
            break;
        }

        if reached_cutoff {
            emit(
                app,
                t_with(
                    "parser_days_limit",
                    &[("days", &cfg.parsing_days.to_string())],
                ),
            );
            break;
        }

        // advance offset_id to the last message id in this page
        offset_id = messages_batch.last_msg_id;

        if total_messages % 500 == 0 || collected % 50 == 0 {
            emit(
                app,
                t_with(
                    "parser_msg_progress",
                    &[
                        ("messages", &total_messages.to_string()),
                        ("collected", &collected.to_string()),
                    ],
                ),
            );
        }

        interruptible_sleep(PAGE_DELAY_MS, token).await;
    }

    emit(
        app,
        t_with(
            "parser_msg_total",
            &[
                ("messages", &total_messages.to_string()),
                ("collected", &collected.to_string()),
            ],
        ),
    );
    Ok((collected, total_messages))
}

// ─── Comments mode (iterate posts, fetch replies for each) ─────────────────

const COMMENT_POST_PAGE: i32 = 50;
const COMMENT_REPLY_PAGE: i32 = 100;
const COMMENT_REPLY_DELAY_MS: u64 = 200;

async fn run_comments_mode(
    account_id: &str,
    client: &mut MtpClient,
    resolved: &crate::mtproto::invite::ResolvedChannel,
    cfg: &ParserConfig,
    max_flood_wait: u64,
    db: &Arc<std::sync::Mutex<rusqlite::Connection>>,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(u32, u32), String> {
    let cutoff_ts: i32 = if cfg.parsing_days > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as i64;
        (now - cfg.parsing_days as i64 * 86400) as i32
    } else {
        0
    };

    if cfg.parsing_days > 0 {
        emit(
            app,
            t_with(
                "parser_comment_mode_days",
                &[("days", &cfg.parsing_days.to_string())],
            ),
        );
    } else {
        emit(app, t("parser_comment_mode_all"));
    }

    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut collected = 0u32;
    let mut posts_scanned = 0u32;
    let mut posts_with_comments = 0u32;
    let mut offset_id = 0i32;

    let peer_bytes =
        tl_gen::serialize_input_peer_channel(resolved.channel_id, resolved.access_hash);

    loop {
        if !token.load(Ordering::Relaxed) {
            emit(app, t("stopped_by_user"));
            break;
        }

        let req = tl::build_get_history_channel_paged(
            resolved.channel_id,
            resolved.access_hash,
            offset_id,
            0,
            COMMENT_POST_PAGE,
            0,
            0,
        );

        let data = match client.invoke(&req).await {
            Ok(d) => d,
            Err(e) => {
                if crate::mtproto::is_fatal_session_error(&e) {
                    return Err(e);
                }
                if let Some(wait) = parse_flood_wait_secs(&e) {
                    if max_flood_wait > 0 && wait > max_flood_wait {
                        break;
                    }
                    emit(
                        app,
                        t_with("parser_flood_wait_short", &[("seconds", &wait.to_string())]),
                    );
                    interruptible_sleep((wait + 10) * 1000, token).await;
                    let _ = reconnect_client(client, account_id).await;
                    continue;
                }
                if crate::mtproto::is_network_error(&e) {
                    let _ = reconnect_client(client, account_id).await;
                    continue;
                }
                emit(app, t_with("parser_error", &[("error", &e)]));
                break;
            }
        };

        // parse posts
        let posts = parse_posts_with_replies(&data, cutoff_ts);
        if posts.items.is_empty() {
            emit(app, t("parser_posts_history_end"));
            break;
        }

        for post in &posts.items {
            posts_scanned += 1;
            offset_id = post.id;

            if post.replies_count == 0 {
                continue;
            }
            posts_with_comments += 1;

            // fetch replies for this post
            let mut reply_offset_id = 0i32;
            loop {
                if !token.load(Ordering::Relaxed) {
                    break;
                }

                let req = tl_gen::build_messages_getReplies(
                    &peer_bytes,
                    post.id,
                    reply_offset_id,
                    0,
                    0,
                    COMMENT_REPLY_PAGE,
                    0,
                    0,
                    0,
                );

                let reply_data = match client.invoke(&req).await {
                    Ok(d) => d,
                    Err(e) => {
                        if let Some(wait) = parse_flood_wait_secs(&e) {
                            if max_flood_wait > 0 && wait > max_flood_wait {
                                break;
                            }
                            interruptible_sleep((wait + 5) * 1000, token).await;
                            let _ = reconnect_client(client, account_id).await;
                            continue;
                        }
                        break; // MSG_ID_INVALID, CHANNEL_PRIVATE, etc.
                    }
                };

                let batch = parse_reply_messages(&reply_data);
                if batch.replies.is_empty() {
                    break;
                }

                for reply in &batch.replies {
                    if let Some(user_id) = reply.from_user_id {
                        if !seen.insert(user_id) {
                            continue;
                        }

                        if let Some(u) = batch.user_map.get(&user_id) {
                            if u.bot && !cfg.parse_bots {
                                continue;
                            }
                            if cfg.exclude_no_username && u.username.is_empty() {
                                continue;
                            }
                            if cfg.premium_only && !u.premium {
                                continue;
                            }

                            let db = db.lock().unwrap();
                            db.execute(
                                "INSERT INTO users (user_id, access_hash, username, phone, first_name, last_name, is_bot, is_deleted, premium, status, source, source_group, msg_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'unknown','comment',?10,?11)
                                 ON CONFLICT(user_id) DO UPDATE SET source = excluded.source, source_group = excluded.source_group, msg_id = excluded.msg_id",
                                params![u.id, u.access_hash, u.username, u.phone, u.first_name, u.last_name, u.bot as i32, u.deleted as i32, u.premium as i32, resolved.channel_id, reply.id],
                            ).ok();
                        } else {
                            let db = db.lock().unwrap();
                            db.execute(
                                "INSERT INTO users (user_id, source, source_group, msg_id) VALUES (?1, 'comment', ?2, ?3)
                                 ON CONFLICT(user_id) DO UPDATE SET source = excluded.source, source_group = excluded.source_group, msg_id = excluded.msg_id",
                                params![user_id, resolved.channel_id, reply.id],
                            ).ok();
                        }
                        collected += 1;
                    }
                }

                if let Some(last) = batch.replies.last() {
                    reply_offset_id = last.id;
                } else {
                    break;
                }

                if (batch.replies.len() as i32) < COMMENT_REPLY_PAGE {
                    break;
                }

                interruptible_sleep(COMMENT_REPLY_DELAY_MS, token).await;
            }
        }

        if posts_with_comments % 5 == 0 && posts_with_comments > 0 {
            emit(
                app,
                t_with(
                    "parser_posts_progress",
                    &[
                        ("scanned", &posts_scanned.to_string()),
                        ("with_comments", &posts_with_comments.to_string()),
                        ("collected", &collected.to_string()),
                    ],
                ),
            );
        }

        if posts.reached_cutoff {
            emit(
                app,
                t_with(
                    "parser_days_limit_short",
                    &[("days", &cfg.parsing_days.to_string())],
                ),
            );
            break;
        }

        interruptible_sleep(PAGE_DELAY_MS, token).await;
    }

    emit(
        app,
        t_with(
            "parser_posts_total",
            &[
                ("scanned", &posts_scanned.to_string()),
                ("with_comments", &posts_with_comments.to_string()),
                ("collected", &collected.to_string()),
            ],
        ),
    );
    Ok((collected, posts_scanned))
}

struct PostWithReplies {
    id: i32,
    replies_count: i32,
}
struct PostsWithRepliesPage {
    items: Vec<PostWithReplies>,
    reached_cutoff: bool,
}

fn parse_posts_with_replies(data: &[u8], cutoff_ts: i32) -> PostsWithRepliesPage {
    let empty = PostsWithRepliesPage {
        items: Vec::new(),
        reached_cutoff: false,
    };
    let inner = match tl_gen::unwrap_rpc(data) {
        Ok(d) => d,
        Err(_) => return empty,
    };
    let mut cursor = Cursor::new(inner.as_slice());
    let resp = match tl_gen::TlMessagesMessages::deserialize(&mut cursor) {
        Ok(r) => r,
        Err(_) => return empty,
    };
    let raw_messages = match resp {
        tl_gen::TlMessagesMessages::Messages { messages, .. } => messages,
        tl_gen::TlMessagesMessages::Slice { messages, .. } => messages,
        tl_gen::TlMessagesMessages::ChannelMessages { messages, .. } => messages,
        tl_gen::TlMessagesMessages::NotModified { .. } => return empty,
    };
    let mut items = Vec::new();
    let mut reached_cutoff = false;
    for raw in &raw_messages {
        if let Ok(msg) = tl_gen::deserialize_tl_obj::<tl_gen::TlMessage>(raw) {
            match msg {
                tl_gen::TlMessage::Message {
                    id, date, replies, ..
                } => {
                    if cutoff_ts > 0 && date < cutoff_ts {
                        reached_cutoff = true;
                        break;
                    }
                    let replies_count = replies
                        .as_ref()
                        .and_then(|r| parse_replies_header(r))
                        .unwrap_or(0);
                    items.push(PostWithReplies { id, replies_count });
                }
                tl_gen::TlMessage::Empty { id, .. } | tl_gen::TlMessage::Service { id, .. } => {
                    items.push(PostWithReplies {
                        id,
                        replies_count: 0,
                    });
                }
            }
        }
    }
    PostsWithRepliesPage {
        items,
        reached_cutoff,
    }
}

/// Extract replies count from messageReplies TL bytes
fn parse_replies_header(data: &[u8]) -> Option<i32> {
    use byteorder::{LittleEndian, ReadBytesExt};
    let mut cursor = Cursor::new(data);
    let _ctor = cursor.read_u32::<LittleEndian>().ok()?;
    let _flags = cursor.read_u32::<LittleEndian>().ok()?;
    let replies = cursor.read_i32::<LittleEndian>().ok()?;
    Some(replies)
}

struct ReplyInfo {
    id: i32,
    from_user_id: Option<i64>,
}
struct ReplyUserInfo {
    id: i64,
    access_hash: i64,
    username: String,
    phone: String,
    first_name: String,
    last_name: String,
    premium: bool,
    bot: bool,
    deleted: bool,
}
struct ReplyBatch {
    replies: Vec<ReplyInfo>,
    user_map: std::collections::HashMap<i64, ReplyUserInfo>,
}

fn parse_reply_messages(data: &[u8]) -> ReplyBatch {
    let empty = ReplyBatch {
        replies: Vec::new(),
        user_map: std::collections::HashMap::new(),
    };
    let inner = match tl_gen::unwrap_rpc(data) {
        Ok(d) => d,
        Err(_) => return empty,
    };
    let mut cursor = Cursor::new(inner.as_slice());
    let resp = match tl_gen::TlMessagesMessages::deserialize(&mut cursor) {
        Ok(r) => r,
        Err(_) => return empty,
    };
    let (raw_messages, raw_users) = match resp {
        tl_gen::TlMessagesMessages::Messages {
            messages, users, ..
        } => (messages, users),
        tl_gen::TlMessagesMessages::Slice {
            messages, users, ..
        } => (messages, users),
        tl_gen::TlMessagesMessages::ChannelMessages {
            messages, users, ..
        } => (messages, users),
        tl_gen::TlMessagesMessages::NotModified { .. } => return empty,
    };
    let mut user_map = std::collections::HashMap::new();
    for raw in &raw_users {
        if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
            if let tl_gen::TlUser::User {
                id,
                access_hash,
                first_name,
                last_name,
                username,
                phone,
                premium,
                bot,
                deleted,
                ..
            } = user
            {
                user_map.insert(
                    id,
                    ReplyUserInfo {
                        id,
                        access_hash: access_hash.unwrap_or(0),
                        username: username.unwrap_or_default(),
                        phone: phone.unwrap_or_default(),
                        first_name: first_name.unwrap_or_default(),
                        last_name: last_name.unwrap_or_default(),
                        premium,
                        bot,
                        deleted,
                    },
                );
            }
        }
    }
    let mut replies = Vec::new();
    for raw in &raw_messages {
        if let Ok(msg) = tl_gen::deserialize_tl_obj::<tl_gen::TlMessage>(raw) {
            if let tl_gen::TlMessage::Message { id, from_id, .. } = msg {
                let from_user_id = from_id.as_ref().and_then(|b| extract_user_id_from_peer(b));
                replies.push(ReplyInfo { id, from_user_id });
            }
        }
    }
    ReplyBatch { replies, user_map }
}

struct MessagesBatchResult {
    message_count: u32,
    last_msg_id: i32,
}

/// Parse messages response, extract user senders, insert into DB.
/// Returns batch info and whether we've reached the date cutoff.
fn parse_messages_senders(
    data: &[u8],
    cutoff_ts: i32,
    cfg: &ParserConfig,
    seen: &mut std::collections::HashSet<i64>,
    db: &std::sync::Mutex<rusqlite::Connection>,
    collected: &mut u32,
    source_group: i64,
) -> (MessagesBatchResult, bool) {
    let empty = (
        MessagesBatchResult {
            message_count: 0,
            last_msg_id: 0,
        },
        false,
    );

    let inner = match tl_gen::unwrap_rpc(data) {
        Ok(d) => d,
        Err(_) => return empty,
    };

    let mut cursor = Cursor::new(inner.as_slice());
    let resp = match tl_gen::TlMessagesMessages::deserialize(&mut cursor) {
        Ok(r) => r,
        Err(_) => return empty,
    };

    let (raw_messages, raw_users) = match resp {
        tl_gen::TlMessagesMessages::Messages {
            messages, users, ..
        } => (messages, users),
        tl_gen::TlMessagesMessages::Slice {
            messages, users, ..
        } => (messages, users),
        tl_gen::TlMessagesMessages::ChannelMessages {
            messages, users, ..
        } => (messages, users),
        tl_gen::TlMessagesMessages::NotModified { .. } => return empty,
    };

    if raw_messages.is_empty() {
        return empty;
    }

    // build user lookup map from the users vector
    let mut user_map: std::collections::HashMap<i64, UserFromMsg> =
        std::collections::HashMap::new();
    for raw in &raw_users {
        if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
            if let tl_gen::TlUser::User {
                id,
                access_hash,
                first_name,
                last_name,
                username,
                phone,
                premium,
                bot,
                deleted,
                ..
            } = user
            {
                user_map.insert(
                    id,
                    UserFromMsg {
                        id,
                        access_hash: access_hash.unwrap_or(0),
                        username: username.unwrap_or_default(),
                        phone: phone.unwrap_or_default(),
                        first_name: first_name.unwrap_or_default(),
                        last_name: last_name.unwrap_or_default(),
                        premium,
                        bot,
                        deleted,
                    },
                );
            }
        }
    }

    let mut last_msg_id = 0i32;
    let mut message_count = 0u32;
    let mut reached_cutoff = false;

    for raw in &raw_messages {
        if let Ok(msg) = tl_gen::deserialize_tl_obj::<tl_gen::TlMessage>(raw) {
            match msg {
                tl_gen::TlMessage::Message {
                    id, from_id, date, ..
                } => {
                    message_count += 1;
                    last_msg_id = id;

                    // check date cutoff
                    if cutoff_ts > 0 && date < cutoff_ts {
                        reached_cutoff = true;
                        break;
                    }

                    // extract sender user_id from from_id peer
                    if let Some(from_bytes) = from_id {
                        if let Some(user_id) = extract_user_id_from_peer(&from_bytes) {
                            if !seen.insert(user_id) {
                                continue;
                            }

                            if let Some(u) = user_map.get(&user_id) {
                                if u.bot && !cfg.parse_bots {
                                    continue;
                                }
                                if cfg.exclude_no_username && u.username.is_empty() {
                                    continue;
                                }
                                if cfg.premium_only && !u.premium {
                                    continue;
                                }

                                insert_user_from_msg(db, u, id, source_group);
                                *collected += 1;
                            } else {
                                // user not in users vector — insert minimal record
                                let db = db.lock().unwrap();
                                db.execute(
                                    "INSERT INTO users (user_id, source, source_group, msg_id) VALUES (?1, 'message', ?2, ?3)
                                     ON CONFLICT(user_id) DO UPDATE SET source = excluded.source, source_group = excluded.source_group, msg_id = excluded.msg_id",
                                    params![user_id, source_group, id],
                                ).ok();
                                *collected += 1;
                            }
                        }
                    }
                }
                tl_gen::TlMessage::Empty { id, .. } => {
                    message_count += 1;
                    last_msg_id = id;
                }
                tl_gen::TlMessage::Service { id, .. } => {
                    message_count += 1;
                    last_msg_id = id;
                }
            }
        }
    }

    (
        MessagesBatchResult {
            message_count,
            last_msg_id,
        },
        reached_cutoff,
    )
}

struct UserFromMsg {
    id: i64,
    access_hash: i64,
    username: String,
    phone: String,
    first_name: String,
    last_name: String,
    premium: bool,
    bot: bool,
    deleted: bool,
}

fn extract_user_id_from_peer(peer_bytes: &[u8]) -> Option<i64> {
    let mut cursor = Cursor::new(peer_bytes as &[u8]);
    if let Ok(peer) = tl_gen::read_peer(&mut cursor) {
        match peer {
            tl_gen::Peer::User(uid) => Some(uid),
            _ => None, // skip channels/chats posting as themselves
        }
    } else {
        None
    }
}

fn insert_user_from_msg(
    db: &std::sync::Mutex<rusqlite::Connection>,
    u: &UserFromMsg,
    msg_id: i32,
    source_group: i64,
) {
    let db = db.lock().unwrap();
    db.execute(
        "INSERT INTO users (user_id, access_hash, username, phone, first_name, last_name, is_bot, is_deleted, premium, source, source_group, msg_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'message', ?10, ?11)
         ON CONFLICT(user_id) DO UPDATE SET source = excluded.source, source_group = excluded.source_group, msg_id = excluded.msg_id",
        params![
            u.id, u.access_hash, u.username, u.phone,
            u.first_name, u.last_name,
            u.bot as i32, u.deleted as i32, u.premium as i32,
            source_group, msg_id,
        ],
    ).ok();
}

fn insert_user_participant(
    db: &std::sync::Mutex<rusqlite::Connection>,
    u: &ParticipantUser,
    source_group: i64,
) {
    let db = db.lock().unwrap();
    db.execute(
        "INSERT INTO users (user_id, access_hash, username, phone, first_name, last_name, is_bot, is_admin, is_deleted, premium, status, source, source_group) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'participants', ?12)
         ON CONFLICT(user_id) DO UPDATE SET source = excluded.source, source_group = excluded.source_group, is_admin = excluded.is_admin",
        params![
            u.id, u.access_hash, u.username, u.phone,
            u.first_name, u.last_name,
            u.is_bot as i32, u.is_admin as i32, u.is_deleted as i32, u.premium as i32,
            bucket_label(u.bucket), source_group,
        ],
    ).ok();
}

// ─── Shared utilities ──────────────────────────────────────────────────────

fn build_search_alphabet(cfg: &ParserConfig) -> Vec<String> {
    let mut chars: Vec<String> = Vec::new();
    if cfg.chars_en {
        for c in 'a'..='z' {
            chars.push(c.to_string());
        }
    }
    if cfg.chars_ru {
        for c in &[
            'а', 'б', 'в', 'г', 'д', 'е', 'ж', 'з', 'и', 'й', 'к', 'л', 'м', 'н', 'о', 'п', 'р',
            'с', 'т', 'у', 'ф', 'х', 'ц', 'ч', 'ш', 'щ', 'ъ', 'ы', 'ь', 'э', 'ю', 'я', '.',
        ] {
            chars.push(c.to_string());
        }
    }
    if cfg.chars_cn {
        for c in &[
            '的', '一', '是', '不', '了', '人', '我', '在', '有', '他', '这', '中', '大', '来',
            '上', '国', '个', '到', '说', '们', '为', '子', '和', '你', '地', '出', '道', '也',
            '时', '年', '得', '就', '那', '要', '下', '以', '生', '会', '自', '着', '去', '之',
            '过', '家', '学', '对', '可', '她', '里', '后',
        ] {
            chars.push(c.to_string());
        }
    }
    if cfg.chars_ar {
        for c in &[
            'ا', 'ب', 'ت', 'ث', 'ج', 'ح', 'خ', 'د', 'ذ', 'ر', 'ز', 'س', 'ش', 'ص', 'ض', 'ط', 'ظ',
            'ع', 'غ', 'ف', 'ق', 'ك', 'ل', 'م', 'ن', 'ه', 'و', 'ي',
        ] {
            chars.push(c.to_string());
        }
    }
    if cfg.chars_he {
        for c in &[
            'א', 'ב', 'ג', 'ד', 'ה', 'ו', 'ז', 'ח', 'ט', 'כ', 'ל', 'מ', 'נ', 'ס', 'ע', 'פ', 'צ',
            'ק', 'ר', 'ש', 'ת',
        ] {
            chars.push(c.to_string());
        }
    }
    if cfg.chars_fa {
        for c in &[
            'ا', 'ب', 'پ', 'ت', 'ث', 'ج', 'چ', 'ح', 'خ', 'د', 'ذ', 'ر', 'ز', 'ژ', 'س', 'ش', 'ص',
            'ض', 'ط', 'ظ', 'ع', 'غ', 'ف', 'ق', 'ک', 'گ', 'ل', 'م', 'ن', 'و', 'ه', 'ی',
        ] {
            chars.push(c.to_string());
        }
    }
    if cfg.chars_emoji {
        for c in &[
            "😊", "👍", "😀", "☺", "🤓", "😁", "👌", "🧐", "🙈", "😌", "😉", "👇", "👉", "😃",
            "😄", "😅", "🙃", "🙂", "😎", "😏", "🤔", "🤭", "👐", "🤝", "🤟", "✌", "✋", "🙏",
            "🐰", "🐹", "🐭", "🐱", "🐯", "🦁", "🐮", "🐷", "🐵", "🙊", "🐶",
        ] {
            chars.push(c.to_string());
        }
    }
    chars
}

fn any_status_filter(cfg: &ParserConfig) -> bool {
    cfg.parse_deleted || cfg.parse_recent || cfg.parse_week || cfg.parse_month || cfg.parse_long
}

fn passes_filters(u: &ParticipantUser, cfg: &ParserConfig) -> bool {
    if u.is_self {
        return false;
    }
    if u.is_bot && !cfg.parse_bots {
        return false;
    }
    if u.is_admin && !cfg.parse_admins {
        return false;
    }
    if cfg.premium_only && !u.premium {
        return false;
    }
    match u.bucket {
        OnlineBucket::Deleted => cfg.parse_deleted,
        OnlineBucket::Recent => cfg.parse_recent,
        OnlineBucket::Week => cfg.parse_week,
        OnlineBucket::Month => cfg.parse_month,
        OnlineBucket::Long | OnlineBucket::Unknown => cfg.parse_long,
    }
}

fn bucket_label(b: OnlineBucket) -> &'static str {
    match b {
        OnlineBucket::Recent => "recent",
        OnlineBucket::Week => "last_week",
        OnlineBucket::Month => "last_month",
        OnlineBucket::Long => "long_ago",
        OnlineBucket::Deleted => "deleted",
        OnlineBucket::Unknown => "unknown",
    }
}

async fn fetch_all_admins(
    client: &mut MtpClient,
    channel_id: i64,
    access_hash: i64,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<std::collections::HashSet<i64>, String> {
    let mut ids = std::collections::HashSet::new();
    let mut offset = 0i32;
    loop {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let req = tl::build_channels_get_participants(
            channel_id,
            access_hash,
            ParticipantsFilter::Admins,
            offset,
            PAGE_SIZE,
        );
        let data = client
            .invoke(&req)
            .await
            .map_err(|e| format!("admins: {e}"))?;
        let batch =
            tl::parse_channel_participants(&data).map_err(|e| format!("parse admins: {e}"))?;
        if batch.users.is_empty() {
            break;
        }
        for u in batch.users {
            ids.insert(u.id);
        }
        let advance = if batch.participants_count > 0 {
            batch.participants_count
        } else {
            PAGE_SIZE as u32
        };
        offset += advance as i32;
        interruptible_sleep(150, token).await;
    }
    let _ = app;
    Ok(ids)
}

async fn reconnect_client(client: &mut MtpClient, account_id: &str) -> Result<(), String> {
    *client = connect_account(account_id).await?;
    Ok(())
}

fn parse_flood_wait_secs(err: &str) -> Option<u64> {
    let msg = err
        .strip_prefix("RPC ")
        .and_then(|s| s.split_once(": ").map(|(_, m)| m))
        .unwrap_or(err);
    if let Some(pos) = msg.rfind('_') {
        if let Ok(secs) = msg[pos + 1..].parse::<u64>() {
            return Some(secs);
        }
    }
    None
}

fn resolve_output_path(user_path: &str, channel_id: &i64) -> PathBuf {
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
        .join("parser");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    base.join(format!("parsed_{channel_id}_{now}.db"))
}

fn emit(app: &tauri::AppHandle, msg: String) {
    let _ = app.emit("parser-log", msg);
}
