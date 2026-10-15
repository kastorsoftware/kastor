// global_search: search telegram via contacts.search + messages.searchGlobal by keywords.
// Features:
// - SQLite database for word list with unique distribution between accounts
// - SQLite database for structured results (groups + users with full info)
// - messages.SearchGlobal in addition to contacts.Search
// - "unic" mode: each account gets unique words / "all" mode: all words to all accounts
// - kol_per_acc: limit words per account
// - Typeahead simulation (character-by-character search buildup)
// - DelayOnline (online status emulation during delays)
// - Deduplication of results by username

use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use byteorder::{LittleEndian, ReadBytesExt};
use rusqlite::{params, Connection};
use serde::Deserialize;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex as TokioMutex;

use crate::accounts::connect::connect_account;
use crate::i18n::{t, t_with};
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::mtproto::tl_gen;
use crate::queue::TaskQueue;

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

/// Emulates online activity during delay (like Python's DelayOnline)
async fn delay_online(client: &mut MtpClient, ms: u64, token: &Arc<AtomicBool>) {
    let online_req = tl_gen::build_account_updateStatus(false);
    let _ = client.invoke(&online_req).await;
    interruptible_sleep(ms, token).await;
    let offline_req = tl_gen::build_account_updateStatus(true);
    let _ = client.invoke(&offline_req).await;
}

#[derive(Deserialize, Clone, Debug)]
pub struct GlobalSearchConfig {
    pub input_path: String,
    pub output_path: String,
    pub mode: String,        // "all" | "channels" | "groups" | "users"
    pub output_type: String, // "links" | "usernames"
    pub delay_min: u32,
    pub delay_max: u32,
    pub max_flood_wait: u32,
    #[serde(default)]
    pub distribution: String, // "unic" | "all" (default: "all")
    #[serde(default)]
    pub kol_per_acc: u32, // 0 = unlimited
    #[serde(default)]
    pub typeahead: bool, // simulate character-by-character typing
    #[serde(default = "default_true")]
    pub use_search_global: bool, // also call messages.searchGlobal
    #[serde(default)]
    pub save_to_db: bool, // save structured results to SQLite
}

fn default_true() -> bool {
    true
}

// ─── SQLite for words ──────────────────────────────────────────────────────

fn init_words_db(words: &[String]) -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|e| format!("open words db: {e}"))?;
    conn.execute_batch("
        CREATE TABLE words (id INTEGER PRIMARY KEY, word TEXT NOT NULL, status TEXT DEFAULT 'pending');
        CREATE INDEX idx_words_status ON words(status);
    ").map_err(|e| format!("create words table: {e}"))?;
    {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("tx: {e}"))?;
        {
            let mut stmt = tx.prepare("INSERT INTO words (word) VALUES (?1)").unwrap();
            for w in words {
                stmt.execute(params![w]).ok();
            }
        }
        tx.commit().map_err(|e| format!("commit: {e}"))?;
    }
    Ok(conn)
}

fn get_words_unique(conn: &Connection, limit: usize) -> Vec<(i64, String)> {
    let mut stmt = conn
        .prepare("SELECT id, word FROM words WHERE status = 'pending' LIMIT ?1")
        .unwrap();
    let results: Vec<(i64, String)> = stmt
        .query_map(params![limit as u32], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    // Mark as taken
    for (id, _) in &results {
        conn.execute(
            "UPDATE words SET status = 'taken' WHERE id = ?1",
            params![id],
        )
        .ok();
    }
    results
}

fn get_words_all(conn: &Connection, limit: usize) -> Vec<(i64, String)> {
    let mut stmt = conn.prepare("SELECT id, word FROM words LIMIT ?1").unwrap();
    stmt.query_map(params![limit as u32], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
}

// ─── SQLite for results ────────────────────────────────────────────────────

fn init_results_db(path: &PathBuf) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| format!("open results db: {e}"))?;
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS groups (
            id INTEGER PRIMARY KEY,
            username TEXT DEFAULT '',
            title TEXT DEFAULT '',
            participants_count INTEGER DEFAULT 0,
            is_broadcast INTEGER DEFAULT 0,
            is_megagroup INTEGER DEFAULT 0,
            found_by TEXT DEFAULT '',
            found_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            username TEXT DEFAULT '',
            first_name TEXT DEFAULT '',
            last_name TEXT DEFAULT '',
            phone TEXT DEFAULT '',
            premium INTEGER DEFAULT 0,
            bot INTEGER DEFAULT 0,
            found_by TEXT DEFAULT '',
            found_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_groups_username ON groups(username);
        CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
    ",
    )
    .map_err(|e| format!("create results tables: {e}"))?;
    Ok(conn)
}

