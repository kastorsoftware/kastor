// botcreator: mass bot creation via @BotFather

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
pub struct CreateBotsConfig {
    pub name_mode: String,
    pub name_single: String,
    pub name_file_path: String,

    pub username_mode: String,
    pub username_file_path: String,

    pub bots_min: u32,
    pub bots_max: u32,

    pub output_path: String,

    pub set_description: bool,
    pub description_mode: String,
    pub description_single: String,
    pub description_file_path: String,

    pub set_about: bool,
    pub about_mode: String,
    pub about_single: String,
    pub about_file_path: String,

    pub set_photo: bool,
    pub photo_mode: String,
    pub photo_single_path: String,
    pub photo_folder_path: String,

    pub set_privacy: bool,

    pub delay_min: u32,
    pub delay_max: u32,
}

#[tauri::command]
pub async fn create_bots_start(
    ids: Vec<String>,
    mut config: CreateBotsConfig,
    max_flood_wait: u64,
    app: tauri::AppHandle,
) -> Result<String, String> {
    config.bots_min = config.bots_min.clamp(1, 20);
    config.bots_max = config.bots_max.clamp(config.bots_min, 20);

    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue.register_task(
        task_id.clone(),
        "create_bots".to_string(),
        t_with("botcreator_task_name", &[("count", &ids.len().to_string())]),
    ).await;

    let names = load_lines(&config.name_file_path);
    let usernames = load_lines(&config.username_file_path);
    let descriptions = load_lines(&config.description_file_path);
    let abouts = load_lines(&config.about_file_path);
    let photos = load_photo_paths(&config.photo_folder_path);

    // pre-flight validation: check if we have enough resources
    let num_accounts = ids.len() as u32;
    let max_total = num_accounts * config.bots_max;
    let min_total = num_accounts * config.bots_min;

    if config.name_mode == "from_file" && (names.len() as u32) < max_total {
        return Err(t_with(
            "botcreator_not_enough_names",
            &[("available", &names.len().to_string()), ("needed", &max_total.to_string()), ("accounts", &num_accounts.to_string()), ("max", &config.bots_max.to_string())],
        ));
    }
    if config.username_mode == "from_file" && (usernames.len() as u32) < max_total {
        return Err(t_with(
            "botcreator_not_enough_usernames",
            &[("available", &usernames.len().to_string()), ("needed", &max_total.to_string()), ("accounts", &num_accounts.to_string()), ("max", &config.bots_max.to_string())],
        ));
    }

    // warning if min creates too many (emit as log, not a hard error)
    if min_total > 50 {
        let _ = app.emit("create-bots-log", t_with(
            "botcreator_too_many_warning",
            &[("min", &config.bots_min.to_string()), ("accounts", &num_accounts.to_string()), ("total", &min_total.to_string())],
        ));
    }

    let config = Arc::new(config);
    let names = Arc::new(names);
    let usernames = Arc::new(usernames);
    let descriptions = Arc::new(descriptions);
    let abouts = Arc::new(abouts);
    let photos = Arc::new(photos);
    let name_idx = Arc::new(AtomicUsize::new(0));
    let username_idx = Arc::new(AtomicUsize::new(0));

    // init SQLite output
    let output_path = resolve_bot_output_path(&config.output_path);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() { std::fs::create_dir_all(parent).ok(); }
    }
    let db = init_bots_db(&output_path)?;
    let db = Arc::new(tokio::sync::Mutex::new(db));

    let _ = app.emit("create-bots-log", format!("DB: {}", output_path.display()));

    tokio::spawn(async move {
        let total = ids.len();
        let mut handles = Vec::new();

        for (i, id) in ids.into_iter().enumerate() {
            if !token.load(Ordering::Relaxed) { break; }

            let config = config.clone();
            let names = names.clone();
            let usernames = usernames.clone();
            let descriptions = descriptions.clone();
            let abouts = abouts.clone();
            let photos = photos.clone();
            let name_idx = name_idx.clone();
            let username_idx = username_idx.clone();
            let db_clone = db.clone();
            let app_clone = app.clone();
            let token_clone = token.clone();

            handles.push(tokio::spawn(async move {
                if !token_clone.load(Ordering::Relaxed) { return; }

                let result = process_create_bots(
                    &id, i + 1, total, &config,
                    &names, &usernames, &descriptions, &abouts, &photos,
                    &name_idx, &username_idx, &db_clone, &app_clone, &token_clone,
                    max_flood_wait,
                ).await;
                if let Err(e) = result {
                    crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                    let _ = app_clone.emit("create-bots-log", format!("[{}/{}] {}: {}", i + 1, total, t("error"), e));
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }

        let _ = app.emit("create-bots-log", t("done"));

        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn create_bots_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn process_create_bots(
    id: &str,
    idx: usize,
    total: usize,
    config: &CreateBotsConfig,
    names: &[String],
    usernames_list: &[String],
    descriptions: &[String],
    abouts: &[String],
    photos: &[String],
    name_idx: &AtomicUsize,
    username_idx: &AtomicUsize,
    db: &tokio::sync::Mutex<rusqlite::Connection>,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
    max_flood_wait: u64,
) -> Result<(), String> {
    let emit = |msg: String| { let _ = app.emit("create-bots-log", msg); };

    let mut client = connect_account(id).await?;
    client.set_log_target("create-bots-log", app.clone());
    client.set_max_flood_wait(max_flood_wait);

    let storage = get_storage_pub();
    let json_path = storage.json_path(id);
    let json = if json_path.exists() {
        AccountJson::from_file(&json_path).unwrap_or_default()
    } else {
        AccountJson::default()
    };

    let phone = json.phone.clone();
    let prefix = format!("[{}/{}] +{}", idx, total, if phone.is_empty() { "?" } else { &phone });
    client.set_log_prefix(&prefix);

    // resolve @BotFather (loop on FLOOD_WAIT with max_flood_wait limit)
    let resolve_req = tl::build_resolve_username("BotFather");
    let mut resolve_data: Option<Vec<u8>> = None;
    for _attempt in 0..5 {
        match client.invoke(&resolve_req).await {
            Ok(data) => { resolve_data = Some(data); break; }
            Err(e) if e.contains("FLOOD_WAIT") => {
                let wait_secs = e.split('_').last().and_then(|s| s.parse::<u64>().ok()).unwrap_or(30);
                if max_flood_wait > 0 && wait_secs > max_flood_wait {
                    emit(t_with("botcreator_resolve_flood_skip", &[("prefix", &prefix), ("seconds", &wait_secs.to_string()), ("limit", &max_flood_wait.to_string())]));
                    return Ok(());
                }
                emit(t_with("botcreator_resolve_flood", &[("prefix", &prefix), ("seconds", &wait_secs.to_string())]));
                if !interruptible_sleep_secs(wait_secs + 1, token).await { return Ok(()); }
                continue;
            }
            Err(e) => return Err(format!("resolve BotFather: {e}")),
        }
    }
    let resolve_data = resolve_data.ok_or_else(|| "resolve BotFather: max attempts".to_string())?;
    let (bf_id, bf_access_hash) = tl::parse_resolved_peer(&resolve_data)
        .map_err(|e| format!("parse BotFather: {e}"))?;

    dbg_log!("botcreator: BotFather resolved id={} access_hash={:#018x}", bf_id, bf_access_hash);

    // unblock BotFather (in case it was blocked by previous run)
    let unblock_req = tl::build_unblock_peer(bf_id, bf_access_hash);
    if let Err(e) = client.invoke(&unblock_req).await {
        dbg_log!("разблокировка @BotFather не удалась: {e}");
    }

    // mute BotFather
    let mute_req = tl::build_mute_peer(bf_id, bf_access_hash);
    if let Err(e) = client.invoke(&mute_req).await {
        dbg_log!("отключение уведомлений @BotFather не удалось: {e}");
    }

    emit(t_with("botcreator_starting", &[("prefix", &prefix)]));

    // determine how many bots this account will create (random between min and max)
    let bots_count = if config.bots_min == config.bots_max {
        config.bots_min
    } else {
        config.bots_min + (rand::random::<u32>() % (config.bots_max - config.bots_min + 1))
    };

    // send /start first to initialize conversation
    let start_rid: i64 = rand::random();
    let start_req = tl::build_send_message(bf_id, bf_access_hash, "/start", start_rid);
    match client.invoke(&start_req).await {
        Ok(resp) => {
            dbg_log!("botcreator: /start sent OK, response {} bytes", resp.len());
        }
        Err(e) => {
            emit(t_with("botcreator_start_error", &[("prefix", &prefix), ("error", &e.to_string())]));
            return Err(format!("send /start to BotFather: {e}"));
        }
    }
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    for bot_idx in 0..bots_count {
        if !token.load(Ordering::Relaxed) { break; }

        // pick name (with spintax support)
        let bot_name = match config.name_mode.as_str() {
            "from_file" => {
                let i = name_idx.fetch_add(1, Ordering::Relaxed);
                if i >= names.len() {
                    emit(t_with("botcreator_names_exhausted", &[("prefix", &prefix)]));
                    break;
                }
                spin_text(&names[i])
            }
            "single" => spin_text(&config.name_single),
            _ => generate_random_name(8),
        };

        // pick username
        let bot_username = match config.username_mode.as_str() {
            "from_file" => {
                let i = username_idx.fetch_add(1, Ordering::Relaxed);
                if i >= usernames_list.len() {
                    emit(t_with("botcreator_usernames_exhausted", &[("prefix", &prefix)]));
                    break;
                }
                ensure_bot_suffix(&usernames_list[i])
            }
            _ => generate_random_bot_username(8),
        };

        emit(t_with("botcreator_creating", &[("prefix", &prefix), ("idx", &(bot_idx + 1).to_string()), ("total", &bots_count.to_string()), ("name", &bot_name), ("username", &bot_username)]));

        // track message count before sending to detect new replies
        let initial_count = get_history_count(&mut client, bf_id, bf_access_hash).await;

        // send /newbot with retry limit
        let mut newbot_ok = false;
        for newbot_attempt in 0..3 {
            if !token.load(Ordering::Relaxed) { break; }
            let rid: i64 = rand::random();
            let req = tl::build_send_message(bf_id, bf_access_hash, "/newbot", rid);
            match client.invoke(&req).await {
                Ok(resp) => {
                    dbg_log!("botcreator: /newbot sent OK (attempt {}), response {} bytes", newbot_attempt + 1, resp.len());
                }
                Err(e) => {
                    emit(t_with("botcreator_newbot_error", &[("prefix", &prefix), ("error", &e.to_string())]));
                    return Err(format!("send /newbot: {e}"));
                }
            }

            wait_for_new_message(&mut client, bf_id, bf_access_hash, initial_count + 1, token).await;

            let history_req = tl::build_get_history(bf_id, bf_access_hash, 3);
            let history_data = client.invoke(&history_req).await
                .map_err(|e| format!("get_history: {e}"))?;
            let mut rate_limited = false;
            let mut skip_account = false;
            if let Ok(msgs) = tl::parse_messages_history(&history_data) {
                dbg_log!("botcreator: after /newbot, history has {} msgs", msgs.len());
                for (mi, msg) in msgs.iter().enumerate() {
                    dbg_log!("botcreator: msg[{}] = '{}'", mi, &msg[..msg.len().min(120)]);
                }
                if let Some(last) = msgs.first() {
                    // Check for permanent restriction
                    if last.contains("Unfortunately") && last.contains("cannot create") {
                        emit(t_with("botcreator_restricted", &[("prefix", &prefix)]));
                        skip_account = true;
                    } else if let Some(wait_secs) = extract_wait_seconds(last) {
                        if wait_secs > 900 {
                            emit(t_with("botcreator_rate_limit_skip", &[("prefix", &prefix), ("seconds", &wait_secs.to_string())]));
                            skip_account = true;
                        } else {
                            emit(t_with("botcreator_rate_limit_wait", &[("prefix", &prefix), ("seconds", &wait_secs.to_string()), ("attempt", &(newbot_attempt + 1).to_string())]));
                            if !interruptible_sleep_secs(wait_secs + 1, token).await { break; }
                            rate_limited = true;
                        }
                    }
                }
            } else {
                dbg_log!("botcreator: parse_messages_history FAILED after /newbot");
            }
            if skip_account {
                emit(t_with("botcreator_rate_limit_error", &[("prefix", &prefix)]));
                let del_req = tl::build_delete_history(bf_id, bf_access_hash);
                if let Err(e) = client.invoke(&del_req).await {
                    dbg_log!("удаление истории с @BotFather (rate limit cleanup) не удалось: {e}");
                }
                return Ok(());
            }
            if !rate_limited {
                newbot_ok = true;
                break;
            }
            // rate limited — retry unless this was the last attempt
            if newbot_attempt == 2 {
                emit(t_with("botcreator_rate_limit_exhausted", &[("prefix", &prefix)]));
                return Err(t("botcreator_newbot_exhausted"));
            }
        }
        if !newbot_ok {
            return Err(t("botcreator_no_botfather_reply"));
        }

        // send bot name
        let before_name = get_history_count(&mut client, bf_id, bf_access_hash).await;
        let rid3: i64 = rand::random();
        let name_req = tl::build_send_message(bf_id, bf_access_hash, &bot_name, rid3);
        client.invoke(&name_req).await.map_err(|e| format!("send name: {e}"))?;

        // poll for reply
        wait_for_new_message(&mut client, bf_id, bf_access_hash, before_name + 1, token).await;

        // send bot username (retry up to 3 times with new random if occupied)
        let mut bot_token: Option<String> = None;
        let mut last_bf_response = String::new();
        let mut username_attempts = 0;
        let mut current_username = bot_username.clone();
        loop {
            username_attempts += 1;
            let before_uname = get_history_count(&mut client, bf_id, bf_access_hash).await;
            let rid4: i64 = rand::random();
            let uname_req = tl::build_send_message(bf_id, bf_access_hash, &current_username, rid4);
            client.invoke(&uname_req).await.map_err(|e| format!("send username: {e}"))?;

            wait_for_new_message(&mut client, bf_id, bf_access_hash, before_uname + 1, token).await;

            let history_req2 = tl::build_get_history(bf_id, bf_access_hash, 5);
            if let Ok(history_data2) = client.invoke(&history_req2).await {
                if let Ok(msgs) = tl::parse_messages_history(&history_data2) {
                    if let Some(t) = msgs.iter().find_map(|m| extract_bot_token(m)) {
                        bot_token = Some(t);
                        break;
                    }
                    // Save last BotFather response as error reason (skip our own sent messages)
                    for msg in &msgs {
                        // Our own messages are typically short usernames or commands
                        let is_own = msg.ends_with("bot") || msg.ends_with("_bot") || msg.starts_with("/");
                        if !is_own && !msg.is_empty() {
                            last_bf_response = msg.chars().take(200).collect();
                            break;
                        }
                    }
                    // check if BotFather says username is taken
                    let is_taken = msgs.first().map(|m| {
                        let lower = m.to_lowercase();
                        lower.contains("already taken") || lower.contains("is already") || lower.contains("занят")
                    }).unwrap_or(false);
                    if is_taken && username_attempts < 3 {
                        current_username = generate_random_bot_username(8);
                        emit(t_with("botcreator_username_taken", &[("prefix", &prefix), ("username", &current_username)]));
                        continue;
                    }
                }
            }
            break;
        }

        if let Some(ref tok) = bot_token {
            emit(t_with("botcreator_created", &[("prefix", &prefix), ("token_start", &tok[..10.min(tok.len())]), ("token_end", &tok[tok.len().saturating_sub(6)..])]));

            // save to SQLite
            {
                let db = db.lock().await;
                db.execute(
                    "INSERT INTO bots (account, token, username, name, bio, description, photo, restrict_groups, status) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'done')",
                    rusqlite::params![
                        phone,
                        tok,
                        current_username,
                        bot_name,
                        "", // bio filled below if set
                        "", // description filled below if set
                        "", // photo path filled below if set
                        config.set_privacy as i32,
                    ],
                ).ok();
            }

            // set description if enabled
            if config.set_description {
                let desc = spin_text(&pick_text(&config.description_mode, &config.description_single, descriptions));
                if !desc.is_empty() {
                    let _ = botfather_set_field(&mut client, bf_id, bf_access_hash, "/setdescription", &current_username, &desc, token).await;
                    let db = db.lock().await;
                    db.execute("UPDATE bots SET description = ?1 WHERE token = ?2", rusqlite::params![desc, tok]).ok();
                }
            }

            // set about if enabled
            if config.set_about {
                let about = spin_text(&pick_text(&config.about_mode, &config.about_single, abouts));
                if !about.is_empty() {
                    let about_trimmed = if about.len() > 120 { &about[..120] } else { &about };
                    let _ = botfather_set_field(&mut client, bf_id, bf_access_hash, "/setabouttext", &current_username, about_trimmed, token).await;
                    let db = db.lock().await;
                    db.execute("UPDATE bots SET bio = ?1 WHERE token = ?2", rusqlite::params![about_trimmed, tok]).ok();
                }
            }

            // set photo if enabled
            if config.set_photo {
                let photo_path = match config.photo_mode.as_str() {
                    "single" => Some(config.photo_single_path.clone()),
                    _ => {
                        if !photos.is_empty() {
                            Some(photos[rand::random::<usize>() % photos.len()].clone())
                        } else { None }
                    }
                };
                if let Some(ref pp) = photo_path {
                    let _ = botfather_set_photo(&mut client, bf_id, bf_access_hash, &current_username, pp, token).await;
                    let db = db.lock().await;
                    db.execute("UPDATE bots SET photo = ?1 WHERE token = ?2", rusqlite::params![pp, tok]).ok();
                }
            }

            // set privacy if enabled
            if config.set_privacy {
                let _ = botfather_set_privacy(&mut client, bf_id, bf_access_hash, &current_username, token).await;
            }
        } else {
            emit(t_with("botcreator_token_error", &[("prefix", &prefix), ("idx", &(bot_idx + 1).to_string()), ("total", &bots_count.to_string()), ("reason", &last_bf_response)]));
            let db = db.lock().await;
            db.execute(
                "INSERT INTO bots (account, username, name, status) VALUES (?1,?2,?3,'error')",
                rusqlite::params![phone, current_username, bot_name],
            ).ok();
        }

        // configurable delay between bots
        if bot_idx + 1 < bots_count && (config.delay_min > 0 || config.delay_max > 0) {
            let lo = config.delay_min.min(config.delay_max);
            let hi = config.delay_min.max(config.delay_max);
            let delay_ms = if lo == hi { lo } else { lo + (rand::random::<u32>() % (hi - lo + 1)) };
            if delay_ms > 0 {
                if !interruptible_sleep_secs((delay_ms as u64 + 999) / 1000, token).await { break; }
            }
        }
    }

    // cleanup: delete history + block BotFather
    let del_req = tl::build_delete_history(bf_id, bf_access_hash);
    if let Err(e) = client.invoke(&del_req).await {
        dbg_log!("удаление истории с @BotFather не удалось: {e}");
    }
    let block_req = tl::build_block_peer(bf_id, bf_access_hash);
    if let Err(e) = client.invoke(&block_req).await {
        dbg_log!("блокировка @BotFather не удалась: {e}");
    }

    emit(t_with("botcreator_cleanup_done", &[("prefix", &prefix)]));
    if let Some(fatal) = client.fatal_error() {
        return Err(fatal.to_string());
    }
    Ok(())
}

// Interruptible sleep that checks token every 500ms
async fn interruptible_sleep_secs(seconds: u64, token: &Arc<AtomicBool>) -> bool {
    let total_ms = seconds * 1000;
    let mut elapsed = 0u64;
    while elapsed < total_ms {
        if !token.load(Ordering::Relaxed) { return false; }
        let chunk = 500u64.min(total_ms - elapsed);
        tokio::time::sleep(std::time::Duration::from_millis(chunk)).await;
        elapsed += chunk;
    }
    token.load(Ordering::Relaxed)
}

// send /setdescription or /setabouttext, find bot in keyboard, click, send text
async fn rate_limit() {
    let jitter = rand::random::<u64>() % 500;
    tokio::time::sleep(std::time::Duration::from_millis(500 + jitter)).await;
}

async fn botfather_set_field(
    client: &mut MtpClient,
    bf_id: i64,
    bf_access_hash: i64,
    command: &str,
    bot_username: &str,
    text: &str,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    if !token.load(Ordering::Relaxed) { return Ok(()); }
    let rid: i64 = rand::random();
    let req = tl::build_send_message(bf_id, bf_access_hash, command, rid);
    client.invoke(&req).await?;
    rate_limit().await;

    if !token.load(Ordering::Relaxed) { return Ok(()); }
    let history_req = tl::build_get_history(bf_id, bf_access_hash, 3);
    let history_data = client.invoke(&history_req).await?;

    if let Some((msg_id, callback_data)) = find_bot_button_in_keyboard(&history_data, bot_username) {
        let cb_req = tl::build_bot_callback_answer(bf_id, bf_access_hash, msg_id, &callback_data);
        if let Err(e) = client.invoke(&cb_req).await {
            dbg_log!("нажатие кнопки выбора бота (setField) не удалось: {e}");
        }
    } else {
        let rid2: i64 = rand::random();
        let uname_with_at = format!("@{}", bot_username);
        let req2 = tl::build_send_message(bf_id, bf_access_hash, &uname_with_at, rid2);
        client.invoke(&req2).await?;
    }
    rate_limit().await;

    if !token.load(Ordering::Relaxed) { return Ok(()); }
    let rid3: i64 = rand::random();
    let text_req = tl::build_send_message(bf_id, bf_access_hash, text, rid3);
    client.invoke(&text_req).await?;
    rate_limit().await;
    Ok(())
}

// send /setuserpic, find bot in keyboard, click, upload photo
async fn botfather_set_photo(
    client: &mut MtpClient,
    bf_id: i64,
    bf_access_hash: i64,
    bot_username: &str,
    photo_path: &str,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    if !token.load(Ordering::Relaxed) { return Ok(()); }
    let rid: i64 = rand::random();
    let req = tl::build_send_message(bf_id, bf_access_hash, "/setuserpic", rid);
    client.invoke(&req).await?;
    rate_limit().await;

    if !token.load(Ordering::Relaxed) { return Ok(()); }
    let history_req = tl::build_get_history(bf_id, bf_access_hash, 3);
    let history_data = client.invoke(&history_req).await?;

    if let Some((msg_id, callback_data)) = find_bot_button_in_keyboard(&history_data, bot_username) {
        let cb_req = tl::build_bot_callback_answer(bf_id, bf_access_hash, msg_id, &callback_data);
        if let Err(e) = client.invoke(&cb_req).await {
            dbg_log!("нажатие кнопки выбора бота (setPhoto) не удалось: {e}");
        }
    } else {
        let rid2: i64 = rand::random();
        let uname_with_at = format!("@{}", bot_username);
        let req2 = tl::build_send_message(bf_id, bf_access_hash, &uname_with_at, rid2);
        client.invoke(&req2).await?;
    }
    rate_limit().await;

    if !token.load(Ordering::Relaxed) { return Ok(()); }
    let data = tokio::fs::read(photo_path).await.map_err(|e| format!("read photo: {e}"))?;
    let file_id = rand::random::<i64>();
    let part_size = 512 * 1024;
    let total_parts = ((data.len() + part_size - 1) / part_size) as i32;

    for i in 0..total_parts {
        if !token.load(Ordering::Relaxed) { return Ok(()); }
        let start = i as usize * part_size;
        let end = ((i as usize + 1) * part_size).min(data.len());
        let chunk = &data[start..end];
        let req = tl::build_upload_save_file_part(file_id, i, chunk);
        client.invoke(&req).await?;
        rate_limit().await;
    }

    let filename = std::path::Path::new(photo_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("photo.jpg");

    let photo_req = build_send_photo_message(bf_id, bf_access_hash, file_id, total_parts, filename);
    client.invoke(&photo_req).await?;
    rate_limit().await;
    Ok(())
}

// find a keyboard button containing the bot username in history data
fn find_bot_button_in_keyboard(data: &[u8], bot_username: &str) -> Option<(i32, Vec<u8>)> {
    let messages = tl::parse_messages_structured(data).ok()?;
    for msg in messages {
        if let Some(ref text) = msg.first_button_text {
            if text.contains(bot_username) || text.contains(&format!("@{}", bot_username)) {
                if let Some(ref btn_data) = msg.first_button_data {
                    return Some((msg.id, btn_data.clone()));
                }
            }
        }
    }
    None
}

fn extract_bot_token(text: &str) -> Option<String> {
    // token format: digits:alphanumeric+dash+underscore (e.g. 8827304866:AAGkOQFx_Q4ojEIh0yoWEf7eFmA7S9iP5lw)
    let re_pattern = |s: &str| -> Option<String> {
        for word in s.split_whitespace() {
            let word = word.trim_matches(|c: char| !c.is_alphanumeric() && c != ':' && c != '-' && c != '_');
            if word.contains(':') {
                let parts: Vec<&str> = word.splitn(2, ':').collect();
                if parts.len() == 2 && parts[0].chars().all(|c| c.is_ascii_digit()) && parts[0].len() >= 5
                    && parts[1].len() >= 20 && parts[1].chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                {
                    return Some(word.to_string());
                }
            }
        }
        None
    };
    re_pattern(text)
}

fn extract_wait_seconds(text: &str) -> Option<u64> {
    // skip messages that contain a bot token (they have digits:alphanumeric pattern)
    if text.contains(':') && extract_bot_token(text).is_some() {
        return None;
    }
    // recognize rate limit messages from BotFather (e.g. "Please try again in N seconds")
    // the text usually contains markers: "too many attempts", "try again in", "seconds"
    let lower = text.to_lowercase();
    let is_rate_limit = lower.contains("too many") || lower.contains("try again")
        || lower.contains("flood") || lower.contains("wait");
    if !is_rate_limit { return None; }

    // find the first number in the text - it represents the wait period in seconds
    let mut found_num: Option<u64> = None;
    for word in text.split_whitespace() {
        let trimmed: String = word.chars().filter(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = trimmed.parse::<u64>() {
            if n > 0 {
                found_num = Some(n);
                break;
            }
        }
    }
    if text.len() < 300 { found_num } else { None }
}

fn generate_random_name(len: usize) -> String {
    (0..len).map(|_| (b'a' + rand::random::<u8>() % 26) as char).collect()
}

fn generate_random_bot_username(len: usize) -> String {
    let base: String = (0..len).map(|_| (b'a' + rand::random::<u8>() % 26) as char).collect();
    let suffixes = ["bot", "robot", "_bot"];
    let suffix = suffixes[rand::random::<usize>() % suffixes.len()];
    format!("{}{}", base, suffix)
}

fn ensure_bot_suffix(username: &str) -> String {
    let lower = username.to_lowercase();
    if lower.ends_with("bot") || lower.ends_with("robot") || lower.ends_with("_bot") {
        // clean: only allow [a-z0-9_]
        username.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect()
    } else {
        let suffixes = ["bot", "robot", "_bot"];
        let suffix = suffixes[rand::random::<usize>() % suffixes.len()];
        let clean: String = username.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        format!("{}{}", clean, suffix)
    }
}

fn pick_text(mode: &str, single: &str, from_file: &[String]) -> String {
    match mode {
        "single" => single.to_string(),
        "from_file" => {
            if from_file.is_empty() { return String::new(); }
            from_file[rand::random::<usize>() % from_file.len()].clone()
        }
        _ => String::new(),
    }
}

fn load_lines(path: &str) -> Vec<String> {
    if path.is_empty() { return Vec::new(); }
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().trim_start_matches('@').to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn load_photo_paths(folder: &str) -> Vec<String> {
    if folder.is_empty() { return Vec::new(); }
    std::fs::read_dir(folder)
        .ok()
        .map(|entries| {
            entries.flatten()
                .filter(|e| {
                    let p = e.path();
                    p.is_file() && matches!(
                        p.extension().and_then(|x| x.to_str()).unwrap_or(""),
                        "jpg" | "jpeg" | "png" | "webp"
                    )
                })
                .map(|e| e.path().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

// messages.sendMedia with inputMediaUploadedPhoto for sending photo to BotFather
fn build_send_photo_message(peer_id: i64, access_hash: i64, file_id: i64, parts: i32, filename: &str) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(peer_id, access_hash);
    let file = tl_gen::serialize_inputFile(file_id, parts, filename, "");
    let media = tl_gen::serialize_inputMediaUploadedPhoto(false, false, &file, None, None, None);
    tl_gen::build_messages_sendMedia(
        false, false, false, false, false, false, false,
        &peer, None, &media, "", rand::random(), None, None, None, None, None, None, None, None, None,
    )
}

// poll for new message: check history count every 300ms until it increases, up to 5s
async fn wait_for_new_message(client: &mut MtpClient, peer_id: i64, access_hash: i64, expected_min: usize, token: &Arc<AtomicBool>) {
    for _ in 0..10 { // 10 * 500ms = 5s
        if !token.load(Ordering::Relaxed) { return; }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let req = tl::build_get_history(peer_id, access_hash, 5);
        if let Ok(data) = client.invoke(&req).await {
            if let Ok(msgs) = tl::parse_messages_history(&data) {
                if msgs.len() >= expected_min {
                    return;
                }
            }
        }
    }
}

// get current message count in history
async fn get_history_count(client: &mut MtpClient, peer_id: i64, access_hash: i64) -> usize {
    let req = tl::build_get_history(peer_id, access_hash, 10);
    if let Ok(data) = client.invoke(&req).await {
        if let Ok(msgs) = tl::parse_messages_history(&data) {
            return msgs.len();
        }
    }
    0
}

// ─── New helpers ───────────────────────────────────────────────────────────

fn init_bots_db(path: &std::path::PathBuf) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(path).map_err(|e| t_with("botcreator_db_open_error", &[("error", &e.to_string())]))?;
    conn.execute_batch("
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS bots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account TEXT DEFAULT '',
            token TEXT DEFAULT '',
            username TEXT DEFAULT '',
            name TEXT DEFAULT '',
            bio TEXT DEFAULT '',
            description TEXT DEFAULT '',
            photo TEXT DEFAULT '',
            restrict_groups INTEGER DEFAULT 0,
            status TEXT DEFAULT 'pending',
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_bots_status ON bots(status);
    ").map_err(|e| t_with("botcreator_db_tables_error", &[("error", &e.to_string())]))?;
    Ok(conn)
}

fn resolve_bot_output_path(user_path: &str) -> std::path::PathBuf {
    let trimmed = user_path.trim();
    if !trimmed.is_empty() {
        let p = std::path::PathBuf::from(trimmed);
        return if p.extension().map(|e| e == "db").unwrap_or(false) { p } else { p.with_extension("db") };
    }
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("kastor")
        .join("create_bots");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    base.join(format!("bots_{now}.db"))
}

/// Simple spintax processor: {opt1|opt2|opt3} → randomly picks one option.
/// Supports nested spintax.
fn spin_text(input: &str) -> String {
    let mut result = input.to_string();
    // process innermost braces first (handles nesting)
    loop {
        if let Some(start) = result.rfind('{') {
            if let Some(end) = result[start..].find('}') {
                let end = start + end;
                let options: Vec<&str> = result[start + 1..end].split('|').collect();
                let choice = if options.is_empty() {
                    String::new()
                } else {
                    options[rand::random::<usize>() % options.len()].to_string()
                };
                result = format!("{}{}{}", &result[..start], choice, &result[end + 1..]);
                continue;
            }
        }
        break;
    }
    result
}

/// /setprivacy → select bot → click "Disable" (second button / row index 1)
async fn botfather_set_privacy(
    client: &mut MtpClient,
    bf_id: i64,
    bf_access_hash: i64,
    bot_username: &str,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    if !token.load(Ordering::Relaxed) { return Ok(()); }
    let rid: i64 = rand::random();
    let req = tl::build_send_message(bf_id, bf_access_hash, "/setprivacy", rid);
    client.invoke(&req).await?;
    rate_limit().await;

    if !token.load(Ordering::Relaxed) { return Ok(()); }
    // select the bot
    let history_req = tl::build_get_history(bf_id, bf_access_hash, 3);
    let history_data = client.invoke(&history_req).await?;

    if let Some((msg_id, callback_data)) = find_bot_button_in_keyboard(&history_data, bot_username) {
        let cb_req = tl::build_bot_callback_answer(bf_id, bf_access_hash, msg_id, &callback_data);
        let _ = client.invoke(&cb_req).await;
    } else {
        let rid2: i64 = rand::random();
        let req2 = tl::build_send_message(bf_id, bf_access_hash, &format!("@{}", bot_username), rid2);
        client.invoke(&req2).await?;
    }
    rate_limit().await;

    if !token.load(Ordering::Relaxed) { return Ok(()); }
    // now BotFather shows "Enable" / "Disable" buttons. We want "Disable" (restricts group access)
    let history_req2 = tl::build_get_history(bf_id, bf_access_hash, 3);
    let history_data2 = client.invoke(&history_req2).await?;

    // try clicking the first callback button (usually "Disable")
    if let Ok(messages) = tl::parse_messages_structured(&history_data2) {
        for msg in messages {
            if let Some(ref btn_data) = msg.first_button_data {
                let cb_req = tl::build_bot_callback_answer(bf_id, bf_access_hash, msg.id, btn_data);
                let _ = client.invoke(&cb_req).await;
                break;
            }
        }
    }
    rate_limit().await;
    Ok(())
}
