// first_comment: monitors channels for new posts and leaves the first comment
// supports static text or LLM-generated contextual replies

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use serde::Deserialize;
use tauri::{Emitter, Manager};

use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::mtproto::tl_gen;
use crate::mtproto::invite::resolve_channel_link;
use crate::accounts::connect::connect_account;
use crate::queue::TaskQueue;
use crate::i18n::{t, t_with};

const POLL_INTERVAL_MS: u64 = 1500;
const LLM_MAX_CHARS: usize = 200;

async fn interruptible_sleep(ms: u64, token: &AtomicBool) {
    let mut remaining = ms;
    while remaining > 0 {
        if !token.load(Ordering::Relaxed) { break; }
        let chunk = remaining.min(200);
        tokio::time::sleep(std::time::Duration::from_millis(chunk)).await;
        remaining -= chunk;
    }
}

#[derive(Deserialize, Clone)]
pub struct FirstCommentConfig {
    pub target_mode: String, // "channels" | "subscribed"
    pub targets: Vec<String>, // list of channel links (used when target_mode == "channels")
    pub delay_min: u32,
    pub delay_max: u32,
    pub delay_unit: String, // "seconds" | "minutes"

    pub reply_mode: String, // "static" | "llm"
    pub static_text: String,
    #[serde(default)]
    pub static_image_path: String,
    #[serde(default)]
    pub static_video_path: String,
    #[serde(default)]
    pub randomize_static: bool, // homoglyph randomization of the static comment
    pub llm_prompt: String, // e.g. "channel link to promote"
    pub max_flood_wait: u64,
}

