// mailing: mass message sending across multiple modes

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use serde::Deserialize;
use tauri::{Emitter, Manager};

use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::mtproto::tl_gen;
use crate::accounts::connect::connect_account;
use crate::queue::TaskQueue;
use crate::i18n::{t, t_with};

fn init_mailing_db(path: &str) -> Option<rusqlite::Connection> {
    if path.is_empty() { return None; }
    let conn = rusqlite::Connection::open(path).ok()?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS mailing_results (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER,
            username TEXT,
            status TEXT NOT NULL,
            error TEXT,
            account_id TEXT,
            created_at TEXT DEFAULT (datetime('now'))
        );"
    ).ok()?;
    Some(conn)
}

fn db_log_result(conn: &Option<std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>>, user_id: i64, username: &str, status: &str, error: &str, account_id: &str) {
    if let Some(db) = conn {
        if let Ok(db) = db.lock() {
            let _ = db.execute(
                "INSERT INTO mailing_results (user_id, username, status, error, account_id) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![user_id, username, status, error, account_id],
            );
        }
    }
}

async fn interruptible_sleep(ms: u64, token: &Arc<AtomicBool>) {
    let mut remaining = ms;
    while remaining > 0 {
        if !token.load(Ordering::Relaxed) { break; }
        let chunk = remaining.min(200);
        tokio::time::sleep(std::time::Duration::from_millis(chunk)).await;
        remaining -= chunk;
    }
}

fn rate_limit_ms() -> u64 {
    500 + (rand::random::<u64>() % 500)
}

fn parse_scheduled_delay(scheduled: &str) -> Option<u64> {
    use chrono::{Local, NaiveTime};
    let now = Local::now();
    let target_time = NaiveTime::parse_from_str(scheduled.trim(), "%H:%M").ok()?;
    let target = now.date_naive().and_time(target_time);
    let target = if target < now.naive_local() {
        target + chrono::Duration::days(1)
    } else {
        target
    };
    let delay_ms = target.signed_duration_since(now.naive_local()).num_milliseconds();
    if delay_ms < 0 { return None; }
    Some(delay_ms as u64)
}

#[derive(Deserialize, Clone)]
pub struct MailingConfig {
    pub mode: String,             // "dialogs" | "contacts" | "usernames" | "chats" | "comments" | "phones" | "stories"
    pub message_type: String,     // "text" | "postbot" | "forward" | "voice"
    pub message_text: String,
    #[serde(default)]
    pub message_image_path: String,
    #[serde(default)]
    pub message_video_path: String,
    pub text_modify: String,      // "none" | "llm_rewrite" | "randomize"
    pub postbot_hash: String,
    pub forward_msg_id: String,
    pub voice_path: String,
    pub usernames_path: String,
    pub chats_list: String,
    pub comments_target: String,
    pub max_per_account: u32,
    pub max_flood_wait: u64,
    pub silent: bool,
    pub scheduled_time: String,
    pub no_webpage: bool,
    #[serde(default)]
    pub delete_dialog: bool,
    #[serde(default)]
    pub pin_message: bool,
    #[serde(default)]
    pub kol_min: u32,             // random range: if > 0, randomize send count
    #[serde(default)]
    pub kol_max: u32,
    #[serde(default)]
    pub phones_path: String,      // file with phone numbers for "phones" mode
    #[serde(default)]
    pub story_link: String,       // story link for "stories" mode (t.me/user/s/123)
    #[serde(default)]
    pub video_note: bool,         // send voice/video as round video note
    #[serde(default)]
    pub file_ttl: u32,            // self-destruct timer in seconds (0 = disabled)
    #[serde(default)]
    pub autostop_enabled: bool,
    #[serde(default)]
    pub autostop_ban: u32,
    #[serde(default)]
    pub autostop_spamblock: u32,
    #[serde(default)]
    pub autostop_flood: u32,
    #[serde(default)]
    pub auto_repost: bool,        // send to temp group first, then forward to each user
    #[serde(default)]
    pub output_path: String,      // SQLite db path for results logging
}

