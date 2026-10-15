// link_checker: validate telegram links (usernames, channels, groups, bots, private invites)
// via MTProto. threads = number of selected accounts.
// Enhanced: detects entity type (user/group/channel/bot), collects metadata,
// supports private group verification, outputs results to SQLite .db file.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::params;
use serde::Deserialize;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex as TokioMutex;

use crate::accounts::connect::connect_account;
use crate::i18n::{t, t_with};
use crate::mtproto::client::MtpClient;
use crate::mtproto::text_parse;
use crate::mtproto::tl;
use crate::mtproto::tl_gen;
use crate::queue::TaskQueue;

#[derive(Deserialize, Clone, Debug)]
pub struct LinkCheckerConfig {
    pub input_path: String,
    pub output_path: String,
    pub max_flood_wait: u32,
    pub delay_min: u32,
    pub delay_max: u32,
    pub check_private_groups: bool,
    pub standardize_links: bool,
    pub links_per_account: u32,
}

/// Result of checking a single link
#[derive(Clone, Debug)]
#[allow(dead_code)]
enum CheckResult {
    User {
        id: i64,
        access_hash: i64,
        username: String,
        first_name: String,
        last_name: String,
        phone: String,
        premium: bool,
        bot: bool,
        deleted: bool,
    },
    Group {
        id: i64,
        title: String,
        username: String,
        participants_count: u32,
        is_broadcast: bool,
    },
    PrivateGroup {
        title: String,
        link: String,
    },
    Invalid,
    Skipped(String),
}

#[tauri::command]
pub async fn link_checker_start(
    ids: Vec<String>,
    config: LinkCheckerConfig,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() {
        return Err(t("link_checker_no_accounts"));
    }
    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();
    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "link_checker".to_string(),
            t("link_checker_task_name"),
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

#[tauri::command]
pub async fn link_checker_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

/// Initialize SQLite database with proper schema
fn init_db(path: &PathBuf) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(path)
        .map_err(|e| t_with("link_checker_db_open_error", &[("error", &e.to_string())]))?;
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS links (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            link TEXT NOT NULL,
            entity_type TEXT NOT NULL DEFAULT 'unknown',
            entity_id INTEGER DEFAULT 0,
            access_hash INTEGER DEFAULT 0,
            username TEXT DEFAULT '',
            title TEXT DEFAULT '',
            first_name TEXT DEFAULT '',
            last_name TEXT DEFAULT '',
            phone TEXT DEFAULT '',
            participants_count INTEGER DEFAULT 0,
            premium INTEGER DEFAULT 0,
            bot INTEGER DEFAULT 0,
            deleted INTEGER DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'pending',
            checked_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_links_status ON links(status);
        CREATE INDEX IF NOT EXISTS idx_links_entity_type ON links(entity_type);
    ",
    )
    .map_err(|e| t_with("link_checker_db_tables_error", &[("error", &e.to_string())]))?;
    Ok(conn)
}