#[tauri::command]
pub async fn first_comment_start(
    ids: Vec<String>,
    config: FirstCommentConfig,
    threads: Option<usize>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() {
        return Err(t("first_comment_no_accounts"));
    }
    let concurrency = threads.unwrap_or(5).max(1).min(100);

    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue.register_task(
        task_id.clone(),
        "first_comment".to_string(),
        t_with("first_comment_task_name", &[("count", &ids.len().to_string())]),
    ).await;

    let config = Arc::new(config);

    // for "channels" mode distribute the channel list evenly across accounts
    // (round-robin): 5 channels / 5 accounts → 1 each; 5 channels / 4 accounts →
    // one account gets 2. for "subscribed" mode each account uses its own subs.
    let distribute = config.target_mode == "channels";
    let cleaned_targets: Vec<String> = config.targets.iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let n_accounts = ids.len();
    let mut buckets: Vec<Vec<String>> = vec![Vec::new(); n_accounts];
    if distribute {
        for (i, target) in cleaned_targets.iter().enumerate() {
            buckets[i % n_accounts].push(target.clone());
        }
    }

    tokio::spawn(async move {
        // Check for spam-blocked accounts and warn
        {
            let storage = crate::accounts::commands::get_storage_pub();
            let mut spamblock_count = 0u32;
            for id in &ids {
                let json_path = storage.json_path(id);
                if json_path.exists() {
                    if let Ok(json) = crate::accounts::session::AccountJson::from_file(&json_path) {
                        let sb = &json.spamblock;
                        if !sb.is_empty() && sb != "free" && sb != "none" {
                            spamblock_count += 1;
                        } else {
                            // also check status field
                            let status = &json.status;
                            let is_spamblocked = status.contains("спамблок") || status.contains("spamblock")
                                || status.contains("Спамблок") || status.contains("Spamblock");
                            if is_spamblocked {
                                spamblock_count += 1;
                            }
                        }
                    }
                }
            }
            if spamblock_count > 0 {
                let _ = app.emit("first-comment-log", t_with("first_comment_spamblock_warning", &[("count", &spamblock_count.to_string())]));
            }
        }

        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let total = ids.len();
        let mut handles = Vec::new();

        for (i, id) in ids.into_iter().enumerate() {
            if !token.load(Ordering::Relaxed) { break; }
            let sem = sem.clone();
            let config = config.clone();
            let app_clone = app.clone();
            let token_clone = token.clone();
            let assigned = if distribute { buckets[i].clone() } else { Vec::new() };

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                if !token_clone.load(Ordering::Relaxed) { return; }

                // skip accounts that received no channels in round-robin distribution
                if distribute && assigned.is_empty() {
                    let _ = app_clone.emit("first-comment-log", format!("[{}/{}] {}", i + 1, total, t("first_comment_no_channels_assigned")));
                    return;
                }

                let result = run_account(&id, i + 1, total, &config, &assigned, &app_clone, &token_clone).await;
                if let Err(e) = result {
                    crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                    let _ = app_clone.emit("first-comment-log", format!("[{}/{}] {}: {}", i + 1, total, t("error"), e));
                }
            }));
        }

        for h in handles { let _ = h.await; }
        let _ = app.emit("first-comment-log", t("done"));

        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn first_comment_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn run_account(
    account_id: &str,
    idx: usize,
    total: usize,
    config: &FirstCommentConfig,
    assigned_channels: &[String],
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    let prefix = format!("[{}/{}]", idx, total);
    let emit = |msg: String| { let _ = app.emit("first-comment-log", format!("{} {}", prefix, msg)); };

    // Pre-check spamblock from json — warn but don't skip (user chose to include them)
    {
        let storage = crate::accounts::commands::get_storage_pub();
        let json_path = storage.json_path(account_id);
        if json_path.exists() {
            if let Ok(json) = crate::accounts::session::AccountJson::from_file(&json_path) {
                let sb = &json.spamblock;
                if !sb.is_empty() && sb != "free" && sb != "none" {
                    emit(t_with("first_comment_spamblock_skip", &[("status", sb)]));
                }
            }
        }
    }

    let mut client = connect_account(account_id).await?;
    client.set_log_target("first-comment-log", app.clone());
    client.set_max_flood_wait(config.max_flood_wait);

    // resolve channels to monitor
    let channels = resolve_targets(&mut client, config, assigned_channels, app, &prefix).await?;
    if channels.is_empty() {
        if config.target_mode == "subscribed" {
            // account isn't subscribed to any channel — close its thread per spec
            emit(t("first_comment_not_subscribed"));
        } else {
            emit(t("first_comment_no_channels"));
        }
        return Ok(());
    }
    emit(t_with("first_comment_monitoring_count", &[("count", &channels.len().to_string())]));

    // initialize pts for each channel
    let filter_empty = tl_gen::CHANNEL_MESSAGES_FILTER_EMPTY.to_le_bytes().to_vec();
    let mut watchers: Vec<ChannelWatcher> = Vec::new();
    for ch in &channels {
        // the channel pts is a per-channel update counter, not a message id.
        // read the real current pts from getFullChannel so force=false polling
        // returns only genuinely new posts afterwards.
        let pts = match get_channel_pts(&mut client, ch.id, ch.access_hash).await {
            Ok(p) if p > 0 => p,
            _ => {
                // fallback: probe via getChannelDifference with force to learn pts
                let input_ch = tl_gen::serialize_inputChannel(ch.id, ch.access_hash);
                let init_req = tl_gen::build_updates_getChannelDifference(true, &input_ch, &filter_empty, 1, 1);
                match client.invoke(&init_req).await {
                    Ok(data) => { let p = extract_channel_pts(&data); if p > 0 { p } else { 1 } }
                    Err(_) => 1,
                }
            }
        };
        watchers.push(ChannelWatcher { id: ch.id, access_hash: ch.access_hash, pts });
    }

    // main poll loop — round-robin all channels
    loop {
        if !token.load(Ordering::Relaxed) {
            emit(t("first_comment_stopped"));
            break;
        }

        for watcher in watchers.iter_mut() {
            if !token.load(Ordering::Relaxed) { break; }

            let input_ch = tl_gen::serialize_inputChannel(watcher.id, watcher.access_hash);
            let diff_req = tl_gen::build_updates_getChannelDifference(
                false, &input_ch, &filter_empty, watcher.pts, 100,
            );
            let diff_data = match client.invoke(&diff_req).await {
                Ok(d) => d,
                Err(e) => {
                    if crate::mtproto::is_fatal_session_error(&e) {
                        return Err(e);
                    }
                    continue;
                }
            };

            let diff = match tl_gen::parse_updates_getChannelDifference(&diff_data) {
                Ok(d) => d,
                Err(_) => continue,
            };

            match diff {
                tl_gen::TlUpdatesChannelDifference::Empty { pts: new_pts, .. } => {
                    watcher.pts = new_pts;
                }
                tl_gen::TlUpdatesChannelDifference::TooLong { messages, .. } => {
                    let new_pts = extract_max_msg_id(&messages);
                    if new_pts > watcher.pts { watcher.pts = new_pts + 1; }
                }
                tl_gen::TlUpdatesChannelDifference::ChannelDifference { pts: new_pts, new_messages, .. } => {
                    watcher.pts = new_pts;

                    for msg_raw in &new_messages {
                        if !token.load(Ordering::Relaxed) { break; }

                        let post = match parse_channel_post(msg_raw) {
                            Some(p) => p,
                            None => continue,
                        };
                        if post.text.is_empty() && !post.has_media { continue; }

                        emit(t_with("first_comment_new_post_detail", &[("ch", &watcher.id.to_string()), ("id", &post.msg_id.to_string()), ("text", &truncate(&post.text, 60))]));

                        let delay_ms = compute_delay(config);
                        if delay_ms > 0 {
                            interruptible_sleep(delay_ms, token).await;
                        }
                        if !token.load(Ordering::Relaxed) { break; }

                        let comment = match config.reply_mode.as_str() {
                            "llm" => generate_llm_comment(&post.text, &config.llm_prompt),
                            _ => {
                                if config.randomize_static {
                                    crate::randomizer::randomize_text_internal(&config.static_text, 60)
                                } else {
                                    config.static_text.clone()
                                }
                            }
                        };
                        // a media-only comment is valid (photo/video without text)
                        let has_media = config.reply_mode != "llm"
                            && (!config.static_image_path.is_empty() || !config.static_video_path.is_empty());
                        if comment.is_empty() && !has_media { continue; }

                        let (img, vid) = if config.reply_mode == "llm" {
                            ("", "")
                        } else {
                            (config.static_image_path.as_str(), config.static_video_path.as_str())
                        };

                        match send_comment(&mut client, watcher.id, watcher.access_hash, post.msg_id, &comment, img, vid, token).await {
                            Ok(_) => emit(t_with("first_comment_comment_sent_post", &[("id", &post.msg_id.to_string())])),
                            Err(e) => {
                                if crate::mtproto::is_fatal_session_error(&e) { return Err(e); }
                                // USER_BANNED_IN_CHANNEL = account has spamblock or is banned in this specific channel
                                if e.contains("USER_BANNED_IN_CHANNEL") {
                                    emit(t_with("first_comment_banned_in_channel", &[("ch", &watcher.id.to_string())]));
                                    break; // skip remaining posts in this channel
                                }
                                emit(t_with("first_comment_comment_error", &[("id", &post.msg_id.to_string()), ("error", &e)]));
                            }
                        }
                    }
                }
            }
        }

        interruptible_sleep(POLL_INTERVAL_MS, token).await;
    }

    Ok(())
}