#[tauri::command]
pub async fn mailing_start(
    ids: Vec<String>,
    config: MailingConfig,
    threads: Option<usize>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() { return Err(t("mailing_no_accounts")); }
    let concurrency = threads.unwrap_or(5).max(1).min(100);
    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue.register_task(
        task_id.clone(), "mailing".to_string(),
        t_with("mailing_task_name", &[("count", &ids.len().to_string())]),
    ).await;

    let usernames: Vec<String> = if config.mode == "usernames" && !config.usernames_path.is_empty() {
        std::fs::read_to_string(&config.usernames_path).unwrap_or_default()
            .lines().map(|l| l.trim().trim_start_matches('@').to_string())
            .filter(|l| !l.is_empty()).collect()
    } else { Vec::new() };

    let voice_bytes: Option<Arc<Vec<u8>>> = if config.message_type == "voice" && !config.voice_path.is_empty() {
        std::fs::read(&config.voice_path).ok().map(|d| Arc::new(d))
    } else { None };

    let config = Arc::new(config);
    let usernames = Arc::new(usernames);
    let username_idx = Arc::new(AtomicUsize::new(0));

    let db_conn: Option<Arc<std::sync::Mutex<rusqlite::Connection>>> = 
        init_mailing_db(&config.output_path).map(|c| Arc::new(std::sync::Mutex::new(c)));

    tokio::spawn(async move {
        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let total = ids.len();
        let mut handles = Vec::new();
        for (i, id) in ids.into_iter().enumerate() {
            if !token.load(Ordering::Relaxed) { break; }
            let sem = sem.clone(); let config = config.clone();
            let usernames = usernames.clone(); let username_idx = username_idx.clone();
            let voice_bytes = voice_bytes.clone();
            let app_clone = app.clone(); let token_clone = token.clone();
            let db_conn_clone = db_conn.clone();
            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                if !token_clone.load(Ordering::Relaxed) { return; }
                let result = process_account(
                    &id, i+1, total, &config, &usernames, &username_idx,
                    voice_bytes.as_deref(), &app_clone, &token_clone, &db_conn_clone,
                ).await;
                if let Err(e) = result {
                    crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                    let _ = app_clone.emit("mailing-log", format!("[{}/{}] {}: {}", i+1, total, t("error"), e));
                }
            }));
        }
        for h in handles { let _ = h.await; }
        let _ = app.emit("mailing-log", t("done"));
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });
    Ok(tid)
}

