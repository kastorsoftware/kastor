// channelcreator: mass channel/group creation with full Python feature parity.
// Features: random count per account, 4 entity types, admin assignment,
// forward messages, spintax, SQLite output, configurable delays, invite links.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use rusqlite;
use serde::Deserialize;
use tauri::{Emitter, Manager};

use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::mtproto::tl_gen;
use crate::accounts::commands::get_storage_pub;
use crate::accounts::session::AccountJson;
use crate::accounts::connect::connect_account;
use crate::queue::TaskQueue;
use crate::i18n::{t, t_with};

#[derive(Deserialize, Clone)]
pub struct CreateChannelsConfig {
    // channel_public | channel_private | group_public | group_private
    pub channel_type: String,

    pub channels_min: u32,
    pub channels_max: u32,

    pub output_path: String,

    pub title_mode: String,       // "single" | "from_file"
    pub title_single: String,
    pub title_file_path: String,

    pub set_description: bool,
    pub description_mode: String,
    pub description_single: String,
    pub description_file_path: String,

    pub set_photo: bool,
    pub photo_mode: String,       // "single" | "from_folder"
    pub photo_single_path: String,
    pub photo_folder_path: String,

    pub set_username: bool,
    pub username_mode: String,    // "random" | "from_file"
    pub username_file_path: String,

    pub set_profile_channel: bool,

    // admins
    pub add_admins: bool,
    pub admin_ids: String, // comma-separated user_ids or @usernames

    // post
    pub post_enabled: bool,
    pub post_mode: String, // "text" | "forward" | "image"
    pub post_text: String,
    pub post_image_path: String,
    #[serde(default)]
    pub post_video_path: String,
    #[serde(default)]
    pub post_forward_link: String, // t.me/channel/123
    #[serde(default)]
    pub post_randomize: bool,
    #[serde(default)]
    pub post_llm_rewrite: bool,

    // delays
    pub delay_min: u32,
    pub delay_max: u32,
}