fn insert_group_result(
    conn: &Connection,
    id: i64,
    username: &str,
    title: &str,
    count: i32,
    broadcast: bool,
    megagroup: bool,
    word: &str,
) {
    conn.execute(
        "INSERT OR IGNORE INTO groups (id, username, title, participants_count, is_broadcast, is_megagroup, found_by) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![id, username, title, count, broadcast as i32, megagroup as i32, word],
    ).ok();
}

fn insert_user_result(
    conn: &Connection,
    id: i64,
    username: &str,
    first_name: &str,
    last_name: &str,
    premium: bool,
    bot: bool,
    word: &str,
) {
    conn.execute(
        "INSERT OR IGNORE INTO users (id, username, first_name, last_name, premium, bot, found_by) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![id, username, first_name, last_name, premium as i32, bot as i32, word],
    ).ok();
}

// ─── Main logic ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn global_search_start(
    ids: Vec<String>,
    config: GlobalSearchConfig,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() {
        return Err(t("global_search_no_accounts"));
    }
    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();
    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "global_search".to_string(),
            t("global_search_task_name"),
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
pub async fn global_search_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn run(
    account_ids: Vec<String>,
    cfg: Arc<GlobalSearchConfig>,
    app: &tauri::AppHandle,
    token: Arc<AtomicBool>,
) -> Result<(), String> {
    let input_path = cfg.input_path.trim();
    if input_path.is_empty() {
        return Err(t("global_search_no_input_file"));
    }
    let lines = std::fs::read_to_string(input_path)
        .map_err(|e| t_with("global_search_read_error", &[("error", &e.to_string())]))?;
    let mut words: Vec<String> = Vec::new();
    let mut skipped = 0u32;
    let mut seen = std::collections::HashSet::new();
    for line in lines.lines() {
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().count() < 3 {
            skipped += 1;
            continue;
        }
        if !seen.insert(trimmed.to_lowercase()) {
            continue;
        }
        words.push(trimmed);
    }
    if skipped > 0 {
        emit(
            app,
            t_with(
                "global_search_skipped_invalid",
                &[("count", &skipped.to_string())],
            ),
        );
    }
    if words.is_empty() {
        return Err(t("global_search_file_empty"));
    }

    let concurrency = account_ids.len();
    let distribution = if cfg.distribution.is_empty() {
        "all"
    } else {
        &cfg.distribution
    };
    let sg_label = if cfg.use_search_global {
        t("global_search_yes")
    } else {
        t("global_search_no")
    };
    let mode_str = t(mode_label(&cfg.mode));
    emit(
        app,
        t_with(
            "global_search_loaded",
            &[
                ("words", &words.len().to_string()),
                ("accounts", &concurrency.to_string()),
                ("mode", &mode_str),
                ("distribution", distribution),
                ("sg", &sg_label),
            ],
        ),
    );

    // Output text file
    let output_path = resolve_output_path(&cfg.output_path);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    let writer = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&output_path)
        .map_err(|e| {
            t_with(
                "global_search_open_file_error",
                &[("error", &e.to_string())],
            )
        })?;
    let writer = Arc::new(TokioMutex::new(writer));

    // Results SQLite DB (optional)
    let results_db: Option<Arc<TokioMutex<Connection>>> = if cfg.save_to_db {
        let db_path = output_path.with_extension("db");
        let conn = init_results_db(&db_path)?;
        emit(
            app,
            t_with(
                "global_search_results_db",
                &[("path", &db_path.display().to_string())],
            ),
        );
        Some(Arc::new(TokioMutex::new(conn)))
    } else {
        None
    };

    // Words DB for distribution
    let words_db = Arc::new(TokioMutex::new(
        init_words_db(&words).map_err(|e| format!("init words db: {e}"))?,
    ));

    let found_set: Arc<TokioMutex<std::collections::HashSet<String>>> =
        Arc::new(TokioMutex::new(std::collections::HashSet::new()));
    let found_count = Arc::new(AtomicU32::new(0));
    let total_words = words.len();
    let word_global_idx = Arc::new(AtomicUsize::new(0));

    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut handles = Vec::new();

    for (thread_idx, account_id) in account_ids.into_iter().enumerate() {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let sem = sem.clone();
        let token_clone = token.clone();
        let app_clone = app.clone();
        let writer_clone = writer.clone();
        let found_set_clone = found_set.clone();
        let found_count_clone = found_count.clone();
        let cfg_clone = cfg.clone();
        let words_db_clone = words_db.clone();
        let results_db_clone = results_db.clone();
        let word_global_idx_clone = word_global_idx.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            if !token_clone.load(Ordering::Relaxed) {
                return;
            }

            let mut client = match connect_account(&account_id).await {
                Ok(c) => c,
                Err(e) => {
                    let _ = app_clone.emit(
                        "global-search-log",
                        t_with(
                            "global_search_thread_connect_error",
                            &[("idx", &(thread_idx + 1).to_string()), ("error", &e)],
                        ),
                    );
                    return;
                }
            };
            client.set_max_flood_wait(cfg_clone.max_flood_wait as u64);

            // Get words for this account
            let my_words: Vec<(i64, String)> = {
                let db = words_db_clone.lock().await;
                let limit = if cfg_clone.kol_per_acc > 0 {
                    cfg_clone.kol_per_acc as usize
                } else {
                    total_words
                };
                let dist = if cfg_clone.distribution.is_empty() {
                    "all"
                } else {
                    &cfg_clone.distribution
                };
                if dist == "unic" {
                    get_words_unique(&db, limit)
                } else {
                    get_words_all(&db, limit)
                }
            };

            if my_words.is_empty() {
                let _ = app_clone.emit(
                    "global-search-log",
                    t_with(
                        "global_search_thread_no_words",
                        &[("idx", &(thread_idx + 1).to_string())],
                    ),
                );
                return;
            }

            let _ = app_clone.emit(
                "global-search-log",
                t_with(
                    "global_search_thread_words",
                    &[
                        ("idx", &(thread_idx + 1).to_string()),
                        ("count", &my_words.len().to_string()),
                    ],
                ),
            );

            for (_word_id, word) in &my_words {
                if !token_clone.load(Ordering::Relaxed) {
                    break;
                }

                let global_idx = word_global_idx_clone.fetch_add(1, Ordering::Relaxed);

                // Typeahead simulation: search with progressively longer prefixes
                if cfg_clone.typeahead && word.chars().count() > 3 {
                    let chars: Vec<char> = word.chars().collect();
                    // Send intermediate searches (don't collect results, just simulate typing)
                    for prefix_len in 3..chars.len() {
                        if !token_clone.load(Ordering::Relaxed) {
                            break;
                        }
                        let prefix: String = chars[..prefix_len].iter().collect();
                        let req = tl::build_contacts_search(&prefix, 5);
                        let _ = client.invoke(&req).await;
                        // Small delay between "keystrokes"
                        interruptible_sleep(random_delay(100, 300) as u64, &token_clone).await;
                    }
                }

                // Main search: contacts.search with full word
                let capitalized = capitalize_first(&word);
                let entries = search_with_flood_wait(
                    &mut client,
                    &capitalized,
                    cfg_clone.max_flood_wait,
                    &token_clone,
                )
                .await;

                // Also call messages.searchGlobal if enabled
                let global_entries = if cfg_clone.use_search_global {
                    search_global_with_flood_wait(
                        &mut client,
                        &capitalized,
                        cfg_clone.max_flood_wait,
                        &token_clone,
                    )
                    .await
                } else {
                    Ok(Vec::new())
                };

                // Merge results
                let mut all_results: Vec<SearchResult> = Vec::new();
                if let Ok(found) = entries {
                    all_results.extend(found);
                }
                if let Ok(found) = global_entries {
                    all_results.extend(found);
                }

                let filtered = filter_by_mode(&all_results, &cfg_clone.mode);
                let mut new_count = 0u32;

                for entry in &filtered {
                    if entry.username.is_empty() {
                        continue;
                    }
                    let mut set = found_set_clone.lock().await;
                    if set.insert(entry.username.to_lowercase()) {
                        new_count += 1;
                        let line = if cfg_clone.output_type == "links" {
                            format!("https://t.me/{}", entry.username)
                        } else {
                            entry.username.clone()
                        };
                        let mut w = writer_clone.lock().await;
                        writeln!(w, "{}", line).ok();
                        w.flush().ok();

                        // Save to results DB
                        if let Some(ref db_arc) = results_db_clone {
                            let db = db_arc.lock().await;
                            if entry.is_user {
                                insert_user_result(
                                    &db,
                                    entry.id,
                                    &entry.username,
                                    &entry.first_name,
                                    &entry.last_name,
                                    entry.premium,
                                    entry.bot,
                                    &word,
                                );
                            } else {
                                insert_group_result(
                                    &db,
                                    entry.id,
                                    &entry.username,
                                    &entry.title,
                                    entry.participants_count,
                                    entry.is_channel,
                                    entry.is_group,
                                    &word,
                                );
                            }
                        }
                    }
                }

                if new_count > 0 {
                    found_count_clone.fetch_add(new_count, Ordering::Relaxed);
                }
                let _ = app_clone.emit(
                    "global-search-log",
                    t_with(
                        "global_search_word_result",
                        &[
                            ("idx", &(global_idx + 1).to_string()),
                            ("total", &total_words.to_string()),
                            ("word", word),
                            ("found", &filtered.len().to_string()),
                            ("new", &new_count.to_string()),
                        ],
                    ),
                );

                // Delay with online emulation
                let delay = random_delay(cfg_clone.delay_min, cfg_clone.delay_max);
                if delay > 0 {
                    delay_online(&mut client, delay as u64, &token_clone).await;
                }
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }

    let total_found = found_count.load(Ordering::Relaxed);
    emit(
        app,
        t_with(
            "global_search_result",
            &[
                ("count", &total_found.to_string()),
                ("path", &output_path.display().to_string()),
            ],
        ),
    );
    Ok(())
}