#[tauri::command]
pub async fn mailing_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn process_account(
    id: &str, idx: usize, total: usize,
    config: &MailingConfig, usernames: &[String], username_idx: &AtomicUsize,
    voice_bytes: Option<&Vec<u8>>, app: &tauri::AppHandle, token: &Arc<AtomicBool>,
    db_conn: &Option<Arc<std::sync::Mutex<rusqlite::Connection>>>,
) -> Result<(), String> {
    let prefix = format!("[{}/{}]", idx, total);
    let emit = |msg: &str| { let _ = app.emit("mailing-log", format!("{} {}", prefix, msg)); };

    let mut client = connect_account(id).await?;
    client.set_log_target("mailing-log", app.clone());
    client.set_log_prefix(&prefix);
    client.set_max_flood_wait(config.max_flood_wait);

    // Pre-check spamblock from json — skip account if spamblocked
    {
        let storage = crate::accounts::commands::get_storage_pub();
        let json_path = storage.json_path(id);
        if json_path.exists() {
            if let Ok(json) = crate::accounts::session::AccountJson::from_file(&json_path) {
                let sb = &json.spamblock;
                if !sb.is_empty() && sb != "none" && sb != "Без ограничений" && sb != "No restrictions" {
                    emit(&t_with("mailing_skip_spamblock", &[("status", sb)]));
                    return Ok(());
                }
            }
        }
    }

    if !config.scheduled_time.is_empty() {
        if let Some(delay_ms) = parse_scheduled_delay(&config.scheduled_time) {
            emit(&t_with("mailing_waiting", &[("time", &config.scheduled_time), ("ms", &delay_ms.to_string())]));
            interruptible_sleep(delay_ms, token).await;
            if !token.load(Ordering::Relaxed) { return Ok(()); }
        }
    }

    let mut sent = 0u32;
    let mut ban_count = 0u32;
    let mut spamblock_count = 0u32;
    let mut flood_count = 0u32;
    let effective_limit = if config.kol_min > 0 && config.kol_max > 0 {
        let lo = config.kol_min.min(config.kol_max);
        let hi = config.kol_min.max(config.kol_max);
        if lo == hi { lo } else { lo + rand::random::<u32>() % (hi - lo + 1) }
    } else {
        config.max_per_account
    };

    // auto-repost: create temp group, send message there, get msg_id for forwarding
    let repost_info: Option<(i64, i32)> = if config.auto_repost && config.message_type == "text" {
        let create_req = tl::build_create_temp_chat("temp_mailing");
        match client.invoke(&create_req).await {
            Ok(data) => {
                if let Some(chat_id) = tl::parse_created_chat_id(&data) {
                    emit(&t("mailing_repost_group_created"));
                    // send message to the temp chat
                    let text = prepare_text(&config.message_text, &config.text_modify);
                    let peer = tl_gen::serialize_input_peer_chat(chat_id);
                    let random_id: i64 = rand::random();
                    let send_req = tl_gen::build_messages_sendMessage(
                        config.no_webpage, false, false, false, false, false, false, false,
                        &peer, None, &text, random_id, None, None, None, None, None, None, None, None, None, None,
                    );
                    match client.invoke(&send_req).await {
                        Ok(resp) => {
                            if let Some(msg_id) = tl::extract_sent_msg_id(&resp) {
                                emit(&t("mailing_repost_msg_sent"));
                                Some((chat_id, msg_id))
                            } else {
                                emit(&t("mailing_repost_no_msg_id"));
                                None
                            }
                        }
                        Err(e) => { emit(&t_with("mailing_repost_send_error", &[("error", &e)])); None }
                    }
                } else {
                    emit(&t("mailing_repost_no_chat_id"));
                    None
                }
            }
            Err(e) => { emit(&t_with("mailing_repost_create_error", &[("error", &e)])); None }
        }
    } else { None };

    match config.mode.as_str() {
        "dialogs" => {
            let req = tl::build_get_dialogs_with_folder(0, 500);
            let data = client.invoke(&req).await.map_err(|e| format!("getDialogs: {e}"))?;
            let peers = tl::parse_dialog_peers(&data).unwrap_or_default();
            for peer in peers {
                if !token.load(Ordering::Relaxed) || sent >= effective_limit { break; }
                if let tl::DialogPeer::User { id: uid, access_hash, is_bot } = peer {
                    if is_bot { continue; }
                    match send_to_user(&mut client, uid, access_hash, config, voice_bytes, app, &prefix, token, "", repost_info).await {
                        SendResult::Ok => { sent += 1; db_log_result(db_conn, uid, "", "ok", "", id); }
                        SendResult::Fatal(e) => { db_log_result(db_conn, uid, "", "fatal", &e, id); return Err(e); }
                        SendResult::ErrorBan => { ban_count += 1; db_log_result(db_conn, uid, "", "ban", "", id); }
                        SendResult::ErrorSpamblock => { spamblock_count += 1; db_log_result(db_conn, uid, "", "spamblock", "", id); }
                        SendResult::ErrorFlood => { flood_count += 1; db_log_result(db_conn, uid, "", "flood", "", id); }
                        _ => { db_log_result(db_conn, uid, "", "error", "", id); }
                    }
                    if should_autostop_mailing(config, ban_count, spamblock_count, flood_count) { break; }
                    interruptible_sleep(rate_limit_ms(), token).await;
                }
            }
        }
        "contacts" => {
            let req = tl::build_contacts_get_contacts();
            let data = client.invoke(&req).await.map_err(|e| format!("getContacts: {e}"))?;
            let contacts = tl::parse_contacts_response(&data).unwrap_or_default();
            for (uid, access_hash) in contacts {
                if !token.load(Ordering::Relaxed) || sent >= effective_limit { break; }
                match send_to_user(&mut client, uid, access_hash, config, voice_bytes, app, &prefix, token, "", repost_info).await {
                    SendResult::Ok => { sent += 1; db_log_result(db_conn, uid, "", "ok", "", id); }
                    SendResult::Fatal(e) => { db_log_result(db_conn, uid, "", "fatal", &e, id); return Err(e); }
                    SendResult::ErrorBan => { ban_count += 1; db_log_result(db_conn, uid, "", "ban", "", id); }
                    SendResult::ErrorSpamblock => { spamblock_count += 1; db_log_result(db_conn, uid, "", "spamblock", "", id); }
                    SendResult::ErrorFlood => { flood_count += 1; db_log_result(db_conn, uid, "", "flood", "", id); }
                    _ => { db_log_result(db_conn, uid, "", "error", "", id); }
                }
                if should_autostop_mailing(config, ban_count, spamblock_count, flood_count) { break; }
                interruptible_sleep(rate_limit_ms(), token).await;
            }
        }
        "usernames" => {
            for _ in 0..effective_limit {
                if !token.load(Ordering::Relaxed) { break; }
                let i = username_idx.fetch_add(1, Ordering::Relaxed);
                if i >= usernames.len() { break; }
                let resolve_req = tl::build_resolve_username(&usernames[i]);
                if let Ok(data) = client.invoke(&resolve_req).await {
                    if let Ok((uid, hash)) = tl::parse_resolved_peer(&data) {
                        match send_to_user(&mut client, uid, hash, config, voice_bytes, app, &prefix, token, &usernames[i], repost_info).await {
                            SendResult::Ok => { sent += 1; db_log_result(db_conn, uid, &usernames[i], "ok", "", id); }
                            SendResult::Fatal(e) => { db_log_result(db_conn, uid, &usernames[i], "fatal", &e, id); return Err(e); }
                            SendResult::ErrorBan => { ban_count += 1; db_log_result(db_conn, uid, &usernames[i], "ban", "", id); }
                            SendResult::ErrorSpamblock => { spamblock_count += 1; db_log_result(db_conn, uid, &usernames[i], "spamblock", "", id); }
                            SendResult::ErrorFlood => { flood_count += 1; db_log_result(db_conn, uid, &usernames[i], "flood", "", id); }
                            _ => { db_log_result(db_conn, uid, &usernames[i], "error", "", id); }
                        }
                        if should_autostop_mailing(config, ban_count, spamblock_count, flood_count) { break; }
                    }
                }
                interruptible_sleep(rate_limit_ms(), token).await;
            }
        }
        "chats" => {
            let targets: Vec<&str> = config.chats_list.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
            for target in targets {
                if !token.load(Ordering::Relaxed) || sent >= effective_limit { break; }
                match crate::mtproto::invite::resolve_channel_link(&mut client, target).await {
                    Ok(resolved) => {
                        let peer = tl_gen::serialize_input_peer_channel(resolved.channel_id, resolved.access_hash);
                        let random_id: i64 = rand::random();
                        let ok = match config.message_type.as_str() {
                            "forward" => {
                                let msg_id: i32 = config.forward_msg_id.parse().unwrap_or(0);
                                if msg_id == 0 { false } else {
                                    let from_peer = tl_gen::serialize_input_peer_self();
                                    let req = tl_gen::build_messages_forwardMessages(
                                        config.silent, false, false, true, false, false, false,
                                        &from_peer, &[msg_id], &[random_id], &peer, None, None, None, None, None, None, None, None, None, None,
                                    );
                                    client.invoke(&req).await.is_ok()
                                }
                            }
                            "voice" => {
                                if let Some(vb) = voice_bytes {
                                    crate::mtproto::tl::send_voice_message(&mut client, resolved.channel_id, resolved.access_hash, vb).await.is_ok()
                                } else { false }
                            }
                            "postbot" => {
                                let req = tl_gen::build_messages_sendMessage(
                                    config.no_webpage, config.silent, false, false, false, false, false, false,
                                    &peer, None, &format!("@PostBot {}", config.postbot_hash), random_id, None, None, None, None, None, None, None, None, None, None,
                                );
                                client.invoke(&req).await.is_ok()
                            }
                            _ => {
                                let text = prepare_text(&config.message_text, &config.text_modify);
                                if !config.message_image_path.is_empty() {
                                    send_media_to_user(&mut client, resolved.channel_id, resolved.access_hash, &text, &config.message_image_path, "photo", config.silent, false, config.file_ttl, token).await.is_ok()
                                } else if !config.message_video_path.is_empty() {
                                    send_media_to_user(&mut client, resolved.channel_id, resolved.access_hash, &text, &config.message_video_path, "video", config.silent, config.video_note, config.file_ttl, token).await.is_ok()
                                } else {
                                    let req = tl_gen::build_messages_sendMessage(
                                        config.no_webpage, config.silent, false, false, false, false, false, false,
                                        &peer, None, &text, random_id, None, None, None, None, None, None, None, None, None, None,
                                    );
                                    client.invoke(&req).await.is_ok()
                                }
                            }
                        };
                        if ok { emit(&t_with("mailing_sent_to_chat", &[("target", target)])); sent += 1; }
                        else { emit(&t_with("mailing_send_error_chat", &[("target", target)])); }
                        interruptible_sleep(rate_limit_ms(), token).await;
                    }
                    Err(e) => emit(&format!("resolve {}: {}", target, e)),
                }
            }
        }
        "comments" => {
            let targets: Vec<&str> = config.comments_target.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
            for target in targets {
                if !token.load(Ordering::Relaxed) || sent >= effective_limit { break; }
                match crate::mtproto::invite::resolve_channel_link(&mut client, target).await {
                    Ok(resolved) => {
                        // get last post
                        let hist_req = tl::build_get_history_channel(resolved.channel_id, resolved.access_hash, 1);
                        let hist_data = match client.invoke(&hist_req).await {
                            Ok(d) => d, Err(e) => { emit(&format!("getHistory {}: {}", target, e)); continue; }
                        };
                        let msgs = tl::parse_messages_structured(&hist_data).unwrap_or_default();
                        let msg_id = match msgs.first() { Some(m) => m.id, None => { emit(&t_with("mailing_no_posts_in", &[("target", target)])); continue; } };
                        let text = prepare_text(&config.message_text, &config.text_modify);
                        match crate::first_comment::send_comment_pub(&mut client, resolved.channel_id, resolved.access_hash, msg_id, &text).await {
                            Ok(_) => { emit(&t_with("mailing_comment_sent", &[("target", target), ("msg_id", &msg_id.to_string())])); sent += 1; }
                            Err(e) => emit(&t_with("mailing_comment_error", &[("target", target), ("error", &e)])),
                        }
                        interruptible_sleep(rate_limit_ms(), token).await;
                    }
                    Err(e) => emit(&format!("resolve {}: {}", target, e)),
                }
            }
        }
        "phones" => {
            let phones: Vec<String> = if !config.phones_path.is_empty() {
                std::fs::read_to_string(&config.phones_path).unwrap_or_default()
                    .lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
            } else { Vec::new() };
            if phones.is_empty() {
                emit(&t("mailing_no_phones"));
            } else {
                for phone in &phones {
                    if !token.load(Ordering::Relaxed) || sent >= effective_limit { break; }
                    // import contact to resolve user
                    let import_req = tl::import_phone_contact(phone);
                    match client.invoke(&import_req).await {
                        Ok(data) => {
                            if let Some((uid, hash)) = tl::parse_imported_contact(&data) {
                                match send_to_user(&mut client, uid, hash, config, voice_bytes, app, &prefix, token, phone, repost_info).await {
                                    SendResult::Ok => { sent += 1; db_log_result(db_conn, uid, phone, "ok", "", id); }
                                    SendResult::Fatal(e) => { db_log_result(db_conn, uid, phone, "fatal", &e, id); return Err(e); }
                                    SendResult::ErrorBan => { ban_count += 1; db_log_result(db_conn, uid, phone, "ban", "", id); }
                                    SendResult::ErrorSpamblock => { spamblock_count += 1; db_log_result(db_conn, uid, phone, "spamblock", "", id); }
                                    SendResult::ErrorFlood => { flood_count += 1; db_log_result(db_conn, uid, phone, "flood", "", id); }
                                    _ => { db_log_result(db_conn, uid, phone, "error", "", id); }
                                }
                                if should_autostop_mailing(config, ban_count, spamblock_count, flood_count) { break; }
                                // delete imported contact
                                let del_req = tl::build_contacts_delete_contacts(&[(uid, hash)]);
                                let _ = client.invoke(&del_req).await;
                            } else {
                                emit(&t_with("mailing_phone_resolve_error", &[("phone", phone.as_str())]));
                            }
                        }
                        Err(e) => emit(&format!("importContacts {}: {}", phone, e)),
                    }
                    interruptible_sleep(rate_limit_ms(), token).await;
                }
            }
        }
        "stories" => {
            // forward/share a story to users from usernames list
            let story_link = config.story_link.trim();
            if story_link.is_empty() {
                emit(&t("mailing_no_story_link"));
            } else {
                let (story_user, story_id) = parse_story_link(story_link);
                if story_id == 0 {
                    emit(&t_with("mailing_story_parse_error", &[("link", story_link)]));
                } else {
                    // resolve story owner
                    let resolve_req = tl::build_resolve_username(&story_user);
                    match client.invoke(&resolve_req).await {
                        Ok(data) => {
                            if tl::parse_resolved_peer(&data).is_ok() {
                                for _ in 0..effective_limit {
                                    if !token.load(Ordering::Relaxed) { break; }
                                    let i = username_idx.fetch_add(1, Ordering::Relaxed);
                                    if i >= usernames.len() { break; }
                                    let resolve_req2 = tl::build_resolve_username(&usernames[i]);
                                    if let Ok(data2) = client.invoke(&resolve_req2).await {
                                        if let Ok((uid, hash)) = tl::parse_resolved_peer(&data2) {
                                            let peer = tl_gen::serialize_input_peer_user(uid, hash);
                                            let random_id: i64 = rand::random();
                                            // Story IDs are not message IDs and cannot be passed to
                                            // messages.forwardMessages. Send the canonical story link.
                                            let req = tl_gen::build_messages_sendMessage(
                                                true, config.silent, false, false, false, false, false, false,
                                                &peer, None, story_link, random_id, None, None,
                                                None, None, None, None, None, None, None, None,
                                            );
                                            match client.invoke(&req).await {
                                                Ok(_) => { emit(&t_with("mailing_story_forwarded", &[("username", usernames[i].as_str())])); sent += 1; }
                                                Err(e) => emit(&t_with("mailing_story_forward_error", &[("username", usernames[i].as_str()), ("error", &e)])),
                                            }
                                        }
                                    }
                                    interruptible_sleep(rate_limit_ms(), token).await;
                                }
                            } else {
                                emit(&t_with("mailing_story_owner_resolve_error", &[("username", story_user.as_str())]));
                            }
                        }
                        Err(e) => emit(&format!("resolve {}: {}", story_user, e)),
                    }
                }
            }
        }
        _ => {}
    }

    // post-send actions: delete dialog / pin
    if config.delete_dialog && sent > 0 {
        // deletion is handled per-message in send_to_user for DM modes
    }

    // cleanup temp chat if auto-repost was used
    if let Some((chat_id, _)) = repost_info {
        let del_req = tl::build_delete_chat(chat_id);
        let _ = client.invoke(&del_req).await;
        emit(&t("mailing_repost_group_deleted"));
    }

    emit(&t_with("mailing_total_sent", &[("count", &sent.to_string())]));
    // surface a fatal session error even if a send arm swallowed it mid-loop
    if let Some(fatal) = client.fatal_error() {
        return Err(fatal.to_string());
    }
    Ok(())
}