#[tauri::command]
pub async fn create_channels_start(
    ids: Vec<String>,
    mut config: CreateChannelsConfig,
    max_flood_wait: u64,
    app: tauri::AppHandle,
) -> Result<String, String> {
    config.channels_min = config.channels_min.clamp(1, 50);
    config.channels_max = config.channels_max.clamp(config.channels_min, 50);

    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue.register_task(
        task_id.clone(),
        "create_channels".to_string(),
        t_with("channelcreator_task_name", &[("count", &ids.len().to_string())]),
    ).await;

    let titles = load_lines(&config.title_file_path);
    let descriptions = load_lines(&config.description_file_path);
    let usernames = load_lines(&config.username_file_path);
    let photos = load_photo_paths(&config.photo_folder_path);

    // pre-flight validation
    let num_accounts = ids.len() as u32;
    let max_total = num_accounts * config.channels_max;
    if config.title_mode == "from_file" && (titles.len() as u32) < max_total {
        return Err(t_with("channelcreator_not_enough_titles", &[("available", &titles.len().to_string()), ("needed", &max_total.to_string())]));
    }
    if config.set_username && config.username_mode == "from_file" && (usernames.len() as u32) < max_total {
        return Err(t_with("channelcreator_not_enough_usernames", &[("available", &usernames.len().to_string()), ("needed", &max_total.to_string())]));
    }

    // init SQLite
    let output_path = resolve_output_path(&config.output_path);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() { std::fs::create_dir_all(parent).ok(); }
    }
    let db = init_db(&output_path)?;
    let db = Arc::new(tokio::sync::Mutex::new(db));

    let _ = app.emit("create-channels-log", format!("DB: {}", output_path.display()));

    let config = Arc::new(config);
    let titles = Arc::new(titles);
    let descriptions = Arc::new(descriptions);
    let usernames = Arc::new(usernames);
    let photos = Arc::new(photos);
    let title_idx = Arc::new(AtomicUsize::new(0));
    let username_idx = Arc::new(AtomicUsize::new(0));

    tokio::spawn(async move {
        let total = ids.len();
        let mut handles = Vec::new();

        for (i, id) in ids.into_iter().enumerate() {
            if !token.load(Ordering::Relaxed) { break; }

            let config = config.clone();
            let titles = titles.clone();
            let descriptions = descriptions.clone();
            let usernames = usernames.clone();
            let photos = photos.clone();
            let title_idx = title_idx.clone();
            let username_idx = username_idx.clone();
            let db_clone = db.clone();
            let app_clone = app.clone();
            let token_clone = token.clone();

            handles.push(tokio::spawn(async move {
                if !token_clone.load(Ordering::Relaxed) { return; }
                let result = process_create_channels(
                    &id, i + 1, total, &config,
                    &titles, &descriptions, &usernames, &photos,
                    &title_idx, &username_idx, &db_clone, &app_clone, &token_clone,
                    max_flood_wait,
                ).await;
                if let Err(e) = result {
                    crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                    let _ = app_clone.emit("create-channels-log", format!("[{}/{}] {}: {}", i + 1, total, t("error"), e));
                }
            }));
        }

        for h in handles { let _ = h.await; }
        let _ = app.emit("create-channels-log", t("done"));
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn create_channels_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

fn init_db(path: &std::path::PathBuf) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(path).map_err(|e| t_with("channelcreator_db_open_error", &[("error", &e.to_string())]))?;
    conn.execute_batch("
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        CREATE TABLE IF NOT EXISTS channels (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account TEXT DEFAULT '',
            channel_id INTEGER DEFAULT 0,
            title TEXT DEFAULT '',
            channel_type TEXT DEFAULT '',
            link TEXT DEFAULT '',
            message TEXT DEFAULT '',
            status TEXT DEFAULT 'done',
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );
        CREATE INDEX IF NOT EXISTS idx_channels_status ON channels(status);
    ").map_err(|e| t_with("channelcreator_db_tables_error", &[("error", &e.to_string())]))?;
    Ok(conn)
}

async fn process_create_channels(
    id: &str,
    idx: usize,
    total: usize,
    config: &CreateChannelsConfig,
    titles: &[String],
    descriptions: &[String],
    usernames_list: &[String],
    photos: &[String],
    title_idx: &AtomicUsize,
    username_idx: &AtomicUsize,
    db: &tokio::sync::Mutex<rusqlite::Connection>,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
    max_flood_wait: u64,
) -> Result<(), String> {
    let emit = |msg: String| { let _ = app.emit("create-channels-log", msg); };

    let mut client = connect_account(id).await?;
    client.set_log_target("create-channels-log", app.clone());
    client.set_max_flood_wait(max_flood_wait);

    let storage = get_storage_pub();
    let json_path = storage.json_path(id);
    let json = if json_path.exists() { AccountJson::from_file(&json_path).unwrap_or_default() } else { AccountJson::default() };
    let phone = json.phone.clone();
    let prefix = format!("[{}/{}] +{}", idx, total, if phone.is_empty() { "?" } else { &phone });

    // determine channel count for this account
    let channels_count = if config.channels_min == config.channels_max {
        config.channels_min
    } else {
        config.channels_min + (rand::random::<u32>() % (config.channels_max - config.channels_min + 1))
    };

    // determine entity type flags
    let is_broadcast = config.channel_type.contains("channel");
    let is_megagroup = config.channel_type.contains("group");
    let is_public = config.channel_type.contains("public");

    let entity_type = if is_broadcast { t("channelcreator_entity_channels") } else { t("channelcreator_entity_groups") };
    emit(t_with("channelcreator_creating", &[("prefix", &prefix), ("count", &channels_count.to_string()), ("entity_type", &entity_type), ("channel_type", &config.channel_type)]));

    for ch_idx in 0..channels_count {
        if !token.load(Ordering::Relaxed) { break; }

        // configurable delay between channels
        if ch_idx > 0 && (config.delay_min > 0 || config.delay_max > 0) {
            let delay = random_delay(config.delay_min, config.delay_max);
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay as u64)).await;
            }
        }

        // pick title (with spintax)
        let title = spin_text(&match config.title_mode.as_str() {
            "from_file" => {
                let i = title_idx.fetch_add(1, Ordering::Relaxed);
                if i >= titles.len() { emit(t_with("channelcreator_titles_exhausted", &[("prefix", &prefix)])); break; }
                titles[i].clone()
            }
            _ => config.title_single.clone(),
        });

        emit(t_with("channelcreator_creating_title", &[("prefix", &prefix), ("idx", &(ch_idx + 1).to_string()), ("total", &channels_count.to_string()), ("title", &title)]));

        // description (with spintax)
        let about_arg = if config.set_description {
            spin_text(&match config.description_mode.as_str() {
                "from_file" => {
                    if descriptions.is_empty() { String::new() }
                    else { descriptions[rand::random::<usize>() % descriptions.len()].clone() }
                }
                _ => config.description_single.clone(),
            })
        } else { String::new() };

        // create channel/group
        let create_req = tl::build_create_channel(&title, &about_arg, is_broadcast, is_megagroup);
        let create_resp = client.invoke(&create_req).await
            .map_err(|e| format!("create_channel: {e}"))?;
        let (channel_id, access_hash) = tl::parse_created_channel(&create_resp)
            .map_err(|e| format!("parse_channel: {e}"))?;

        emit(t_with("channelcreator_created_id", &[("prefix", &prefix), ("id", &channel_id.to_string())]));

        // set photo
        if config.set_photo {
            let photo_path = match config.photo_mode.as_str() {
                "single" => Some(config.photo_single_path.clone()),
                _ => {
                    if photos.is_empty() { None }
                    else { Some(photos[rand::random::<usize>() % photos.len()].clone()) }
                }
            };
            if let Some(pp) = photo_path {
                if let Err(e) = upload_and_set_photo(&mut client, channel_id, access_hash, &pp, token).await {
                    emit(t_with("channelcreator_photo_error", &[("prefix", &prefix), ("error", &e)]));
                }
            }
        }

        // set username (makes public) or get invite link (private)
        let mut channel_link = String::new();
        if is_public && config.set_username {
            match try_set_channel_username(&mut client, channel_id, access_hash, config, usernames_list, username_idx, token).await {
                Ok(uname) => {
                    emit(t_with("channelcreator_username_set", &[("prefix", &prefix), ("username", &uname)]));
                    channel_link = format!("https://t.me/{}", uname);
                }
                Err(e) => emit(t_with("channelcreator_username_error", &[("prefix", &prefix), ("error", &e)])),
            }
        }

        // get invite link for private channels
        if channel_link.is_empty() {
            let req = tl::build_export_channel_invite(channel_id, access_hash);
            match client.invoke(&req).await {
                Ok(resp) => {
                    if let Ok(link) = tl::parse_exported_invite_link(&resp) {
                        channel_link = link;
                    }
                }
                Err(e) => emit(t_with("channelcreator_invite_error", &[("prefix", &prefix), ("error", &e.to_string())])),
            }
        }

        // set as personal channel
        if config.set_profile_channel && ch_idx == 0 && is_public {
            let req = tl::build_update_personal_channel(channel_id, access_hash);
            if let Err(e) = client.invoke(&req).await {
                emit(t_with("channelcreator_profile_error", &[("prefix", &prefix), ("error", &e.to_string())]));
            }
        }

        // add admins
        if config.add_admins && !config.admin_ids.is_empty() {
            let admin_usernames = parse_admin_usernames(&config.admin_ids);
            for username in &admin_usernames {
                match resolve_and_add_admin(&mut client, channel_id, access_hash, username).await {
                    Ok(()) => {},
                    Err(e) => {
                        emit(t_with("channelcreator_admin_error", &[("prefix", &prefix), ("username", username), ("error", &e)]));
                        // Stop trying admins on this account if restricted
                        if e.contains("USER_PRIVACY_RESTRICTED") || e.contains("USER_RESTRICTED") || e.contains("CHAT_ADMIN_REQUIRED") {
                            break;
                        }
                    }
                }
                rate_limit().await;
            }
        }

        // publish post
        let mut message_sent = String::new();
        if config.post_enabled {
            match config.post_mode.as_str() {
                "forward" => {
                    // forward from another channel
                    if !config.post_forward_link.is_empty() {
                        match forward_post(&mut client, channel_id, access_hash, &config.post_forward_link).await {
                            Ok(_) => { message_sent = format!("fwd:{}", config.post_forward_link); }
                            Err(e) => emit(t_with("channelcreator_forward_error", &[("prefix", &prefix), ("error", &e)])),
                        }
                    }
                }
                "image" => {
                    // send image with optional caption
                    let caption = spin_text(&config.post_text);
                    if let Err(e) = publish_post(&mut client, channel_id, access_hash, &caption, &config.post_image_path, &config.post_video_path, token).await {
                        emit(t_with("channelcreator_post_error", &[("prefix", &prefix), ("error", &e)]));
                    } else {
                        message_sent = caption;
                    }
                }
                _ => {
                    // text post (with spintax/LLM/randomize)
                    let post_text = if config.post_llm_rewrite {
                        crate::llm::complete(
                            "Перефразируй этот текст другими словами, сохрани смысл и длину. Без кавычек, без эмодзи.",
                            &config.post_text,
                        ).unwrap_or_else(|_| config.post_text.clone()).trim().to_string()
                    } else if config.post_randomize {
                        crate::randomizer::randomize_text_internal(&config.post_text, 40)
                    } else {
                        spin_text(&config.post_text)
                    };
                    if !post_text.is_empty() {
                        if let Err(e) = publish_post(&mut client, channel_id, access_hash, &post_text, "", "", token).await {
                            emit(t_with("channelcreator_post_error", &[("prefix", &prefix), ("error", &e)]));
                        } else {
                            message_sent = post_text;
                        }
                    }
                }
            }
        }

        // save to SQLite
        {
            let db = db.lock().await;
            db.execute(
                "INSERT INTO channels (account, channel_id, title, channel_type, link, message, status) VALUES (?1,?2,?3,?4,?5,?6,'done')",
                rusqlite::params![phone, channel_id, title, config.channel_type, channel_link, message_sent],
            ).ok();
        }

        rate_limit().await;
    }

    if let Some(fatal) = client.fatal_error() {
        return Err(fatal.to_string());
    }
    Ok(())
}

