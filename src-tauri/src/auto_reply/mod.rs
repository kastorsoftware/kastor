// auto_reply: listens for incoming private messages and sends automatic replies

use serde::Deserialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};

use crate::accounts::commands::get_storage_pub;
use crate::accounts::connect::connect_account;
use crate::accounts::devices;
use crate::accounts::session::AccountJson;
use crate::i18n::{t, t_with};
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::mtproto::tl_gen;
use crate::queue::TaskQueue;

const POLL_INTERVAL_MS: u64 = 3000;
const MEDIA_CHUNK_SIZE: usize = 512 * 1024;

fn load_replied_users(path: &std::path::Path) -> HashSet<i64> {
    if let Ok(content) = std::fs::read_to_string(path) {
        if let Ok(ids) = serde_json::from_str::<Vec<i64>>(&content) {
            return ids.into_iter().collect();
        }
    }
    HashSet::new()
}

fn save_replied_users(path: &std::path::Path, replied_users: &HashSet<i64>) {
    let ids: Vec<i64> = replied_users.iter().copied().collect();
    let _ = std::fs::write(path, serde_json::to_string(&ids).unwrap_or_default());
}

async fn rate_limit() {
    let jitter = rand::random::<u64>() % 50;
    tokio::time::sleep(std::time::Duration::from_millis(100 + jitter)).await;
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DelayUnit {
    Seconds,
    Minutes,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ReplyMode {
    Infinite,
    Limit,
    Whitelist,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AutoReplyConfig {
    pub delay_min: u32,
    pub delay_max: u32,
    pub delay_unit: DelayUnit,

    pub message_type: String, // "text" | "forward" | "voice"
    pub reply_text: String,
    #[serde(default)]
    pub image_path: String,
    #[serde(default)]
    pub video_path: String,
    #[serde(default)]
    pub text_modify: String, // "none" | "llm_rewrite" | "randomize"
    pub use_voice: bool,
    pub voice_path: String,
    #[serde(default)]
    pub forward_msg_id: String,

    #[serde(default)]
    pub max_flood_wait: u64,

    pub ban_words: String,
    pub reply_mode: ReplyMode,
    pub reply_limit: u32,
    pub whitelist_path: String,

    #[serde(default)]
    pub keep_online: bool,
    #[serde(default)]
    pub silent: bool,
    #[serde(default)]
    pub no_webpage: bool,
    #[serde(default = "default_true")]
    pub mark_read: bool,

    #[serde(default)]
    pub autostop_enabled: bool,
    #[serde(default)]
    pub autostop_ban: u32,
    #[serde(default)]
    pub autostop_spamblock: u32,
    #[serde(default)]
    pub autostop_flood: u32,

    #[serde(default)]
    pub output_path: String,
}

fn default_true() -> bool {
    true
}

fn init_output_db(path: &str) -> Result<rusqlite::Connection, String> {
    let p = std::path::Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    let conn = rusqlite::Connection::open(p)
        .map_err(|e| t_with("auto_reply_db_open_error", &[("error", &e.to_string())]))?;
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS replies (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            username TEXT DEFAULT '',
            first_name TEXT DEFAULT '',
            last_name TEXT DEFAULT '',
            incoming_text TEXT DEFAULT '',
            status TEXT NOT NULL DEFAULT 'sent',
            message_type TEXT DEFAULT 'text',
            replied_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_replies_user_id ON replies(user_id);
        CREATE INDEX IF NOT EXISTS idx_replies_status ON replies(status);
    ",
    )
    .map_err(|e| t_with("auto_reply_db_tables_error", &[("error", &e.to_string())]))?;
    Ok(conn)
}

#[tauri::command]
pub async fn auto_reply_start(
    ids: Vec<String>,
    config: AutoReplyConfig,
    threads: Option<usize>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() {
        return Err(t("auto_reply_no_accounts"));
    }
    let concurrency = threads.unwrap_or(5).max(1).min(100);

    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "auto_reply".to_string(),
            t_with("auto_reply_task_name", &[("count", &ids.len().to_string())]),
        )
        .await;

    let voice_bytes: Option<Arc<Vec<u8>>> =
        if (config.message_type == "voice" || config.use_voice) && !config.voice_path.is_empty() {
            let path_lower = config.voice_path.to_lowercase();
            let needs_conversion = path_lower.ends_with(".mp3")
                || path_lower.ends_with(".wav")
                || path_lower.ends_with(".m4a")
                || path_lower.ends_with(".mp4");
            if needs_conversion {
                match crate::audio::convert_to_ogg_opus(&config.voice_path) {
                    Ok(data) => Some(Arc::new(data)),
                    Err(e) => {
                        let _ = app.emit(
                            "auto-reply-log",
                            t_with("auto_reply_audio_convert_error", &[("error", &e)]),
                        );
                        None
                    }
                }
            } else {
                match std::fs::read(&config.voice_path) {
                    Ok(data) => Some(Arc::new(data)),
                    Err(e) => {
                        let _ = app.emit(
                            "auto-reply-log",
                            t_with("auto_reply_voice_read_error", &[("error", &e.to_string())]),
                        );
                        None
                    }
                }
            }
        } else {
            None
        };

    let image_bytes: Option<Arc<Vec<u8>>> =
        if config.message_type == "text" && !config.image_path.is_empty() {
            match std::fs::read(&config.image_path) {
                Ok(data) => Some(Arc::new(data)),
                Err(e) => {
                    let _ = app.emit(
                        "auto-reply-log",
                        t_with("auto_reply_image_read_error", &[("error", &e.to_string())]),
                    );
                    None
                }
            }
        } else {
            None
        };

    let video_bytes: Option<Arc<Vec<u8>>> =
        if config.message_type == "text" && !config.video_path.is_empty() {
            match std::fs::read(&config.video_path) {
                Ok(data) => Some(Arc::new(data)),
                Err(e) => {
                    let _ = app.emit(
                        "auto-reply-log",
                        t_with("auto_reply_video_read_error", &[("error", &e.to_string())]),
                    );
                    None
                }
            }
        } else {
            None
        };

    let ban_words: Vec<String> = config
        .ban_words
        .lines()
        .map(|l| l.trim().to_lowercase())
        .filter(|l| !l.is_empty())
        .collect();

    let whitelist: HashSet<String> =
        if matches!(config.reply_mode, ReplyMode::Whitelist) && !config.whitelist_path.is_empty() {
            std::fs::read_to_string(&config.whitelist_path)
                .unwrap_or_default()
                .lines()
                .map(|l| l.trim().trim_start_matches('@').to_lowercase())
                .filter(|l| !l.is_empty())
                .collect()
        } else {
            HashSet::new()
        };

    let config = Arc::new(config);
    let ban_words = Arc::new(ban_words);
    let whitelist = Arc::new(whitelist);

    // init output SQLite DB if path specified
    let db: Option<Arc<tokio::sync::Mutex<rusqlite::Connection>>> =
        if !config.output_path.is_empty() {
            match init_output_db(&config.output_path) {
                Ok(conn) => Some(Arc::new(tokio::sync::Mutex::new(conn))),
                Err(e) => {
                    let _ = app.emit(
                        "auto-reply-log",
                        t_with("auto_reply_db_create_error", &[("error", &e)]),
                    );
                    None
                }
            }
        } else {
            None
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
            let voice_bytes = voice_bytes.clone();
            let image_bytes = image_bytes.clone();
            let video_bytes = video_bytes.clone();
            let ban_words = ban_words.clone();
            let whitelist = whitelist.clone();
            let db_clone = db.clone();
            let app_clone = app.clone();
            let token_clone = token.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                if !token_clone.load(Ordering::Relaxed) {
                    return;
                }

                let result = run_account(
                    &id,
                    i + 1,
                    total,
                    &config,
                    voice_bytes.as_deref(),
                    image_bytes.as_deref(),
                    video_bytes.as_deref(),
                    &ban_words,
                    &whitelist,
                    db_clone.as_ref(),
                    &app_clone,
                    &token_clone,
                )
                .await;
                if let Err(e) = result {
                    crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                    let _ = app_clone.emit(
                        "auto-reply-log",
                        format!("[{}/{}] {}: {}", i + 1, total, t("error"), e),
                    );
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }
        let _ = app.emit("auto-reply-log", t("done"));

        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn auto_reply_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn run_account(
    account_id: &str,
    idx: usize,
    total: usize,
    config: &AutoReplyConfig,
    voice_bytes: Option<&Vec<u8>>,
    image_bytes: Option<&Vec<u8>>,
    video_bytes: Option<&Vec<u8>>,
    ban_words: &[String],
    whitelist: &HashSet<String>,
    db: Option<&Arc<tokio::sync::Mutex<rusqlite::Connection>>>,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    let prefix = format!("[{}/{}]", idx, total);
    let emit = |msg: String| {
        let _ = app.emit("auto-reply-log", format!("{} {}", prefix, msg));
    };

    let storage = get_storage_pub();
    let json_path = storage.json_path(account_id);
    let replied_users_path = storage
        .session_json_dir()
        .join(format!("{}_replied.json", account_id));

    let mut client = connect_account(account_id).await?;
    client.set_log_target("auto-reply-log", app.clone());
    client.set_max_flood_wait(config.max_flood_wait);

    let json = if json_path.exists() {
        AccountJson::from_file(&json_path).unwrap_or_default()
    } else {
        AccountJson::default()
    };
    let dev = if !json.device.is_empty() && !json.sdk.is_empty() {
        devices::DeviceInfo {
            device: json.device.clone(),
            sdk: json.sdk.clone(),
            app_version: json.app_version.clone(),
        }
    } else {
        devices::generate_random_device()
    };
    let app_id = if json.app_id == 0 {
        crate::get_app_config().app_id
    } else {
        json.app_id
    };
    let get_me =
        tl::build_get_me_request(app_id, &dev.device, &dev.sdk, &dev.app_version, "en", "en");
    let me_resp = client
        .invoke(&get_me)
        .await
        .map_err(|e| format!("get_me: {e}"))?;
    let my_user_id = tl::parse_users_response(&me_resp)
        .map(|u| u.id)
        .unwrap_or(0);

    emit(t_with(
        "auto_reply_connected",
        &[("user_id", &my_user_id.to_string())],
    ));

    // get current state to start polling from now
    let state_req = tl_gen::build_updates_getState();
    let state_data = client
        .invoke(&state_req)
        .await
        .map_err(|e| format!("getState: {e}"))?;
    let state =
        tl_gen::parse_updates_getState(&state_data).map_err(|e| format!("parse state: {e}"))?;

    let mut pts = state.pts;
    let mut date = state.date;
    let mut qts = state.qts;
    let reply_count = AtomicU32::new(0);
    let mut replied_users = load_replied_users(&replied_users_path);
    let mut pts_empty_retries: u32 = 0;
    let mut last_online_ping: u64 = 0;

    // autostop counters
    let mut autostop_ban_count: u32 = 0;
    let mut autostop_spamblock_count: u32 = 0;
    let mut autostop_flood_count: u32 = 0;

    // upload media once (stays valid for the session)
    let mut uploaded_image_file: Option<Vec<u8>> = None;
    let mut image_filename = String::new();
    if let Some(img_data) = image_bytes {
        emit(t("auto_reply_uploading_image"));
        image_filename = std::path::Path::new(&config.image_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("photo.jpg")
            .to_string();
        let file_id: i64 = rand::random();
        let total_parts = ((img_data.len() + MEDIA_CHUNK_SIZE - 1) / MEDIA_CHUNK_SIZE) as i32;
        let is_big = img_data.len() >= 10 * 1024 * 1024;
        for part in 0..total_parts {
            let offset = part as usize * MEDIA_CHUNK_SIZE;
            let end = (offset + MEDIA_CHUNK_SIZE).min(img_data.len());
            let chunk = &img_data[offset..end];
            let req = if is_big {
                tl_gen::build_upload_saveBigFilePart(file_id, part, total_parts, chunk)
            } else {
                tl_gen::build_upload_saveFilePart(file_id, part, chunk)
            };
            client
                .invoke(&req)
                .await
                .map_err(|e| format!("upload image part {}: {e}", part))?;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        uploaded_image_file = Some(if is_big {
            tl_gen::serialize_inputFileBig(file_id, total_parts, &image_filename)
        } else {
            tl_gen::serialize_inputFile(file_id, total_parts, &image_filename, "")
        });
        emit(t("auto_reply_image_uploaded"));
    }

    let mut uploaded_video_file: Option<Vec<u8>> = None;
    let mut video_filename = String::new();
    if let Some(vid_data) = video_bytes {
        emit(t("auto_reply_uploading_video"));
        video_filename = std::path::Path::new(&config.video_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("video.mp4")
            .to_string();
        let file_id: i64 = rand::random();
        let total_parts = ((vid_data.len() + MEDIA_CHUNK_SIZE - 1) / MEDIA_CHUNK_SIZE) as i32;
        let is_big = vid_data.len() >= 10 * 1024 * 1024;
        for part in 0..total_parts {
            let offset = part as usize * MEDIA_CHUNK_SIZE;
            let end = (offset + MEDIA_CHUNK_SIZE).min(vid_data.len());
            let chunk = &vid_data[offset..end];
            let req = if is_big {
                tl_gen::build_upload_saveBigFilePart(file_id, part, total_parts, chunk)
            } else {
                tl_gen::build_upload_saveFilePart(file_id, part, chunk)
            };
            client
                .invoke(&req)
                .await
                .map_err(|e| format!("upload video part {}: {e}", part))?;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        uploaded_video_file = Some(if is_big {
            tl_gen::serialize_inputFileBig(file_id, total_parts, &video_filename)
        } else {
            tl_gen::serialize_inputFile(file_id, total_parts, &video_filename, "")
        });
        emit(t("auto_reply_video_uploaded"));
    }

    // send initial online status
    if config.keep_online {
        let req = tl_gen::build_account_updateStatus(false);
        let _ = client.invoke(&req).await;
    }

    emit(t("auto_reply_listening"));

    loop {
        if !token.load(Ordering::Relaxed) {
            emit(t("auto_reply_stopped"));
            break;
        }

        // check reply limit
        if matches!(config.reply_mode, ReplyMode::Limit) {
            if reply_count.load(Ordering::Relaxed) >= config.reply_limit {
                emit(t_with(
                    "auto_reply_limit_reached",
                    &[("limit", &config.reply_limit.to_string())],
                ));
                break;
            }
        }

        let jitter = rand::random::<u64>() % 1000;
        tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS + jitter)).await;

        // keep online: send updateStatus(offline=false) every 25s
        if config.keep_online {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now - last_online_ping >= 25 {
                let req = tl_gen::build_account_updateStatus(false);
                let _ = client.invoke(&req).await;
                last_online_ping = now;
            }
        }

        let diff_req = tl_gen::build_updates_getDifference(pts, None, None, date, qts, None);
        let diff_data = match client.invoke(&diff_req).await {
            Ok(d) => d,
            Err(e) => {
                if crate::mtproto::is_fatal_session_error(&e) {
                    return Err(e);
                }
                if e.contains("PERSISTENT_TIMESTAMP_EMPTY")
                    || e.contains("PERSISTENT_TIMESTAMP_INVALID")
                {
                    pts_empty_retries += 1;
                    if pts_empty_retries <= 3 {
                        // re-fetch state to get valid pts
                        if let Ok(new_state_data) =
                            client.invoke(&tl_gen::build_updates_getState()).await
                        {
                            if let Ok(new_state) = tl_gen::parse_updates_getState(&new_state_data) {
                                pts = new_state.pts;
                                date = new_state.date;
                                qts = new_state.qts;
                            }
                        }
                    }
                    // after 3 retries just keep polling silently — pts will become valid after first real message
                    continue;
                }
                emit(t_with("auto_reply_get_diff_error", &[("error", &e)]));
                continue;
            }
        };

        let diff = match tl_gen::parse_updates_getDifference(&diff_data) {
            Ok(d) => d,
            Err(e) => {
                emit(t_with("auto_reply_parse_diff_error", &[("error", &e)]));
                continue;
            }
        };

        match diff {
            tl_gen::TlUpdatesDifference::Empty { date: d, seq: _ } => {
                date = d;
            }
            tl_gen::TlUpdatesDifference::Difference {
                new_messages,
                users,
                state,
                ..
            } => {
                dbg_log!(
                    "auto_reply: getDifference returned Difference with {} messages",
                    new_messages.len()
                );
                let next_state = tl_gen::TlUpdatesState::deserialize(&mut std::io::Cursor::new(
                    state.as_slice(),
                ))
                .ok();
                let errs = process_messages(
                    &new_messages,
                    &users,
                    my_user_id,
                    config,
                    voice_bytes,
                    uploaded_image_file.as_deref(),
                    &image_filename,
                    uploaded_video_file.as_deref(),
                    &video_filename,
                    ban_words,
                    whitelist,
                    db,
                    &mut client,
                    &prefix,
                    app,
                    token,
                    &reply_count,
                    &mut replied_users,
                    &replied_users_path,
                )
                .await?;
                if let Some(state) = next_state {
                    pts = state.pts;
                    date = state.date;
                    qts = state.qts;
                }
                if config.autostop_enabled {
                    autostop_ban_count += errs.bans;
                    autostop_spamblock_count += errs.spamblocks;
                    autostop_flood_count += errs.floods;
                    if should_autostop(
                        config,
                        autostop_ban_count,
                        autostop_spamblock_count,
                        autostop_flood_count,
                    ) {
                        emit(t("inviter_autostop"));
                        break;
                    }
                }
            }
            tl_gen::TlUpdatesDifference::Slice {
                new_messages,
                users,
                intermediate_state,
                ..
            } => {
                dbg_log!(
                    "auto_reply: getDifference returned Slice with {} messages",
                    new_messages.len()
                );
                let next_state = tl_gen::TlUpdatesState::deserialize(&mut std::io::Cursor::new(
                    intermediate_state.as_slice(),
                ))
                .ok();
                let errs = process_messages(
                    &new_messages,
                    &users,
                    my_user_id,
                    config,
                    voice_bytes,
                    uploaded_image_file.as_deref(),
                    &image_filename,
                    uploaded_video_file.as_deref(),
                    &video_filename,
                    ban_words,
                    whitelist,
                    db,
                    &mut client,
                    &prefix,
                    app,
                    token,
                    &reply_count,
                    &mut replied_users,
                    &replied_users_path,
                )
                .await?;
                if let Some(state) = next_state {
                    pts = state.pts;
                    date = state.date;
                    qts = state.qts;
                }
                if config.autostop_enabled {
                    autostop_ban_count += errs.bans;
                    autostop_spamblock_count += errs.spamblocks;
                    autostop_flood_count += errs.floods;
                    if should_autostop(
                        config,
                        autostop_ban_count,
                        autostop_spamblock_count,
                        autostop_flood_count,
                    ) {
                        emit(t("inviter_autostop"));
                        break;
                    }
                }
            }
            tl_gen::TlUpdatesDifference::TooLong { pts: new_pts } => {
                emit(t("auto_reply_difference_too_long"));
                pts = new_pts;
                continue;
            }
        }
    }

    save_replied_users(&replied_users_path, &replied_users);
    Ok(())
}

struct ErrorCounts {
    bans: u32,
    spamblocks: u32,
    floods: u32,
}

fn should_autostop(config: &AutoReplyConfig, bans: u32, spamblocks: u32, floods: u32) -> bool {
    if config.autostop_ban > 0 && bans >= config.autostop_ban {
        return true;
    }
    if config.autostop_spamblock > 0 && spamblocks >= config.autostop_spamblock {
        return true;
    }
    if config.autostop_flood > 0 && floods >= config.autostop_flood {
        return true;
    }
    false
}

fn classify_error(e: &str) -> Option<&'static str> {
    if e.contains("USER_DEACTIVATED")
        || e.contains("AUTH_KEY_UNREGISTERED")
        || e.contains("SESSION_REVOKED")
    {
        Some("ban")
    } else if e.contains("PEER_FLOOD") || e.contains("PeerFlood") {
        Some("spamblock")
    } else if e.contains("FLOOD_WAIT") || e.contains("FloodWait") {
        Some("flood")
    } else {
        None
    }
}

fn apply_placeholders(text: &str, username: &str, first_name: &str, last_name: &str) -> String {
    text.replace("%USERNAME%", username)
        .replace("%FIRST_NAME%", first_name)
        .replace("%LAST_NAME%", last_name)
}

fn prepare_reply_text(
    base: &str,
    text_modify: &str,
    username: &str,
    first_name: &str,
    last_name: &str,
) -> String {
    let spun = crate::randomizer::spin_text(base);
    let with_placeholders = apply_placeholders(&spun, username, first_name, last_name);
    match text_modify {
        "llm_rewrite" => {
            crate::llm::complete(
                "Перефразируй это сообщение немного другими словами, сохрани смысл и длину. Без кавычек, без эмодзи. От себя ничего не добавляй",
                &with_placeholders,
            ).unwrap_or_else(|_| with_placeholders.clone()).trim().to_string()
        }
        "randomize" => crate::randomizer::randomize_text_internal(&with_placeholders, 60),
        _ => with_placeholders,
    }
}

async fn process_messages(
    messages: &[Vec<u8>],
    users: &[Vec<u8>],
    my_user_id: i64,
    config: &AutoReplyConfig,
    voice_bytes: Option<&Vec<u8>>,
    uploaded_image_file: Option<&[u8]>,
    _image_filename: &str,
    uploaded_video_file: Option<&[u8]>,
    video_filename: &str,
    ban_words: &[String],
    whitelist: &HashSet<String>,
    db: Option<&Arc<tokio::sync::Mutex<rusqlite::Connection>>>,
    client: &mut MtpClient,
    prefix: &str,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
    reply_count: &AtomicU32,
    replied_users: &mut HashSet<i64>,
    replied_users_path: &std::path::Path,
) -> Result<ErrorCounts, String> {
    let emit = |msg: String| {
        let _ = app.emit("auto-reply-log", format!("{} {}", prefix, msg));
    };

    let mut errs = ErrorCounts {
        bans: 0,
        spamblocks: 0,
        floods: 0,
    };

    // build user_id -> access_hash map from users vector
    let user_map = build_user_access_hash_map(users);
    let user_info_map = build_user_info_map(users);
    dbg_log!("auto_reply: user_map has {} entries", user_map.len());

    for msg_raw in messages {
        if !token.load(Ordering::Relaxed) {
            break;
        }

        if matches!(config.reply_mode, ReplyMode::Limit)
            && reply_count.load(Ordering::Relaxed) >= config.reply_limit
        {
            break;
        }

        // parse incoming message via codegen TlMessage deserializer
        let mut parsed = match parse_incoming_pm(msg_raw, my_user_id) {
            Some(m) => m,
            None => {
                dbg_log!(
                    "auto_reply: parse_incoming_pm returned None (msg {} bytes, ctor={:#010x})",
                    msg_raw.len(),
                    if msg_raw.len() >= 4 {
                        u32::from_le_bytes([msg_raw[0], msg_raw[1], msg_raw[2], msg_raw[3]])
                    } else {
                        0
                    }
                );
                continue;
            }
        };

        dbg_log!(
            "auto_reply: parsed PM from_id={} access_hash={} text='{}'",
            parsed.from_id,
            parsed.access_hash,
            &parsed.text[..parsed.text.len().min(50)]
        );

        // resolve access_hash from users vector
        if parsed.access_hash == 0 {
            if let Some(&hash) = user_map.get(&parsed.from_id) {
                parsed.access_hash = hash;
                dbg_log!(
                    "auto_reply: resolved access_hash={:#018x} for user_id={}",
                    hash,
                    parsed.from_id
                );
            }
        }

        if parsed.access_hash == 0 {
            emit(t_with(
                "auto_reply_no_access_hash",
                &[("user_id", &parsed.from_id.to_string())],
            ));
            continue;
        }

        // skip bots
        if let Some(info) = user_info_map.get(&parsed.from_id) {
            if info.is_bot {
                continue;
            }
            // fill username/names from user info
            if parsed.username.is_empty() && !info.username.is_empty() {
                parsed.username = info.username.clone();
            }
            if parsed.first_name.is_empty() && !info.first_name.is_empty() {
                parsed.first_name = info.first_name.clone();
            }
            if parsed.last_name.is_empty() && !info.last_name.is_empty() {
                parsed.last_name = info.last_name.clone();
            }
        }

        // skip if already replied to this user (dedup within session)
        if replied_users.contains(&parsed.from_id) {
            continue;
        }

        // whitelist check
        if matches!(config.reply_mode, ReplyMode::Whitelist) && !whitelist.is_empty() {
            let user_id_str = parsed.from_id.to_string();
            let username_lower = parsed.username.to_lowercase();
            if !whitelist.contains(&user_id_str) && !whitelist.contains(&username_lower) {
                emit(t_with(
                    "auto_reply_not_in_list",
                    &[("user_id", &parsed.from_id.to_string())],
                ));
                continue;
            }
        }

        // ban words check
        let msg_lower = parsed.text.to_lowercase();
        let has_ban_word = ban_words.iter().any(|w| msg_lower.contains(w));
        if has_ban_word {
            emit(t_with(
                "auto_reply_ban_word_skip",
                &[("user_id", &parsed.from_id.to_string())],
            ));
            continue;
        }

        // mark as read
        if config.mark_read {
            let input_peer = tl_gen::serialize_input_peer_user(parsed.from_id, parsed.access_hash);
            let read_req = tl_gen::build_messages_readHistory(&input_peer, parsed.msg_id);
            let _ = client.invoke(&read_req).await;
        }

        // delay
        let delay_ms = compute_delay(config);
        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }

        if !token.load(Ordering::Relaxed) {
            break;
        }

        // send reply based on message_type
        let send_result = match config.message_type.as_str() {
            "voice" => {
                if let Some(voice) = voice_bytes {
                    tl::send_voice_message(client, parsed.from_id, parsed.access_hash, voice)
                        .await
                        .map(|_| ())
                } else {
                    Err(t("auto_reply_voice_not_loaded"))
                }
            }
            "forward" => {
                let msg_id: i32 = config.forward_msg_id.parse().unwrap_or(0);
                if msg_id == 0 {
                    Err(t("auto_reply_invalid_forward_id"))
                } else {
                    let peer =
                        tl_gen::serialize_input_peer_user(parsed.from_id, parsed.access_hash);
                    let from_peer = tl_gen::serialize_input_peer_self();
                    let random_id: i64 = rand::random();
                    let req = tl_gen::build_messages_forwardMessages(
                        config.silent,
                        false,
                        false,
                        true,
                        false,
                        false,
                        false,
                        &from_peer,
                        &[msg_id],
                        &[random_id],
                        &peer,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                    client.invoke(&req).await.map(|_| ())
                }
            }
            _ => {
                // text mode (possibly with image/video)
                let text = prepare_reply_text(
                    &config.reply_text,
                    &config.text_modify,
                    &parsed.username,
                    &parsed.first_name,
                    &parsed.last_name,
                );

                if let Some(img_file) = uploaded_image_file {
                    // send photo with caption
                    let peer =
                        tl_gen::serialize_input_peer_user(parsed.from_id, parsed.access_hash);
                    let random_id: i64 = rand::random();
                    let media = tl_gen::serialize_inputMediaUploadedPhoto(
                        false, false, img_file, None, None, None,
                    );
                    let req = tl_gen::build_messages_sendMedia(
                        false,
                        config.silent,
                        false,
                        false,
                        false,
                        false,
                        false,
                        &peer,
                        None,
                        &media,
                        &text,
                        random_id,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                    client.invoke(&req).await.map(|_| ())
                } else if let Some(vid_file) = uploaded_video_file {
                    // send video with caption
                    let peer =
                        tl_gen::serialize_input_peer_user(parsed.from_id, parsed.access_hash);
                    let random_id: i64 = rand::random();
                    let video_attr = tl_gen::serialize_documentAttributeVideo(
                        false, true, false, 0.0, 0, 0, None, None, None,
                    );
                    let filename_attr = tl_gen::serialize_documentAttributeFilename(video_filename);
                    let attrs: &[&[u8]] = &[&video_attr, &filename_attr];
                    let media = tl_gen::serialize_inputMediaUploadedDocument(
                        false,
                        false,
                        false,
                        vid_file,
                        None,
                        "video/mp4",
                        attrs,
                        None,
                        None,
                        None,
                        None,
                    );
                    let req = tl_gen::build_messages_sendMedia(
                        false,
                        config.silent,
                        false,
                        false,
                        false,
                        false,
                        false,
                        &peer,
                        None,
                        &media,
                        &text,
                        random_id,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                    client.invoke(&req).await.map(|_| ())
                } else if !text.is_empty() {
                    // plain text
                    let peer =
                        tl_gen::serialize_input_peer_user(parsed.from_id, parsed.access_hash);
                    let random_id: i64 = rand::random();
                    let req = tl_gen::build_messages_sendMessage(
                        config.no_webpage,
                        config.silent,
                        false,
                        false,
                        false,
                        false,
                        false,
                        false,
                        &peer,
                        None,
                        &text,
                        random_id,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                    client.invoke(&req).await.map(|_| ())
                } else {
                    Ok(())
                }
            }
        };

        match send_result {
            Ok(_) => {
                let label = match config.message_type.as_str() {
                    "voice" => t("auto_reply_voice_label"),
                    "forward" => t("auto_reply_forward_label"),
                    _ => t("auto_reply_reply_label"),
                };
                emit(format!("{} -> user_id={}", label, parsed.from_id));
                reply_count.fetch_add(1, Ordering::Relaxed);
                replied_users.insert(parsed.from_id);
                save_replied_users(replied_users_path, replied_users);
                // write to SQLite
                if let Some(db_ref) = db {
                    let conn = db_ref.lock().await;
                    conn.execute(
                        "INSERT INTO replies (user_id, username, first_name, last_name, incoming_text, status, message_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        rusqlite::params![
                            parsed.from_id,
                            &parsed.username,
                            &parsed.first_name,
                            &parsed.last_name,
                            &parsed.text,
                            "sent",
                            config.message_type,
                        ],
                    ).ok();
                }
            }
            Err(e) => {
                if crate::mtproto::is_fatal_session_error(&e) {
                    return Err(e);
                }
                // classify error for autostop
                match classify_error(&e) {
                    Some("ban") => errs.bans += 1,
                    Some("spamblock") => errs.spamblocks += 1,
                    Some("flood") => errs.floods += 1,
                    _ => {}
                }
                emit(t_with(
                    "auto_reply_send_error",
                    &[("user_id", &parsed.from_id.to_string()), ("error", &e)],
                ));
                // write error to SQLite
                if let Some(db_ref) = db {
                    let conn = db_ref.lock().await;
                    conn.execute(
                        "INSERT INTO replies (user_id, username, first_name, last_name, incoming_text, status, message_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        rusqlite::params![
                            parsed.from_id,
                            &parsed.username,
                            &parsed.first_name,
                            &parsed.last_name,
                            &parsed.text,
                            &format!("error: {}", e),
                            config.message_type,
                        ],
                    ).ok();
                }
            }
        }
        rate_limit().await;
    }
    Ok(errs)
}

struct IncomingPm {
    from_id: i64,
    access_hash: i64,
    text: String,
    username: String,
    first_name: String,
    last_name: String,
    msg_id: i32,
}

// parse a raw TL message object looking for incoming private messages
fn parse_incoming_pm(data: &[u8], my_user_id: i64) -> Option<IncomingPm> {
    use byteorder::{LittleEndian, ReadBytesExt};
    use std::io::Cursor;

    let mut cursor = Cursor::new(data);
    let msg = match tl_gen::TlMessage::deserialize(&mut cursor) {
        Ok(m) => m,
        Err(e) => {
            dbg_log!("auto_reply: TlMessage::deserialize failed: {}", e);
            return None;
        }
    };

    match msg {
        tl_gen::TlMessage::Message {
            out,
            from_id,
            peer_id,
            message,
            id,
            ..
        } => {
            if out {
                dbg_log!("auto_reply: skipping outgoing message");
                return None;
            }

            let mut pc = Cursor::new(peer_id.as_slice());
            let peer_ctor2 = pc.read_u32::<LittleEndian>().ok()?;
            if peer_ctor2 != tl_gen::PEER_USER {
                dbg_log!(
                    "auto_reply: peer_id is not peerUser (ctor={:#x})",
                    peer_ctor2
                );
                return None;
            }
            let peer_user_id = pc.read_i64::<LittleEndian>().ok()?;

            // determine sender: from_id if present, otherwise peer_id (in PM without from_id, sender = peer)
            let from_user_id = if let Some(ref from_id_raw) = from_id {
                let mut fc = Cursor::new(from_id_raw.as_slice());
                let peer_ctor = fc.read_u32::<LittleEndian>().ok()?;
                if peer_ctor != tl_gen::PEER_USER {
                    dbg_log!(
                        "auto_reply: from_id is not peerUser (ctor={:#x})",
                        peer_ctor
                    );
                    return None;
                }
                fc.read_i64::<LittleEndian>().ok()?
            } else {
                // no from_id means sender is the peer (incoming PM optimization)
                peer_user_id
            };

            if from_user_id == my_user_id {
                dbg_log!("auto_reply: message from self");
                return None;
            }

            // in PM: if from_id is absent, peer_id is the sender (not us)
            // if from_id is present and peer_id != my_user_id, it's not addressed to us
            if from_id.is_some() && peer_user_id != my_user_id {
                dbg_log!(
                    "auto_reply: peer_id={} != my_user_id={}",
                    peer_user_id,
                    my_user_id
                );
                return None;
            }

            dbg_log!(
                "auto_reply: valid PM from user_id={} text='{}'",
                from_user_id,
                &message[..message.len().min(40)]
            );
            Some(IncomingPm {
                from_id: from_user_id,
                access_hash: 0,
                text: message,
                username: String::new(),
                first_name: String::new(),
                last_name: String::new(),
                msg_id: id,
            })
        }
        tl_gen::TlMessage::Empty { .. } => {
            dbg_log!("auto_reply: TlMessage::Empty");
            None
        }
        _ => {
            dbg_log!("auto_reply: TlMessage is service or unknown variant");
            None
        }
    }
}

// extract user_id -> access_hash from the users vector in getDifference response
fn build_user_access_hash_map(users: &[Vec<u8>]) -> std::collections::HashMap<i64, i64> {
    let mut map = std::collections::HashMap::new();
    for user_raw in users {
        if let Ok(tl_gen::TlUser::User {
            id,
            access_hash: Some(hash),
            ..
        }) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(user_raw)
        {
            map.insert(id, hash);
        }
    }
    map
}

struct UserInfo {
    username: String,
    first_name: String,
    last_name: String,
    is_bot: bool,
}

fn build_user_info_map(users: &[Vec<u8>]) -> std::collections::HashMap<i64, UserInfo> {
    let mut map = std::collections::HashMap::new();
    for user_raw in users {
        if let Ok(tl_gen::TlUser::User {
            id,
            username,
            first_name,
            last_name,
            bot,
            ..
        }) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(user_raw)
        {
            map.insert(
                id,
                UserInfo {
                    username: username.unwrap_or_default(),
                    first_name: first_name.unwrap_or_default(),
                    last_name: last_name.unwrap_or_default(),
                    is_bot: bot,
                },
            );
        }
    }
    map
}

fn compute_delay(config: &AutoReplyConfig) -> u64 {
    if config.delay_min == 0 && config.delay_max == 0 {
        return 0;
    }
    let min = config.delay_min as u64;
    let max = config.delay_max.max(config.delay_min) as u64;
    let value = if min == max {
        min
    } else {
        min + (rand::random::<u64>() % (max - min + 1))
    };
    match config.delay_unit {
        DelayUnit::Minutes => value * 60 * 1000,
        DelayUnit::Seconds => value * 1000,
    }
}