struct ChannelInfo {
    id: i64,
    access_hash: i64,
}

struct ChannelWatcher {
    id: i64,
    access_hash: i64,
    pts: i32,
}

async fn resolve_targets(
    client: &mut MtpClient,
    config: &FirstCommentConfig,
    assigned_channels: &[String],
    app: &tauri::AppHandle,
    prefix: &str,
) -> Result<Vec<ChannelInfo>, String> {
    let emit = |msg: String| { let _ = app.emit("first-comment-log", format!("{} {}", prefix, msg)); };

    if config.target_mode == "subscribed" {
        let req = tl::build_get_dialogs_with_folder(0, 500);
        let data = client.invoke(&req).await.map_err(|e| format!("getDialogs: {e}"))?;
        let peers = tl::parse_dialog_peers(&data).unwrap_or_default();
        let mut channels = Vec::new();
        for peer in peers {
            if let tl::DialogPeer::Channel { id, access_hash } = peer {
                channels.push(ChannelInfo { id, access_hash });
            }
        }
        emit(t_with("first_comment_channels_found", &[("count", &channels.len().to_string())]));
        Ok(channels)
    } else {
        let mut channels = Vec::new();
        for target in assigned_channels {
            let trimmed = target.trim();
            if trimmed.is_empty() { continue; }
            match resolve_channel_link(client, trimmed).await {
                Ok(resolved) => {
                    emit(t_with("first_comment_channel_resolved", &[("title", if resolved.title_hint.is_empty() { trimmed } else { &resolved.title_hint }), ("id", &resolved.channel_id.to_string())]));
                    channels.push(ChannelInfo { id: resolved.channel_id, access_hash: resolved.access_hash });
                }
                Err(e) => emit(t_with("first_comment_resolve_error", &[("target", trimmed), ("error", &e)])),
            }
        }
        Ok(channels)
    }
}