// ─── Search result struct (richer than before) ─────────────────────────────

#[derive(Debug, Clone)]
struct SearchResult {
    id: i64,
    username: String,
    title: String,
    first_name: String,
    last_name: String,
    is_channel: bool,
    is_group: bool,
    is_user: bool,
    participants_count: i32,
    premium: bool,
    bot: bool,
}

// ─── Search functions ──────────────────────────────────────────────────────

async fn search_with_flood_wait(
    client: &mut MtpClient,
    query: &str,
    max_flood_wait: u32,
    token: &Arc<AtomicBool>,
) -> Result<Vec<SearchResult>, String> {
    for _attempt in 0..3 {
        let req = tl::build_contacts_search(query, 50);
        match client.invoke(&req).await {
            Ok(data) => return parse_contacts_found_rich(&data),
            Err(e) => {
                if let Some(wait_secs) = parse_flood_wait(&e) {
                    if wait_secs <= max_flood_wait {
                        interruptible_sleep(wait_secs as u64 * 1000, token).await;
                        if !token.load(Ordering::Relaxed) {
                            return Err("stopped".into());
                        }
                        continue;
                    }
                }
                return Err(e);
            }
        }
    }
    Err("search retries exhausted".into())
}

async fn search_global_with_flood_wait(
    client: &mut MtpClient,
    query: &str,
    max_flood_wait: u32,
    token: &Arc<AtomicBool>,
) -> Result<Vec<SearchResult>, String> {
    for _attempt in 0..3 {
        let req = tl::build_search_global(query);
        match client.invoke(&req).await {
            Ok(data) => return parse_search_global_rich(&data),
            Err(e) => {
                if let Some(wait_secs) = parse_flood_wait(&e) {
                    if wait_secs <= max_flood_wait {
                        interruptible_sleep(wait_secs as u64 * 1000, token).await;
                        if !token.load(Ordering::Relaxed) {
                            return Err("stopped".into());
                        }
                        continue;
                    }
                }
                return Err(e);
            }
        }
    }
    Err("searchGlobal retries exhausted".into())
}

