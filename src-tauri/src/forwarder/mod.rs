// forwarder: monitors incoming PMs and forwards them to a group.
// When someone replies to a forwarded message in the group,
// the account copies that reply back to the original sender's DM.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use serde::Deserialize;
use tauri::{Emitter, Manager};

use crate::accounts::connect::connect_account;
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::mtproto::tl_gen;
use crate::queue::TaskQueue;
use crate::i18n::{t, t_with};

const POLL_INTERVAL_MS: u64 = 2500;

async fn interruptible_sleep_ms(ms: u64, token: &Arc<AtomicBool>) {
    let mut remaining = ms;
    while remaining > 0 {
        if !token.load(Ordering::Relaxed) { break; }
        let chunk = remaining.min(200);
        tokio::time::sleep(Duration::from_millis(chunk)).await;
        remaining -= chunk;
    }
}

#[derive(Deserialize, Clone)]
pub struct ForwarderConfig {
    pub group_link: String,
    pub max_flood_wait: u32,
    pub typing_min: u32,
    pub typing_max: u32,
    // new: separate delays for each stage
    #[serde(default)]
    pub mess_wait_min: u32, // delay before forwarding PM to group
    #[serde(default)]
    pub mess_wait_max: u32,
    #[serde(default)]
    pub send_wait_min: u32, // delay before sending reply back to DM
    #[serde(default)]
    pub send_wait_max: u32,
    // resend old unread PMs on start
    #[serde(default)]
    pub resend_old: bool,
    #[serde(default)]
    pub resend_old_wait_min: u32,
    #[serde(default)]
    pub resend_old_wait_max: u32,
    // leave group on stop (checkbox)
    #[serde(default)]
    pub leave_on_stop: bool,
    // send reaction on processed message
    #[serde(default)]
    pub send_reaction: bool,
}

struct ForwardedInfo {
    sender_id: i64,
    sender_access_hash: i64,
}

#[tauri::command]
pub async fn forwarder_start(
    ids: Vec<String>,
    config: ForwarderConfig,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() {
        return Err(t("forwarder_no_accounts"));
    }
    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();
    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue.register_task(
        task_id.clone(),
        "forwarder".to_string(),
        t_with("forwarder_task_name", &[("count", &ids.len().to_string())]),
    ).await;
    let cfg = Arc::new(config);
    tokio::spawn(async move {
        let result = run(ids, cfg, &app, token.clone()).await;
        match &result {
            Ok(_) => { emit(&app, t("done")); }
            Err(e) => { emit(&app, format!("{}: {e}", t("error"))); emit(&app, t("done")); }
        }
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });
    Ok(tid)
}