struct ChannelPost {
    msg_id: i32,
    text: String,
    has_media: bool,
}

fn parse_channel_post(data: &[u8]) -> Option<ChannelPost> {
    let msg = tl_gen::deserialize_tl_obj::<tl_gen::TlMessage>(data).ok()?;
    match msg {
        tl_gen::TlMessage::Message { id, message, media, .. } => {
            Some(ChannelPost { msg_id: id, text: message, has_media: media.is_some() })
        }
        _ => None,
    }
}

fn extract_channel_pts(data: &[u8]) -> i32 {
    match tl_gen::parse_updates_getChannelDifference(data) {
        Ok(tl_gen::TlUpdatesChannelDifference::Empty { pts, .. }) => pts,
        Ok(tl_gen::TlUpdatesChannelDifference::ChannelDifference { pts, .. }) => pts,
        _ => 0,
    }
}

// read the current per-channel pts via channels.getFullChannel → channelFull.pts
async fn get_channel_pts(client: &mut MtpClient, channel_id: i64, access_hash: i64) -> Result<i32, String> {
    let req = tl::build_get_full_channel(channel_id, access_hash);
    let data = client.invoke(&req).await.map_err(|e| format!("getFullChannel: {e}"))?;
    let inner = tl_gen::unwrap_rpc(&data).map_err(|e| format!("unwrap: {e}"))?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlMessagesChatFull>(&inner)
        .map_err(|e| format!("chatFull: {e}"))?;
    match tl_gen::deserialize_tl_obj::<tl_gen::TlChatFull>(&obj.full_chat) {
        Ok(tl_gen::TlChatFull::ChannelFull { pts, .. }) => Ok(pts),
        _ => Err("not a channelFull".into()),
    }
}

fn extract_max_msg_id(messages: &[Vec<u8>]) -> i32 {
    let mut max = 0i32;
    for msg in messages {
        if let Ok(tl_gen::TlMessage::Message { id, .. }) = tl_gen::deserialize_tl_obj::<tl_gen::TlMessage>(msg) {
            if id > max { max = id; }
        }
    }
    max
}

fn compute_delay(config: &FirstCommentConfig) -> u64 {
    if config.delay_min == 0 && config.delay_max == 0 {
        return 0;
    }
    let min = config.delay_min as u64;
    let max = config.delay_max.max(config.delay_min) as u64;
    let value = if min == max { min } else { min + (rand::random::<u64>() % (max - min + 1)) };
    match config.delay_unit.as_str() {
        "minutes" => value * 60 * 1000,
        _ => value * 1000,
    }
}

fn generate_llm_comment(post_text: &str, promo_link: &str) -> String {
    let system = format!(
        "You are a regular Telegram user. Write a short comment to a channel post. \
         The comment must be in THE SAME LANGUAGE as the post text. \
         Show genuine interest or mild concern about the topic, then casually mention {}. \
         RULES: max 2 sentences, max {} chars total, no emoji, no quotes. \
         Do NOT start with \"Yes,\" or \"I agree\". Be varied and natural.",
        promo_link, LLM_MAX_CHARS
    );
    let user_msg = if post_text.len() > 300 {
        format!("{}", t_with("first_comment_post_prefix", &[("text", &post_text[..300])]))
    } else {
        format!("{}", t_with("first_comment_post_prefix", &[("text", post_text)]))
    };

    match crate::llm::complete(&system, &user_msg) {
        Ok(mut reply) => {
            // trim to max length
            reply = reply.trim().replace('\n', " ");
            if reply.len() > LLM_MAX_CHARS {
                // cut at last space before limit
                let cut = reply[..LLM_MAX_CHARS].rfind(' ').unwrap_or(LLM_MAX_CHARS);
                reply = reply[..cut].to_string();
            }
            // remove wrapping quotes if LLM added them
            if reply.starts_with('"') && reply.ends_with('"') {
                reply = reply[1..reply.len()-1].to_string();
            }
            reply
        }
        Err(e) => {
            dbg_log!("llm comment error: {e}");
            String::new()
        }
    }
}

