// masslooking: mass view/react/reply to user stories

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

#[derive(Deserialize, Clone)]
pub struct MasslookingConfig {
    pub source_mode: String,      // "usernames" | "inbox" | "chat"
    pub usernames_path: String,
    pub chat_target: String,
    pub react_after_view: bool,
    pub reaction: String,
    pub reply_to_story: bool,
    pub reply_text_mode: String,  // "none" | "llm_rewrite" | "randomize"
    pub reply_text: String,
    pub max_flood_wait: u64,
    pub max_per_account: u32,
}

#[tauri::command]
pub async fn masslooking_start(
    ids: Vec<String>,
    config: MasslookingConfig,
    threads: Option<usize>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() { return Err(t("masslooking_no_accounts")); }
    let concurrency = threads.unwrap_or(5).max(1).min(100);
    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue.register_task(
        task_id.clone(), "masslooking".to_string(),
        t_with("masslooking_task_name", &[("count", &ids.len().to_string())]),
    ).await;

    let usernames: Vec<String> = if config.source_mode == "usernames" && !config.usernames_path.is_empty() {
        std::fs::read_to_string(&config.usernames_path).unwrap_or_default()
            .lines().map(|l| l.trim().trim_start_matches('@').to_string())
            .filter(|l| !l.is_empty()).collect()
    } else { Vec::new() };

    let config = Arc::new(config);
    let usernames = Arc::new(usernames);
    let username_idx = Arc::new(AtomicUsize::new(0));

    tokio::spawn(async move {
        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let total = ids.len();
        let mut handles = Vec::new();
        for (i, id) in ids.into_iter().enumerate() {
            if !token.load(Ordering::Relaxed) { break; }
            let sem = sem.clone(); let config = config.clone();
            let usernames = usernames.clone(); let username_idx = username_idx.clone();
            let app_clone = app.clone(); let token_clone = token.clone();
            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                if !token_clone.load(Ordering::Relaxed) { return; }
                let result = process_account(
                    &id, i+1, total, &config, &usernames, &username_idx, &app_clone, &token_clone
                ).await;
                if let Err(e) = result {
                    crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                    let _ = app_clone.emit("masslooking-log", format!("[{}/{}] {}: {}", i+1, total, t("error"), e));
                }
            }));
        }
        for h in handles { let _ = h.await; }
        let _ = app.emit("masslooking-log", t("done"));
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });
    Ok(tid)
}

#[tauri::command]
pub async fn masslooking_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn process_account(
    id: &str, idx: usize, total: usize,
    config: &MasslookingConfig, usernames: &[String],
    username_idx: &AtomicUsize, app: &tauri::AppHandle, token: &Arc<AtomicBool>,
) -> Result<(), String> {
    let prefix = format!("[{}/{}]", idx, total);
    let emit = |msg: &str| { let _ = app.emit("masslooking-log", format!("{} {}", prefix, msg)); };

    let mut client = connect_account(id).await?;
    client.set_log_target("masslooking-log", app.clone());
    client.set_max_flood_wait(config.max_flood_wait);

    // collect target users
    let targets = collect_targets(&mut client, config, usernames, username_idx, app, &prefix, token).await?;
    emit(&t_with("masslooking_targets_count", &[("count", &targets.len().to_string())]));

    let mut processed = 0u32;
    for (user_id, access_hash) in &targets {
        if !token.load(Ordering::Relaxed) { break; }
        if processed >= config.max_per_account { break; }

        // view stories
        let peer = tl_gen::serialize_input_peer_user(*user_id, *access_hash);
        let stories_req = tl_gen::build_stories_getPinnedStories(&peer, 0, 20);
        let stories_data = match client.invoke(&stories_req).await {
            Ok(d) => d,
            Err(e) => {
                if crate::mtproto::is_fatal_session_error(&e) { return Err(e); }
                emit(&t_with("masslooking_stories_error", &[("user_id", &user_id.to_string()), ("error", &e)])); continue;
            }
        };
        let story_ids = extract_story_ids(&stories_data);
        if story_ids.is_empty() { continue; }

        // read/view stories
        let max_id = *story_ids.last().unwrap_or(&0);
        if max_id > 0 {
            let read_req = build_read_stories_user(*user_id, *access_hash, max_id);
            let _ = client.invoke(&read_req).await;
            emit(&t_with("masslooking_stories_viewed", &[("count", &story_ids.len().to_string()), ("user_id", &user_id.to_string())]));
        }

        // react
        if config.react_after_view && !story_ids.is_empty() {
            let story_id = story_ids[0];
            let react_req = build_send_story_reaction(*user_id, *access_hash, story_id, &config.reaction);
            match client.invoke(&react_req).await {
                Ok(_) => emit(&t_with("masslooking_reaction_sent", &[("emoji", &config.reaction), ("user_id", &user_id.to_string())])),
                Err(e) => {
                    if crate::mtproto::is_fatal_session_error(&e) { return Err(e); }
                    emit(&t_with("masslooking_reaction_error", &[("user_id", &user_id.to_string()), ("error", &e)]));
                }
            }
        }

        // reply
        if config.reply_to_story && !config.reply_text.is_empty() && !story_ids.is_empty() {
            let text = prepare_reply_text(&config.reply_text, &config.reply_text_mode);
            let story_id = story_ids[0];
            let random_id: i64 = rand::random();
            let reply_req = build_send_story_reply(*user_id, *access_hash, story_id, &text, random_id);
            match client.invoke(&reply_req).await {
                Ok(_) => emit(&t_with("masslooking_reply_sent", &[("user_id", &user_id.to_string())])),
                Err(e) => {
                    if crate::mtproto::is_fatal_session_error(&e) { return Err(e); }
                    emit(&t_with("masslooking_reply_error", &[("user_id", &user_id.to_string()), ("error", &e)]));
                }
            }
        }

        processed += 1;
        interruptible_sleep(rate_limit_ms(), token).await;
    }

    emit(&t_with("masslooking_processed", &[("count", &processed.to_string())]));
    // surface a fatal session error even if a view/react/reply arm swallowed it
    if let Some(fatal) = client.fatal_error() {
        return Err(fatal.to_string());
    }
    Ok(())
}

