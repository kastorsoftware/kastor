// account actions: bulk profile modifications via mtproto

use rusqlite;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, Manager};

use super::commands::get_storage_pub;
use super::session::AccountJson;
use crate::i18n::{t, t_with};
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::mtproto::tl_gen;
use crate::queue::TaskQueue;

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ActionConfig {
    pub delete_username: bool,
    pub change_username: bool,
    pub username_mode: String,
    pub username_prefix: String,
    pub username_list_path: String,

    pub set_photo: bool,
    pub photo_folder_path: String,
    pub delete_all_photos: bool,

    pub change_name: bool,
    pub name_only: bool,
    pub names_file_path: String,
    pub surnames_file_path: String,

    pub change_bio: bool,
    pub bio_file_path: String,
    pub bio_mode: String,
    pub bio_single: String,
    pub delete_bio: bool,

    pub set_birthday: bool,
    pub birthday_day_range: String,
    pub birthday_months: Vec<u32>,
    pub birthday_year_range: String,

    pub set_emoji_avatar: bool,

    pub reset_password: bool,
    pub set_password: bool,
    pub password_value: String,

    pub delete_all_stories: bool,

    pub read_all_dialogs: bool,

    pub hide_phone_number: bool,
    pub hide_online_status: bool,

    pub delete_contacts: bool,
    pub delete_all_dialogs: bool,
    pub delete_bot_dialogs: bool,
    pub delete_folders: bool,
    pub unsubscribe_channels: bool,

    pub delete_account: bool,
    pub logout_after: bool,

    #[serde(default)]
    pub randomize_order: bool,
    #[serde(default)]
    pub set_auto_photo: bool,
    #[serde(default)]
    pub delay_between_min: u32,
    #[serde(default)]
    pub delay_between_max: u32,
    #[serde(default)]
    pub account_ttl: i32, // 0 = don't change
    #[serde(default)]
    pub session_ttl: i32, // 0 = don't change

    #[serde(default)]
    pub max_flood_wait: u64,
}

#[tauri::command]
pub async fn account_actions_start(
    ids: Vec<String>,
    config: ActionConfig,
    threads: Option<usize>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let concurrency = threads.unwrap_or(5).clamp(1, 1000);
    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "account_actions".to_string(),
            t_with(
                "actions_working_on_accounts",
                &[("count", &ids.len().to_string())],
            ),
        )
        .await;

    // backend validation: username list must have enough entries
    if config.change_username
        && config.username_mode == "from_list"
        && !config.username_list_path.is_empty()
    {
        let usernames = load_lines(&config.username_list_path);
        let usable = usernames.iter().filter(|l| !l.is_empty()).count();
        if usable < ids.len() {
            return Err(t_with(
                "actions_not_enough_usernames",
                &[
                    ("usable", &usable.to_string()),
                    ("total", &ids.len().to_string()),
                ],
            ));
        }
    }

    let app_arc = app;

    // load resources
    let names = load_lines(&config.names_file_path);
    let surnames = load_lines(&config.surnames_file_path);
    let bios = load_lines(&config.bio_file_path);
    let usernames = load_lines(&config.username_list_path);
    let photos = load_photo_paths(&config.photo_folder_path);

    let config = Arc::new(config);
    let names = Arc::new(names);
    let surnames = Arc::new(surnames);
    let bios = Arc::new(bios);
    let usernames = Arc::new(usernames);
    let photos = Arc::new(photos);
    let username_idx = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    tokio::spawn(async move {
        // Create shared actions SQLite DB for this session
        let db_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("kastor")
            .join("actions");
        std::fs::create_dir_all(&db_dir).ok();
        let ts = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let db_path = db_dir.join(format!("{}_actions.db", ts));
        let actions_db = init_actions_db(&db_path);
        if let Ok(ref _db) = actions_db {
            let _ = app_arc.emit(
                "account-actions-log",
                t_with(
                    "actions_db_path",
                    &[("path", &db_path.display().to_string())],
                ),
            );
        }
        let actions_db = Arc::new(tokio::sync::Mutex::new(actions_db.ok()));

        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let total = ids.len();
        let mut handles = Vec::new();

        for (i, id) in ids.into_iter().enumerate() {
            if !token.load(Ordering::Relaxed) {
                break;
            }

            let sem = sem.clone();
            let config = config.clone();
            let names = names.clone();
            let surnames = surnames.clone();
            let bios = bios.clone();
            let usernames = usernames.clone();
            let photos = photos.clone();
            let app_clone = app_arc.clone();
            let username_idx = username_idx.clone();
            let token_clone = token.clone();
            let actions_db_clone = actions_db.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                if !token_clone.load(Ordering::Relaxed) {
                    return false;
                }

                let result = process_account(
                    &id,
                    i + 1,
                    total,
                    &config,
                    &names,
                    &surnames,
                    &bios,
                    &usernames,
                    &photos,
                    &username_idx,
                    &actions_db_clone,
                    &app_clone,
                    &token_clone,
                )
                .await;
                match result {
                    Ok(_) => {
                        let _ =
                            app_clone.emit("account-actions-log", format!("__DONE__:{}", i + 1));
                        false
                    }
                    Err(e) => {
                        crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                        let msg = if crate::mtproto::client::is_fatal_session_error(&e) {
                            format!("[{}/{}] {}", i + 1, total, t("actions_account_dead"))
                        } else {
                            format!(
                                "[{}/{}] {}",
                                i + 1,
                                total,
                                t_with("actions_error_generic", &[("error", &e)])
                            )
                        };
                        let _ = app_clone.emit("account-actions-log", msg);
                        false
                    }
                }
            }));
        }

        for h in handles {
            if let Ok(_) = h.await {}
        }

        let _ = app_arc.emit("account-actions-log", t("done"));

        let queue: tauri::State<'_, TaskQueue> = app_arc.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn account_actions_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn rate_limit() {
    let jitter = rand::random::<u64>() % 50;
    tokio::time::sleep(std::time::Duration::from_millis(100 + jitter)).await;
}

async fn delay_between_actions(_client: &mut MtpClient, min_sec: u32, max_sec: u32) {
    if min_sec == 0 && max_sec == 0 {
        return;
    }
    let min = min_sec.max(1) as u64;
    let max = max_sec.max(min as u32) as u64;
    let secs = if min == max {
        min
    } else {
        min + rand::random::<u64>() % (max - min + 1)
    };
    dbg_log!(
        "account_actions::delay_between_actions range={}..{}, selected={} sec",
        min,
        max,
        secs
    );
    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
}