pub async fn send_comment_pub(
    client: &mut MtpClient,
    channel_id: i64,
    access_hash: i64,
    msg_id: i32,
    text: &str,
) -> Result<(), String> {
    let token = Arc::new(AtomicBool::new(true));
    send_comment(client, channel_id, access_hash, msg_id, text, "", "", &token).await
}

async fn send_comment(
    client: &mut MtpClient,
    channel_id: i64,
    access_hash: i64,
    msg_id: i32,
    text: &str,
    image_path: &str,
    video_path: &str,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    // get discussion message to find the linked group
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    let disc_req = tl_gen::build_messages_getDiscussionMessage(&peer, msg_id);
    let disc_data = client.invoke(&disc_req).await
        .map_err(|e| format!("getDiscussionMessage: {e}"))?;

    let disc = tl_gen::parse_messages_getDiscussionMessage(&disc_data)
        .map_err(|e| format!("parse discussion: {e}"))?;

    // the first message in the discussion is the "linked" post in the group
    // we need to reply to it. extract the discussion group channel_id from chats
    let (disc_channel_id, disc_access_hash) = extract_discussion_channel(&disc.chats, channel_id)?;
    let disc_msg_id = extract_first_msg_id(&disc.messages)?;

    // build reply_to pointing to the discussion message
    let reply_to = tl_gen::serialize_inputReplyToMessage(
        disc_msg_id, None, None, None, None, None, None, None, None,
    );
    let disc_peer = tl_gen::serialize_input_peer_channel(disc_channel_id, disc_access_hash);

    // parse markdown into plain text + telegram entities
    let (plain, entities) = tl::parse_markdown_v2(text);
    let random_id: i64 = rand::random();

    // media comment: upload then sendMedia with reply_to
    if !image_path.is_empty() {
        let media = upload_photo_media(client, image_path, token).await?;
        return send_media_comment(client, &disc_peer, &reply_to, &media, &plain, &entities, random_id).await;
    }
    if !video_path.is_empty() {
        let media = upload_video_media(client, video_path, token).await?;
        return send_media_comment(client, &disc_peer, &reply_to, &media, &plain, &entities, random_id).await;
    }

    // text comment with optional entities
    let entity_bufs: Vec<Vec<u8>> = entities.iter().map(|e| e.serialize()).collect();
    let entity_refs: Vec<&[u8]> = entity_bufs.iter().map(|b| b.as_slice()).collect();
    let entities_opt = if entity_refs.is_empty() { None } else { Some(entity_refs.as_slice()) };

    let req = tl_gen::build_messages_sendMessage(
        true, false, false, false, false, false, false, false,
        &disc_peer, Some(&reply_to), &plain, random_id, None, entities_opt, None, None, None, None, None, None, None,
    );
    client.invoke(&req).await.map_err(|e| format!("sendMessage: {e}"))?;
    Ok(())
}

async fn send_media_comment(
    client: &mut MtpClient,
    peer: &[u8],
    reply_to: &[u8],
    media: &[u8],
    caption: &str,
    entities: &[tl::MarkdownEntity],
    random_id: i64,
) -> Result<(), String> {
    let entity_bufs: Vec<Vec<u8>> = entities.iter().map(|e| e.serialize()).collect();
    let entity_refs: Vec<&[u8]> = entity_bufs.iter().map(|b| b.as_slice()).collect();
    let entities_opt = if entity_refs.is_empty() { None } else { Some(entity_refs.as_slice()) };

    let req = tl_gen::build_messages_sendMedia(
        false, false, false, false, false, false, false,
        peer, Some(reply_to), media, caption, random_id, None, entities_opt, None, None, None, None, None, None, None,
    );
    client.invoke(&req).await.map_err(|e| format!("sendMedia: {e}"))?;
    Ok(())
}

const UPLOAD_CHUNK: usize = 512 * 1024;