async fn run(
    account_ids: Vec<String>,
    cfg: Arc<LinkCheckerConfig>,
    app: &tauri::AppHandle,
    token: Arc<AtomicBool>,
) -> Result<(), String> {
    let input_path = cfg.input_path.trim();
    if input_path.is_empty() {
        return Err(t("link_checker_no_input_file"));
    }
    let lines = std::fs::read_to_string(input_path)
        .map_err(|e| t_with("link_checker_read_error", &[("error", &e.to_string())]))?;
    let mut links: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in lines.lines() {
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        let link = if cfg.standardize_links {
            standardize_link(&trimmed)
        } else {
            trimmed
        };
        if !seen.insert(link.clone()) {
            continue;
        }
        links.push(link);
    }
    if links.is_empty() {
        return Err(t("link_checker_file_empty"));
    }

    let concurrency = account_ids.len();
    emit(
        app,
        t_with(
            "link_checker_loaded",
            &[
                ("links", &links.len().to_string()),
                ("accounts", &concurrency.to_string()),
            ],
        ),
    );

    // resolve output path (ensure .db extension)
    let output_path = resolve_output_path(&cfg.output_path);
    if output_path.as_os_str().is_empty() {
        return Err(t("link_checker_no_output_file"));
    }
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }

    // init SQLite database
    let db = init_db(&output_path)?;
    let db = Arc::new(TokioMutex::new(db));

    let valid_count = Arc::new(AtomicU32::new(0));
    let invalid_count = Arc::new(AtomicU32::new(0));
    let skipped_count = Arc::new(AtomicU32::new(0));
    let total = links.len();

    // distribute links
    let mut batches: Vec<Vec<(usize, String)>> = vec![Vec::new(); concurrency];
    let per_acc = if cfg.links_per_account > 0 {
        cfg.links_per_account as usize
    } else {
        usize::MAX
    };
    let mut acc_counts = vec![0usize; concurrency];
    for (i, link) in links.iter().enumerate() {
        let target_acc = i % concurrency;
        if acc_counts[target_acc] < per_acc {
            batches[target_acc].push((i, link.clone()));
            acc_counts[target_acc] += 1;
        }
    }

    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut handles = Vec::new();

    for (thread_idx, batch) in batches.into_iter().enumerate() {
        if batch.is_empty() {
            continue;
        }
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let account_id = account_ids[thread_idx].clone();
        let sem = sem.clone();
        let token_clone = token.clone();
        let app_clone = app.clone();
        let db_clone = db.clone();
        let valid_clone = valid_count.clone();
        let invalid_clone = invalid_count.clone();
        let skipped_clone = skipped_count.clone();
        let cfg_clone = cfg.clone();
        let max_fw = cfg_clone.max_flood_wait;
        let check_private = cfg_clone.check_private_groups;

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            if !token_clone.load(Ordering::Relaxed) { return; }

            let mut client = match connect_account(&account_id).await {
                Ok(c) => c,
                Err(e) => {
                    let _ = app_clone.emit("link-checker-log", t_with("link_checker_thread_connect_error", &[("idx", &(thread_idx + 1).to_string()), ("error", &e)]));
                    return;
                }
            };

            for (idx, link) in batch {
                if !token_clone.load(Ordering::Relaxed) { break; }

                let result = check_link_full(&mut client, &link, max_fw, check_private, &token_clone).await;

                // write result to SQLite
                match &result {
                    CheckResult::User { id, access_hash, username, first_name, last_name, phone, premium, bot, deleted } => {
                        valid_clone.fetch_add(1, Ordering::Relaxed);
                        let type_str = if *bot { "bot" } else { "user" };
                        let name = format!("{} {}", first_name, last_name).trim().to_string();
                        let display_name = if name.is_empty() { username.as_str() } else { &name };
                        let _ = app_clone.emit("link-checker-log", t_with("link_checker_valid", &[
                            ("idx", &(idx + 1).to_string()), ("total", &total.to_string()),
                            ("link", &link), ("kind", type_str), ("name", display_name),
                        ]));
                        let db = db_clone.lock().await;
                        db.execute(
                            "INSERT INTO links (link, entity_type, entity_id, access_hash, username, first_name, last_name, phone, premium, bot, deleted, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'valid')",
                            params![link, type_str, id, access_hash, username, first_name, last_name, phone, *premium as i32, *bot as i32, *deleted as i32],
                        ).ok();
                    }
                    CheckResult::Group { id, title, username, participants_count, is_broadcast } => {
                        valid_clone.fetch_add(1, Ordering::Relaxed);
                        let type_str = if *is_broadcast { "channel" } else { "group" };
                        let display_name = format!("{} [{}]", title, participants_count);
                        let _ = app_clone.emit("link-checker-log", t_with("link_checker_valid", &[
                            ("idx", &(idx + 1).to_string()), ("total", &total.to_string()),
                            ("link", &link), ("kind", type_str), ("name", &display_name),
                        ]));
                        let db = db_clone.lock().await;
                        db.execute(
                            "INSERT INTO links (link, entity_type, entity_id, username, title, participants_count, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'valid')",
                            params![link, type_str, id, username, title, *participants_count as i64],
                        ).ok();
                    }
                    CheckResult::PrivateGroup { title, link: orig_link } => {
                        valid_clone.fetch_add(1, Ordering::Relaxed);
                        let _ = app_clone.emit("link-checker-log", t_with("link_checker_valid", &[
                            ("idx", &(idx + 1).to_string()), ("total", &total.to_string()),
                            ("link", orig_link), ("kind", "private_group"), ("name", title),
                        ]));
                        let db = db_clone.lock().await;
                        db.execute(
                            "INSERT INTO links (link, entity_type, title, status) VALUES (?1, 'private_group', ?2, 'valid')",
                            params![orig_link, title],
                        ).ok();
                    }
                    CheckResult::Invalid => {
                        invalid_clone.fetch_add(1, Ordering::Relaxed);
                        let _ = app_clone.emit("link-checker-log", t_with("link_checker_invalid", &[
                            ("idx", &(idx + 1).to_string()), ("total", &total.to_string()), ("link", &link),
                        ]));
                        let db = db_clone.lock().await;
                        db.execute(
                            "INSERT INTO links (link, entity_type, status) VALUES (?1, 'unknown', 'invalid')",
                            params![link],
                        ).ok();
                    }
                    CheckResult::Skipped(reason) => {
                        skipped_clone.fetch_add(1, Ordering::Relaxed);
                        let _ = app_clone.emit("link-checker-log", t_with("link_checker_skipped", &[
                            ("idx", &(idx + 1).to_string()), ("total", &total.to_string()), ("link", &link), ("reason", reason.as_str()),
                        ]));
                        let db = db_clone.lock().await;
                        db.execute(
                            "INSERT INTO links (link, entity_type, status) VALUES (?1, 'unknown', 'skipped')",
                            params![link],
                        ).ok();
                    }
                }

                let delay = random_delay(cfg_clone.delay_min, cfg_clone.delay_max);
                let jitter = if delay > 0 { rand::random::<u32>() % 200 } else { 0 };
                let total_delay = delay + jitter;
                if total_delay > 0 {
                    interruptible_sleep(total_delay as u64, &token_clone).await;
                }
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }

    let v = valid_count.load(Ordering::Relaxed);
    let inv = invalid_count.load(Ordering::Relaxed);
    let sk = skipped_count.load(Ordering::Relaxed);
    emit(
        app,
        t_with(
            "link_checker_result",
            &[
                ("valid", &v.to_string()),
                ("invalid", &inv.to_string()),
                ("skipped", &sk.to_string()),
                ("path", &output_path.display().to_string()),
            ],
        ),
    );
    Ok(())
}

