// user_lookup: resolve usernames/phones and collect user info into a txt file.
// multi-threaded: distributes targets across selected accounts round-robin.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use serde::Deserialize;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex as TokioMutex;

use crate::accounts::connect::connect_account;
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::mtproto::tl_gen;
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

#[derive(Deserialize, Clone, Debug)]
pub struct UserLookupConfig {
    pub input_path: String,
    pub output_path: String,

    pub save_name: bool,
    pub save_surname: bool,
    pub save_username: bool,
    pub save_phone: bool,
    pub save_nft_gifts: bool,
    pub save_personal_channel: bool,
}

#[tauri::command]
pub async fn user_lookup_start(
    ids: Vec<String>,
    config: UserLookupConfig,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() {
        return Err(t("user_lookup_no_accounts"));
    }

    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "user_lookup".to_string(),
            t_with("user_lookup_task_name", &[("count", &ids.len().to_string())]),
        )
        .await;

    let cfg = Arc::new(config);
    tokio::spawn(async move {
        let result = run(ids, cfg.clone(), &app, token.clone()).await;
        match &result {
            Ok(_) => { emit(&app, t("done")); }
            Err(e) => {
                emit(&app, format!("{}: {e}", t("error")));
                emit(&app, t("done"));
            }
        }
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn user_lookup_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn run(
    ids: Vec<String>,
    cfg: Arc<UserLookupConfig>,
    app: &tauri::AppHandle,
    token: Arc<AtomicBool>,
) -> Result<(), String> {
    let input_path = cfg.input_path.trim();
    if input_path.is_empty() {
        return Err(t("user_lookup_no_input_file"));
    }
    let lines = std::fs::read_to_string(input_path)
        .map_err(|e| t_with("user_lookup_read_file_error", &[("error", &e.to_string())]))?;
    let targets: Vec<String> = lines
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| l.chars().any(|c| c.is_alphanumeric()))
        .collect();

    if targets.is_empty() {
        return Err(t("user_lookup_file_empty"));
    }

    // deduplicate targets
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::new();
    for t in targets {
        let key = t.trim_start_matches('@').trim_start_matches('+').to_lowercase();
        if seen.insert(key) {
            deduped.push(t);
        }
    }
    let skipped = lines.lines().count() - deduped.len();
    if skipped > 0 {
        emit(app, t_with("user_lookup_duplicates_skipped", &[("count", &skipped.to_string())]));
    }
    let targets = deduped;

    let total = targets.len();
    let num_accounts = ids.len();
    emit(app, t_with("user_lookup_targets_loaded", &[("total", &total.to_string()), ("accounts", &num_accounts.to_string())]));

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
        .map_err(|e| t_with("user_lookup_open_output_error", &[("path", &output_path.display().to_string()), ("error", &e.to_string())]))?;
    let writer = Arc::new(TokioMutex::new(writer));

    let found = Arc::new(AtomicU32::new(0));
    let not_found = Arc::new(AtomicU32::new(0));

    // distribute targets round-robin across accounts
    let mut batches: Vec<Vec<(usize, String)>> = vec![Vec::new(); num_accounts];
    for (i, target) in targets.into_iter().enumerate() {
        batches[i % num_accounts].push((i, target));
    }

    let sem = Arc::new(tokio::sync::Semaphore::new(num_accounts));
    let mut handles = Vec::new();

    for (acc_idx, batch) in batches.into_iter().enumerate() {
        if batch.is_empty() { continue; }
        if !token.load(Ordering::Relaxed) { break; }

        let sem = sem.clone();
        let cfg = cfg.clone();
        let app_clone = app.clone();
        let token_clone = token.clone();
        let writer_clone = writer.clone();
        let found_clone = found.clone();
        let not_found_clone = not_found.clone();
        let account_id = ids[acc_idx].clone();
        let total_targets = total;

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            if !token_clone.load(Ordering::Relaxed) { return; }

            let result = process_account_batch(
                &account_id, batch, total_targets, &cfg,
                &app_clone, &token_clone, &writer_clone,
                &found_clone, &not_found_clone,
            ).await;

            if let Err(e) = result {
                crate::accounts::commands::check_and_mark_dead_session(&e, &account_id);
                let _ = app_clone.emit("user-lookup-log", t_with("user_lookup_account_error", &[("idx", &(acc_idx + 1).to_string()), ("error", &e.to_string())]));
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    let f = found.load(Ordering::Relaxed);
    let nf = not_found.load(Ordering::Relaxed);
    emit(app, t_with("user_lookup_result", &[("found", &f.to_string()), ("not_found", &nf.to_string()), ("path", &output_path.display().to_string())]));

    Ok(())
}

async fn process_account_batch(
    account_id: &str,
    batch: Vec<(usize, String)>,
    total: usize,
    cfg: &UserLookupConfig,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
    writer: &Arc<TokioMutex<std::fs::File>>,
    found: &Arc<AtomicU32>,
    not_found: &Arc<AtomicU32>,
) -> Result<(), String> {
    let mut client = connect_account(account_id).await?;

    for (idx, target) in batch {
        if !token.load(Ordering::Relaxed) { break; }

        let is_phone = target.chars().all(|c| c.is_ascii_digit() || c == '+');
        let clean_target = target.trim_start_matches('@');

        if is_phone {
            emit(app, t_with("user_lookup_progress_phone", &[("idx", &(idx+1).to_string()), ("total", &total.to_string()), ("target", &target)]));
        } else {
            emit(app, t_with("user_lookup_progress_username", &[("idx", &(idx+1).to_string()), ("total", &total.to_string()), ("target", clean_target)]));
        }

        let resolve_result = if is_phone {
            let phone = target.trim_start_matches('+');
            resolve_by_username_or_phone(&mut client, phone, true).await
        } else {
            resolve_by_username_or_phone(&mut client, clean_target, false).await
        };

        let (user_id, access_hash, mut info) = match resolve_result {
            Ok(tuple) => tuple,
            Err(e) => {
                if e.contains("пропускаем") || e.contains("skipping") {
                    emit(app, format!("  {e}"));
                } else {
                    not_found.fetch_add(1, Ordering::Relaxed);
                    let line = format_not_found_line(&target, cfg);
                    let mut w = writer.lock().await;
                    writeln!(w, "{line}").ok();
                    emit(app, t_with("user_lookup_not_found", &[("error", &e)]));
                }
                interruptible_sleep(500, token).await;
                continue;
            }
        };

        if !is_phone && info.username.is_empty() {
            info.username = clean_target.to_string();
        }

        let nft_links = if cfg.save_nft_gifts {
            match fetch_nft_gifts(&mut client, user_id, access_hash).await {
                Ok(slugs) => slugs.into_iter()
                    .map(|s| {
                        if s.starts_with("https://") || s.starts_with("http://") { s }
                        else { format!("https://t.me/nft/{s}") }
                    })
                    .collect::<Vec<_>>(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };

        let personal_channel = if cfg.save_personal_channel {
            fetch_personal_channel(&mut client, user_id, access_hash).await.unwrap_or_default()
        } else {
            String::new()
        };

        found.fetch_add(1, Ordering::Relaxed);
        let line = format_found_line(&info, &nft_links, &personal_channel, cfg);
        {
            let mut w = writer.lock().await;
            writeln!(w, "{line}").ok();
            w.flush().ok();
        }

        emit(app, t_with("user_lookup_found", &[("first_name", &info.first_name), ("last_name", &info.last_name), ("username", &info.username)]));

        interruptible_sleep(800, token).await;
    }

    // surface a fatal session error even if a per-target arm swallowed it mid-loop
    if let Some(fatal) = client.fatal_error() {
        return Err(fatal.to_string());
    }
    Ok(())
}

async fn resolve_by_username_or_phone(
    client: &mut MtpClient,
    target: &str,
    is_phone: bool,
) -> Result<(i64, i64, tl::UserInfo), String> {
    if is_phone {
        resolve_by_phone(client, target).await
    } else {
        resolve_by_username(client, target).await
    }
}

async fn resolve_by_username(client: &mut MtpClient, username: &str) -> Result<(i64, i64, tl::UserInfo), String> {
    let req = tl::build_resolve_username(username);
    let data = client.invoke(&req).await.map_err(|e| {
        let es = e.to_string();
        if es.contains("USERNAME_NOT_OCCUPIED") || es.contains("USERNAME_INVALID") {
            return t_with("user_lookup_username_not_exists", &[("username", username)]);
        }
        t_with("user_lookup_resolve_username_error", &[("error", &e.to_string())])
    })?;
    let (id, hash) = tl::parse_resolved_peer(&data).map_err(|e| {
        if e.contains("USERNAME_NOT_OCCUPIED") || e.contains("USERNAME_INVALID") {
            return t_with("user_lookup_username_not_exists", &[("username", username)]);
        }
        if e.contains("channel not found") || e.contains("unexpected peer type") {
            return t_with("user_lookup_channel_skip", &[("username", username)]);
        }
        e
    })?;
    let info = parse_user_from_resolve_response(&data).map_err(|_| {
        t_with("user_lookup_channel_skip", &[("username", username)])
    })?;
    Ok((id, hash, info))
}

async fn resolve_by_phone(client: &mut MtpClient, phone: &str) -> Result<(i64, i64, tl::UserInfo), String> {
    let req = tl_gen::build_contacts_resolvePhone(phone);
    let data = client.invoke(&req).await.map_err(|e| {
        let es = e.to_string();
        if es.contains("PHONE_NOT_OCCUPIED") {
            return t_with("user_lookup_phone_not_registered", &[("phone", phone)]);
        }
        if es.contains("PHONE_NUMBER_INVALID") {
            return t_with("user_lookup_phone_invalid", &[("phone", phone)]);
        }
        t_with("user_lookup_resolve_phone_error", &[("error", &e.to_string())])
    })?;
    let (id, hash) = tl::parse_resolved_peer(&data).map_err(|e| {
        if e.contains("PHONE_NOT_OCCUPIED") {
            return t_with("user_lookup_phone_not_registered", &[("phone", phone)]);
        }
        if e.contains("PHONE_NUMBER_INVALID") {
            return t_with("user_lookup_phone_invalid", &[("phone", phone)]);
        }
        e
    })?;
    let info = parse_user_from_resolve_response(&data)
        .unwrap_or_else(|_| minimal_user_info(id));
    Ok((id, hash, info))
}

fn minimal_user_info(id: i64) -> tl::UserInfo {
    tl::UserInfo {
        id, first_name: String::new(), last_name: String::new(),
        phone: String::new(), username: String::new(), premium: false, nft_usernames: Vec::new(),
    }
}

fn parse_user_from_resolve_response(data: &[u8]) -> Result<tl::UserInfo, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let resolved = tl_gen::deserialize_tl_obj::<tl_gen::TlContactsResolvedPeer>(&inner)
        .map_err(|e| format!("resolvedPeer: {e}"))?;

    // the peer must resolve to a user (not a channel/group)
    let peer_user_id = {
        let mut pc = std::io::Cursor::new(resolved.peer.as_slice());
        use byteorder::{LittleEndian, ReadBytesExt};
        let ctor = pc.read_u32::<LittleEndian>().map_err(|_| "peer ctor")?;
        if ctor != tl_gen::PEER_USER {
            return Err(t("user_lookup_is_channel"));
        }
        pc.read_i64::<LittleEndian>().map_err(|_| "peer user_id")?
    };

    for raw in &resolved.users {
        if let Ok(info) = tl::parse_single_user(raw) {
            if info.id == peer_user_id { return Ok(info); }
        }
    }
    // fall back to the first parseable user if id matching failed
    for raw in &resolved.users {
        if let Ok(info) = tl::parse_single_user(raw) { return Ok(info); }
    }
    Err("could not find User in resolvedPeer response".into())
}

async fn fetch_nft_gifts(client: &mut MtpClient, user_id: i64, access_hash: i64) -> Result<Vec<String>, String> {
    let peer = tl_gen::serialize_input_peer_user(user_id, access_hash);
    let req = tl_gen::build_payments_getSavedStarGifts(
        false, false, false, false, false, false, false, false, false,
        &peer, None, "", 100,
    );
    let data = client.invoke(&req).await
        .map_err(|e| format!("getSavedStarGifts: {e}"))?;
    let (_, slugs) = tl::parse_saved_star_gifts(&data)?;
    Ok(slugs)
}

async fn fetch_personal_channel(client: &mut MtpClient, user_id: i64, access_hash: i64) -> Result<String, String> {
    let input_user = tl_gen::serialize_input_user(user_id, access_hash);
    let req = tl_gen::build_users_getFullUser(&input_user);
    let data = client.invoke(&req).await
        .map_err(|e| format!("users.getFullUser: {e}"))?;
    parse_personal_channel_from_full_user(&data)
}

fn format_found_line(info: &tl::UserInfo, nft_links: &[String], personal_channel: &str, cfg: &UserLookupConfig) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push("found".to_string());
    if cfg.save_name { parts.push(if info.first_name.is_empty() { String::new() } else { info.first_name.clone() }); }
    if cfg.save_surname { parts.push(if info.last_name.is_empty() { String::new() } else { info.last_name.clone() }); }
    if cfg.save_username { parts.push(if info.username.is_empty() { String::new() } else { info.username.clone() }); }
    if cfg.save_phone { parts.push(if info.phone.is_empty() { String::new() } else { info.phone.clone() }); }
    if cfg.save_nft_gifts { parts.push(nft_links.join(";")); }
    if cfg.save_personal_channel { parts.push(personal_channel.to_string()); }
    parts.join(":")
}

fn format_not_found_line(target: &str, cfg: &UserLookupConfig) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("not_found({})", target));
    if cfg.save_name { parts.push(String::new()); }
    if cfg.save_surname { parts.push(String::new()); }
    if cfg.save_username { parts.push(String::new()); }
    if cfg.save_phone { parts.push(String::new()); }
    if cfg.save_nft_gifts { parts.push(String::new()); }
    if cfg.save_personal_channel { parts.push(String::new()); }
    parts.join(":")
}

fn resolve_output_path(user_path: &str) -> PathBuf {
    let trimmed = user_path.trim();
    if !trimmed.is_empty() { return PathBuf::from(trimmed); }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kastor")
        .join("users.txt")
}

fn emit(app: &tauri::AppHandle, msg: String) {
    let _ = app.emit("user-lookup-log", msg);
}

fn parse_personal_channel_from_full_user(data: &[u8]) -> Result<String, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let wrapper = tl_gen::deserialize_tl_obj::<tl_gen::TlUsersUserFull>(&inner)
        .map_err(|e| format!("users.userFull: {e}"))?;
    let full = tl_gen::deserialize_tl_obj::<tl_gen::TlUserFull>(&wrapper.full_user)
        .map_err(|e| format!("userFull: {e}"))?;

    let channel_id = match full.personal_channel_id {
        Some(id) if id != 0 => id,
        _ => return Ok(String::new()),
    };

    // find the matching channel in the chats vector to build a public/private link
    for chat_raw in &wrapper.chats {
        if let Ok(tl_gen::TlChat::Channel { id, username, usernames, .. }) =
            tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(chat_raw)
        {
            if id != channel_id { continue; }
            if let Some(u) = username.filter(|u| !u.is_empty()) {
                return Ok(format!("https://t.me/{u}"));
            }
            // collapse usernames vector (username#b4073647 editable/active flags + username)
            if let Some(active) = usernames.as_ref().and_then(|v| first_active_username(v)) {
                return Ok(format!("https://t.me/{active}"));
            }
            return Ok(format!("https://t.me/c/{channel_id}"));
        }
    }
    Ok(format!("https://t.me/c/{channel_id}"))
}

// extract the first username string from a Vector<Username>
fn first_active_username(raw_usernames: &[Vec<u8>]) -> Option<String> {
    for raw in raw_usernames {
        if let Ok(u) = tl_gen::deserialize_tl_obj::<tl_gen::TlUsername>(raw) {
            if !u.username.is_empty() { return Some(u.username); }
        }
    }
    None
}