async fn upload_photo_media(client: &mut MtpClient, path: &str, token: &Arc<AtomicBool>) -> Result<Vec<u8>, String> {
    let data = tokio::fs::read(path).await.map_err(|e| format!("read photo: {e}"))?;
    let file_id: i64 = rand::random();
    let total_parts = ((data.len() + UPLOAD_CHUNK - 1) / UPLOAD_CHUNK) as i32;
    for part in 0..total_parts {
        if !token.load(Ordering::Relaxed) { return Ok(Vec::new()); }
        let offset = part as usize * UPLOAD_CHUNK;
        let end = (offset + UPLOAD_CHUNK).min(data.len());
        let req = tl_gen::build_upload_saveFilePart(file_id, part, &data[offset..end]);
        client.invoke(&req).await.map_err(|e| format!("upload photo part {part}: {e}"))?;
        let jitter = rand::random::<u64>() % 500;
        tokio::time::sleep(std::time::Duration::from_millis(500 + jitter)).await;
    }
    let filename = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("photo.jpg");
    let input_file = tl_gen::serialize_inputFile(file_id, total_parts, filename, "");
    Ok(tl_gen::serialize_inputMediaUploadedPhoto(false, false, &input_file, None, None, None))
}

async fn upload_video_media(client: &mut MtpClient, path: &str, token: &Arc<AtomicBool>) -> Result<Vec<u8>, String> {
    let data = tokio::fs::read(path).await.map_err(|e| format!("read video: {e}"))?;
    let file_id: i64 = rand::random();
    let total_parts = ((data.len() + UPLOAD_CHUNK - 1) / UPLOAD_CHUNK) as i32;
    let is_big = data.len() >= 10 * 1024 * 1024;
    for part in 0..total_parts {
        if !token.load(Ordering::Relaxed) { return Ok(Vec::new()); }
        let offset = part as usize * UPLOAD_CHUNK;
        let end = (offset + UPLOAD_CHUNK).min(data.len());
        let chunk = &data[offset..end];
        let req = if is_big {
            tl_gen::build_upload_saveBigFilePart(file_id, part, total_parts, chunk)
        } else {
            tl_gen::build_upload_saveFilePart(file_id, part, chunk)
        };
        client.invoke(&req).await.map_err(|e| format!("upload video part {part}: {e}"))?;
        let jitter = rand::random::<u64>() % 500;
        tokio::time::sleep(std::time::Duration::from_millis(500 + jitter)).await;
    }
    let filename = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("video.mp4");
    let input_file = if is_big {
        tl_gen::serialize_inputFileBig(file_id, total_parts, filename)
    } else {
        tl_gen::serialize_inputFile(file_id, total_parts, filename, "")
    };
    let video_attr = tl_gen::serialize_documentAttributeVideo(false, true, false, 0.0, 0, 0, None, None, None);
    let filename_attr = tl_gen::serialize_documentAttributeFilename(filename);
    let attrs: &[&[u8]] = &[&video_attr, &filename_attr];
    Ok(tl_gen::serialize_inputMediaUploadedDocument(
        false, false, false, &input_file, None, "video/mp4", attrs, None, None, None, None,
    ))
}

fn extract_discussion_channel(chats: &[Vec<u8>], source_channel_id: i64) -> Result<(i64, i64), String> {
    // find the linked discussion group: a channel in `chats` that isn't the source.
    // use the generated deserializer — channel#1c32b11c has two flag words and a
    // conditional access_hash, too fragile to parse by hand.
    for chat_raw in chats {
        match tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(chat_raw) {
            Ok(tl_gen::TlChat::Channel { id, access_hash, .. }) => {
                if id != 0 && id != source_channel_id {
                    return Ok((id, access_hash.unwrap_or(0)));
                }
            }
            Ok(tl_gen::TlChat::ChannelForbidden { id, access_hash, .. }) => {
                if id != 0 && id != source_channel_id {
                    return Ok((id, access_hash));
                }
            }
            _ => {}
        }
    }
    Err(t("first_comment_no_discussion"))
}

fn extract_first_msg_id(messages: &[Vec<u8>]) -> Result<i32, String> {
    for msg_raw in messages {
        if let Ok(tl_gen::TlMessage::Message { id, .. }) = tl_gen::deserialize_tl_obj::<tl_gen::TlMessage>(msg_raw) {
            if id > 0 {
                return Ok(id);
            }
        }
    }
    Err(t("first_comment_no_msg_id"))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() }
    else { format!("{}...", &s[..max]) }
}