enum SendResult {
    Ok,
    ErrorBan,
    ErrorSpamblock,
    ErrorFlood,
    ErrorOther,
    Fatal(String),
}

async fn send_to_user(
    client: &mut MtpClient, user_id: i64, access_hash: i64,
    config: &MailingConfig, voice_bytes: Option<&Vec<u8>>,
    app: &tauri::AppHandle, prefix: &str,
    token: &Arc<AtomicBool>,
    recipient_username: &str,
    repost_info: Option<(i64, i32)>,
) -> SendResult {
    let mut text = prepare_text(&config.message_text, &config.text_modify);
    // apply placeholders — resolve user info if needed
    if text.contains('%') {
        let (uname, fname, lname) = if !recipient_username.is_empty() && !text.contains("%FIRST_NAME%") && !text.contains("%LAST_NAME%") {
            (recipient_username.to_string(), String::new(), String::new())
        } else {
            // fetch user info via users.getUsers
            let req = tl::build_get_user_info(user_id, access_hash);
            match client.invoke(&req).await {
                Ok(data) => tl::parse_user_info(&data).unwrap_or_else(|| (recipient_username.to_string(), String::new(), String::new())),
                Err(_) => (recipient_username.to_string(), String::new(), String::new()),
            }
        };
        text = text.replace("%USERNAME%", &uname);
        text = text.replace("%FIRST_NAME%", &fname);
        text = text.replace("%LAST_NAME%", &lname);
    }
    let random_id: i64 = rand::random();

    // if auto-repost is active, forward from temp chat instead of sending directly
    let result = if let Some((chat_id, msg_id)) = repost_info {
        let from_peer = tl_gen::serialize_input_peer_chat(chat_id);
        let to_peer = tl_gen::serialize_input_peer_user(user_id, access_hash);
        let req = tl_gen::build_messages_forwardMessages(
            config.silent, false, false, true, false, false, false,
            &from_peer, &[msg_id], &[random_id], &to_peer, None, None, None, None, None, None, None, None, None, None,
        );
        client.invoke(&req).await
    } else {
    match config.message_type.as_str() {
        "postbot" => {
            // Resolve @PostBot and send as inline query
            let resolve_req = tl::build_resolve_username("PostBot");
            match client.invoke(&resolve_req).await {
                Ok(resolved) => {
                    match tl::parse_resolved_peer(&resolved) {
                        Ok((bot_id, bot_hash)) => {
                            let bot_peer = tl_gen::serialize_input_user(bot_id, bot_hash);
                            let user_peer = tl_gen::serialize_input_peer_user(user_id, access_hash);
                            let get_req = tl_gen::build_messages_getInlineBotResults(&bot_peer, &user_peer, None, &config.postbot_hash, "");
                            match client.invoke(&get_req).await {
                                Ok(results_data) => {
                                    match tl_gen::parse_messages_getInlineBotResults(&results_data) {
                                        Ok(bot_results) => {
                                            if bot_results.results.is_empty() {
                                                Err("inline query returned no results".into())
                                            } else {
                                                // Parse first result to get its id
                                                let first = &bot_results.results[0];
                                                let mut cursor = std::io::Cursor::new(first.as_slice());
                                                match tl_gen::TlBotInlineResult::deserialize(&mut cursor) {
                                                    Ok(result) => {
                                                        let result_id = match &result {
                                                            tl_gen::TlBotInlineResult::Result { id, .. } => id.clone(),
                                                            tl_gen::TlBotInlineResult::MediaResult { id, .. } => id.clone(),
                                                        };
                                                        let send_req = tl_gen::build_messages_sendInlineBotResult(
                                                            config.silent, false, false, false,
                                                            &user_peer, None, random_id,
                                                            bot_results.query_id, &result_id,
                                                            None, None, None, None,
                                                        );
                                                        client.invoke(&send_req).await
                                                    }
                                                    Err(e) => Err(format!("parse inline result: {e}"))
                                                }
                                            }
                                        }
                                        Err(e) => Err(format!("parse bot results: {e}"))
                                    }
                                }
                                Err(e) => Err(e)
                            }
                        }
                        Err(e) => Err(format!("resolve PostBot: {e}"))
                    }
                }
                Err(e) => Err(e)
            }
        }
        "forward" => {
            let msg_id: i32 = config.forward_msg_id.parse().unwrap_or(0);
            if msg_id == 0 { return SendResult::ErrorOther; }
            let peer = tl_gen::serialize_input_peer_user(user_id, access_hash);
            let from_peer = tl_gen::serialize_input_peer_self();
            let req = tl_gen::build_messages_forwardMessages(
                config.silent, false, false, true, false, false, false,
                &from_peer, &[msg_id], &[random_id], &peer, None, None, None, None, None, None, None, None, None, None,
            );
            client.invoke(&req).await
        }
        "voice" => {
            if let Some(vb) = voice_bytes {
                match crate::mtproto::tl::send_voice_message(client, user_id, access_hash, vb).await {
                    Ok(_) => Ok(Vec::new()),
                    Err(e) => Err(e),
                }
            } else { return SendResult::ErrorOther; }
        }
        _ => {
            // text mode: if image or video is attached, send as media; otherwise plain text
            if !config.message_image_path.is_empty() {
                send_media_to_user(client, user_id, access_hash, &text, &config.message_image_path, "photo", config.silent, false, config.file_ttl, token).await
            } else if !config.message_video_path.is_empty() {
                send_media_to_user(client, user_id, access_hash, &text, &config.message_video_path, "video", config.silent, config.video_note, config.file_ttl, token).await
            } else {
                // Parse markdown and send with entities
                let (plain_text, entities) = tl::parse_markdown_v2(&text);
                let req = tl::build_send_message_with_entities(user_id, access_hash, false, &plain_text, &entities, random_id);
                client.invoke(&req).await
            }
        }
    } // end else (non-repost path)
    }; // end let result

    match result {
        Ok(resp) => {
            let _ = app.emit("mailing-log", format!("{} {}", prefix, t_with("mailing_sent_user", &[("user_id", &user_id.to_string())])));
            // pin message if configured
            if config.pin_message {
                if let Some(msg_id) = tl::extract_sent_msg_id(&resp) {
                    let pin_req = tl::build_pin_message(user_id, access_hash, msg_id, config.silent);
                    let _ = client.invoke(&pin_req).await;
                }
            }
            // delete dialog after sending if configured
            if config.delete_dialog {
                let del_req = tl::build_delete_history(user_id, access_hash);
                let _ = client.invoke(&del_req).await;
            }
            SendResult::Ok
        }
        Err(e) => {
            if crate::mtproto::is_fatal_session_error(&e) {
                let _ = app.emit("mailing-log", format!("{} {} user_id={}: {}", prefix, t("error"), user_id, e));
                return SendResult::Fatal(e);
            }
            let _ = app.emit("mailing-log", format!("{} {}", prefix, t_with("mailing_error_user", &[("user_id", &user_id.to_string()), ("error", &e)])));
            if e.contains("USER_DEACTIVATED") || e.contains("AUTH_KEY_UNREGISTERED") || e.contains("SESSION_REVOKED") {
                SendResult::ErrorBan
            } else if e.contains("PEER_FLOOD") || e.contains("PeerFlood") {
                SendResult::ErrorSpamblock
            } else if e.contains("FLOOD_WAIT") || e.contains("FloodWait") {
                SendResult::ErrorFlood
            } else {
                SendResult::ErrorOther
            }
        }
    }
}