// ─── Rich parsing ──────────────────────────────────────────────────────────

fn parse_contacts_found_rich(data: &[u8]) -> Result<Vec<SearchResult>, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let mut cursor = Cursor::new(inner.as_slice());
    let _ctor = cursor.read_u32::<LittleEndian>().map_err(|_| "read ctor")?;
    let found = tl_gen::TlContactsFound::deserialize(&mut cursor)?;
    let mut entries = Vec::new();

    for raw in &found.users {
        if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
            if let tl_gen::TlUser::User {
                id,
                username,
                first_name,
                last_name,
                premium,
                bot,
                ..
            } = user
            {
                if let Some(ref uname) = username {
                    if !uname.is_empty() {
                        entries.push(SearchResult {
                            id,
                            username: uname.clone(),
                            title: String::new(),
                            first_name: first_name.unwrap_or_default(),
                            last_name: last_name.unwrap_or_default(),
                            is_channel: false,
                            is_group: false,
                            is_user: true,
                            participants_count: 0,
                            premium,
                            bot,
                        });
                    }
                }
            }
        }
    }

    for raw in &found.chats {
        if let Ok(chat) = tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(raw) {
            match chat {
                tl_gen::TlChat::Channel {
                    id,
                    broadcast,
                    megagroup,
                    username,
                    title,
                    participants_count,
                    ..
                } => {
                    if let Some(ref uname) = username {
                        if !uname.is_empty() {
                            entries.push(SearchResult {
                                id,
                                username: uname.clone(),
                                title: title.clone(),
                                first_name: String::new(),
                                last_name: String::new(),
                                is_channel: broadcast && !megagroup,
                                is_group: megagroup,
                                is_user: false,
                                participants_count: participants_count.unwrap_or(0),
                                premium: false,
                                bot: false,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(entries)
}

fn parse_search_global_rich(data: &[u8]) -> Result<Vec<SearchResult>, String> {
    let msgs = tl_gen::parse_messages_searchGlobal(data)?;
    let (chats, users) = match &msgs {
        tl_gen::TlMessagesMessages::Messages { chats, users, .. } => (chats, users),
        tl_gen::TlMessagesMessages::Slice { chats, users, .. } => (chats, users),
        tl_gen::TlMessagesMessages::ChannelMessages { chats, users, .. } => (chats, users),
        tl_gen::TlMessagesMessages::NotModified { .. } => return Ok(Vec::new()),
    };

    let mut entries = Vec::new();

    for raw in users {
        if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
            if let tl_gen::TlUser::User {
                id,
                username,
                first_name,
                last_name,
                premium,
                bot,
                ..
            } = user
            {
                if let Some(ref uname) = username {
                    if !uname.is_empty() {
                        entries.push(SearchResult {
                            id,
                            username: uname.clone(),
                            title: String::new(),
                            first_name: first_name.unwrap_or_default(),
                            last_name: last_name.unwrap_or_default(),
                            is_channel: false,
                            is_group: false,
                            is_user: true,
                            participants_count: 0,
                            premium,
                            bot,
                        });
                    }
                }
            }
        }
    }

    for raw in chats {
        if let Ok(chat) = tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(raw) {
            match chat {
                tl_gen::TlChat::Channel {
                    id,
                    broadcast,
                    megagroup,
                    username,
                    title,
                    participants_count,
                    ..
                } => {
                    if let Some(ref uname) = username {
                        if !uname.is_empty() {
                            entries.push(SearchResult {
                                id,
                                username: uname.clone(),
                                title: title.clone(),
                                first_name: String::new(),
                                last_name: String::new(),
                                is_channel: broadcast && !megagroup,
                                is_group: megagroup,
                                is_user: false,
                                participants_count: participants_count.unwrap_or(0),
                                premium: false,
                                bot: false,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(entries)
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn filter_by_mode(entries: &[SearchResult], mode: &str) -> Vec<SearchResult> {
    entries
        .iter()
        .filter(|e| match mode {
            "channels" => e.is_channel,
            "groups" => e.is_group,
            "users" => e.is_user,
            _ => true, // "all"
        })
        .cloned()
        .collect()
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
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

fn random_delay(min: u32, max: u32) -> u32 {
    if min == 0 && max == 0 {
        return 0;
    }
    let lo = min.min(max);
    let hi = min.max(max);
    if lo == hi {
        return lo;
    }
    lo + (rand_simple() % (hi - lo + 1))
}

fn rand_simple() -> u32 {
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    (t.subsec_nanos() ^ (t.as_millis() as u32)) % 100_000
}

fn mode_label(mode: &str) -> &'static str {
    match mode {
        "channels" => "global_search_mode_channels",
        "groups" => "global_search_mode_groups",
        "users" => "global_search_mode_users",
        _ => "global_search_mode_all",
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
        .join("global_search.txt")
}

fn emit(app: &tauri::AppHandle, msg: String) {
    let _ = app.emit("global-search-log", msg);
}
