// reporter: mass report peers/channels via account.reportPeer and messages.report
// Features: spintax message randomization, SQLite results DB, /start for bots

use rusqlite::{params, Connection};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex as TokioMutex;

use crate::accounts::connect::connect_account;
use crate::i18n::{t, t_with};
use crate::mtproto::tl;
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

fn reason_ctor(key: &str) -> u32 {
    match key {
        "spam" => tl_gen::INPUT_REPORT_REASON_SPAM,
        "violence" => tl_gen::INPUT_REPORT_REASON_VIOLENCE,
        "pornography" => tl_gen::INPUT_REPORT_REASON_PORNOGRAPHY,
        "child_abuse" => tl_gen::INPUT_REPORT_REASON_CHILD_ABUSE,
        "copyright" => tl_gen::INPUT_REPORT_REASON_COPYRIGHT,
        "fake" => tl_gen::INPUT_REPORT_REASON_FAKE,
        "geo_irrelevant" => tl_gen::INPUT_REPORT_REASON_GEO_IRRELEVANT,
        "illegal_drugs" => tl_gen::INPUT_REPORT_REASON_ILLEGAL_DRUGS,
        "personal_details" => tl_gen::INPUT_REPORT_REASON_PERSONAL_DETAILS,
        _ => tl_gen::INPUT_REPORT_REASON_OTHER,
    }
}

const ALL_REASONS: &[&str] = &[
    "spam",
    "violence",
    "pornography",
    "child_abuse",
    "copyright",
    "fake",
    "geo_irrelevant",
    "illegal_drugs",
    "personal_details",
    "other",
];

fn channel_report_option(key: &str) -> &'static [u8] {
    match key {
        "spam" => b"9",
        "violence" => b"3",
        "pornography" => b"5",
        "child_abuse" => b"2",
        "copyright" => b"8",
        "fake" => b"7",
        "illegal_drugs" => b"4",
        "personal_details" => b"6",
        "other" => b"a",
        _ => b"a",
    }
}

fn channel_report_suboptions(key: &str) -> &'static [&'static [u8]] {
    match key {
        "child_abuse" => &[b"3231", b"3232"],
        "violence" => &[
            b"3331", b"3332", b"3333", b"3334", b"3335", b"3336", b"3337", b"3338",
        ],
        "illegal_drugs" => &[
            b"3431", b"3432", b"3433", b"3434", b"3435", b"3436", b"3437",
        ],
        "pornography" => &[b"3536", b"3532", b"3535", b"3533", b"3537", b"3534"],
        "personal_details" => &[b"3631", b"3632", b"3633", b"3634", b"3635"],
        "fake" => &[b"3731", b"3732", b"3733", b"3734"],
        "spam" => &[b"3933", b"3931", b"3932"],
        "other" => &[b"6133", b"6131", b"6134", b"6135", b"6132"],
        _ => &[],
    }
}

#[derive(Deserialize, Clone)]
pub struct ReporterConfig {
    pub mode: String,
    pub target: String,
    #[serde(default)]
    pub targets: Vec<String>,
    pub random_reason: bool,
    pub reasons: Vec<String>,
    pub delay_min: u32,
    pub delay_max: u32,
    pub limit_per_account: u32,
    pub message_mode: String,
    pub message_single: String,
    pub message_file_paths: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub view_after_report: bool,
    #[serde(default = "default_last")]
    pub post_target: String, // "last" | "all"
    #[serde(default = "default_post_count")]
    pub post_count: u32,
    #[serde(default = "default_all")]
    pub photo_option: String, // "one" | "all"
}

fn default_true() -> bool {
    true
}
fn default_last() -> String {
    "last".to_string()
}
fn default_post_count() -> u32 {
    5
}
fn default_all() -> String {
    "all".to_string()
}