#[tauri::command]
pub async fn forwarder_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn run(
    account_ids: Vec<String>,
    cfg: Arc<ForwarderConfig>,
    app: &tauri::AppHandle,
    token: Arc<AtomicBool>,
) -> Result<(), String> {
    let group_link = cfg.group_link.trim();
    if group_link.is_empty() { return Err(t("forwarder_no_group")); }

    let total = account_ids.len();
    emit(app, t_with("forwarder_start", &[("total", &total.to_string()), ("group", group_link)]));

    let mut handles = Vec::new();
    for (idx, account_id) in account_ids.into_iter().enumerate() {
        if !token.load(Ordering::Relaxed) { break; }
        let token_clone = token.clone();
        let app_clone = app.clone();
        let cfg_clone = cfg.clone();
        handles.push(tokio::spawn(async move {
            if !token_clone.load(Ordering::Relaxed) { return; }
            let prefix = format!("[{}/{}]", idx + 1, total);
            let emit = |msg: String| { let _ = app_clone.emit("forwarder-log", format!("{} {}", prefix, msg)); };

            let mut client = match connect_account(&account_id).await {
                Ok(c) => c,
                Err(e) => {
                    emit(t_with("forwarder_connect_error", &[("error", &e)]));
                    return;
                }
            };
            client.set_max_flood_wait(cfg_clone.max_flood_wait as u64);
            emit(t("forwarder_connected"));

            // resolve group (supports public links and private invites)
            let resolved = match crate::mtproto::invite::resolve_channel_link(&mut client, &cfg_clone.group_link).await {
                Ok(r) => r,
                Err(e) => { emit(t_with("forwarder_resolve_error", &[("error", &e)])); return; }
            };
            let group_id = resolved.channel_id;
            let group_hash = resolved.access_hash;
            emit(t_with("forwarder_group_resolved", &[("id", &group_id.to_string())]));
            let input_channel = tl_gen::serialize_input_channel(group_id, group_hash);

            // get initial state
            let state_req = tl_gen::build_updates_getState();
            let (mut pts, mut date, mut qts) = match client.invoke(&state_req).await {
                Ok(data) => match tl_gen::parse_updates_getState(&data) {
                    Ok(s) => (s.pts, s.date, s.qts),
                    Err(e) => { emit(t_with("forwarder_getstate_parse_error", &[("error", &e)])); return; }
                },
                Err(e) => { emit(t_with("forwarder_getstate_error", &[("error", &e.to_string())])); return; }
            };

            let mut forward_map: HashMap<i32, ForwardedInfo> = HashMap::new();
            let forwarded_count = AtomicU32::new(0);
            let replied_count = AtomicU32::new(0);
            let mut joined = false;

            // Resend old unread PMs if enabled
            if cfg_clone.resend_old {
                emit(t("forwarder_resend_old"));
                let dialogs_req = tl::build_get_dialogs_with_folder(0, 100);
                if let Ok(dialogs_data) = client.invoke(&dialogs_req).await {
                    if let Ok(peers) = tl::parse_dialog_peers(&dialogs_data) {
                        for peer in &peers {
                            if !token_clone.load(Ordering::Relaxed) { break; }
                            if let tl::DialogPeer::User { id: uid, access_hash: uhash, is_bot } = peer {
                                if *is_bot { continue; }
                                if *uid == 777000 || *uid == 178220800 { continue; }
                                // get last message from this user
                                let hist_req = tl::build_get_history(*uid, *uhash, 1);
                                if let Ok(hist_data) = client.invoke(&hist_req).await {
                                    if let Ok(msgs) = tl::parse_messages_structured(&hist_data) {
                                        if let Some(msg) = msgs.first() {
                                            if msg.text.is_empty() { continue; }
                                            // delay before resending old
                                            let delay = random_delay_secs(cfg_clone.resend_old_wait_min, cfg_clone.resend_old_wait_max);
                                            interruptible_sleep_ms(delay as u64 * 1000, &token_clone).await;
                                            if !token_clone.load(Ordering::Relaxed) { break; }

                                            // join group if not yet
                                            if !joined {
                                                let join_req = tl_gen::build_channels_joinChannel(&input_channel);
                                                if client.invoke(&join_req).await.is_ok() { joined = true; }
                                            }

                                            // send modified message to group
                                            let modified_text = format!("{} #{}", msg.text, uid);
                                            let to_peer = tl_gen::serialize_input_peer_channel(group_id, group_hash);
                                            let rid: i64 = rand::random();
                                            let send_req = tl_gen::build_messages_sendMessage(
                                                false, false, false, false, false, false, false, false,
                                                &to_peer, None, &modified_text, rid, None, None, None, None, None, None, None, None, None, None,
                                            );
                                            if let Ok(resp) = client.invoke(&send_req).await {
                                                if let Some(fwd_id) = extract_forwarded_msg_id(&resp) {
                                                    forward_map.insert(fwd_id, ForwardedInfo { sender_id: *uid, sender_access_hash: *uhash });
                                                    forwarded_count.fetch_add(1, Ordering::Relaxed);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                emit(t_with("forwarder_old_forwarded", &[("count", &forwarded_count.load(Ordering::Relaxed).to_string())]));
            }

            // polling loop
            loop {
                if !token_clone.load(Ordering::Relaxed) {
                    emit(t("forwarder_stopped"));
                    break;
                }
                interruptible_sleep_ms(POLL_INTERVAL_MS, &token_clone).await;
                let diff_req = tl_gen::build_updates_getDifference(pts, None, None, date, qts, None);
                let diff_data = match client.invoke(&diff_req).await {
                    Ok(d) => d,
                    Err(e) => {
                        if crate::mtproto::is_fatal_session_error(&e) {
                            emit(t_with("forwarder_fatal_error", &[("error", &e)]));
                            break;
                        }
                        if e.contains("PERSISTENT_TIMESTAMP_EMPTY") || e.contains("PERSISTENT_TIMESTAMP_INVALID") {
                            if let Ok(sd) = client.invoke(&tl_gen::build_updates_getState()).await {
                                if let Ok(s) = tl_gen::parse_updates_getState(&sd) {
                                    pts = s.pts; date = s.date; qts = s.qts;
                                }
                            }
                            continue;
                        }
                        continue;
                    }
                };

                let diff = match tl_gen::parse_updates_getDifference(&diff_data) {
                    Ok(d) => d,
                    Err(_) => continue,
                };

                let (new_messages, other_updates, users, new_state) = match diff {
                    tl_gen::TlUpdatesDifference::Empty { date: d, .. } => { date = d; continue; }
                    tl_gen::TlUpdatesDifference::Difference { new_messages, other_updates, users, state, .. } => (new_messages, other_updates, users, Some(state)),
                    tl_gen::TlUpdatesDifference::Slice { new_messages, other_updates, users, intermediate_state, .. } => (new_messages, other_updates, users, Some(intermediate_state)),
                    tl_gen::TlUpdatesDifference::TooLong { pts: p } => { pts = p; continue; }
                };

                if let Some(state_raw) = new_state {
                    if let Ok(s) = tl_gen::TlUpdatesState::deserialize(&mut Cursor::new(state_raw.as_slice())) {
                        pts = s.pts; date = s.date; qts = s.qts;
                    }
                }

                let mut all_new_messages = new_messages;
                let all_users = users;

                // other_updates may contain updateNewChannelMessage / updateNewMessage
                for upd_raw in &other_updates {
                    if let Some(msg_bytes) = extract_message_from_update(upd_raw) {
                        all_new_messages.push(msg_bytes);
                    }
                }

                let user_map = build_user_access_hash_map(&all_users);
                let user_name_map = build_user_name_map(&all_users);
                let user_username_map = build_user_username_map(&all_users);

                for msg_raw in &all_new_messages {
                    if !token_clone.load(Ordering::Relaxed) { break; }
                    let msg = match tl_gen::TlMessage::deserialize(&mut Cursor::new(msg_raw.as_slice())) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    match msg {
                        tl_gen::TlMessage::Message { out, from_id, peer_id, reply_to, message, media, entities, id, .. } => {
                            if out { continue; }

                            let peer = match tl_gen::TlPeer::deserialize(&mut Cursor::new(peer_id.as_slice())) {
                                Ok(p) => p,
                                Err(_) => continue,
                            };

                            match peer {
                                tl_gen::TlPeer::User { user_id: sender_user_id } => {
                                    // Incoming PM — forward to group
                                    let sender_from = if let Some(ref fid) = from_id {
                                        match tl_gen::TlPeer::deserialize(&mut Cursor::new(fid.as_slice())) {
                                            Ok(tl_gen::TlPeer::User { user_id }) => user_id,
                                            _ => sender_user_id,
                                        }
                                    } else { sender_user_id };
                                    let sender_hash = user_map.get(&sender_from).copied().unwrap_or(0);
                                    if sender_hash == 0 { continue; }

                                    // join group before first forward
                                    if !joined {
                                        let join_req = tl_gen::build_channels_joinChannel(&input_channel);
                                        match client.invoke(&join_req).await {
                                            Ok(_) => {
                                                joined = true;
                                                emit(t("forwarder_subscribed"));
                                            }
                                            Err(e) if e.contains("CHANNEL_INVALID") => {
                                                emit(t("forwarder_subscribe_failed_perm"));
                                                break;
                                            }
                                            Err(e) => {
                                                emit(t_with("forwarder_subscribe_failed", &[("error", &e.to_string())]));
                                                continue;
                                            }
                                        }
                                    }

                                    // delay before forwarding PM to group
                                    let mess_delay = random_delay_secs(cfg_clone.mess_wait_min, cfg_clone.mess_wait_max);
                                    if mess_delay > 0 {
                                        interruptible_sleep_ms(mess_delay as u64 * 1000, &token_clone).await;
                                        if !token_clone.load(Ordering::Relaxed) { break; }
                                    }

                                    // get sender info for text modification
                                    let sender_name = user_name_map.get(&sender_from).cloned().unwrap_or_default();
                                    let sender_uname = user_username_map.get(&sender_from).cloned().unwrap_or_default();

                                    // send modified message to group (like Python: text + first_name @username #sender_id)
                                    let modified_text = format!(
                                        "{} {} @{} #{}",
                                        message,
                                        sender_name,
                                        if sender_uname.is_empty() { "-" } else { &sender_uname },
                                        sender_from
                                    );
                                    let to_peer = tl_gen::serialize_input_peer_channel(group_id, group_hash);
                                    let rid: i64 = rand::random();
                                    let send_req = tl_gen::build_messages_sendMessage(
                                        false, false, false, false, false, false, false, false,
                                        &to_peer, None, &modified_text, rid, None, None, None, None, None, None, None, None, None, None,
                                    );
                                    match client.invoke(&send_req).await {
                                        Ok(resp) => {
                                            if let Some(fwd_id) = extract_forwarded_msg_id(&resp) {
                                                forward_map.insert(fwd_id, ForwardedInfo { sender_id: sender_from, sender_access_hash: sender_hash });
                                                forwarded_count.fetch_add(1, Ordering::Relaxed);
                                                emit(t_with("forwarder_forwarded", &[("id", &fwd_id.to_string())]));
                                            } else {
                                                emit(t("forwarder_forwarded_no_id"));
                                            }
                                        }
                                        Err(e) if e.contains("CHAT_WRITE_FORBIDDEN") => {
                                            emit(t("forwarder_write_forbidden"));
                                        }
                                        Err(e) => { emit(t_with("forwarder_forward_error", &[("error", &e.to_string())])); }
                                    }
                                }
                                tl_gen::TlPeer::Channel { channel_id: chan_id } => {
                                    // Message in group — check if it's a reply to our forward
                                    if chan_id != group_id { continue; }

                                    // check if this is a reply to one of our forwards
                                    let reply_to_id = match &reply_to {
                                        Some(rt) => parse_reply_to_msg_id(rt),
                                        None => None,
                                    };
                                    let reply_to_id = match reply_to_id {
                                        Some(id) => id,
                                        None => continue,
                                    };
                                    let info = match forward_map.get(&reply_to_id) {
                                        Some(i) => i,
                                        None => continue,
                                    };

                                // send_wait delay before sending reply
                                let send_delay = random_delay_secs(cfg_clone.send_wait_min, cfg_clone.send_wait_max);
                                if send_delay > 0 {
                                    interruptible_sleep_ms(send_delay as u64 * 1000, &token_clone).await;
                                    if !token_clone.load(Ordering::Relaxed) { break; }
                                }

                                // typing simulation
                                let user_peer = tl_gen::serialize_input_peer_user(info.sender_id, info.sender_access_hash);
                                let typing_action = serialize_typing_action();
                                let typing_req = tl_gen::build_messages_setTyping(&user_peer, None, &typing_action);
                                let _ = client.invoke(&typing_req).await;

                                let typing_secs = random_delay_secs(cfg_clone.typing_min, cfg_clone.typing_max);
                                interruptible_sleep_ms(typing_secs as u64 * 1000, &token_clone).await;
                                if !token_clone.load(Ordering::Relaxed) { break; }

                                // copy message to sender's DM
                                let copy_result = copy_message_to_user(
                                    &mut client, info.sender_id, info.sender_access_hash,
                                    &message, &media, &entities,
                                ).await;
                                match copy_result {
                                    Ok(_) => {
                                        replied_count.fetch_add(1, Ordering::Relaxed);
                                        emit(t_with("forwarder_reply_copied", &[("user_id", &info.sender_id.to_string())]));
                                    }
                                    Err(e) => { emit(t_with("forwarder_copy_error", &[("error", &e)])); }
                                }

                                // send reaction if enabled
                                if cfg_clone.send_reaction {
                                    let group_peer = tl_gen::serialize_input_peer_channel(group_id, group_hash);
                                    let reaction_bytes = tl_gen::serialize_reactionEmoji("🕊");
                                    let reaction_refs: &[&[u8]] = &[&reaction_bytes];
                                    let react_req = tl_gen::build_messages_sendReaction(false, true, &group_peer, id, Some(reaction_refs));
                                    let _ = client.invoke(&react_req).await;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }

            if joined && cfg_clone.leave_on_stop {
                let leave_req = tl::build_leave_channel(group_id, group_hash);
                if let Err(e) = client.invoke(&leave_req).await {
                    emit(t_with("forwarder_leave_error", &[("error", &e.to_string())]));
                } else {
                    emit(t("forwarder_left_group"));
                }
            }
        }));
    }
    for h in handles { let _ = h.await; }
    Ok(())
}
async fn copy_message_to_user(
    client: &mut MtpClient,
    user_id: i64,
    access_hash: i64,
    text: &str,
    media: &Option<Vec<u8>>,
    entities: &Option<Vec<Vec<u8>>>,
) -> Result<(), String> {
    let peer = tl_gen::serialize_input_peer_user(user_id, access_hash);
    let random_id: i64 = rand::random();

    if let Some(media_raw) = media {
        // has media — parse and send via sendMedia
        if let Some(input_media) = parse_media_to_input(media_raw) {
            let ent_refs: Vec<&[u8]> = entities.as_ref().map(|v| v.iter().map(|e| e.as_slice()).collect()).unwrap_or_default();
            let ent_opt: Option<&[&[u8]]> = if ent_refs.is_empty() { None } else { Some(&ent_refs) };
            let req = tl_gen::build_messages_sendMedia(
                false, false, false, false, false, false, false,
                &peer, None, &input_media, text, random_id,
                None, ent_opt, None, None, None, None, None, None, None,
            );
            client.invoke(&req).await.map_err(|e| format!("sendMedia: {e}"))?;
        } else if !text.is_empty() {
            // couldn't parse media, send text only
            send_text_message(client, &peer, text, entities, random_id).await?;
        }
    } else if !text.is_empty() {
        // text only
        send_text_message(client, &peer, text, entities, random_id).await?;
    }
    Ok(())
}

async fn send_text_message(
    client: &mut MtpClient,
    peer: &[u8],
    text: &str,
    entities: &Option<Vec<Vec<u8>>>,
    random_id: i64,
) -> Result<(), String> {
    let ent_refs: Vec<&[u8]> = entities.as_ref().map(|v| v.iter().map(|e| e.as_slice()).collect()).unwrap_or_default();
    let ent_opt: Option<&[&[u8]]> = if ent_refs.is_empty() { None } else { Some(&ent_refs) };
    let req = tl_gen::build_messages_sendMessage(
        false, false, false, false, false, false, false, false,
        peer, None, text, random_id, None, ent_opt, None, None, None, None, None, None, None, None,
    );
    client.invoke(&req).await.map_err(|e| format!("sendMessage: {e}"))?;
    Ok(())
}
/// Parse MessageMedia bytes and build inputMedia for sendMedia
fn parse_media_to_input(media_raw: &[u8]) -> Option<Vec<u8>> {
    let media = tl_gen::TlMessageMedia::deserialize(&mut Cursor::new(media_raw)).ok()?;
    match media {
        tl_gen::TlMessageMedia::Photo { photo: Some(photo_raw), .. } => {
            let photo = tl_gen::TlPhoto::deserialize(&mut Cursor::new(photo_raw.as_slice())).ok()?;
            match photo {
                tl_gen::TlPhoto::Photo { id, access_hash, file_reference, .. } => {
                    let input_photo = tl_gen::serialize_inputPhoto(id, access_hash, &file_reference);
                    Some(tl_gen::serialize_inputMediaPhoto(false, false, &input_photo, None, None))
                }
                _ => None,
            }
        }
        tl_gen::TlMessageMedia::Document { document: Some(doc_raw), .. } => {
            let doc = tl_gen::TlDocument::deserialize(&mut Cursor::new(doc_raw.as_slice())).ok()?;
            match doc {
                tl_gen::TlDocument::Document { id, access_hash, file_reference, .. } => {
                    let input_doc = tl_gen::serialize_inputDocument(id, access_hash, &file_reference);
                    Some(tl_gen::serialize_inputMediaDocument(false, &input_doc, None, None, None, None))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn extract_forwarded_msg_id(response: &[u8]) -> Option<i32> {
    let inner = tl_gen::unwrap_rpc(response).ok()?;
    let mut cursor = Cursor::new(inner.as_slice());
    let updates = tl_gen::TlUpdates::deserialize(&mut cursor).ok()?;
    match updates {
        tl_gen::TlUpdates::Updates { updates, .. } => {
            for upd_raw in &updates {
                if let Some(msg_id) = extract_msg_id_from_update(upd_raw) {
                    return Some(msg_id);
                }
            }
            None
        }
        tl_gen::TlUpdates::HortMessage { id, .. } => Some(id),
        tl_gen::TlUpdates::HortChatMessage { id, .. } => Some(id),
        tl_gen::TlUpdates::HortSentMessage { id, .. } => Some(id),
        _ => None,
    }
}

fn extract_msg_id_from_update(upd_raw: &[u8]) -> Option<i32> {
    let update = tl_gen::TlUpdate::deserialize(&mut Cursor::new(upd_raw)).ok()?;
    match update {
        tl_gen::TlUpdate::NewMessage { message, .. } |
        tl_gen::TlUpdate::NewChannelMessage { message, .. } => {
            let msg = tl_gen::TlMessage::deserialize(&mut Cursor::new(message.as_slice())).ok()?;
            match msg {
                tl_gen::TlMessage::Message { id, .. } => Some(id),
                _ => None,
            }
        }
        _ => None,
    }
}

fn extract_message_from_update(upd_raw: &[u8]) -> Option<Vec<u8>> {
    let update = tl_gen::TlUpdate::deserialize(&mut Cursor::new(upd_raw)).ok()?;
    match update {
        tl_gen::TlUpdate::NewMessage { message, .. } |
        tl_gen::TlUpdate::NewChannelMessage { message, .. } => Some(message),
        _ => None,
    }
}
fn parse_reply_to_msg_id(reply_to_raw: &[u8]) -> Option<i32> {
    let mut cursor = Cursor::new(reply_to_raw);
    let header = tl_gen::TlMessageReplyHeader::deserialize(&mut cursor).ok()?;
    match header {
        tl_gen::TlMessageReplyHeader::MessageReplyHeader { reply_to_msg_id, .. } => reply_to_msg_id,
        _ => None,
    }
}

fn serialize_typing_action() -> Vec<u8> {
    tl_gen::SEND_MESSAGE_TYPING_ACTION.to_le_bytes().to_vec()
}

fn random_delay_secs(min: u32, max: u32) -> u32 {
    if min == 0 && max == 0 { return 0; }
    let lo = min.min(max);
    let hi = min.max(max);
    if lo == hi { return lo; }
    lo + (rand::random::<u32>() % (hi - lo + 1))
}

fn build_user_access_hash_map(users: &[Vec<u8>]) -> HashMap<i64, i64> {
    let mut map = HashMap::new();
    for user_raw in users {
        if let Ok(tl_gen::TlUser::User { id, access_hash: Some(hash), .. }) =
            tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(user_raw)
        {
            map.insert(id, hash);
        }
    }
    map
}

fn build_user_name_map(users: &[Vec<u8>]) -> HashMap<i64, String> {
    let mut map = HashMap::new();
    for user_raw in users {
        if let Ok(tl_gen::TlUser::User { id, first_name, .. }) =
            tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(user_raw)
        {
            map.insert(id, first_name.unwrap_or_default());
        }
    }
    map
}

fn build_user_username_map(users: &[Vec<u8>]) -> HashMap<i64, String> {
    let mut map = HashMap::new();
    for user_raw in users {
        if let Ok(tl_gen::TlUser::User { id, username, .. }) =
            tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(user_raw)
        {
            map.insert(id, username.unwrap_or_default());
        }
    }
    map
}

fn emit(app: &tauri::AppHandle, msg: String) {
    let _ = app.emit("forwarder-log", msg);
}