/// Full check of a link: resolves entity, determines type, collects metadata.
async fn check_link_full(
    client: &mut MtpClient,
    link: &str,
    max_flood_wait: u32,
    check_private: bool,
    token: &AtomicBool,
) -> CheckResult {
    let trimmed = link.trim();

    // private invite link
    if let Some((kind, body)) = text_parse::parse_invite_link(trimmed) {
        if kind == "private" {
            if !check_private {
                return match check_invite_valid(client, &body, max_flood_wait, token).await {
                    Ok(Some(title)) => CheckResult::PrivateGroup {
                        title,
                        link: trimmed.to_string(),
                    },
                    Ok(None) => CheckResult::Invalid,
                    Err(e) => CheckResult::Skipped(e),
                };
            }
            return match check_invite_valid(client, &body, max_flood_wait, token).await {
                Ok(Some(title)) => CheckResult::PrivateGroup {
                    title,
                    link: trimmed.to_string(),
                },
                Ok(None) => CheckResult::Invalid,
                Err(e) => CheckResult::Skipped(e),
            };
        }
        if kind == "addlist" {
            return CheckResult::Invalid;
        }
    }

    // public username
    let username = match extract_public_username(trimmed) {
        Some(u) => u,
        None => return CheckResult::Invalid,
    };

    // resolve with flood-wait retry
    for attempt in 1..=3 {
        let req = tl::build_resolve_username(&username);
        match client.invoke(&req).await {
            Ok(data) => {
                return parse_resolved_entity(&data, &username);
            }
            Err(e) => {
                if e.contains("USERNAME_NOT_OCCUPIED") || e.contains("USERNAME_INVALID") {
                    return CheckResult::Invalid;
                }
                if let Some(wait_secs) = parse_flood_wait(&e) {
                    if wait_secs <= max_flood_wait {
                        if !token.load(Ordering::Relaxed) {
                            return CheckResult::Invalid;
                        }
                        if attempt < 3 {
                            interruptible_sleep(wait_secs as u64 * 1000, token).await;
                            continue;
                        }
                    }
                    return CheckResult::Skipped(format!("FLOOD_WAIT {}s", wait_secs));
                }
                if attempt == 3 {
                    return CheckResult::Skipped(e);
                }
            }
        }
    }
    CheckResult::Skipped(t("link_checker_retry_limit"))
}

/// Parse a resolved peer response to determine the entity type and extract metadata.
fn parse_resolved_entity(data: &[u8], username: &str) -> CheckResult {
    let inner = match tl_gen::unwrap_rpc(data) {
        Ok(d) => d,
        Err(_) => return CheckResult::Invalid,
    };

    use byteorder::{LittleEndian, ReadBytesExt};
    use std::io::Cursor;

    let mut cursor = Cursor::new(inner.as_slice());
    let _ctor = match cursor.read_u32::<LittleEndian>() {
        Ok(c) => c,
        Err(_) => return CheckResult::Invalid,
    };

    let resolved = match tl_gen::TlContactsResolvedPeer::deserialize(&mut cursor) {
        Ok(r) => r,
        Err(_) => return CheckResult::Invalid,
    };

    let mut peer_cursor = Cursor::new(resolved.peer.as_slice());
    let peer = match tl_gen::read_peer(&mut peer_cursor) {
        Ok(p) => p,
        Err(_) => return CheckResult::Invalid,
    };

    match peer {
        tl_gen::Peer::User(uid) => {
            for raw in &resolved.users {
                if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
                    if let tl_gen::TlUser::User {
                        id,
                        access_hash,
                        first_name,
                        last_name,
                        username: u_name,
                        phone,
                        premium,
                        bot,
                        deleted,
                        ..
                    } = user
                    {
                        if id == uid {
                            return CheckResult::User {
                                id,
                                access_hash: access_hash.unwrap_or(0),
                                username: u_name.unwrap_or_else(|| username.to_string()),
                                first_name: first_name.unwrap_or_default(),
                                last_name: last_name.unwrap_or_default(),
                                phone: phone.unwrap_or_default(),
                                premium,
                                bot,
                                deleted,
                            };
                        }
                    }
                }
            }
            CheckResult::Invalid
        }
        tl_gen::Peer::Channel(cid) => {
            for raw in &resolved.chats {
                if let Ok(chat) = tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(raw) {
                    if let tl_gen::TlChat::Channel {
                        id,
                        broadcast,
                        megagroup,
                        title,
                        username: ch_username,
                        participants_count,
                        ..
                    } = chat
                    {
                        if id == cid {
                            return CheckResult::Group {
                                id,
                                title,
                                username: ch_username.unwrap_or_else(|| username.to_string()),
                                participants_count: participants_count.unwrap_or(0) as u32,
                                is_broadcast: broadcast && !megagroup,
                            };
                        }
                    }
                }
            }
            CheckResult::Invalid
        }
        tl_gen::Peer::Chat(_) => CheckResult::PrivateGroup {
            title: "basic_chat".to_string(),
            link: format!("@{}", username),
        },
    }
}

