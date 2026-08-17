// interceptor: monitor target groups/channels for keywords, apply replacements, forward to destination
// group mode: join targets + destination group, monitor, forward, leave after work
// channel mode: main account promotes workers to admin in destination channel,
//               workers join targets + destination, monitor, forward, leave after work
//               main account stays in destination channel

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use serde::Deserialize;
use tauri::{Emitter, Manager};

use crate::accounts::commands::get_storage_pub;
use crate::accounts::devices;
use crate::accounts::session::AccountJson;
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::mtproto::tl_gen;
use crate::mtproto::invite::resolve_channel_link;
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

/// Emulates online activity during delay (like Python's DelayOnline)
async fn delay_online(client: &mut MtpClient, ms: u64, token: &Arc<AtomicBool>) {
    let online_req = tl_gen::build_account_updateStatus(false);
    let _ = client.invoke(&online_req).await;
    interruptible_sleep(ms, token).await;
    let offline_req = tl_gen::build_account_updateStatus(true);
    let _ = client.invoke(&offline_req).await;
}

#[derive(Deserialize, Clone, Debug)]
pub struct InterceptorConfig {
    pub keywords: Vec<String>,
    pub targets: Vec<String>,
    pub replacements: String,
    pub destinations: Vec<String>,
    pub mode: String,              // "group" | "channel"
    pub send_mode: String,         // "copy" | "repost"
    pub admin_account_id: String,
    pub max_flood_wait: u64,
    pub poll_interval: u64,
    #[serde(default = "default_true")]
    pub revoke_admin_after: bool,
    #[serde(default = "default_true")]
    pub leave_after_work: bool,
}

fn default_true() -> bool { true }

struct JoinedTarget {
    channel_id: i64,
    access_hash: i64,
    joined_now: bool,
}

#[tauri::command]
pub async fn interceptor_start(
    ids: Vec<String>,
    config: InterceptorConfig,
    threads: Option<usize>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() { return Err(t("interceptor_no_accounts")); }
    let concurrency = threads.unwrap_or(ids.len()).max(1).min(100);
    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue.register_task(
        task_id.clone(),
        "interceptor".to_string(),
        t_with("interceptor_task_name", &[("count", &ids.len().to_string())]),
    ).await;

    let config = Arc::new(config);

    tokio::spawn(async move {
        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let total = ids.len();
        let mut handles = Vec::new();

        // if channel mode, process main account first to promote workers
        let main_id = if config.mode == "channel" && !config.admin_account_id.is_empty() {
            Some(config.admin_account_id.clone())
        } else { None };

        // collect user_ids for all accounts (needed for admin mode)
        let mut user_ids: HashMap<String, i64> = HashMap::new();
        if config.mode == "channel" && !config.admin_account_id.is_empty() {
            let _ = app.emit("interceptor-log", t("interceptor_collecting_ids"));
            for id in &ids {
                if !token.load(Ordering::Relaxed) { break; }
                match get_account_user_id(id).await {
                    Ok(uid) => { user_ids.insert(id.clone(), uid); }
                    Err(e) => {
                        let _ = app.emit("interceptor-log", t_with("interceptor_uid_error", &[("id", id), ("error", &e)]));
                    }
                }
            }
        }
        let user_ids = Arc::new(user_ids);

        // in channel mode, main account joins destination and promotes workers first
        let promoted: Vec<i64> = if let Some(ref mid) = main_id {
            match setup_main_account(mid, &ids, &user_ids, &config, &app, &token).await {
                Ok(promoted) => {
                    let _ = app.emit("interceptor-log", t_with("interceptor_main_setup", &[("count", &promoted.len().to_string())]));
                    promoted
                }
                Err(e) => {
                    let _ = app.emit("interceptor-log", t_with("interceptor_main_error", &[("error", &e)]));
                    Vec::new()
                }
            }
        } else { Vec::new() };
        let admin_promised = !promoted.is_empty();

        for (i, id) in ids.into_iter().enumerate() {
            if !token.load(Ordering::Relaxed) { break; }
            let is_main = main_id.as_ref() == Some(&id);

            if config.mode == "channel" && !is_main {
                if let Some(ref mid) = main_id {
                    if !admin_promised {
                        if let Err(e) = try_promote_single(mid, &id, &user_ids, &config, &app, &token).await {
                            let _ = app.emit("interceptor-log", t_with("interceptor_admin_skip", &[("idx", &(i+1).to_string()), ("total", &total.to_string()), ("error", &e)]));
                            continue;
                        }
                    }
                }
            }

            let sem = sem.clone();
            let config = config.clone();
            let app_clone = app.clone();
            let token_clone = token.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                if !token_clone.load(Ordering::Relaxed) { return; }
                let result = process_account(
                    &id, i + 1, total, &config,
                    is_main, admin_promised,
                    &app_clone, &token_clone,
                ).await;
                if let Err(e) = result {
                    crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                    let _ = app_clone.emit("interceptor-log", t_with("interceptor_thread_error", &[("idx", &(i+1).to_string()), ("total", &total.to_string()), ("error", &e)]));
                }
            }));
        }
        for h in handles { let _ = h.await; }

        // channel mode: main account revokes admin from workers after all done
        if config.mode == "channel" && admin_promised && config.revoke_admin_after {
            if let Some(ref mid) = main_id {
                let _ = revoke_all_admins(mid, &promoted, &config, &app).await;
            }
        }

        let _ = app.emit("interceptor-log", t("done"));
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn interceptor_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn get_account_user_id(id: &str) -> Result<i64, String> {
    let storage = get_storage_pub();
    let json_path = storage.json_path(id);
    let json = if json_path.exists() { AccountJson::from_file(&json_path).unwrap_or_default() } else { AccountJson::default() };
    if json.user_id > 0 {
        return Ok(json.user_id);
    }
    let mut client = crate::accounts::connect::connect_account(id).await?;
    let cfg = crate::get_app_config();
    let app_id = if json.app_id == 0 { cfg.app_id } else { json.app_id };
    let dev = devices::generate_random_device();
    let get_me = tl::build_get_me_request(app_id, &dev.device, &dev.sdk, &dev.app_version, "en", "en");
    let resp = client.invoke(&get_me).await.map_err(|e| format!("get_me: {e}"))?;
    let info = tl::parse_users_response(&resp).map_err(|e| format!("parse me: {e}"))?;
    Ok(info.id)
}