#[tauri::command]
pub async fn reporter_start(
    ids: Vec<String>,
    mut config: ReporterConfig,
    threads: Option<usize>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let concurrency = threads.unwrap_or(5).clamp(1, 100);
    // clamp to match frontend bounds and guard against malicious payloads
    config.delay_min = config.delay_min.clamp(2, 30);
    config.delay_max = config.delay_max.clamp(2, 30);
    if config.delay_min > config.delay_max {
        config.delay_min = config.delay_max;
    }
    config.limit_per_account = config.limit_per_account.clamp(1, 100);

    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "reporter".to_string(),
            t_with("reporter_task_name", &[("count", &ids.len().to_string())]),
        )
        .await;

    let mut message_files: HashMap<String, Vec<String>> = HashMap::new();
    if config.message_mode == "from_file" {
        for (reason, path) in &config.message_file_paths {
            let lines = load_lines(path);
            if !lines.is_empty() {
                message_files.insert(reason.clone(), lines);
            }
        }
    }

    let config = Arc::new(config);
    let message_files = Arc::new(message_files);

    // Initialize SQLite results DB
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kastor")
        .join("reporter");
    std::fs::create_dir_all(&data_dir).ok();
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let db_path = data_dir.join(format!("{}_reports.db", timestamp));
    let reporter_db = match init_reporter_db(&db_path) {
        Ok(c) => {
            let _ = app.emit(
                "reporter-log",
                t_with(
                    "reporter_db_path",
                    &[("path", &db_path.display().to_string())],
                ),
            );
            Some(Arc::new(TokioMutex::new(c)))
        }
        Err(e) => {
            let _ = app.emit(
                "reporter-log",
                t_with("reporter_db_create_error", &[("error", &e)]),
            );
            None
        }
    };

    tokio::spawn(async move {
        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let total = ids.len();
        let mut handles = Vec::new();

        for (i, id) in ids.into_iter().enumerate() {
            if !token.load(Ordering::Relaxed) {
                break;
            }

            let sem = sem.clone();
            let config = config.clone();
            let message_files = message_files.clone();
            let reporter_db_clone = reporter_db.clone();
            let app_clone = app.clone();
            let token_clone = token.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                if !token_clone.load(Ordering::Relaxed) {
                    return;
                }

                let result = process_reporter_account(
                    &id,
                    i + 1,
                    total,
                    &config,
                    &message_files,
                    &reporter_db_clone,
                    &app_clone,
                    &token_clone,
                )
                .await;
                if let Err(e) = result {
                    crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                    let _ = app_clone.emit(
                        "reporter-log",
                        format!("[{}/{}] {}: {}", i + 1, total, t("error"), e),
                    );
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }

        let _ = app.emit("reporter-log", t("done"));

        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn reporter_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn process_reporter_account(
    id: &str,
    idx: usize,
    total: usize,
    config: &ReporterConfig,
    message_files: &HashMap<String, Vec<String>>,
    reporter_db: &Option<Arc<TokioMutex<Connection>>>,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    let emit = |msg: String| {
        let _ = app.emit("reporter-log", msg);
    };
    let mut client = connect_account(id).await?;
    client.set_log_target("reporter-log", app.clone());

    let prefix = format!("[{}/{}] {}", idx, total, id);

    // Resolve target list
    let targets: Vec<String> = if !config.targets.is_empty() {
        config.targets.clone()
    } else {
        vec![config.target.clone()]
    };

    for target_raw in &targets {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let target = target_raw.trim().trim_start_matches('@');
        if target.is_empty() {
            continue;
        }

        match config.mode.as_str() {
            "peer" => {
                let resolve_req = tl::build_resolve_username(target);
                let resolve_data = match client.invoke(&resolve_req).await {
                    Ok(d) => d,
                    Err(e) if e.contains("USERNAME_INVALID") => {
                        return Err(t_with("reporter_username_invalid", &[("target", target)]));
                    }
                    Err(e) if e.contains("USERNAME_NOT_OCCUPIED") => {
                        return Err(t_with(
                            "reporter_username_not_exists",
                            &[("target", target)],
                        ));
                    }
                    Err(e) => return Err(format!("resolve {}: {e}", target)),
                };
                let (peer_id, access_hash) = tl::parse_resolved_peer(&resolve_data)
                    .map_err(|e| format!("parse peer: {e}"))?;

                let is_channel = resolve_data
                    .windows(4)
                    .any(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) == tl_gen::PEER_CHANNEL);

                emit(format!(
                    "{} {}",
                    prefix,
                    t_with(
                        "reporter_target_info",
                        &[
                            ("target", target),
                            ("id", &peer_id.to_string()),
                            (
                                "kind",
                                &(if is_channel {
                                    t("reporter_target_channel")
                                } else {
                                    t("reporter_target_user")
                                })
                            )
                        ]
                    )
                ));

                // If target is a bot, send /start first (like Python does)
                let is_bot = resolve_data
                    .windows(4)
                    .any(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) == tl_gen::PEER_USER)
                    && (target.ends_with("bot") || target.ends_with("Bot"));
                if is_bot && !is_channel {
                    let random_id: i64 = rand::random();
                    let start_req =
                        tl::build_send_message(peer_id, access_hash, "/start", random_id);
                    let _ = client.invoke(&start_req).await;
                    interruptible_sleep(1000, token).await;
                }

                for rep_idx in 0..config.limit_per_account {
                    if !token.load(Ordering::Relaxed) {
                        break;
                    }

                    let reason_key = pick_reason(config);
                    let message = pick_message(config, &reason_key, message_files);
                    let reason_id = reason_ctor(&reason_key);

                    let req = if is_channel {
                        build_account_report_peer_channel(peer_id, access_hash, reason_id, &message)
                    } else {
                        build_account_report_peer(peer_id, access_hash, reason_id, &message)
                    };
                    let status = match client.invoke(&req).await {
                        Ok(_) => {
                            emit(format!(
                                "{} {}",
                                prefix,
                                t_with(
                                    "reporter_report_sent",
                                    &[
                                        ("idx", &(rep_idx + 1).to_string()),
                                        ("total", &config.limit_per_account.to_string()),
                                        ("reason", &reason_key)
                                    ]
                                )
                            ));
                            "done"
                        }
                        Err(e) => {
                            if crate::mtproto::is_fatal_session_error(&e) {
                                return Err(e);
                            }
                            emit(format!(
                                "{} {}",
                                prefix,
                                t_with("reporter_error", &[("error", &e)])
                            ));
                            "error"
                        }
                    };

                    // Record to SQLite
                    if let Some(ref db_arc) = reporter_db {
                        let db = db_arc.lock().await;
                        record_report(
                            &db,
                            id,
                            &format!("@{}", target),
                            &reason_key,
                            &message,
                            status,
                        );
                    }

                    if rep_idx + 1 < config.limit_per_account {
                        let delay = random_delay(config.delay_min, config.delay_max);
                        interruptible_sleep(delay, token).await;
                    }
                }
            }
            "channel" => {
                let (channel_username, msg_id) =
                    parse_channel_link(target_raw).ok_or_else(|| t("reporter_invalid_link"))?;

                let (peer_id, access_hash) = if channel_username.starts_with("__private_") {
                    let id_str = channel_username.strip_prefix("__private_").unwrap_or("0");
                    let channel_id: i64 = id_str
                        .parse()
                        .map_err(|_| t("reporter_invalid_channel_id"))?;
                    (channel_id, 0i64)
                } else {
                    let resolve_req = tl::build_resolve_username(&channel_username);
                    let resolve_data = match client.invoke(&resolve_req).await {
                        Ok(d) => d,
                        Err(e) if e.contains("USERNAME_INVALID") => {
                            emit(format!(
                                "{} {}",
                                prefix,
                                t_with(
                                    "reporter_username_invalid",
                                    &[("target", &channel_username)]
                                )
                            ));
                            continue;
                        }
                        Err(e) => return Err(format!("resolve {}: {e}", channel_username)),
                    };
                    tl::parse_resolved_peer(&resolve_data)
                        .map_err(|e| format!("parse peer: {e}"))?
                };

                let target_label = if channel_username.starts_with("__private_") {
                    "private".to_string()
                } else {
                    format!("@{}", channel_username)
                };

                // Determine which messages to report
                let msg_ids: Vec<i32> = if msg_id > 0 {
                    // Specific post link
                    vec![msg_id]
                } else if config.post_target == "all" {
                    // Get all posts from channel
                    let req = tl::build_get_history_channel(peer_id, access_hash, 500);
                    match client.invoke(&req).await {
                        Ok(data) => tl::parse_messages_structured(&data)
                            .unwrap_or_default()
                            .iter()
                            .filter(|m| m.id > 0 && !m.is_service)
                            .map(|m| m.id)
                            .collect(),
                        Err(_) => vec![],
                    }
                } else {
                    // Last N posts
                    let limit = config.post_count.max(1).min(500) as i32;
                    let req = tl::build_get_history_channel(peer_id, access_hash, limit);
                    match client.invoke(&req).await {
                        Ok(data) => tl::parse_messages_structured(&data)
                            .unwrap_or_default()
                            .iter()
                            .filter(|m| m.id > 0 && !m.is_service)
                            .take(limit as usize)
                            .map(|m| m.id)
                            .collect(),
                        Err(_) => vec![],
                    }
                };

                if msg_ids.is_empty() {
                    emit(format!(
                        "{} {}",
                        prefix,
                        t_with("reporter_no_posts", &[("target", &target_label)])
                    ));
                    continue;
                }

                emit(format!(
                    "{} {}",
                    prefix,
                    t_with(
                        "reporter_target_posts",
                        &[
                            ("target", &target_label),
                            ("count", &msg_ids.len().to_string())
                        ]
                    )
                ));

                for (post_idx, mid) in msg_ids.iter().enumerate() {
                    if !token.load(Ordering::Relaxed) {
                        break;
                    }

                    let reason_key = pick_reason(config);
                    let message = pick_message(config, &reason_key, message_files);

                    let option = channel_report_option(&reason_key);
                    let req = build_messages_report(peer_id, access_hash, *mid, option, "");
                    let resp = client.invoke(&req).await;

                    let status = match resp {
                        Ok(_data) => {
                            let subopts = channel_report_suboptions(&reason_key);
                            if !subopts.is_empty() {
                                let sub = subopts[rand::random::<usize>() % subopts.len()];
                                let req2 = build_messages_report(
                                    peer_id,
                                    access_hash,
                                    *mid,
                                    sub,
                                    &message,
                                );
                                match client.invoke(&req2).await {
                                    Ok(_) => {
                                        emit(format!(
                                            "{} {}",
                                            prefix,
                                            t_with(
                                                "reporter_channel_report_sent",
                                                &[
                                                    ("target", &target_label),
                                                    ("msg_id", &mid.to_string()),
                                                    ("idx", &(post_idx + 1).to_string()),
                                                    ("total", &msg_ids.len().to_string()),
                                                    ("reason", &reason_key)
                                                ]
                                            )
                                        ));
                                        "done"
                                    }
                                    Err(e) => {
                                        emit(format!(
                                            "{} {}",
                                            prefix,
                                            t_with("reporter_error_sub", &[("error", &e)])
                                        ));
                                        "error"
                                    }
                                }
                            } else if reason_key == "copyright" {
                                let comment_option = b"383a63";
                                let req2 = build_messages_report(
                                    peer_id,
                                    access_hash,
                                    *mid,
                                    comment_option,
                                    &message,
                                );
                                match client.invoke(&req2).await {
                                    Ok(_) => {
                                        emit(format!(
                                            "{} {}",
                                            prefix,
                                            t_with(
                                                "reporter_channel_report_sent",
                                                &[
                                                    ("target", &target_label),
                                                    ("msg_id", &mid.to_string()),
                                                    ("idx", &(post_idx + 1).to_string()),
                                                    ("total", &msg_ids.len().to_string()),
                                                    ("reason", &reason_key)
                                                ]
                                            )
                                        ));
                                        "done"
                                    }
                                    Err(e) => {
                                        emit(format!(
                                            "{} {}",
                                            prefix,
                                            t_with("reporter_error_comment", &[("error", &e)])
                                        ));
                                        "error"
                                    }
                                }
                            } else {
                                emit(format!(
                                    "{} {}",
                                    prefix,
                                    t_with(
                                        "reporter_channel_report_sent",
                                        &[
                                            ("target", &target_label),
                                            ("msg_id", &mid.to_string()),
                                            ("idx", &(post_idx + 1).to_string()),
                                            ("total", &msg_ids.len().to_string()),
                                            ("reason", &reason_key)
                                        ]
                                    )
                                ));
                                "done"
                            }
                        }
                        Err(e) => {
                            emit(format!(
                                "{} {}",
                                prefix,
                                t_with("reporter_error", &[("error", &e)])
                            ));
                            "error"
                        }
                    };

                    if let Some(db_arc) = reporter_db {
                        let db = db_arc.lock().await;
                        record_report(
                            &db,
                            id,
                            &format!("{target_label}/{mid}"),
                            &reason_key,
                            &message,
                            status,
                        );
                    }

                    // View the post after reporting (like Python does)
                    if config.view_after_report {
                        let view_req = tl::build_get_messages_views_channel(
                            peer_id,
                            access_hash,
                            &[*mid],
                            true,
                        );
                        let _ = client.invoke(&view_req).await;
                    }

                    let delay = random_delay(config.delay_min, config.delay_max);
                    interruptible_sleep(delay, token).await;
                }
            }
            "bot" => {
                let bot_username = "SearchReportBot";
                let resolve_req = tl::build_resolve_username(bot_username);
                let resolve_data = client
                    .invoke(&resolve_req)
                    .await
                    .map_err(|e| format!("resolve {}: {e}", bot_username))?;
                let (bot_id, bot_access_hash) = tl::parse_resolved_peer(&resolve_data)
                    .map_err(|e| format!("parse bot peer: {e}"))?;

                emit(format!(
                    "{} {}",
                    prefix,
                    t_with("reporter_bot_resolved", &[("id", &bot_id.to_string())])
                ));

                // unblock bot (in case it was blocked by previous run)
                let unblock_req = tl::build_unblock_peer(bot_id, bot_access_hash);
                if let Err(e) = client.invoke(&unblock_req).await {
                    dbg_log!("разблокировка @SearchReportBot не удалась: {e}");
                }

                let mute_req = tl::build_mute_peer(bot_id, bot_access_hash);
                if let Err(e) = client.invoke(&mute_req).await {
                    dbg_log!("отключение уведомлений @SearchReportBot не удалось: {e}");
                }

                let search_query = config.target.trim().to_string();

                for rep_idx in 0..config.limit_per_account {
                    if !token.load(Ordering::Relaxed) {
                        break;
                    }

                    let random_id: i64 = rand::random();
                    let start_req =
                        tl::build_send_message(bot_id, bot_access_hash, "/start", random_id);
                    client
                        .invoke(&start_req)
                        .await
                        .map_err(|e| format!("send /start: {e}"))?;

                    interruptible_sleep(1500, token).await;
                    if !token.load(Ordering::Relaxed) {
                        break;
                    }

                    let history_req = tl::build_get_history(bot_id, bot_access_hash, 3);
                    let history_data = client
                        .invoke(&history_req)
                        .await
                        .map_err(|e| format!("get_history: {e}"))?;

                    if let Some((msg_id, callback_data)) =
                        tl::parse_first_callback_button(&history_data)
                    {
                        let cb_req = tl::build_bot_callback_answer(
                            bot_id,
                            bot_access_hash,
                            msg_id,
                            &callback_data,
                        );
                        if let Err(e) = client.invoke(&cb_req).await {
                            dbg_log!("нажатие кнопки @SearchReportBot не удалось: {e}");
                        }
                    }

                    interruptible_sleep(1000, token).await;
                    if !token.load(Ordering::Relaxed) {
                        break;
                    }

                    let random_id2: i64 = rand::random();
                    let query_req =
                        tl::build_send_message(bot_id, bot_access_hash, &search_query, random_id2);
                    client
                        .invoke(&query_req)
                        .await
                        .map_err(|e| format!("send query: {e}"))?;

                    emit(format!(
                        "{} {}",
                        prefix,
                        t_with(
                            "reporter_bot_report_sent",
                            &[
                                ("idx", &(rep_idx + 1).to_string()),
                                ("total", &config.limit_per_account.to_string()),
                                ("query", &search_query)
                            ]
                        )
                    ));

                    if rep_idx + 1 < config.limit_per_account {
                        let delay = random_delay(config.delay_min, config.delay_max);
                        interruptible_sleep(delay, token).await;
                    }
                }

                let del_req = tl::build_delete_history(bot_id, bot_access_hash);
                if let Err(e) = client.invoke(&del_req).await {
                    dbg_log!("удаление истории с @SearchReportBot не удалось: {e}");
                }
                let block_req = tl::build_block_peer(bot_id, bot_access_hash);
                if let Err(e) = client.invoke(&block_req).await {
                    dbg_log!("блокировка @SearchReportBot не удалась: {e}");
                }

                emit(format!("{} {}", prefix, t("reporter_bot_blocked")));
            }
            "photo" => {
                let resolve_req = tl::build_resolve_username(target);
                let resolve_data = match client.invoke(&resolve_req).await {
                    Ok(d) => d,
                    Err(e) => {
                        emit(format!("{} resolve @{}: {}", prefix, target, e));
                        continue;
                    }
                };
                let (peer_id, access_hash) = match tl::parse_resolved_peer(&resolve_data) {
                    Ok(p) => p,
                    Err(e) => {
                        emit(format!("{} parse @{}: {}", prefix, target, e));
                        continue;
                    }
                };

                emit(format!(
                    "{} {}",
                    prefix,
                    t_with(
                        "reporter_photo_target",
                        &[("target", target), ("id", &peer_id.to_string())]
                    )
                ));

                // Get profile photos
                let photos_req = tl_gen::build_photos_getUserPhotos(
                    &tl_gen::serialize_input_user(peer_id, access_hash),
                    0,
                    0,
                    20,
                );
                let photos_data = match client.invoke(&photos_req).await {
                    Ok(d) => d,
                    Err(e) => {
                        emit(format!(
                            "{} {}",
                            prefix,
                            t_with("reporter_photo_fetch_error", &[("error", &e)])
                        ));
                        continue;
                    }
                };
                let photos = tl::parse_user_photos(&photos_data).unwrap_or_default();

                if photos.is_empty() {
                    emit(format!(
                        "{} {}",
                        prefix,
                        t_with("reporter_no_photos", &[("target", target)])
                    ));
                    continue;
                }

                emit(format!(
                    "{} {}",
                    prefix,
                    t_with(
                        "reporter_photos_count",
                        &[("count", &photos.len().to_string())]
                    )
                ));

                for (photo_idx, (photo_id, photo_access_hash, ref file_ref)) in
                    photos.iter().enumerate()
                {
                    if !token.load(Ordering::Relaxed) {
                        break;
                    }

                    let reason_key = pick_reason(config);
                    let message = pick_message(config, &reason_key, message_files);
                    let reason_id = reason_ctor(&reason_key);

                    let peer = tl_gen::serialize_input_peer_user(peer_id, access_hash);
                    let photo_input =
                        tl_gen::serialize_inputPhoto(*photo_id, *photo_access_hash, file_ref);
                    let reason = tl_gen::serialize_bare_ctor(reason_id);
                    let req = tl_gen::build_account_reportProfilePhoto(
                        &peer,
                        &photo_input,
                        &reason,
                        &message,
                    );

                    match client.invoke(&req).await {
                        Ok(_) => emit(format!(
                            "{} {}",
                            prefix,
                            t_with(
                                "reporter_photo_report_sent",
                                &[
                                    ("idx", &(photo_idx + 1).to_string()),
                                    ("total", &photos.len().to_string()),
                                    ("reason", &reason_key)
                                ]
                            )
                        )),
                        Err(e) => {
                            if crate::mtproto::is_fatal_session_error(&e) {
                                return Err(e);
                            }
                            emit(format!(
                                "{} {}",
                                prefix,
                                t_with("reporter_photo_report_error", &[("error", &e)])
                            ));
                        }
                    }

                    if config.photo_option == "one" {
                        break;
                    }

                    let delay = random_delay(config.delay_min, config.delay_max);
                    interruptible_sleep(delay, token).await;
                }
            }
            _ => {
                return Err(format!("unknown mode: {}", config.mode));
            }
        } // match
    } // for targets

    // surface a fatal session error even if a report arm swallowed it mid-loop
    if let Some(fatal) = client.fatal_error() {
        return Err(fatal.to_string());
    }
    Ok(())
}

fn pick_reason(config: &ReporterConfig) -> String {
    if config.random_reason || config.reasons.is_empty() {
        ALL_REASONS[rand::random::<usize>() % ALL_REASONS.len()].to_string()
    } else {
        config.reasons[rand::random::<usize>() % config.reasons.len()].clone()
    }
}

fn pick_message(
    config: &ReporterConfig,
    reason_key: &str,
    files: &HashMap<String, Vec<String>>,
) -> String {
    let raw = match config.message_mode.as_str() {
        "single" => config.message_single.clone(),
        "from_file" => {
            if let Some(lines) = files.get(reason_key) {
                if !lines.is_empty() {
                    lines[rand::random::<usize>() % lines.len()].clone()
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        }
        _ => String::new(),
    };
    // Apply spintax: {вариант1|вариант2} → random choice
    spin_text(&raw)
}

fn random_delay(min_sec: u32, max_sec: u32) -> u64 {
    let min_ms = (min_sec as u64) * 1000;
    let max_ms = (max_sec as u64) * 1000;
    if max_ms <= min_ms {
        return min_ms;
    }
    min_ms + (rand::random::<u64>() % (max_ms - min_ms))
}

fn parse_channel_link(link: &str) -> Option<(String, i32)> {
    let link = link.trim();
    let path = link
        .strip_prefix("https://t.me/")
        .or_else(|| link.strip_prefix("http://t.me/"))
        .or_else(|| link.strip_prefix("t.me/"))?;
    let parts: Vec<&str> = path.split('/').collect();
    // public: t.me/channel/123
    if parts.len() >= 2 && parts[0] != "c" {
        let channel = parts[0].to_string();
        let msg_id = parts[1].parse::<i32>().ok()?;
        Some((channel, msg_id))
    }
    // private: t.me/c/CHANNEL_ID/123
    else if parts.len() >= 3 && parts[0] == "c" {
        let channel_id = parts[1].to_string();
        let msg_id = parts[2].parse::<i32>().ok()?;
        Some((format!("__private_{}", channel_id), msg_id))
    } else {
        None
    }
}

fn load_lines(path: &str) -> Vec<String> {
    if path.is_empty() {
        return Vec::new();
    }
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn build_account_report_peer(
    peer_id: i64,
    access_hash: i64,
    reason_ctor_id: u32,
    message: &str,
) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(peer_id, access_hash);
    let reason = tl_gen::serialize_bare_ctor(reason_ctor_id);
    tl_gen::build_account_reportPeer(&peer, &reason, message)
}

fn build_account_report_peer_channel(
    peer_id: i64,
    access_hash: i64,
    reason_ctor_id: u32,
    message: &str,
) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(peer_id, access_hash);
    let reason = tl_gen::serialize_bare_ctor(reason_ctor_id);
    tl_gen::build_account_reportPeer(&peer, &reason, message)
}

fn build_messages_report(
    channel_id: i64,
    access_hash: i64,
    msg_id: i32,
    option: &[u8],
    message: &str,
) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    tl_gen::build_messages_report(&peer, &[msg_id], option, message)
}

// ─── Spintax support ───────────────────────────────────────────────────────
// Syntax: {option1|option2|option3} — picks one random option from each group

fn spin_text(input: &str) -> String {
    let mut result = input.to_string();
    loop {
        // Find innermost {…} (no nested braces inside)
        let start = match result.rfind('{') {
            Some(i) => i,
            None => break,
        };
        let end = match result[start..].find('}') {
            Some(i) => start + i,
            None => break,
        };
        let inner = &result[start + 1..end];
        let options: Vec<&str> = inner.split('|').collect();
        let chosen = if options.is_empty() {
            ""
        } else {
            options[rand::random::<usize>() % options.len()]
        };
        result = format!("{}{}{}", &result[..start], chosen, &result[end + 1..]);
    }
    result
}

// ─── SQLite results database ───────────────────────────────────────────────

fn init_reporter_db(path: &PathBuf) -> Result<Connection, String> {
    let conn = Connection::open(path)
        .map_err(|e| t_with("reporter_db_open_error", &[("error", &e.to_string())]))?;
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS reports (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id TEXT DEFAULT '',
            target TEXT DEFAULT '',
            reason TEXT DEFAULT '',
            message TEXT DEFAULT '',
            status TEXT DEFAULT '',
            reported_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_reports_account ON reports(account_id);
        CREATE INDEX IF NOT EXISTS idx_reports_status ON reports(status);
    ",
    )
    .map_err(|e| t_with("reporter_db_tables_error", &[("error", &e.to_string())]))?;
    Ok(conn)
}

fn record_report(
    conn: &Connection,
    account_id: &str,
    target: &str,
    reason: &str,
    message: &str,
    status: &str,
) {
    conn.execute(
        "INSERT INTO reports (account_id, target, reason, message, status) VALUES (?1,?2,?3,?4,?5)",
        params![account_id, target, reason, message, status],
    )
    .ok();
}