// ─── Forward a post from another channel ───────────────────────────────────

async fn forward_post(
    client: &mut MtpClient,
    to_channel_id: i64,
    to_access_hash: i64,
    forward_link: &str,
) -> Result<(), String> {
    // parse t.me/channel/123 format
    let (source_username, msg_id) = crate::mtproto::text_parse::parse_post_link(forward_link)
        .ok_or_else(|| t_with("channelcreator_forward_link_error", &[("link", forward_link)]))?;

    // resolve source channel
    let resolve_req = tl::build_resolve_username(&source_username);
    let resolve_data = client.invoke(&resolve_req).await
        .map_err(|e| format!("resolve source: {e}"))?;
    let (from_id, from_hash) = tl::parse_resolved_peer(&resolve_data)
        .map_err(|e| format!("parse source peer: {e}"))?;

    let fwd_req = tl::build_forward_messages(
        from_id, from_hash,
        to_channel_id, to_access_hash,
        &[msg_id], false, false, false,
    );
    client.invoke(&fwd_req).await.map_err(|e| format!("forward: {e}"))?;
    Ok(())
}

// ─── Add admin to channel ──────────────────────────────────────────────────

async fn resolve_and_add_admin(
    client: &mut MtpClient,
    channel_id: i64,
    channel_access_hash: i64,
    username: &str,
) -> Result<(), String> {
    let resolve_req = tl::build_resolve_username(username);
    let resolve_data = client.invoke(&resolve_req).await
        .map_err(|e| format!("resolve @{}: {e}", username))?;
    let (user_id, user_hash) = tl::parse_resolved_peer(&resolve_data)
        .map_err(|e| format!("parse resolve @{}: {e}", username))?;

    let channel = tl_gen::serialize_input_channel(channel_id, channel_access_hash);
    let user = tl_gen::serialize_input_user(user_id, user_hash);
    let rights = tl_gen::serialize_chatAdminRights(
        true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, false, false, false,
    );
    let req = tl_gen::build_channels_editAdmin(&channel, &user, &rights, None);
    client.invoke(&req).await.map_err(|e| format!("editAdmin @{}: {e}", username))?;
    Ok(())
}