fn parse_replacements(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        // support escaped colons in the left side: "\:" -> ":"
        let temp = line.replace("\\:", "\x01");
        if let Some((from_raw, to_raw)) = temp.split_once(':') {
            let from = from_raw.replace('\x01', ":").trim().to_string();
            let to = to_raw.replace('\x01', ":").trim().to_string();
            if !from.is_empty() {
                out.push((from, to));
            }
        }
    }
    out
}

fn apply_replacements(text: &str, replacements: &[(String, String)]) -> (String, bool) {
    if replacements.is_empty() || text.is_empty() {
        return (text.to_string(), false);
    }
    let mut current = text.to_string();
    let mut changed = false;
    for (from, to) in replacements {
        if from.is_empty() { continue; }
        if current.contains(from.as_str()) {
            current = current.replace(from.as_str(), to.as_str());
            changed = true;
        }
    }
    (current, changed)
}

fn contains_keyword(text: &str, keywords: &[String]) -> bool {
    let lower = text.to_lowercase();
    keywords.iter().any(|kw| lower.contains(&kw.to_lowercase()))
}

async fn try_promote_single(
    main_id: &str,
    worker_id: &str,
    user_ids: &HashMap<String, i64>,
    config: &InterceptorConfig,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    if !token.load(Ordering::Relaxed) { return Ok(()); }
    let user_id = user_ids.get(worker_id).copied().ok_or(t("interceptor_no_uid"))?;
    let emit = |msg: String| { let _ = app.emit("interceptor-log", format!("{} {}", t("interceptor_main_prefix"), msg)); };

    let mut client = crate::accounts::connect::connect_account(main_id).await?;
    client.set_max_flood_wait(config.max_flood_wait);

    let admin_rights = tl_gen::serialize_chatAdminRights(
        false, true, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    );

    for dest_link in &config.destinations {
        let dest = resolve_channel_link(&mut client, dest_link).await
            .map_err(|e| format!("dest {}: {e}", dest_link))?;
        let channel_input = tl_gen::serialize_input_channel(dest.channel_id, dest.access_hash);
        let user_input = tl_gen::serialize_input_user(user_id, 0);
        let req = tl_gen::build_channels_editAdmin(&channel_input, &user_input, &admin_rights, None);
        match client.invoke(&req).await {
            Ok(_) => emit(t_with("interceptor_admin_granted", &[("uid", &user_id.to_string()), ("dest", dest_link)])),
            Err(e) => {
                emit(t_with("interceptor_admin_error", &[("uid", &user_id.to_string()), ("dest", dest_link), ("error", &e)]));
                return Err(e);
            }
        }
    }
    Ok(())
}