fn action_name(action_tag: u8) -> &'static str {
    match action_tag {
        0 => "delete_username",
        1 => "change_username",
        2 => "delete_all_photos",
        3 => "delete_all_stories",
        4 => "set_photo",
        5 => "set_auto_photo",
        6 => "set_emoji_avatar",
        7 => "change_name",
        8 => "change_bio",
        9 => "delete_bio",
        10 => "set_birthday",
        11 => "delete_contacts",
        12 => "delete_all_dialogs",
        13 => "delete_bot_dialogs",
        14 => "read_all_dialogs",
        15 => "delete_folders",
        16 => "unsubscribe_channels",
        17 => "hide_phone_number",
        18 => "hide_online_status",
        19 => "account_ttl",
        20 => "session_ttl",
        21 => "password",
        _ => "unknown",
    }
}

async fn process_account(
    id: &str,
    idx: usize,
    total: usize,
    config: &ActionConfig,
    names: &[String],
    surnames: &[String],
    bios: &[String],
    usernames_list: &[String],
    photos: &[String],
    username_idx: &std::sync::atomic::AtomicUsize,
    actions_db: &Arc<tokio::sync::Mutex<Option<rusqlite::Connection>>>,
    app: &tauri::AppHandle,
    cancel_token: &Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    let emit = |msg: String| {
        let _ = app.emit("account-actions-log", msg);
    };
    let storage = get_storage_pub();
    let session_path = storage.session_path(id);
    let json_path = storage.json_path(id);

    let mut client = crate::accounts::connect::connect_account(id).await?;
    client.set_max_flood_wait(config.max_flood_wait);
    client.set_log_target("account-actions-log", app.clone());

    let mut json = if json_path.exists() {
        AccountJson::from_file(&json_path).unwrap_or_default()
    } else {
        AccountJson::default()
    };

    let prefix = format!(
        "[{}/{}] +{}",
        idx,
        total,
        if json.phone.is_empty() {
            "?"
        } else {
            &json.phone
        }
    );
    client.set_log_prefix(&prefix);

    let online_req = tl_gen::build_account_updateStatus(false);
    if let Err(e) = client.invoke(&online_req).await {
        dbg_log!(
            "account_actions::process_account prefix='{}' could not set online: {e}",
            prefix
        );
    }

    macro_rules! action_err {
        ($e:expr, $($arg:tt)*) => {{
            if crate::mtproto::is_fatal_session_error(&$e) {
                return Err($e);
            }
            emit(format!($($arg)*));
        }};
    }

    macro_rules! check_cancel {
        () => {
            if !cancel_token.load(Ordering::Relaxed) {
                return Ok(());
            }
        };
    }

    let result = async {
    // execute actions
    if config.delete_account {
        emit(format!("{} {}", prefix, t("actions_delete_account")));
        let req = tl::build_account_delete("User requested deletion");
        client.invoke(&req).await.map_err(|e| format!("delete_account: {e}"))?;
        emit(format!("{} {}", prefix, t("actions_account_deleted")));
        let _ = std::fs::remove_file(&session_path);
        let _ = std::fs::remove_file(&json_path);
        let tdata_dir = storage.tdata_dir(id);
        if tdata_dir.exists() { let _ = std::fs::remove_dir_all(&tdata_dir); }
        super::commands::invalidate_accounts_cache();
        return Ok(());
    }

    // randomize_order: shuffle actual action execution order
    let mut action_queue: Vec<u8> = Vec::new();
    if config.delete_username { action_queue.push(0); }
    if config.change_username { action_queue.push(1); }
    if config.delete_all_photos { action_queue.push(2); }
    if config.delete_all_stories { action_queue.push(3); }
    if config.set_photo && !photos.is_empty() { action_queue.push(4); }
    if config.set_auto_photo && !config.set_photo { action_queue.push(5); }
    if config.set_emoji_avatar { action_queue.push(6); }
    if config.change_name && !names.is_empty() { action_queue.push(7); }
    if config.change_bio { action_queue.push(8); }
    if config.delete_bio { action_queue.push(9); }
    if config.set_birthday && !config.birthday_months.is_empty() { action_queue.push(10); }
    if config.delete_contacts { action_queue.push(11); }
    if config.delete_all_dialogs { action_queue.push(12); }
    if config.delete_bot_dialogs && !config.delete_all_dialogs { action_queue.push(13); }
    if config.read_all_dialogs { action_queue.push(14); }
    if config.delete_folders { action_queue.push(15); }
    if config.unsubscribe_channels { action_queue.push(16); }
    if config.hide_phone_number { action_queue.push(17); }
    if config.hide_online_status { action_queue.push(18); }
    if config.account_ttl > 0 { action_queue.push(19); }
    if config.session_ttl > 0 { action_queue.push(20); }
    if config.reset_password || config.set_password { action_queue.push(21); }

    if config.randomize_order {
        use rand::seq::SliceRandom;
        action_queue.shuffle(&mut rand::thread_rng());
    }

    dbg_log!("account_actions::process_account id={} prefix='{}' actions={:?}", id, prefix, action_queue);

    for action_tag in &action_queue {
    check_cancel!();
    dbg_log!("account_actions::process_account prefix='{}' starting action={} ({})", prefix, action_tag, action_name(*action_tag));
    match *action_tag {

    0 => { // delete username
        emit(format!("{} {}", prefix, t("actions_delete_username")));
        let req = tl::build_account_update_username("");
        match client.invoke(&req).await {
            Ok(_) => {
                json.username.clear();
                let _ = json.to_file(&json_path);
                super::commands::invalidate_accounts_cache();
                emit(format!("{} {}", prefix, t("actions_username_deleted")));
            }
            Err(e) if e.contains("USERNAME_NOT_MODIFIED") => {
                emit(format!("{} {}", prefix, t("actions_username_not_set")));
            }
            Err(e) => return Err(format!("delete_username: {e}")),
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    1 => { // change username
        let is_random_mode = config.username_mode != "from_list";
        let max_attempts = if is_random_mode { 5 } else { 1 };
        let mut username_set = false;

        for attempt in 0..max_attempts {
            let new_username = match config.username_mode.as_str() {
                "from_list" => {
                    if usernames_list.is_empty() { return Err("username list empty".into()); }
                    let i = username_idx.fetch_add(1, Ordering::Relaxed);
                    if i >= usernames_list.len() {
                        return Err("usernames exhausted".into());
                    }
                    usernames_list[i].clone()
                }
                "prefix_random" => {
                    let prefix_str = &config.username_prefix;
                    let prefix_chars = prefix_str.chars().count();
                    if prefix_chars > 30 {
                        return Err(t_with("actions_prefix_too_long", &[("chars", &prefix_chars.to_string())]));
                    }
                    let max_suffix = 32usize.saturating_sub(prefix_chars).saturating_sub(1);
                    let suffix_len = max_suffix.min(6).max(3);
                    let suffix: String = (0..suffix_len).map(|_| (b'a' + rand::random::<u8>() % 26) as char).collect();
                    format!("{}_{}", prefix_str, suffix)
                }
                _ => {
                    let len = 5 + rand::random::<usize>() % 4;
                    (0..len).map(|_| (b'a' + rand::random::<u8>() % 26) as char).collect()
                }
            };

            if attempt == 0 {
                emit(format!("{} {}", prefix, t_with("actions_changing_username", &[("username", &new_username)])));
            }

            let req = tl::build_account_update_username(&new_username);
            match client.invoke(&req).await {
                Ok(_) => {
                    json.username = new_username.clone();
                    let _ = json.to_file(&json_path);
                    super::commands::invalidate_accounts_cache();
                    emit(format!("{} {}", prefix, t_with("actions_username_changed", &[("username", &new_username)])));
                    username_set = true;
                    break;
                }
                Err(e) if e.contains("USERNAME_NOT_MODIFIED") => {
                    emit(format!("{} {}", prefix, t_with("actions_username_unchanged", &[("username", &new_username)])));
                    username_set = true;
                    break;
                }
                Err(e) if e.contains("USERNAME_OCCUPIED") || e.contains("USERNAME_INVALID") || e.contains("USERNAME_PURCHASE_AVAILABLE") => {
                    if is_random_mode && attempt + 1 < max_attempts {
                        emit(format!("{} {}", prefix, t_with("actions_username_unavailable", &[("username", &new_username)])));
                        continue;
                    }
                    emit(format!("{} {}", prefix, t_with("actions_username_unavail_err", &[("username", &new_username), ("error", &e)])));
                    break;
                }
                Err(e) => {
                    action_err!(e, "{} {}", prefix, t_with("actions_username_error", &[("error", &e)]));
                    break;
                }
            }
        }
        let _ = username_set;
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    2 => { // delete all photos
        emit(format!("{} {}", prefix, t("actions_delete_avatars")));
        let req = tl::build_photos_get_user_photos(100);
        match client.invoke(&req).await {
            Ok(data) => {
                if let Ok(photos) = tl::parse_user_photos(&data) {
                    if !photos.is_empty() {
                        let del_req = tl::build_photos_delete(&photos);
                        if let Err(e) = client.invoke(&del_req).await {
                            dbg_log!("удаление аватарок не удалось: {e}");
                        }
                        emit(format!("{} {}", prefix, t_with("actions_avatars_deleted", &[("count", &photos.len().to_string())])));
                    } else {
                        emit(format!("{} {}", prefix, t("actions_no_avatars")));
                    }
                }
            }
            Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_photo_get_error", &[("error", &e)])),
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    3 => { // delete all stories
        emit(format!("{} {}", prefix, t("actions_delete_stories")));
        let peer = crate::mtproto::tl_gen::serialize_input_peer_self();
        let mut offset_id = 0i32;
        let mut total_deleted = 0u32;
        loop {
            let req = crate::mtproto::tl_gen::build_stories_getStoriesArchive(&peer, offset_id, 100);
            match client.invoke(&req).await {
                Ok(data) => {
                    let ids = extract_story_ids_from_response(&data);
                    if ids.is_empty() { break; }
                    offset_id = *ids.last().unwrap();
                    let del_req = tl::build_delete_stories(&ids);
                    if let Err(e) = client.invoke(&del_req).await {
                        emit(format!("{} {}", prefix, t_with("actions_stories_error", &[("error", &e)])));
                        break;
                    }
                    total_deleted += ids.len() as u32;
                }
                Err(e) => {
                    action_err!(e, "{} {}", prefix, t_with("actions_stories_get_error", &[("error", &e)]));
                    break;
                }
            }
        }
        if total_deleted > 0 {
            emit(format!("{} {}", prefix, t_with("actions_stories_deleted", &[("count", &total_deleted.to_string())])));
        } else {
            emit(format!("{} {}", prefix, t("actions_no_stories")));
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    4 => { // set photo from folder
        emit(format!("{} {}", prefix, t("actions_set_photo")));
        let photo_path = &photos[rand::random::<usize>() % photos.len()];
        match upload_and_set_photo(&mut client, photo_path).await {
            Ok(_) => emit(format!("{} {}", prefix, t("actions_photo_set"))),
            Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_photo_error", &[("error", &e)])),
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    5 => { // set auto-generated photo
        emit(format!("{} {}", prefix, t("actions_gen_photo")));
        match download_random_face() {
            Ok(image_data) => {
                match upload_and_set_photo_bytes(&mut client, &image_data, "auto_photo.jpg").await {
                    Ok(_) => emit(format!("{} {}", prefix, t("actions_autophoto_set"))),
                    Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_photo_ul_error", &[("error", &e)])),
                }
            }
            Err(e) => emit(format!("{} {}", prefix, t_with("actions_photo_dl_error", &[("error", &e)]))),
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    6 => { // set emoji avatar
        emit(format!("{} {}", prefix, t("actions_set_emoji_avatar")));
        let emoji_req = tl::build_get_default_profile_photo_emojis();
        match client.invoke(&emoji_req).await {
            Ok(data) => {
                match tl::parse_emoji_list(&data) {
                    Ok(ids) if !ids.is_empty() => {
                        let mut success = false;
                        for _ in 0..3 {
                            let emoji_id = ids[rand::random::<usize>() % ids.len()];
                            let colors = random_background_colors();
                            let req = tl::build_photos_upload_emoji_avatar(emoji_id, &colors);
                            match client.invoke(&req).await {
                                Ok(_) => {
                                    emit(format!("{} {}", prefix, t("actions_emoji_avatar_set")));
                                    success = true;
                                    break;
                                }
                                Err(e) if e.contains("EMOJI_MARKUP_INVALID") => {
                                    continue;
                                }
                                Err(e) => {
                                    action_err!(e, "{} {}", prefix, t_with("actions_emoji_avatar_error", &[("error", &e)]));
                                    break;
                                }
                            }
                        }
                        if !success {
                            emit(format!("{} {}", prefix, t("actions_emoji_no_valid")));
                        }
                    }
                    Ok(_) => emit(format!("{} {}", prefix, t("actions_emoji_list_empty"))),
                    Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_emoji_get_error", &[("error", &e)])),
                }
            }
            Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_emoji_req_error", &[("error", &e)])),
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    7 => { // change name
        let first = &names[rand::random::<usize>() % names.len()];
        let last = if config.name_only || surnames.is_empty() { "" } else { &surnames[rand::random::<usize>() % surnames.len()] };
        emit(format!("{} {}", prefix, t_with("actions_changing_name", &[("first", first), ("last", last)])));
        let req = tl::build_account_update_profile(Some(first), Some(last), None);
        client.invoke(&req).await.map_err(|e| format!("update_profile: {e}"))?;
        json.first_name = first.clone();
        json.last_name = last.to_string();
        let _ = json.to_file(&json_path);
        super::commands::invalidate_accounts_cache();
        emit(format!("{} {}", prefix, t_with("actions_name_set", &[("first", first), ("last", last)])));
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    8 => { // change bio
        let bio = if config.bio_mode == "single" {
            if config.bio_single.is_empty() { return Err("bio_single empty".into()); }
            config.bio_single.clone()
        } else {
            if bios.is_empty() { return Err("bio file empty".into()); }
            bios[rand::random::<usize>() % bios.len()].clone()
        };
        emit(format!("{} {}", prefix, t("actions_set_bio")));
        let req = tl::build_account_update_profile(None, None, Some(&bio));
        client.invoke(&req).await.map_err(|e| format!("update_bio: {e}"))?;
        emit(format!("{} {}", prefix, t("actions_bio_updated")));
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    9 => { // delete bio
        emit(format!("{} {}", prefix, t("actions_delete_bio")));
        let req = tl::build_account_update_profile(None, None, Some(""));
        client.invoke(&req).await.map_err(|e| format!("delete_bio: {e}"))?;
        emit(format!("{} {}", prefix, t("actions_bio_deleted")));
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    10 => { // set birthday
        let (day_min, day_max) = parse_range(&config.birthday_day_range, 1, 28);
        let now_secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let current_year = 1970 + (now_secs / 31_557_600) as i32; // approximate
        let (year_min, year_max) = parse_range(&config.birthday_year_range, 1960, current_year);
        // clamp
        let day_min = day_min.max(1).min(28);
        let day_max = day_max.max(1).min(28);
        let year_min = year_min.max(1960).min(current_year);
        let year_max = year_max.max(1960).min(current_year);

        let day = day_min + rand::random::<i32>().unsigned_abs() as i32 % (day_max - day_min + 1).max(1);
        let month = config.birthday_months[rand::random::<usize>() % config.birthday_months.len()] as i32;
        let year = year_min + rand::random::<i32>().unsigned_abs() as i32 % (year_max - year_min + 1).max(1);

        emit(format!("{} {}", prefix, t_with("actions_set_birthday", &[("d", &day.to_string()), ("m", &month.to_string()), ("y", &year.to_string())])));
        let req = tl::build_account_update_birthday(day, month, Some(year));
        client.invoke(&req).await.map_err(|e| format!("birthday: {e}"))?;
        emit(format!("{} {}", prefix, t("actions_birthday_set")));
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    11 => { // delete all contacts (one by one with 1s delay)
        emit(format!("{} {}", prefix, t("actions_delete_contacts")));
        let req = tl::build_contacts_get_contacts();
        match client.invoke(&req).await {
            Ok(data) => {
                let detailed = tl::parse_contacts_detailed(&data).unwrap_or_default();
                match tl::parse_contacts_response(&data) {
                    Ok(contacts) if !contacts.is_empty() => {
                        let total_contacts = contacts.len();
                        dbg_log!("account_actions::delete_contacts prefix='{}' contacts={}", prefix, total_contacts);
                        let mut deleted = 0u32;
                        for contact in &contacts {
                            check_cancel!();
                            let del_req = tl::build_contacts_delete_contacts(&[*contact]);
                            match client.invoke(&del_req).await {
                                Ok(_) => deleted += 1,
                                Err(e) => {
                                    if let Some(wait) = extract_flood_seconds(&e) {
                                        emit(format!("{} {}", prefix, t_with("mtproto_flood_waiting", &[("wait", &wait.to_string())])));
                                        tokio::time::sleep(std::time::Duration::from_secs(wait + 1)).await;
                                        if client.invoke(&del_req).await.is_ok() { deleted += 1; }
                                    }
                                }
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        }
                        emit(format!("{} {}", prefix, t_with("actions_contacts_deleted", &[("count", &deleted.to_string())])));
                        // Log to shared DB
                        let db_lock = actions_db.lock().await;
                        if let Some(ref conn) = *db_lock {
                            for c in &detailed {
                                log_action(conn, id, "delete_contact", &format!("user_id={} @{} +{} {} {}", c.user_id, c.username, c.phone, c.first_name, c.last_name), "done");
                            }
                        }
                        let _ = total_contacts;
                    }
                    Ok(_) => emit(format!("{} {}", prefix, t("actions_no_contacts"))),
                    Err(e) => emit(format!("{} {}", prefix, t_with("actions_contacts_get_error", &[("error", &e)]))),
                }
            }
            Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_contacts_req_error", &[("error", &e)])),
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    12 => { // delete all dialogs (batched via msg_container)
        emit(format!("{} {}", prefix, t("actions_delete_dialogs")));
        let offset_date = 0i32;
        let mut offset_id = 0i32;
        let mut offset_peer = tl_gen::INPUT_PEER_EMPTY.to_le_bytes().to_vec();
        let mut total_deleted = 0u32;
        let mut page = 0u32;
        loop {
            check_cancel!();
            let req = tl::build_get_dialogs_paged(0, 100, offset_date, offset_id, &offset_peer);
            match client.invoke(&req).await {
                Ok(data) => {
                    let inner = tl_gen::unwrap_rpc(&data).map_err(|e| format!("unwrap: {e}"))?;
                    let mut cursor = Cursor::new(inner.as_slice());
                    let resp = tl_gen::TlMessagesDialogs::deserialize(&mut cursor)
                        .map_err(|e| format!("deserialize: {e}"))?;
                    let (dialogs_raw, chats_raw, users_raw) = match resp {
                        tl_gen::TlMessagesDialogs::Dialogs { dialogs, chats, users, .. } => (dialogs, chats, users),
                        tl_gen::TlMessagesDialogs::Slice { dialogs, chats, users, .. } => (dialogs, chats, users),
                        tl_gen::TlMessagesDialogs::NotModified { .. } => break,
                    };
                    if dialogs_raw.is_empty() { break; }
                    page += 1;
                    let peers = tl::parse_dialog_peers_from_parts(&chats_raw, &users_raw);
                    dbg_log!("account_actions::delete_all_dialogs prefix='{}' page={} cursor_id={} dialogs={} peers={}", prefix, page, offset_id, dialogs_raw.len(), peers.len());

                    // Delete one by one with 1s delay
                    for peer in &peers {
                        check_cancel!();
                        let del_req = match peer {
                            tl::DialogPeer::User { id, access_hash, .. } => tl::build_delete_history(*id, *access_hash),
                            tl::DialogPeer::Chat { id } => tl::build_delete_history_chat(*id),
                            tl::DialogPeer::Channel { .. } => continue,
                        };
                        if client.invoke(&del_req).await.is_ok() {
                            total_deleted += 1;
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }

                    let (next_id, next_peer) = extract_last_dialog_offset(&dialogs_raw);
                    let cursor_changed = next_id != offset_id || next_peer != offset_peer;
                    dbg_log!("account_actions::delete_all_dialogs prefix='{}' page={} next_cursor_id={} cursor_changed={}", prefix, page, next_id, cursor_changed);
                    if !cursor_changed {
                        dbg_log!("account_actions::delete_all_dialogs prefix='{}' stopping: pagination cursor did not advance", prefix);
                        break;
                    }
                    offset_id = next_id;
                    offset_peer = next_peer;
                    rate_limit().await;
                }
                Err(e) => {
                    // Handle FLOOD_WAIT: parse seconds from error, sleep, retry
                    if let Some(wait) = extract_flood_seconds(&e) {
                        emit(format!("{} {}", prefix, t_with("mtproto_flood_waiting", &[("wait", &wait.to_string())])));
                        tokio::time::sleep(std::time::Duration::from_secs(wait + 1)).await;
                        continue;
                    }
                    action_err!(e, "{} {}", prefix, t_with("actions_dialogs_get_error", &[("error", &e)]));
                    break;
                }
            }
        }
        emit(format!("{} {}", prefix, t_with("actions_dialogs_deleted", &[("count", &total_deleted.to_string())])));
        {
            let db_lock = actions_db.lock().await;
            if let Some(ref conn) = *db_lock {
                log_action(conn, id, "delete_all_dialogs", &format!("deleted={}", total_deleted), "done");
            }
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    13 => { // delete bot dialogs only
        emit(format!("{} {}", prefix, t("actions_delete_bot_dialogs")));
        let mut deleted = 0u32;
        // Check both main dialogs and archive
        for folder_id in [0, 1] {
            let req = tl::build_get_dialogs_with_folder(folder_id, 500);
            match client.invoke(&req).await {
                Ok(data) => {
                    match tl::parse_dialog_peers(&data) {
                        Ok(peers) => {
                            for peer in &peers {
                                if let tl::DialogPeer::User { id, access_hash, is_bot } = peer {
                                    if *is_bot {
                                        let r = tl::build_delete_history(*id, *access_hash);
                                        if let Err(e) = client.invoke(&r).await {
                                            dbg_log!("удаление истории с ботом id={} не удалось: {e}", id);
                                        }
                                        let b = tl::build_block_peer(*id, *access_hash);
                                        if let Err(e) = client.invoke(&b).await {
                                            dbg_log!("блокировка бота id={} не удалась: {e}", id);
                                        }
                                        deleted += 1;
                                    }
                                }
                            }
                        }
                        Err(e) => emit(format!("{} {}", prefix, t_with("actions_dialogs_parse_error", &[("error", &e)]))),
                    }
                }
                Err(e) => { dbg_log!("get dialogs folder={} error: {e}", folder_id); }
            }
        }
        emit(format!("{} {}", prefix, t_with("actions_bots_deleted", &[("count", &deleted.to_string())])));
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    14 => { // read all dialogs
        emit(format!("{} {}", prefix, t("actions_read_dialogs")));
        let offset_date = 0i32;
        let mut offset_id = 0i32;
        let mut offset_peer = tl_gen::INPUT_PEER_EMPTY.to_le_bytes().to_vec();
        let mut total_read = 0u32;
        let mut page = 0u32;
        loop {
            check_cancel!();
            let req = tl::build_get_dialogs_paged(0, 100, offset_date, offset_id, &offset_peer);
            match client.invoke(&req).await {
                Ok(data) => {
                    let inner = tl_gen::unwrap_rpc(&data).map_err(|e| format!("unwrap: {e}"))?;
                    let mut cursor = Cursor::new(inner.as_slice());
                    let resp = tl_gen::TlMessagesDialogs::deserialize(&mut cursor)
                        .map_err(|e| format!("deserialize: {e}"))?;
                    let (dialogs_raw, chats_raw, users_raw) = match resp {
                        tl_gen::TlMessagesDialogs::Dialogs { dialogs, chats, users, .. } => (dialogs, chats, users),
                        tl_gen::TlMessagesDialogs::Slice { dialogs, chats, users, .. } => (dialogs, chats, users),
                        tl_gen::TlMessagesDialogs::NotModified { .. } => break,
                    };
                    if dialogs_raw.is_empty() { break; }
                    page += 1;
                    dbg_log!("account_actions::read_all_dialogs prefix='{}' page={} cursor_id={} dialogs={}", prefix, page, offset_id, dialogs_raw.len());
                    match mark_dialogs_read_from_parts(&dialogs_raw, &chats_raw, &users_raw, &mut client).await {
                        Ok(count) => total_read += count,
                        Err(e) => { action_err!(e, "{} {}", prefix, t_with("actions_read_error", &[("error", &e)])); break; }
                    }
                    let (next_id, next_peer) = extract_last_dialog_offset(&dialogs_raw);
                    let cursor_changed = next_id != offset_id || next_peer != offset_peer;
                    dbg_log!("account_actions::read_all_dialogs prefix='{}' page={} next_cursor_id={} cursor_changed={}", prefix, page, next_id, cursor_changed);
                    if !cursor_changed {
                        dbg_log!("account_actions::read_all_dialogs prefix='{}' stopping: pagination cursor did not advance", prefix);
                        break;
                    }
                    offset_id = next_id;
                    offset_peer = next_peer;
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                Err(e) => {
                    if let Some(wait) = extract_flood_seconds(&e) {
                        emit(format!("{} {}", prefix, t_with("mtproto_flood_waiting", &[("wait", &wait.to_string())])));
                        tokio::time::sleep(std::time::Duration::from_secs(wait + 1)).await;
                        continue;
                    }
                    action_err!(e, "{} {}", prefix, t_with("actions_dialogs_get_error", &[("error", &e)]));
                    break;
                }
            }
        }
        emit(format!("{} {}", prefix, t_with("actions_dialogs_read", &[("count", &total_read.to_string())])));
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    15 => { // delete all folders
        emit(format!("{} {}", prefix, t("actions_delete_folders")));
        let req = tl::build_get_dialog_filters();
        match client.invoke(&req).await {
            Ok(data) => {
                match tl::parse_dialog_filter_ids(&data) {
                    Ok(ids) if !ids.is_empty() => {
                        let mut deleted = 0u32;
                        dbg_log!("account_actions::delete_folders prefix='{}' folders={}", prefix, ids.len());
                        for fid in &ids {
                            let del_req = tl::build_delete_dialog_filter(*fid);
                            if client.invoke(&del_req).await.is_ok() { deleted += 1; }
                        }
                        emit(format!("{} {}", prefix, t_with("actions_folders_deleted", &[("count", &deleted.to_string())])));
                    }
                    Ok(_) => emit(format!("{} {}", prefix, t("actions_no_folders"))),
                    Err(e) => emit(format!("{} {}", prefix, t_with("actions_folders_get_error", &[("error", &e)]))),
                }
            }
            Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_folders_req_error", &[("error", &e)])),
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    16 => { // unsubscribe from channels
        emit(format!("{} {}", prefix, t("actions_leaving_channels")));
        let req = tl::build_get_dialogs_with_folder(0, 500);
        match client.invoke(&req).await {
            Ok(data) => {
                match tl::parse_dialog_peers(&data) {
                    Ok(peers) => {
                        let mut left = 0u32;
                        let channels = peers.iter().filter(|peer| matches!(peer, tl::DialogPeer::Channel { .. })).count();
                        dbg_log!("account_actions::unsubscribe_channels prefix='{}' peers={} channels={}", prefix, peers.len(), channels);
                        for peer in &peers {
                            if let tl::DialogPeer::Channel { id, access_hash } = peer {
                                check_cancel!();
                                let r = tl::build_leave_channel(*id, *access_hash);
                                match client.invoke(&r).await {
                                    Ok(_) => left += 1,
                                    Err(e) => {
                                        if let Some(wait) = extract_flood_seconds(&e) {
                                            emit(format!("{} {}", prefix, t_with("mtproto_flood_waiting", &[("wait", &wait.to_string())])));
                                            tokio::time::sleep(std::time::Duration::from_secs(wait + 1)).await;
                                            // retry this one
                                            if client.invoke(&r).await.is_ok() { left += 1; }
                                        }
                                    }
                                }
                                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            }
                        }
                        emit(format!("{} {}", prefix, t_with("actions_channels_left", &[("count", &left.to_string())])));
                    }
                    Err(e) => emit(format!("{} {}", prefix, t_with("actions_dialogs_parse_error", &[("error", &e)]))),
                }
            }
            Err(e) => {
                if let Some(wait) = extract_flood_seconds(&e) {
                    emit(format!("{} {}", prefix, t_with("mtproto_flood_waiting", &[("wait", &wait.to_string())])));
                    tokio::time::sleep(std::time::Duration::from_secs(wait + 1)).await;
                } else {
                    action_err!(e, "{} {}", prefix, t_with("actions_dialogs_get_error", &[("error", &e)]));
                }
            }
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    17 => { // hide phone number
        emit(format!("{} {}", prefix, t("actions_hide_phone")));
        let key = serialize_input_privacy_key_phone_number();
        let rule = serialize_input_privacy_value_disallow_all();
        let req = tl_gen::build_account_setPrivacy(&key, &[&rule]);
        match client.invoke(&req).await {
            Ok(_) => emit(format!("{} {}", prefix, t("actions_phone_hidden"))),
            Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_phone_hide_error", &[("error", &e)])),
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    18 => { // hide online status
        emit(format!("{} {}", prefix, t("actions_hide_online")));
        let key = serialize_input_privacy_key_status_timestamp();
        let rule = serialize_input_privacy_value_disallow_all();
        let req = tl_gen::build_account_setPrivacy(&key, &[&rule]);
        match client.invoke(&req).await {
            Ok(_) => emit(format!("{} {}", prefix, t("actions_online_hidden"))),
            Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_online_hide_error", &[("error", &e)])),
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    19 => { // account TTL
        emit(format!("{} {}", prefix, t_with("actions_set_ttl", &[("days", &config.account_ttl.to_string())])));
        let ttl = tl_gen::serialize_accountDaysTTL(config.account_ttl);
        let req = tl_gen::build_account_setAccountTTL(&ttl);
        match client.invoke(&req).await {
            Ok(_) => emit(format!("{} {}", prefix, t_with("actions_ttl_set", &[("days", &config.account_ttl.to_string())]))),
            Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_ttl_error", &[("error", &e)])),
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    20 => { // session TTL
        emit(format!("{} {}", prefix, t_with("actions_set_session_ttl", &[("days", &config.session_ttl.to_string())])));
        let req = tl_gen::build_account_setAuthorizationTTL(config.session_ttl);
        match client.invoke(&req).await {
            Ok(_) => emit(format!("{} {}", prefix, t_with("actions_session_ttl_set", &[("days", &config.session_ttl.to_string())]))),
            Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_session_ttl_error", &[("error", &e)])),
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    21 => { // reset 2fa / set 2fa
        let has_2fa = !json.two_fa.is_empty() && !json.two_fa.starts_with(&t("two_fa_unknown")) && json.two_fa != t("two_fa_unknown_set");

        if config.reset_password && config.set_password {
            // smart mode: reset where 2fa exists, set where it doesn't
            if has_2fa || json.two_fa.starts_with(&t("two_fa_unknown")) || json.two_fa == t("two_fa_unknown_set") {
                emit(format!("{} {}", prefix, t("actions_reset_2fa")));
                let req = tl::build_account_reset_password();
                match client.invoke(&req).await {
                    Ok(_) => emit(format!("{} {}", prefix, t("actions_reset_2fa_sent"))),
                    Err(e) if e.contains("PASSWORD_EMPTY") => {
                        json.two_fa.clear();
                        let _ = json.to_file(&json_path);
                        super::commands::invalidate_accounts_cache();
                        emit(format!("{} {}", prefix, t("actions_2fa_not_set")));
                    }
                    Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_reset_2fa_error", &[("error", &e)])),
                }
            } else {
                emit(format!("{} {}", prefix, t("actions_set_2fa")));
                match set_2fa_password(&mut client, &config.password_value).await {
                    Ok(_) => {
                        json.two_fa = config.password_value.clone();
                        let _ = json.to_file(&json_path);
                        super::commands::invalidate_accounts_cache();
                        emit(format!("{} {}", prefix, t("actions_2fa_set")));
                    }
                    Err(e) if e.contains("уже установлен") => {
                        emit(format!("{} {}", prefix, t("actions_2fa_already_set")));
                    }
                    Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_2fa_error", &[("error", &e)])),
                }
            }
        } else if config.reset_password {
            emit(format!("{} {}", prefix, t("actions_reset_2fa")));
            let req = tl::build_account_reset_password();
            match client.invoke(&req).await {
                Ok(_) => emit(format!("{} {}", prefix, t("actions_reset_2fa_sent"))),
                Err(e) if e.contains("PASSWORD_EMPTY") => {
                    json.two_fa.clear();
                    let _ = json.to_file(&json_path);
                    super::commands::invalidate_accounts_cache();
                    emit(format!("{} {}", prefix, t("actions_2fa_not_set")));
                }
                Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_reset_2fa_error", &[("error", &e)])),
            }
        } else if config.set_password && !config.password_value.is_empty() {
            emit(format!("{} {}", prefix, t("actions_set_2fa")));
            match set_2fa_password(&mut client, &config.password_value).await {
                Ok(_) => {
                    json.two_fa = config.password_value.clone();
                    let _ = json.to_file(&json_path);
                    super::commands::invalidate_accounts_cache();
                    emit(format!("{} {}", prefix, t("actions_2fa_set")));
                }
                Err(e) if e.contains("уже установлен") => {
                    emit(format!("{} {}", prefix, t("actions_2fa_already_set")));
                }
                Err(e) => action_err!(e, "{} {}", prefix, t_with("actions_2fa_error", &[("error", &e)])),
            }
        }
        rate_limit().await;
        if config.delay_between_min > 0 || config.delay_between_max > 0 { delay_between_actions(&mut client, config.delay_between_min, config.delay_between_max).await; }
    }

    _ => {} // skip disabled actions

    } // match
    } // for action_queue

    // logout (always last, not shuffled)
    if config.logout_after {
        emit(format!("{} {}", prefix, t("actions_logout")));
        let req = tl::build_auth_log_out();
        if let Err(e) = client.invoke(&req).await {
            dbg_log!("выход из аккаунта (auth.logOut) не удался: {e}");
        }
        let _ = std::fs::remove_file(&session_path);
        let _ = std::fs::remove_file(&json_path);
        let tdata_dir = storage.tdata_dir(id);
        if tdata_dir.exists() { let _ = std::fs::remove_dir_all(&tdata_dir); }
        super::commands::invalidate_accounts_cache();
        emit(format!("{} {}", prefix, t("actions_logged_out")));
    }

    Ok(())
    }.await;

    let offline_req = tl_gen::build_account_updateStatus(true);
    if let Err(e) = client.invoke(&offline_req).await {
        dbg_log!(
            "account_actions::process_account prefix='{}' could not set offline: {e}",
            prefix
        );
    }

    result
}

fn extract_last_dialog_offset(dialogs_raw: &[Vec<u8>]) -> (i32, Vec<u8>) {
    if let Some(last) = dialogs_raw.last() {
        let mut c = Cursor::new(last.as_slice());
        if let Ok(d) = tl_gen::TlDialog::deserialize(&mut c) {
            if let tl_gen::TlDialog::Dialog {
                peer, top_message, ..
            } = d
            {
                return (top_message, peer);
            }
        }
    }
    (0, tl_gen::INPUT_PEER_EMPTY.to_le_bytes().to_vec())
}

fn load_lines(path: &str) -> Vec<String> {
    if path.is_empty() {
        return Vec::new();
    }
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().trim_start_matches('@').to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn load_photo_paths(folder: &str) -> Vec<String> {
    if folder.is_empty() {
        return Vec::new();
    }
    std::fs::read_dir(folder)
        .ok()
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let p = e.path();
                    p.is_file()
                        && matches!(
                            p.extension().and_then(|x| x.to_str()).unwrap_or(""),
                            "jpg" | "jpeg" | "png" | "webp"
                        )
                })
                .map(|e| e.path().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Extract FLOOD_WAIT seconds from error string like "FLOOD_WAIT_25" or "rpc 420: FLOOD_WAIT_25"
fn extract_flood_seconds(err: &str) -> Option<u64> {
    for pattern in &["FLOOD_WAIT_", "SLOWMODE_WAIT_", "FLOOD_PREMIUM_WAIT_"] {
        if let Some(pos) = err.find(pattern) {
            let after = &err[pos + pattern.len()..];
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = num_str.parse::<u64>() {
                return Some(n);
            }
        }
    }
    None
}

fn parse_range(s: &str, default_min: i32, default_max: i32) -> (i32, i32) {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 2 {
        let min = parts[0].trim().parse().unwrap_or(default_min);
        let max = parts[1].trim().parse().unwrap_or(default_max);
        (min.min(max), min.max(max))
    } else {
        (default_min, default_max)
    }
}

fn random_background_colors() -> Vec<i32> {
    let palettes: &[&[i32]] = &[
        &[0x6FB9F0_u32 as i32, 0x0088CC_u32 as i32],
        &[0xFFD67E_u32 as i32, 0xFC5C51_u32 as i32],
        &[0x82E1B8_u32 as i32, 0x0DC47A_u32 as i32],
        &[0xF48AAE_u32 as i32, 0xD4145A_u32 as i32],
        &[0xB694F9_u32 as i32, 0x7B61FF_u32 as i32],
        &[0xFFA85C_u32 as i32, 0xFF6B2C_u32 as i32],
    ];
    palettes[rand::random::<usize>() % palettes.len()].to_vec()
}

async fn upload_and_set_photo(client: &mut MtpClient, path: &str) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| format!("read file: {e}"))?;
    let file_id = rand::random::<i64>();
    let part_size = 128 * 1024; // 128KB parts
    let total_parts = ((data.len() + part_size - 1) / part_size) as i32;

    for i in 0..total_parts {
        let start = i as usize * part_size;
        let end = ((i as usize + 1) * part_size).min(data.len());
        let chunk = &data[start..end];
        let req = tl::build_upload_save_file_part(file_id, i, chunk);
        client
            .invoke(&req)
            .await
            .map_err(|e| format!("upload part {}: {e}", i))?;
    }

    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("photo.jpg");

    let req = tl::build_photos_upload_profile_photo(file_id, total_parts, filename);
    client
        .invoke(&req)
        .await
        .map_err(|e| format!("set photo: {e}"))?;
    Ok(())
}

async fn set_2fa_password(client: &mut MtpClient, password: &str) -> Result<(), String> {
    use crate::mtproto::auth::compute_new_password_verifier;

    let pw_req = tl::build_account_get_password();
    let pw_data = client
        .invoke(&pw_req)
        .await
        .map_err(|e| format!("get_password: {e}"))?;
    let pw_info =
        tl::parse_account_password(&pw_data).map_err(|e| format!("parse_password: {e}"))?;

    if pw_info.has_password {
        return Err(t("actions_2fa_already_set_err"));
    }

    if pw_info.new_p.is_empty() || pw_info.new_g == 0 {
        return Err(t("actions_2fa_no_algo_params"));
    }

    // salt1 must be new_algo.salt1 with 32 fresh random bytes appended (telegram spec)
    let mut salt1 = pw_info.new_salt1.clone();
    salt1.extend_from_slice(&generate_random_salt(32));
    let salt2 = pw_info.new_salt2.clone();

    // new_password_hash is the SRP verifier v = g^x mod p
    let verifier =
        compute_new_password_verifier(pw_info.new_g, &pw_info.new_p, &salt1, &salt2, password)?;

    let req = tl::build_account_set_password(
        pw_info.new_g,
        &pw_info.new_p,
        &salt1,
        &salt2,
        &verifier,
        "",
    );
    match client.invoke(&req).await {
        Ok(_) => Ok(()),
        Err(e)
            if e.contains("SRP_ID_INVALID")
                || e.contains("SRP_G_P_INVALID")
                || e.contains("NEW_SALT_INVALID") =>
        {
            Err(t("actions_2fa_srp_stale"))
        }
        Err(e) => Err(format!("set_password: {e}")),
    }
}

fn generate_random_salt(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut buf);
    buf
}

fn extract_story_ids_from_response(data: &[u8]) -> Vec<i32> {
    use crate::mtproto::tl_gen;
    let mut ids = Vec::new();
    let mut i = 0usize;
    while i + 4 <= data.len() {
        let c = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        if c == tl_gen::STORY_ITEM {
            let mut cursor = std::io::Cursor::new(&data[i..]);
            if let Ok(tl_gen::TlStoryItem::StoryItem { id, .. }) =
                tl_gen::TlStoryItem::deserialize(&mut cursor)
            {
                if id > 0 && !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        i += 4;
    }
    ids
}

async fn mark_dialogs_read_from_parts(
    dialogs_raw: &[Vec<u8>],
    chats_raw: &[Vec<u8>],
    users_raw: &[Vec<u8>],
    client: &mut MtpClient,
) -> Result<u32, String> {
    let mut user_hashes: HashMap<i64, i64> = HashMap::new();
    for raw in users_raw {
        if let Ok(user) =
            crate::mtproto::tl_gen::deserialize_tl_obj::<crate::mtproto::tl_gen::TlUser>(raw)
        {
            if let crate::mtproto::tl_gen::TlUser::User {
                id, access_hash, ..
            } = user
            {
                if let Some(ah) = access_hash {
                    user_hashes.insert(id, ah);
                }
            }
        }
    }

    let mut channel_hashes: HashMap<i64, i64> = HashMap::new();
    for raw in chats_raw {
        if let Ok(chat) =
            crate::mtproto::tl_gen::deserialize_tl_obj::<crate::mtproto::tl_gen::TlChat>(raw)
        {
            match chat {
                crate::mtproto::tl_gen::TlChat::Channel {
                    id, access_hash, ..
                } => {
                    if let Some(ah) = access_hash {
                        channel_hashes.insert(id, ah);
                    }
                }
                crate::mtproto::tl_gen::TlChat::ChannelForbidden {
                    id, access_hash, ..
                } => {
                    channel_hashes.insert(id, access_hash);
                }
                _ => {}
            }
        }
    }

    let mut read_count = 0u32;
    for dialog_raw in dialogs_raw {
        let mut cursor = Cursor::new(dialog_raw.as_slice());
        let dialog = match crate::mtproto::tl_gen::TlDialog::deserialize(&mut cursor) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let (peer, top_message, unread_count) = match dialog {
            crate::mtproto::tl_gen::TlDialog::Dialog {
                peer,
                top_message,
                unread_count,
                ..
            } => (peer, top_message, unread_count),
            crate::mtproto::tl_gen::TlDialog::Folder { .. } => continue,
            crate::mtproto::tl_gen::TlDialog::Community { .. } => continue,
        };

        // Skip already-read dialogs (optimization: avoid unnecessary API calls)
        if unread_count == 0 {
            continue;
        }

        let mut cursor = Cursor::new(peer.as_slice());
        let peer_type = match crate::mtproto::tl_gen::TlPeer::deserialize(&mut cursor) {
            Ok(p) => p,
            Err(_) => continue,
        };

        let req = match peer_type {
            crate::mtproto::tl_gen::TlPeer::User { user_id } => {
                let ah = user_hashes.get(&user_id).copied().unwrap_or(0);
                let input_peer = crate::mtproto::tl_gen::serialize_input_peer_user(user_id, ah);
                crate::mtproto::tl_gen::build_messages_readHistory(&input_peer, top_message)
            }
            crate::mtproto::tl_gen::TlPeer::Chat { chat_id } => {
                let input_peer = crate::mtproto::tl_gen::serialize_input_peer_chat(chat_id);
                crate::mtproto::tl_gen::build_messages_readHistory(&input_peer, top_message)
            }
            crate::mtproto::tl_gen::TlPeer::Channel { channel_id } => {
                let ah = channel_hashes.get(&channel_id).copied().unwrap_or(0);
                let input_channel = crate::mtproto::tl_gen::serialize_input_channel(channel_id, ah);
                crate::mtproto::tl_gen::build_channels_readHistory(&input_channel, top_message)
            }
        };

        if let Ok(_) = client.invoke(&req).await {
            read_count += 1;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    Ok(read_count)
}

fn serialize_input_privacy_key_phone_number() -> Vec<u8> {
    tl_gen::serialize_bare_ctor(tl_gen::INPUT_PRIVACY_KEY_PHONE_NUMBER)
}

fn serialize_input_privacy_key_status_timestamp() -> Vec<u8> {
    tl_gen::serialize_bare_ctor(tl_gen::INPUT_PRIVACY_KEY_STATUS_TIMESTAMP)
}

fn serialize_input_privacy_value_disallow_all() -> Vec<u8> {
    tl_gen::serialize_bare_ctor(tl_gen::INPUT_PRIVACY_VALUE_DISALLOW_ALL)
}

fn download_random_face() -> Result<Vec<u8>, String> {
    let resp = ureq::get("https://thispersondoesnotexist.com/")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .call()
        .map_err(|e| format!("HTTP: {e}"))?;
    let body = resp
        .into_body()
        .read_to_vec()
        .map_err(|e| format!("read body: {e}"))?;
    if body.len() < 1000 {
        return Err(t("actions_response_too_small"));
    }
    Ok(body)
}

async fn upload_and_set_photo_bytes(
    client: &mut MtpClient,
    data: &[u8],
    filename: &str,
) -> Result<(), String> {
    let file_id = rand::random::<i64>();
    let part_size = 128 * 1024;
    let total_parts = ((data.len() + part_size - 1) / part_size) as i32;

    for i in 0..total_parts {
        let start = i as usize * part_size;
        let end = ((i as usize + 1) * part_size).min(data.len());
        let chunk = &data[start..end];
        let req = tl::build_upload_save_file_part(file_id, i, chunk);
        client
            .invoke(&req)
            .await
            .map_err(|e| format!("upload part {}: {e}", i))?;
    }

    let req = tl::build_photos_upload_profile_photo(file_id, total_parts, filename);
    client
        .invoke(&req)
        .await
        .map_err(|e| format!("set photo: {e}"))?;
    Ok(())
}

// ─── Shared actions SQLite database ────────────────────────────────────────

fn init_actions_db(path: &std::path::PathBuf) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(path)
        .map_err(|e| t_with("actions_db_open_error", &[("error", &e.to_string())]))?;
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS actions_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id TEXT DEFAULT '',
            action TEXT DEFAULT '',
            details TEXT DEFAULT '',
            status TEXT DEFAULT '',
            performed_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_actions_account ON actions_log(account_id);
        CREATE INDEX IF NOT EXISTS idx_actions_action ON actions_log(action);
    ",
    )
    .map_err(|e| format!("create actions tables: {e}"))?;
    Ok(conn)
}

fn log_action(
    conn: &rusqlite::Connection,
    account_id: &str,
    action: &str,
    details: &str,
    status: &str,
) {
    conn.execute(
        "INSERT INTO actions_log (account_id, action, details, status) VALUES (?1,?2,?3,?4)",
        rusqlite::params![account_id, action, details, status],
    )
    .ok();
}