fn parse_admin_usernames(input: &str) -> Vec<String> {
    input.split(',')
        .map(|s| s.trim().trim_start_matches('@').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

// ─── Existing helpers (preserved) ──────────────────────────────────────────

async fn rate_limit() {
    let jitter = rand::random::<u64>() % 500;
    tokio::time::sleep(std::time::Duration::from_millis(500 + jitter)).await;
}

async fn upload_and_set_photo(
    client: &mut MtpClient, channel_id: i64, access_hash: i64, photo_path: &str, token: &Arc<AtomicBool>,
) -> Result<(), String> {
    let data = tokio::fs::read(photo_path).await.map_err(|e| format!("read photo: {e}"))?;
    let file_id = rand::random::<i64>();
    let part_size = 512 * 1024;
    let total_parts = ((data.len() + part_size - 1) / part_size) as i32;
    for i in 0..total_parts {
        if !token.load(Ordering::Relaxed) { return Ok(()); }
        let start = i as usize * part_size;
        let end = ((i as usize + 1) * part_size).min(data.len());
        let req = tl::build_upload_save_file_part(file_id, i, &data[start..end]);
        client.invoke(&req).await.map_err(|e| format!("upload part {}: {e}", i))?;
        rate_limit().await;
    }
    let filename = std::path::Path::new(photo_path).file_name().and_then(|n| n.to_str()).unwrap_or("photo.jpg");
    let edit_req = tl::build_channel_edit_photo_uploaded(channel_id, access_hash, file_id, total_parts, filename);
    client.invoke(&edit_req).await.map_err(|e| format!("editPhoto: {e}"))?;
    Ok(())
}

async fn try_set_channel_username(
    client: &mut MtpClient, channel_id: i64, access_hash: i64,
    config: &CreateChannelsConfig, usernames_list: &[String],
    username_idx: &AtomicUsize, token: &Arc<AtomicBool>,
) -> Result<String, String> {
    for _ in 0..5 {
        if !token.load(Ordering::Relaxed) { return Ok(String::new()); }
        let candidate = match config.username_mode.as_str() {
            "from_file" => {
                let i = username_idx.fetch_add(1, Ordering::Relaxed);
                if i >= usernames_list.len() { return Err(t("channelcreator_usernames_exhausted")); }
                sanitize_username(&usernames_list[i])
            }
            _ => generate_random_channel_username(),
        };
        if candidate.len() < 5 { continue; }

        let check_req = tl::build_channel_check_username(channel_id, access_hash, &candidate);
        let resp = client.invoke(&check_req).await.map_err(|e| format!("checkUsername: {e}"))?;
        rate_limit().await;
        if resp.len() < 4 { continue; }
        if u32::from_le_bytes([resp[0], resp[1], resp[2], resp[3]]) != tl_gen::BOOL_TRUE { continue; }

        let upd_req = tl::build_channel_update_username(channel_id, access_hash, &candidate);
        let upd_resp = client.invoke(&upd_req).await.map_err(|e| format!("updateUsername: {e}"))?;
        rate_limit().await;
        if upd_resp.len() >= 4 && u32::from_le_bytes([upd_resp[0], upd_resp[1], upd_resp[2], upd_resp[3]]) == tl_gen::BOOL_TRUE {
            return Ok(candidate);
        }
    }
    Err(t("channelcreator_username_attempts_exhausted"))
}

async fn publish_post(
    client: &mut MtpClient, channel_id: i64, access_hash: i64,
    markdown_text: &str, image_path: &str, video_path: &str, token: &Arc<AtomicBool>,
) -> Result<(), String> {
    let (text, entities) = tl::parse_markdown_v2(markdown_text);
    let random_id: i64 = rand::random();

    let req = if !image_path.is_empty() {
        let data = tokio::fs::read(image_path).await.map_err(|e| format!("read image: {e}"))?;
        let file_id = rand::random::<i64>();
        let part_size = 512 * 1024;
        let total_parts = ((data.len() + part_size - 1) / part_size) as i32;
        for i in 0..total_parts {
            if !token.load(Ordering::Relaxed) { return Ok(()); }
            let start = i as usize * part_size;
            let end = ((i as usize + 1) * part_size).min(data.len());
            let req = tl::build_upload_save_file_part(file_id, i, &data[start..end]);
            client.invoke(&req).await.map_err(|e| format!("upload part {}: {e}", i))?;
            rate_limit().await;
        }
        let filename = std::path::Path::new(image_path).file_name().and_then(|n| n.to_str()).unwrap_or("photo.jpg");
        tl::build_send_media_uploaded_photo(channel_id, access_hash, file_id, total_parts, filename, &text, &entities, random_id)
    } else if !video_path.is_empty() {
        let data = tokio::fs::read(video_path).await.map_err(|e| format!("read video: {e}"))?;
        let file_id = rand::random::<i64>();
        let part_size = 512 * 1024;
        let total_parts = ((data.len() + part_size - 1) / part_size) as i32;
        let is_big = data.len() >= 10 * 1024 * 1024;
        for i in 0..total_parts {
            if !token.load(Ordering::Relaxed) { return Ok(()); }
            let start = i as usize * part_size;
            let end = ((i as usize + 1) * part_size).min(data.len());
            let req = if is_big { tl_gen::build_upload_saveBigFilePart(file_id, i, total_parts, &data[start..end]) }
                      else { tl_gen::build_upload_saveFilePart(file_id, i, &data[start..end]) };
            client.invoke(&req).await.map_err(|e| format!("upload part {}: {e}", i))?;
            rate_limit().await;
        }
        let filename = std::path::Path::new(video_path).file_name().and_then(|n| n.to_str()).unwrap_or("video.mp4");
        let input_file = if is_big { tl_gen::serialize_inputFileBig(file_id, total_parts, filename) }
                         else { tl_gen::serialize_inputFile(file_id, total_parts, filename, "") };
        let video_attr = tl_gen::serialize_documentAttributeVideo(false, true, false, 0.0, 0, 0, None, None, None);
        let filename_attr = tl_gen::serialize_documentAttributeFilename(filename);
        let attrs: &[&[u8]] = &[&video_attr, &filename_attr];
        let media = tl_gen::serialize_inputMediaUploadedDocument(false, false, false, &input_file, None, "video/mp4", attrs, None, None, None, None);
        let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
        tl_gen::build_messages_sendMedia(false, false, false, false, false, false, false, &peer, None, &media, &text, random_id, None, None, None, None, None, None, None, None, None)
    } else {
        tl::build_send_message_with_entities(channel_id, access_hash, true, &text, &entities, random_id)
    };
    client.invoke(&req).await.map_err(|e| format!("send post: {e}"))?;
    Ok(())
}

// ─── Utilities ─────────────────────────────────────────────────────────────

fn spin_text(input: &str) -> String {
    let mut result = input.to_string();
    loop {
        if let Some(start) = result.rfind('{') {
            if let Some(end) = result[start..].find('}') {
                let end = start + end;
                let options: Vec<&str> = result[start + 1..end].split('|').collect();
                let choice = if options.is_empty() { String::new() } else { options[rand::random::<usize>() % options.len()].to_string() };
                result = format!("{}{}{}", &result[..start], choice, &result[end + 1..]);
                continue;
            }
        }
        break;
    }
    result
}

fn random_delay(min: u32, max: u32) -> u32 {
    if min == 0 && max == 0 { return 0; }
    let lo = min.min(max);
    let hi = min.max(max);
    if lo == hi { return lo; }
    lo + (rand::random::<u32>() % (hi - lo + 1))
}

fn generate_random_channel_username() -> String {
    let len = 9 + (rand::random::<usize>() % 4);
    let mut s = String::new();
    s.push((b'a' + rand::random::<u8>() % 26) as char);
    for _ in 1..len {
        let c = match rand::random::<u8>() % 3 {
            0 => (b'a' + rand::random::<u8>() % 26) as char,
            1 => (b'0' + rand::random::<u8>() % 10) as char,
            _ => '_',
        };
        s.push(c);
    }
    s
}

fn sanitize_username(input: &str) -> String {
    let mut out: String = input.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    out.make_ascii_lowercase();
    if out.len() > 32 { out.truncate(32); }
    out
}

fn resolve_output_path(user_path: &str) -> std::path::PathBuf {
    let trimmed = user_path.trim();
    if !trimmed.is_empty() {
        let p = std::path::PathBuf::from(trimmed);
        return if p.extension().map(|e| e == "db").unwrap_or(false) { p } else { p.with_extension("db") };
    }
    let base = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join("kastor").join("create_channels");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    base.join(format!("channels_{now}.db"))
}

fn load_lines(path: &str) -> Vec<String> {
    if path.is_empty() { return Vec::new(); }
    std::fs::read_to_string(path).unwrap_or_default()
        .lines().map(|l| l.trim().trim_start_matches('@').to_string())
        .filter(|l| !l.is_empty()).collect()
}

fn load_photo_paths(folder: &str) -> Vec<String> {
    if folder.is_empty() { return Vec::new(); }
    std::fs::read_dir(folder).ok()
        .map(|entries| entries.flatten()
            .filter(|e| { let p = e.path(); p.is_file() && matches!(p.extension().and_then(|x| x.to_str()).unwrap_or(""), "jpg"|"jpeg"|"png"|"webp") })
            .map(|e| e.path().to_string_lossy().to_string()).collect())
        .unwrap_or_default()
}