async fn collect_targets(
    client: &mut MtpClient, config: &MasslookingConfig,
    usernames: &[String], username_idx: &AtomicUsize,
    app: &tauri::AppHandle, prefix: &str,
    token: &Arc<AtomicBool>,
) -> Result<Vec<(i64, i64)>, String> {
    match config.source_mode.as_str() {
        "usernames" => {
            let mut out = Vec::new();
            for _ in 0..config.max_per_account {
                if !token.load(Ordering::Relaxed) { break; }
                let i = username_idx.fetch_add(1, Ordering::Relaxed);
                if i >= usernames.len() { break; }
                let req = tl::build_resolve_username(&usernames[i]);
                if let Ok(data) = client.invoke(&req).await {
                    if let Ok((id, hash)) = tl::parse_resolved_peer(&data) {
                        out.push((id, hash));
                    }
                }
                interruptible_sleep(300, token).await;
            }
            Ok(out)
        }
        "inbox" => {
            let req = tl::build_get_dialogs_with_folder(0, 200);
            let data = client.invoke(&req).await.map_err(|e| format!("getDialogs: {e}"))?;
            let peers = tl::parse_dialog_peers(&data).unwrap_or_default();
            let mut out = Vec::new();
            for peer in peers {
                if let tl::DialogPeer::User { id, access_hash, is_bot } = peer {
                    if !is_bot { out.push((id, access_hash)); }
                }
            }
            let _ = app.emit("masslooking-log", format!("{} {}", prefix, t_with("masslooking_inbox_found", &[("count", &out.len().to_string())])));
            Ok(out)
        }
        "chat" => {
            let resolved = crate::mtproto::invite::resolve_channel_link(client, &config.chat_target).await?;
            let req = tl::build_channels_get_participants_search(
                resolved.channel_id, resolved.access_hash, "", 0, 200,
            );
            let data = client.invoke(&req).await.map_err(|e| format!("getParticipants: {e}"))?;
            let batch = tl::parse_channel_participants(&data).map_err(|e| format!("parse: {e}"))?;
            let out: Vec<(i64, i64)> = batch.users.iter()
                .filter(|u| !u.is_bot && !u.is_deleted && !u.is_self)
                .map(|u| (u.id, u.access_hash))
                .collect();
            let _ = app.emit("masslooking-log", format!("{} {}", prefix, t_with("masslooking_chat_found", &[("count", &out.len().to_string())])));
            Ok(out)
        }
        _ => Ok(Vec::new()),
    }
}

fn extract_story_ids(data: &[u8]) -> Vec<i32> {
    let mut ids = Vec::new();
    let mut i = 0usize;
    while i + 4 <= data.len() {
        let c = u32::from_le_bytes([data[i], data[i+1], data[i+2], data[i+3]]);
        if c == tl_gen::STORY_ITEM {
            let mut cursor = std::io::Cursor::new(&data[i..]);
            if let Ok(tl_gen::TlStoryItem::StoryItem { id, .. }) = tl_gen::TlStoryItem::deserialize(&mut cursor) {
                if id > 0 && !ids.contains(&id) { ids.push(id); }
            }
        }
        i += 4;
    }
    ids.truncate(20);
    ids
}

fn build_read_stories_user(user_id: i64, access_hash: i64, max_id: i32) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(user_id, access_hash);
    tl_gen::build_stories_readStories(&peer, max_id)
}

fn build_send_story_reaction(user_id: i64, access_hash: i64, story_id: i32, emoji: &str) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(user_id, access_hash);
    let reaction = tl_gen::serialize_reactionEmoji(emoji);
    tl_gen::build_stories_sendReaction(false, &peer, story_id, &reaction)
}

fn build_send_story_reply(user_id: i64, access_hash: i64, story_id: i32, text: &str, random_id: i64) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(user_id, access_hash);
    let reply_to = tl_gen::serialize_inputReplyToMessage(story_id, None, None, None, None, None, None, None, None);
    tl_gen::build_messages_sendMessage(
        true, false, false, false, false, false, false, false,
        &peer, Some(&reply_to), text, random_id, None, None, None, None, None, None, None, None, None,
    )
}

fn prepare_reply_text(base: &str, mode: &str) -> String {
    match mode {
        "llm_rewrite" => {
            match crate::llm::complete(
                &crate::i18n::t("llm_rephrase_prompt"),
                base,
            ) {
                Ok(r) => r.trim().to_string(),
                Err(_) => base.to_string(),
            }
        }
        "randomize" => crate::randomizer::randomize_text_internal(base, 60),
        _ => base.to_string(),
    }
}