fn prepare_text(base: &str, mode: &str) -> String {
    // apply spintax first
    let spun = crate::randomizer::spin_text(base);
    match mode {
        "llm_rewrite" => {
            crate::llm::complete(
                &crate::i18n::t("llm_rephrase_prompt"),
                &spun,
            ).unwrap_or_else(|_| spun.clone()).trim().to_string()
        }
        "randomize" => crate::randomizer::randomize_text_internal(&spun, 60),
        _ => spun,
    }
}

const CHUNK_SIZE: usize = 512 * 1024;

// upload file and send as photo or video media to a user peer
async fn send_media_to_user(
    client: &mut MtpClient,
    user_id: i64,
    access_hash: i64,
    caption: &str,
    file_path: &str,
    media_type: &str, // "photo" | "video"
    silent: bool,
    video_note: bool,
    ttl: u32,
    token: &Arc<AtomicBool>,
) -> Result<Vec<u8>, String> {
    let data = std::fs::read(file_path).map_err(|e| format!("read media: {e}"))?;
    let file_id: i64 = rand::random();
    let total_parts = ((data.len() + CHUNK_SIZE - 1) / CHUNK_SIZE) as i32;
    let is_big = data.len() >= 10 * 1024 * 1024;

    for part in 0..total_parts {
        if !token.load(Ordering::Relaxed) { return Ok(Vec::new()); }
        let offset = part as usize * CHUNK_SIZE;
        let end = (offset + CHUNK_SIZE).min(data.len());
        let chunk = &data[offset..end];
        let req = if is_big {
            tl_gen::build_upload_saveBigFilePart(file_id, part, total_parts, chunk)
        } else {
            tl_gen::build_upload_saveFilePart(file_id, part, chunk)
        };
        client.invoke(&req).await.map_err(|e| format!("upload part {}: {e}", part))?;
        let jitter = rand::random::<u64>() % 500;
        tokio::time::sleep(std::time::Duration::from_millis(500 + jitter)).await;
    }

    let filename = std::path::Path::new(file_path)
        .file_name().and_then(|n| n.to_str()).unwrap_or("media");

    let input_file = if is_big {
        tl_gen::serialize_inputFileBig(file_id, total_parts, filename)
    } else {
        tl_gen::serialize_inputFile(file_id, total_parts, filename, "")
    };

    let peer = tl_gen::serialize_input_peer_user(user_id, access_hash);
    let random_id: i64 = rand::random();

    let ttl_opt = if ttl > 0 { Some(ttl as i32) } else { None };

    // Parse markdown for caption entities
    let (plain_caption, md_entities) = tl::parse_markdown_v2(caption);
    let entity_bufs: Vec<Vec<u8>> = md_entities.iter().map(|e| e.serialize()).collect();
    let entity_refs: Vec<&[u8]> = entity_bufs.iter().map(|v| v.as_slice()).collect();
    let entities_opt: Option<&[&[u8]]> = if entity_refs.is_empty() { None } else { Some(&entity_refs) };

    let media = if media_type == "video" {
        let video_attr = tl_gen::serialize_documentAttributeVideo(video_note, true, false, 0.0, 0, 0, None, None, None);
        let filename_attr = tl_gen::serialize_documentAttributeFilename(filename);
        let attrs: &[&[u8]] = &[&video_attr, &filename_attr];
        tl_gen::serialize_inputMediaUploadedDocument(
            false, false, false,
            &input_file, None, "video/mp4", attrs, None, None, ttl_opt, None,
        )
    } else {
        tl_gen::serialize_inputMediaUploadedPhoto(false, false, &input_file, None, ttl_opt, None)
    };

    let req = tl_gen::build_messages_sendMedia(
        false, silent, false, false, false, false, false,
        &peer, None, &media, &plain_caption, random_id, None, entities_opt, None, None, None, None, None, None, None,
    );
    client.invoke(&req).await
}

fn should_autostop_mailing(config: &MailingConfig, bans: u32, spamblocks: u32, floods: u32) -> bool {
    if !config.autostop_enabled { return false; }
    if config.autostop_ban > 0 && bans >= config.autostop_ban { return true; }
    if config.autostop_spamblock > 0 && spamblocks >= config.autostop_spamblock { return true; }
    if config.autostop_flood > 0 && floods >= config.autostop_flood { return true; }
    false
}

/// Parse a story link like "https://t.me/username/s/123" into (username, story_id as i32)
fn parse_story_link(link: &str) -> (String, i32) {
    let cleaned = link
        .trim()
        .trim_start_matches("https://t.me/")
        .trim_start_matches("http://t.me/")
        .trim_start_matches("t.me/");
    let parts: Vec<&str> = cleaned.split('/').collect();
    // format: username/s/123
    if parts.len() >= 3 && parts[1].eq_ignore_ascii_case("s") {
        let username = parts[0].to_string();
        let story_id = parts[2].parse::<i32>().unwrap_or(0);
        (username, story_id)
    } else {
        (String::new(), 0)
    }
}