/// Check if a private invite hash is valid (without joining).
async fn check_invite_valid(
    client: &mut MtpClient,
    hash: &str,
    max_flood_wait: u32,
    token: &AtomicBool,
) -> Result<Option<String>, String> {
    for attempt in 1..=3 {
        let req = tl::build_check_chat_invite(hash);
        match client.invoke(&req).await {
            Ok(data) => match tl::parse_chat_invite_summary(&data) {
                Ok(summary) => {
                    let title = if summary.title.is_empty() {
                        "private".to_string()
                    } else {
                        summary.title
                    };
                    return Ok(Some(title));
                }
                Err(_) => return Ok(Some("private".to_string())),
            },
            Err(e) => {
                if e.contains("INVITE_HASH_EXPIRED") || e.contains("INVITE_HASH_INVALID") {
                    return Ok(None);
                }
                if let Some(wait_secs) = parse_flood_wait(&e) {
                    if wait_secs <= max_flood_wait {
                        if !token.load(Ordering::Relaxed) {
                            return Ok(None);
                        }
                        if attempt < 3 {
                            interruptible_sleep(wait_secs as u64 * 1000, token).await;
                            continue;
                        }
                    }
                    return Err(format!("FLOOD_WAIT {}s", wait_secs));
                }
                return Err(e);
            }
        }
    }
    Err(t("link_checker_retry_limit"))
}

async fn interruptible_sleep(ms: u64, token: &AtomicBool) {
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

fn parse_flood_wait(err: &str) -> Option<u32> {
    let message = err
        .strip_prefix("RPC ")
        .and_then(|s| s.split_once(": ").map(|(_, m)| m))
        .unwrap_or(err);
    let rpc_err = tl_gen::RpcError {
        code: 0,
        message: message.to_string(),
    };
    rpc_err.flood_seconds().map(|s| s as u32)
}

fn extract_public_username(link: &str) -> Option<String> {
    let trimmed = link.trim();
    if text_parse::parse_invite_link(trimmed).is_some() {
        return None;
    }
    let username = trimmed
        .trim_start_matches("https://t.me/")
        .trim_start_matches("http://t.me/")
        .trim_start_matches("t.me/")
        .trim_start_matches('@')
        .split(['?', '#', '/'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    if username.is_empty() {
        None
    } else {
        Some(username.to_string())
    }
}

fn standardize_link(link: &str) -> String {
    let trimmed = link.trim();
    if text_parse::parse_invite_link(trimmed).is_some() {
        return trimmed.to_string();
    }
    if let Some(username) = extract_public_username(trimmed) {
        format!("@{}", username.to_lowercase())
    } else {
        trimmed.to_string()
    }
}

fn random_delay(min: u32, max: u32) -> u32 {
    if min == 0 && max == 0 {
        return 0;
    }
    let lo = min.min(max);
    let hi = min.max(max);
    if lo == hi {
        return lo;
    }
    lo + (rand::random::<u32>() % (hi - lo + 1))
}

fn resolve_output_path(user_path: &str) -> PathBuf {
    let trimmed = user_path.trim();
    if trimmed.is_empty() {
        return PathBuf::new();
    }
    // ensure .db extension
    let p = PathBuf::from(trimmed);
    if p.extension().map(|e| e == "db").unwrap_or(false) {
        p
    } else {
        p.with_extension("db")
    }
}

fn emit(app: &tauri::AppHandle, msg: String) {
    let _ = app.emit("link-checker-log", msg);
}