async fn setup_main_account(
    main_id: &str,
    worker_ids: &[String],
    user_ids: &HashMap<String, i64>,
    config: &InterceptorConfig,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<Vec<i64>, String> {
    let emit = |msg: String| { let _ = app.emit("interceptor-log", format!("{} {}", t("interceptor_main_prefix"), msg)); };

    let mut client = crate::accounts::connect::connect_account(main_id).await?;
    client.set_max_flood_wait(config.max_flood_wait);

    // resolve and join all destinations
    let mut dest_channels: Vec<(i64, i64)> = Vec::new();
    for dest_link in &config.destinations {
        match resolve_channel_link(&mut client, dest_link).await {
            Ok(dest) => {
                emit(t_with("interceptor_target_channel", &[("id", &dest.channel_id.to_string()), ("title", &dest.title_hint)]));
                if dest.joined_now {
                    emit(t("interceptor_joined_channel"));
                }
                dest_channels.push((dest.channel_id, dest.access_hash));
            }
            Err(e) => {
                emit(t_with("interceptor_assign_error", &[("dest", dest_link), ("error", &e)]));
            }
        }
    }

    if dest_channels.is_empty() {
        return Err(t("interceptor_no_dest_joined"));
    }

    // promote each worker to admin with post_messages in all dest channels
    let admin_rights = tl_gen::serialize_chatAdminRights(
        false, true, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    );
    let mut promoted: Vec<i64> = Vec::new();

    for wid in worker_ids {
        if wid == main_id { continue; }
        if !token.load(Ordering::Relaxed) { break; }
        let user_id = match user_ids.get(wid) {
            Some(&uid) => uid,
            None => {
                emit(t_with("interceptor_unknown_uid", &[("wid", wid)]));
                continue;
            }
        };
        let mut all_ok = true;
        for (ch_id, ch_hash) in &dest_channels {
            let channel_input = tl_gen::serialize_input_channel(*ch_id, *ch_hash);
            let user_input = tl_gen::serialize_input_user(user_id, 0);
            let req = tl_gen::build_channels_editAdmin(&channel_input, &user_input, &admin_rights, None);
            match client.invoke(&req).await {
                Ok(_) => emit(t_with("interceptor_admin_granted_id", &[("uid", &user_id.to_string()), ("id", &ch_id.to_string())])),
                Err(e) => {
                    emit(t_with("interceptor_admin_error_id", &[("uid", &user_id.to_string()), ("id", &ch_id.to_string()), ("error", &e)]));
                    all_ok = false;
                }
            }
        }
        if all_ok {
            promoted.push(user_id);
        }
    }

    Ok(promoted)
}

async fn revoke_all_admins(
    main_id: &str,
    promoted: &[i64],
    config: &InterceptorConfig,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    let emit = |msg: String| { let _ = app.emit("interceptor-log", format!("{} {}", t("interceptor_main_prefix"), msg)); };

    if promoted.is_empty() {
        emit(t("interceptor_nobody_revoke"));
        return Ok(());
    }

    let main_user_id = match get_account_user_id(main_id).await {
        Ok(uid) => uid,
        Err(e) => {
            emit(t_with("interceptor_main_uid_error", &[("error", &e)]));
            return Ok(());
        }
    };

    let Ok(mut client) = crate::accounts::connect::connect_account(main_id).await else { return Ok(()); };
    let no_rights = tl_gen::serialize_chatAdminRights(
        false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    );

    emit(t("interceptor_revoking"));

    let promoted_set: HashSet<i64> = promoted.iter().copied().collect();
    let mut revoked = 0u32;

    for dest_link in &config.destinations {
        let Ok(dest) = resolve_channel_link(&mut client, dest_link).await else { continue; };
        for user_id in &promoted_set {
            if *user_id == main_user_id { continue; }
            let channel_input = tl_gen::serialize_input_channel(dest.channel_id, dest.access_hash);
            let user_input = tl_gen::serialize_input_user(*user_id, 0);
            let req = tl_gen::build_channels_editAdmin(&channel_input, &user_input, &no_rights, None);
            match client.invoke(&req).await {
                Ok(_) => revoked += 1,
                Err(e) => emit(t_with("interceptor_revoke_error", &[("uid", &user_id.to_string()), ("error", &e)])),
            }
        }
    }

    emit(t_with("interceptor_revoked", &[("count", &revoked.to_string())]));
    Ok(())
}

async fn invoke_with_flood_wait(
    client: &mut MtpClient,
    request: &[u8],
    max_flood_wait: u64,
    token: &Arc<AtomicBool>,
) -> Result<Vec<u8>, String> {
    for _attempt in 0..3 {
        match client.invoke(request).await {
            Ok(data) => return Ok(data),
            Err(e) => {
                if let Some(wait) = parse_flood_wait(&e) {
                    if max_flood_wait == 0 || wait <= max_flood_wait {
                        delay_online(client, wait * 1000, token).await;
                        if !token.load(Ordering::Relaxed) { return Err("stopped".into()); }
                        continue;
                    }
                }
                return Err(e);
            }
        }
    }
    Err("retries exhausted".into())
}

fn parse_flood_wait(err: &str) -> Option<u64> {
    let message = err.strip_prefix("RPC ").and_then(|s| s.split_once(": ").map(|(_, m)| m)).unwrap_or(err);
    let rpc_err = tl_gen::RpcError { code: 0, message: message.to_string() };
    rpc_err.flood_seconds().map(|s| s as u64)
}

async fn process_account(
    id: &str,
    idx: usize,
    total: usize,
    config: &InterceptorConfig,
    is_main: bool,
    _admin_promised: bool,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    let emit = |msg: String| { let _ = app.emit("interceptor-log", format!("[{}/{}] {}", idx, total, msg)); };
    let _prefix = format!("[{}/{}]", idx, total);

    let mut client = crate::accounts::connect::connect_account(id).await?;
    client.set_log_target("interceptor-log", app.clone());
    client.set_max_flood_wait(config.max_flood_wait);

    // join all targets
    let mut joined_targets: Vec<JoinedTarget> = Vec::new();
    let mut target_ids: HashSet<i64> = HashSet::new();

    for target_link in &config.targets {
        if !token.load(Ordering::Relaxed) { break; }
        match resolve_channel_link(&mut client, target_link).await {
            Ok(resolved) => {
                if resolved.joined_now {
                    emit(t_with("interceptor_joined", &[("target", target_link), ("id", &resolved.channel_id.to_string())]));
                }
                target_ids.insert(resolved.channel_id);
                joined_targets.push(JoinedTarget {
                    channel_id: resolved.channel_id,
                    access_hash: resolved.access_hash,
                    joined_now: resolved.joined_now,
                });
            }
            Err(e) => {
                if e.contains("заявка на вступление") || e.contains("JOIN_REQUEST") || e.contains("request") {
                    emit(t_with("interceptor_needs_request", &[("target", target_link)]));
                    continue;
                }
                emit(t_with("interceptor_join_failed", &[("target", target_link), ("error", &e)]));
            }
        }
    }

    if joined_targets.is_empty() {
        return Err(t("interceptor_no_groups_joined"));
    }

    // join all destinations
    struct JoinedDest {
        channel_id: i64,
        access_hash: i64,
        joined_now: bool,
    }
    let mut joined_dests: Vec<JoinedDest> = Vec::new();

    for dest_link in &config.destinations {
        if !token.load(Ordering::Relaxed) { break; }
        match resolve_channel_link(&mut client, dest_link).await {
            Ok(resolved) => {
                if resolved.joined_now {
                    emit(t_with("interceptor_joined_dest", &[("dest", dest_link), ("id", &resolved.channel_id.to_string())]));
                }
                joined_dests.push(JoinedDest {
                    channel_id: resolved.channel_id,
                    access_hash: resolved.access_hash,
                    joined_now: resolved.joined_now,
                });
            }
            Err(e) => {
                emit(t_with("interceptor_join_dest_failed", &[("dest", dest_link), ("error", &e)]));
            }
        }
    }

    if joined_dests.is_empty() {
        return Err(t("interceptor_no_dest_joined2"));
    }

    // build set of destination IDs to exclude from monitoring
    let dest_ids: HashSet<i64> = joined_dests.iter().map(|d| d.channel_id).collect();
    // remove destinations from target_ids (in case user put same channel in both)
    let target_ids: HashSet<i64> = target_ids.difference(&dest_ids).copied().collect();

    let replacements = parse_replacements(&config.replacements);
    let keywords: Vec<String> = config.keywords.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();

    if keywords.is_empty() {
        return Err(t("interceptor_no_keywords"));
    }

    // get initial state
    let (mut pts, mut date, mut qts) = {
        let state_req = tl_gen::build_updates_getState();
        let state_data = client.invoke(&state_req).await
            .map_err(|e| format!("getState: {e}"))?;
        let state = tl_gen::parse_updates_getState(&state_data)
            .map_err(|e| format!("parse state: {e}"))?;
        (state.pts, state.date, state.qts)
    };

    emit(t_with("interceptor_monitoring", &[("count", &joined_targets.len().to_string())]));

    let mut intercepted = 0u32;

    loop {
        if !token.load(Ordering::Relaxed) {
            emit(t("interceptor_stopped"));
            break;
        }
        interruptible_sleep(config.poll_interval.max(500).min(15000), token).await;

        let diff_req = tl_gen::build_updates_getDifference(pts, None, None, date, qts, None);
        let diff_data = match client.invoke(&diff_req).await {
            Ok(d) => d,
            Err(e) => {
                if crate::mtproto::is_fatal_session_error(&e) {
                    emit(t_with("interceptor_fatal_error", &[("error", &e)]));
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

        for upd_raw in &other_updates {
            if let Some(msg_bytes) = extract_message_from_update(upd_raw) {
                all_new_messages.push(msg_bytes);
            }
        }

        let _user_map = build_user_access_hash_map(&all_users);

        for msg_raw in &all_new_messages {
            if !token.load(Ordering::Relaxed) { break; }
            let msg = match tl_gen::TlMessage::deserialize(&mut Cursor::new(msg_raw.as_slice())) {
                Ok(m) => m,
                Err(_) => continue,
            };
            match msg {
                tl_gen::TlMessage::Message { out, peer_id, message: msg_text, media, from_id, id: msg_id, .. } => {
                    // skip own outgoing messages
                    if out { continue; }

                    let peer = match tl_gen::TlPeer::deserialize(&mut Cursor::new(peer_id.as_slice())) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };

                    let msg_channel_id = match peer {
                        tl_gen::TlPeer::Channel { channel_id } => channel_id,
                        _ => continue,
                    };

                    if !target_ids.contains(&msg_channel_id) { continue; }
                    if msg_text.is_empty() && media.is_none() { continue; }

                    let text_to_check = msg_text.as_str();
                    if !contains_keyword(text_to_check, &keywords) { continue; }

                    // resolve sender username from from_id + user map
                    let sender_tag = if let Some(ref fid_bytes) = from_id {
                        if let Ok(from_peer) = tl_gen::TlPeer::deserialize(&mut Cursor::new(fid_bytes.as_slice())) {
                            match from_peer {
                                tl_gen::TlPeer::User { user_id } => {
                                    if let Some(info) = _user_map.get(&user_id) {
                                        if !info.username.is_empty() {
                                            format!("@{}", info.username)
                                        } else if !info.first_name.is_empty() {
                                            info.first_name.clone()
                                        } else {
                                            format!("id:{}", user_id)
                                        }
                                    } else {
                                        format!("id:{}", user_id)
                                    }
                                }
                                tl_gen::TlPeer::Channel { channel_id } => format!("ch:{}", channel_id),
                                tl_gen::TlPeer::Chat { chat_id } => format!("chat:{}", chat_id),
                            }
                        } else {
                            "?".to_string()
                        }
                    } else {
                        "?".to_string()
                    };

                    intercepted += 1;
                    emit(t_with("interceptor_intercepted", &[("msg_id", &msg_id.to_string()), ("channel_id", &msg_channel_id.to_string()), ("sender", &sender_tag)]));

                    // apply replacements
                    let (replaced_text, has_replacement) = apply_replacements(text_to_check, &replacements);

                    // find access_hash for source channel
                    let src_access_hash = joined_targets.iter()
                        .find(|t| t.channel_id == msg_channel_id)
                        .map(|t| t.access_hash)
                        .unwrap_or(0);

                    // send to all destinations
                    let mut sent_count = 0u32;
                    for dest in &joined_dests {
                        if !token.load(Ordering::Relaxed) { break; }
                        let dest_peer = tl_gen::serialize_input_peer_channel(dest.channel_id, dest.access_hash);

                        let use_forward = config.send_mode == "repost" && !has_replacement;

                        if use_forward {
                            // forward original message (with author)
                            let from_peer = tl_gen::serialize_input_peer_channel(msg_channel_id, src_access_hash);
                            let random_ids: Vec<i64> = vec![rand::random()];
                            let fwd_req = tl_gen::build_messages_forwardMessages(
                                false, false, false, false, false, false, false,
                                &from_peer, &[msg_id], &random_ids, &dest_peer,
                                None, None, None, None, None, None, None, None, None, None,
                            );
                            match invoke_with_flood_wait(&mut client, &fwd_req, config.max_flood_wait, token).await {
                                Ok(_) => sent_count += 1,
                                Err(e) => emit(t_with("interceptor_forward_error", &[("id", &dest.channel_id.to_string()), ("error", &e)])),
                            }
                        } else {
                            // send as own message (copy mode or has replacements)
                            let text = if has_replacement { &replaced_text } else { text_to_check };
                            let random_id: i64 = rand::random();
                            let send_req = tl_gen::build_messages_sendMessage(
                                false, false, false, false, true, false, false, false,
                                &dest_peer, None, text, random_id,
                                None, None, None, None, None, None, None, None, None,
                            );
                            match invoke_with_flood_wait(&mut client, &send_req, config.max_flood_wait, token).await {
                                Ok(_) => sent_count += 1,
                                Err(e) => emit(t_with("interceptor_send_error", &[("id", &dest.channel_id.to_string()), ("error", &e)])),
                            }
                        }
                    }
                    if sent_count > 0 {
                        emit(t_with("interceptor_forwarded", &[("sent", &sent_count.to_string()), ("total", &joined_dests.len().to_string()), ("text", &intercepted.to_string())]));
                    }
                    interruptible_sleep(500, token).await;
                }
                _ => {}
            }
        }

        if let Some(fatal) = client.fatal_error() {
            return Err(fatal.to_string());
        }
    }

    emit(t_with("interceptor_total", &[("count", &intercepted.to_string())]));

    // leave joined targets
    if config.leave_after_work {
        for t in &joined_targets {
            if t.joined_now {
                let channel = tl_gen::serialize_input_channel(t.channel_id, t.access_hash);
                let leave_req = tl_gen::build_channels_leaveChannel(&channel);
                let _ = client.invoke(&leave_req).await;
            }
        }
    }

    // leave destination (except main account in channel mode)
    if config.leave_after_work {
        if !(is_main && config.mode == "channel") {
            for dest in &joined_dests {
                if dest.joined_now {
                    let channel = tl_gen::serialize_input_channel(dest.channel_id, dest.access_hash);
                    let leave_req = tl_gen::build_channels_leaveChannel(&channel);
                    let _ = client.invoke(&leave_req).await;
                }
            }
        }
    }

    Ok(())
}

fn extract_message_from_update(upd_raw: &[u8]) -> Option<Vec<u8>> {
    let update = tl_gen::TlUpdate::deserialize(&mut Cursor::new(upd_raw)).ok()?;
    match update {
        tl_gen::TlUpdate::NewMessage { message, .. } |
        tl_gen::TlUpdate::NewChannelMessage { message, .. } => Some(message),
        _ => None,
    }
}

fn build_user_access_hash_map(users_raw: &[Vec<u8>]) -> HashMap<i64, UserInfo> {
    let mut map = HashMap::new();
    for raw in users_raw {
        if let Ok(user) = tl_gen::TlUser::deserialize(&mut Cursor::new(raw.as_slice())) {
            match user {
                tl_gen::TlUser::User { id, access_hash, username, first_name, .. } => {
                    map.insert(id, UserInfo {
                        access_hash: access_hash.unwrap_or(0),
                        username: username.unwrap_or_default(),
                        first_name: first_name.unwrap_or_default(),
                    });
                }
                _ => {}
            }
        }
    }
    map
}

struct UserInfo {
    #[allow(dead_code)]
    access_hash: i64,
    username: String,
    first_name: String,
}
