// cloner runtime: walks the source channel oldest-to-newest, applies
// filters, forwards each message to the destination channel, then optionally
// edits the forwarded copy to apply text replacements.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};

use crate::accounts::connect::connect_account;
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl::{self, ParsedMessage};
use crate::queue::TaskQueue;
use crate::i18n::{t, t_with};

use super::config::{ClonerConfig, ClonerConfigPayload};
use super::destination::{resolve_or_create_destination, ChannelHandle, SourceContext, SourcePhoto};
use super::media::{detect_media, MediaKind};
use super::transform::{
    build_edited_text, classify_skip, has_external_link,
    has_telegram_link, SkipReason,
};

const PAGE_SIZE: i32 = 100;

#[tauri::command]
pub async fn cloner_start(
    account_id: String,
    config: ClonerConfigPayload,
    max_flood_wait: Option<u64>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let cfg = ClonerConfig::from_payload(config)?;
    let cfg = Arc::new(cfg);
    let max_flood_wait = max_flood_wait.unwrap_or(0);

    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue.register_task(
        task_id.clone(),
        "cloner".to_string(),
        t_with("cloner_task_name", &[("id", &account_id)]),
    ).await;

    tokio::spawn(async move {
        let result = run(&account_id, cfg, max_flood_wait, &app, token.clone()).await;
        match &result {
            Ok(_) => {
                let _ = app.emit("cloner-log", t("done"));
            }
            Err(e) => {
                crate::accounts::commands::check_and_mark_dead_session(e, &account_id);
                let _ = app.emit("cloner-log", format!("{}: {e}", t("error")));
                let _ = app.emit("cloner-log", t("done"));
            }
        }
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, result.is_ok()).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn cloner_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn run(
    account_id: &str,
    cfg: Arc<ClonerConfig>,
    max_flood_wait: u64,
    app: &tauri::AppHandle,
    token: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut client = connect_account(account_id).await?;
    client.set_log_target("cloner-log", app.clone());
    client.set_max_flood_wait(max_flood_wait);

    // ---- resolve source channel ----
    let source = resolve_source(&mut client, &cfg.source_channel).await?;
    emit(app, t_with("cloner_source", &[("title", &source.title), ("id", &source.channel_id.to_string())]));

    // ---- resolve / create destination ----
    let dest = resolve_or_create_destination(&mut client, &cfg.destination, &source, app).await?;
    emit(app, t_with("cloner_destination", &[("id", &dest.channel_id.to_string())]));

    // when we just created a channel, sweep all service messages (channel-created,
    // photo-update, title-update, about-update etc.) so the cloned posts start
    // from a clean head.
    if matches!(cfg.destination, crate::cloner::config::DestinationSpec::NewChannel { .. }) {
        if let Err(e) = sweep_service_messages(&mut client, &dest, app).await {
            emit(app, t_with("cloner_sweep_error", &[("error", &e)]));
        }
    }

    // ---- iterate messages oldest -> newest ----
    let stats = clone_messages(&mut client, &cfg, &source, &dest, app, token).await?;

    // leave the source channel only if we joined it during this run; staying
    // out of channels the user was already a member of avoids surprise unsubscribes.
    if source.joined_now {
        let leave_req = tl::build_leave_channel(source.channel_id, source.access_hash);
        match client.invoke(&leave_req).await {
            Ok(_) => emit(app, t("cloner_left_source")),
            Err(e) => emit(app, t_with("cloner_leave_source_error", &[("error", &e)])),
        }
    }

    emit(app, t_with("cloner_stats", &[("copied", &stats.copied.to_string()), ("skipped", &stats.skipped.to_string()), ("errors", &stats.errors.to_string())]));
    Ok(())
}

#[derive(Default)]
struct CloneStats {
    copied: u32,
    skipped: u32,
    errors: u32,
}

async fn interruptible_sleep(ms: u64, token: &AtomicBool) {
    let mut remaining = ms;
    while remaining > 0 {
        if !token.load(Ordering::Relaxed) { break; }
        let chunk = remaining.min(200);
        tokio::time::sleep(std::time::Duration::from_millis(chunk)).await;
        remaining -= chunk;
    }
}

async fn clone_messages(
    client: &mut MtpClient,
    cfg: &ClonerConfig,
    source: &SourceContext,
    dest: &ChannelHandle,
    app: &tauri::AppHandle,
    token: Arc<AtomicBool>,
) -> Result<CloneStats, String> {
    let mut stats = CloneStats::default();
    let mut reply_map: HashMap<i32, i32> = HashMap::new();

    let want_all = cfg.from_id == 0 && cfg.to_id == 0;
    let lower = cfg.from_id.max(0);
    let upper_cfg = if cfg.to_id == 0 { i32::MAX } else { cfg.to_id };
    if !want_all && lower > upper_cfg {
        return Err(t("cloner_from_gt_to"));
    }

    // collect all message ids using pyrogram-style pagination (newest first
    // walk via offset_id, bounded by min_id), then sort ascending so we
    // process oldest-first and reply-mapping can resolve forward references.
    let messages = collect_messages(client, source, lower, upper_cfg, &token).await?;

    if messages.is_empty() {
        emit(app, t("cloner_no_messages"));
        return Ok(stats);
    }

    let lo = messages.iter().map(|m| m.id).min().unwrap_or(0);
    let hi = messages.iter().map(|m| m.id).max().unwrap_or(0);
    emit(app, t_with("cloner_collected", &[("count", &messages.len().to_string()), ("lo", &lo.to_string()), ("hi", &hi.to_string())]));

    for msg in &messages {
        if !token.load(Ordering::Relaxed) {
            emit(app, t("stopped_by_user"));
            return Ok(stats);
        }
        match process_message(client, cfg, source, dest, msg, &mut reply_map, app).await {
            Ok(true) => stats.copied += 1,
            Ok(false) => stats.skipped += 1,
            Err(e) => {
                if crate::mtproto::is_fatal_session_error(&e) {
                    return Err(e);
                }
                stats.errors += 1;
                emit(app, t_with("cloner_error_msg", &[("id", &msg.id.to_string()), ("error", &e)]));
            }
        }

        let delay = cfg.random_delay_ms();
        interruptible_sleep(delay, &token).await;
    }

    Ok(stats)
}

// pyrogram/telethon-style traversal: walk newest -> oldest using offset_id,
// bounded by min_id (exclusive), upper bound enforced via offset_id seed.
// returns messages sorted by id ascending.
async fn collect_messages(
    client: &mut MtpClient,
    source: &SourceContext,
    lower: i32,
    upper: i32,
    token: &AtomicBool,
) -> Result<Vec<ParsedMessage>, String> {
    // offset_id seed: 0 means "start from the newest"; if user supplied an upper
    // bound less than i32::MAX we hint the server with offset_id = upper + 1
    // so it skips anything strictly newer than `upper`.
    let mut offset_id: i32 = if upper == i32::MAX || upper <= 0 { 0 } else {
        upper.saturating_add(1)
    };
    // min_id is exclusive on telegram side; we want id >= lower
    let min_id: i32 = (lower - 1).max(0);

    let mut all: Vec<ParsedMessage> = Vec::new();
    let mut seen: std::collections::HashSet<i32> = std::collections::HashSet::new();

    loop {
        if !token.load(Ordering::Relaxed) { break; }
        let req = tl::build_get_history_channel_paged(
            source.channel_id,
            source.access_hash,
            offset_id,
            0,                  // add_offset
            PAGE_SIZE,          // limit
            0,                  // max_id (0 = no upper cap; offset_id handles it)
            min_id,
        );
        let data = client.invoke(&req).await
            .map_err(|e| format!("getHistory: {e}"))?;
        let batch = tl::parse_messages_structured(&data)
            .map_err(|e| format!("parse history: {e}"))?;

        if batch.is_empty() { break; }

        // server returns newest-first
        let mut keepers: Vec<ParsedMessage> = Vec::new();
        for m in &batch {
            if m.id <= 0 { continue; }
            if m.id < lower { continue; }
            if upper != i32::MAX && m.id > upper { continue; }
            if seen.insert(m.id) { keepers.push(m.clone()); }
        }

        let oldest_in_batch = batch.iter().map(|m| m.id).filter(|i| *i > 0).min();
        all.extend(keepers);

        match oldest_in_batch {
            Some(o) if o > min_id + 1 => offset_id = o,
            _ => break,
        }
        if batch.len() < PAGE_SIZE as usize { break; }
    }

    all.sort_by_key(|m| m.id);
    Ok(all)
}

// returns Ok(true) when a message was copied, Ok(false) when skipped.
async fn process_message(
    client: &mut MtpClient,
    cfg: &ClonerConfig,
    source: &SourceContext,
    dest: &ChannelHandle,
    msg: &ParsedMessage,
    reply_map: &mut HashMap<i32, i32>,
    app: &tauri::AppHandle,
) -> Result<bool, String> {
    // skip empty placeholders (e.g. service messages parsed as id-only)
    if msg.id == 0 {
        return Ok(false);
    }
    if msg.is_service {
        emit(app, t_with("cloner_skipped_service", &[("id", &msg.id.to_string())]));
        return Ok(false);
    }

    if let Some(reason) = classify_skip(msg, cfg) {
        emit(app, t_with("cloner_skipped_reason", &[("id", &msg.id.to_string()), ("reason", &reason.ru())]));
        return Ok(false);
    }

    // media-type filter — uses raw bytes if available; ParsedMessage has only text,
    // so we re-fetch the message body here only when at least one media gate is on
    // (otherwise we can skip the round-trip entirely).
    let needs_media_check = !cfg.copy_documents
        || !cfg.copy_photos
        || !cfg.copy_videos
        || !cfg.copy_messages_with_video;
    let mut media_info = None;
    if needs_media_check {
        let raw = fetch_message_blob(client, source, msg.id).await?;
        let info = detect_media(&raw);
        media_info = Some(info);

        // type gates
        let kind_skip = match info.kind {
            MediaKind::Photo if !cfg.copy_photos => Some(SkipReason::PhotoDisabled),
            MediaKind::Video if !cfg.copy_videos => Some(SkipReason::VideoDisabled),
            MediaKind::Video if !cfg.copy_messages_with_video => Some(SkipReason::VideoMessageDisabled),
            MediaKind::Document if !cfg.copy_documents => Some(SkipReason::DocumentDisabled),
            MediaKind::Audio if !cfg.copy_documents => Some(SkipReason::DocumentDisabled),
            _ => None,
        };
        if let Some(reason) = kind_skip {
            emit(app, t_with("cloner_skipped_media", &[("id", &msg.id.to_string()), ("reason", &reason.ru())]));
            return Ok(false);
        }

        // size gates — only enforced when both a limit and a known size are present
        let size_skip = match info.kind {
            MediaKind::Photo if cfg.max_photo_bytes > 0
                && info.size_bytes > 0
                && info.size_bytes > cfg.max_photo_bytes => Some(SkipReason::OversizedPhoto),
            MediaKind::Video if cfg.max_video_bytes > 0
                && info.size_bytes > 0
                && info.size_bytes > cfg.max_video_bytes => Some(SkipReason::OversizedVideo),
            MediaKind::Document | MediaKind::Audio
                if cfg.max_file_bytes > 0
                    && info.size_bytes > 0
                    && info.size_bytes > cfg.max_file_bytes => Some(SkipReason::OversizedFile),
            _ => None,
        };
        if let Some(reason) = size_skip {
            emit(app, t_with("cloner_skipped_size", &[("id", &msg.id.to_string()), ("reason", &reason.ru()), ("kb", &(info.size_bytes / 1024).to_string())]));
            return Ok(false);
        }
    }

    // Determine if we should use forward or send (for reply chains / noforwards)
    let source_reply_to = msg.reply_to_msg_id;
    let mapped_reply = source_reply_to
        .and_then(|orig_id| if cfg.preserve_replies { reply_map.get(&orig_id).copied() } else { None });

    // If there's a mapped reply or content is protected (noforwards),
    // we must re-send instead of forward (forward doesn't support reply_to)
    let use_send_mode = mapped_reply.is_some() || source.noforwards;

    // Re-sending via sendMessage preserves neither photos nor documents. Do
    // not turn a protected media post into a text-only post without telling
    // the user; the regular forward path still preserves media intact.
    if use_send_mode {
        let info = match media_info {
            Some(info) => info,
            None => {
                let raw = fetch_message_blob(client, source, msg.id).await?;
                detect_media(&raw)
            }
        };
        if info.kind != MediaKind::None {
            return Err(t("cloner_media_resend_unsupported"));
        }
    }

    let new_id = if use_send_mode {
        // send as new message (preserves reply chain, works for protected content)
        let text = build_edited_text(&msg.text, &cfg.replacements)
            .unwrap_or_else(|| msg.text.clone());

        // try to send. if MediaCaptionTooLong — send media without caption, then text separately
        match send_as_copy(client, source, dest, msg, &text, mapped_reply).await {
            Ok(id) => id,
            Err(e) if e.contains("MEDIA_CAPTION_TOO_LONG") => {
                // send media without text
                let id = send_as_copy(client, source, dest, msg, "", mapped_reply).await
                    .unwrap_or(0);
                // send text as separate message
                if !text.is_empty() && id > 0 {
                    let (plain, entities) = tl::parse_markdown_v2(&text);
                    let rid: i64 = rand::random();
                    let req = tl::build_send_message_with_entities(
                        dest.channel_id, dest.access_hash, true, &plain, &entities, rid,
                    );
                    let _ = client.invoke(&req).await;
                }
                id
            }
            Err(e) => return Err(e),
        }
    } else {
        // standard forward path (drop_author)
        let req = tl::build_forward_messages(
            source.channel_id,
            source.access_hash,
            dest.channel_id,
            dest.access_hash,
            &[msg.id],
            true,  // drop_author
            false, // keep media captions
            false, // not silent
        );

        match client.invoke(&req).await {
            Ok(resp) => tl::extract_first_new_message_id(&resp).unwrap_or(0),
            Err(e) if e.contains("CHAT_FORWARDS_RESTRICTED") || e.contains("ChatForwardsRestricted") => {
                // content is protected — fall back to send mode
                let text = build_edited_text(&msg.text, &cfg.replacements)
                    .unwrap_or_else(|| msg.text.clone());
                send_as_copy(client, source, dest, msg, &text, mapped_reply).await.unwrap_or(0)
            }
            Err(e) => return Err(e),
        }
    };

    if cfg.preserve_replies && new_id > 0 {
        reply_map.insert(msg.id, new_id);
    }

    // post-process: text replacements + link preview toggle (only for forward path, send already has replacements applied)
    if new_id > 0 && !use_send_mode {
        let edited_text = build_edited_text(&msg.text, &cfg.replacements);
        let needs_preview_toggle = !cfg.show_link_preview
            && (has_external_link(&msg.text) || has_telegram_link(&msg.text));

        if edited_text.is_some() || needs_preview_toggle {
            let final_text = edited_text.unwrap_or_else(|| msg.text.clone());
            if !final_text.is_empty() {
                let edit_req = tl::build_edit_message_channel(
                    dest.channel_id,
                    dest.access_hash,
                    new_id,
                    &final_text,
                    &[],
                    !cfg.show_link_preview,
                );
                if let Err(e) = client.invoke(&edit_req).await {
                    emit(app, format!("editMessage msg={}: {}", new_id, e));
                }
            }
        }
    }

    emit(app, t_with("cloner_copied_msg", &[("id", &msg.id.to_string()), ("dst", &new_id.to_string())]));
    Ok(true)
}

async fn fetch_message_blob(
    client: &mut MtpClient,
    source: &SourceContext,
    msg_id: i32,
) -> Result<Vec<u8>, String> {
    // re-using the channel history call with min_id == msg_id-1 and limit=1
    // is the simplest way to fetch one message without a dedicated builder.
    let req = tl::build_get_history_channel_paged(
        source.channel_id,
        source.access_hash,
        0,
        0,
        1,
        0,
        msg_id - 1,
    );
    client.invoke(&req).await
}

async fn resolve_source(client: &mut MtpClient, source_link: &str) -> Result<SourceContext, String> {
    let resolved = crate::mtproto::invite::resolve_channel_link(client, source_link).await?;
    let mut source = load_source_full(
        client,
        resolved.channel_id,
        resolved.access_hash,
        resolved.username_hint.as_deref(),
        resolved.joined_now,
    ).await;
    if source.title.is_empty() && !resolved.title_hint.is_empty() {
        source.title = resolved.title_hint;
    }
    Ok(source)
}

// fetches channelFull info for a known channel handle and assembles SourceContext.
// optional `username_hint` is used as a fallback title when the structured
// decoder couldn't recover one.
async fn load_source_full(
    client: &mut MtpClient,
    channel_id: i64,
    access_hash: i64,
    username_hint: Option<&str>,
    joined_now: bool,
) -> SourceContext {
    let full_req = tl::build_get_full_channel(channel_id, access_hash);
    let mut title = String::new();
    let mut about = String::new();
    let mut photo: Option<SourcePhoto> = None;
    if let Ok(full_resp) = client.invoke(&full_req).await {
        if let Ok(full) = tl::parse_full_channel(&full_resp) {
            about = full.about.clone();
            title = full.title.clone();
            if full.chat_photo_id != 0 {
                photo = Some(SourcePhoto {
                    photo_id: full.chat_photo_id,
                    access_hash: full.chat_photo_access_hash,
                    file_reference: full.chat_photo_file_reference,
                });
            }
        }
    }
    if title.is_empty() {
        title = match username_hint {
            Some(u) if !u.is_empty() => format!("@{u}"),
            _ => format!("private:{channel_id}"),
        };
    }

    SourceContext { channel_id, access_hash, title, about, photo, joined_now, noforwards: false }
}

/// Send a text message as a copy (for reply chains and noforwards channels).
async fn send_as_copy(
    client: &mut MtpClient,
    _source: &SourceContext,
    dest: &ChannelHandle,
    _msg: &ParsedMessage,
    text: &str,
    reply_to: Option<i32>,
) -> Result<i32, String> {
    let (plain, entities) = tl::parse_markdown_v2(text);
    let rid: i64 = rand::random();

    // Media posts are rejected before this function: sendMessage only handles
    // text, so accepting them here would silently discard the attachment.
    let req = if let Some(reply_id) = reply_to {
        // build with reply_to
        let peer = crate::mtproto::tl_gen::serialize_input_peer_channel(dest.channel_id, dest.access_hash);
        let reply_to_bytes = crate::mtproto::tl_gen::serialize_inputReplyToMessage(reply_id, None, None, None, None, None, None, None, None);
        crate::mtproto::tl_gen::build_messages_sendMessage(
            false, false, false, false, false, false, false, false,
            &peer, Some(&reply_to_bytes), &plain, rid, None, None, None, None, None, None, None, None, None, None,
        )
    } else {
        tl::build_send_message_with_entities(dest.channel_id, dest.access_hash, true, &plain, &entities, rid)
    };

    let resp = client.invoke(&req).await
        .map_err(|e| format!("sendMessage copy: {e}"))?;
    let new_id = tl::extract_first_new_message_id(&resp).unwrap_or(0);
    Ok(new_id)
}

fn emit(app: &tauri::AppHandle, msg: String) {
    let _ = app.emit("cloner-log", msg);
}

// after creating a fresh channel the head contains 1..N service messages that
// describe channel creation, photo upload, title/about edits. we delete all
// of them in a single batched call so the cloned posts start at a clean head.
async fn sweep_service_messages(
    client: &mut MtpClient,
    dest: &ChannelHandle,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    // pull a larger page so we cover photo-update, title-update, about-update
    // even if telegram inserts placeholders between them
    let req = tl::build_get_history_channel_paged(
        dest.channel_id,
        dest.access_hash,
        0, 0, 100, 0, 0,
    );
    let data = client.invoke(&req).await.map_err(|e| format!("getHistory dest: {e}"))?;
    let msgs = tl::parse_messages_structured(&data).map_err(|e| format!("parse dest history: {e}"))?;
    // a freshly created channel only contains service messages — delete all of them.
    // mixing in regular Message ids (would happen on re-runs against an existing
    // channel) is avoided by limiting deletion to is_service entries.
    let ids: Vec<i32> = msgs
        .iter()
        .filter(|m| m.id > 0 && m.is_service)
        .map(|m| m.id)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }

    let del = tl::build_channels_delete_messages(dest.channel_id, dest.access_hash, &ids);
    match client.invoke(&del).await {
        Ok(_) => {
            emit(app, t_with("cloner_sweep_count", &[("count", &ids.len().to_string())]));
            Ok(())
        }
        Err(e) => {
            emit(app, t_with("cloner_sweep_result", &[("error", &e)]));
            Ok(())
        }
    }
}
