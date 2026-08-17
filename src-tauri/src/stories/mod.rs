// stories: upload photo/video stories with optional user tagging

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use serde::Deserialize;
use tauri::{Emitter, Manager};

use crate::mtproto::tl;
use crate::mtproto::tl_gen;
use crate::accounts::commands::get_storage_pub;
use crate::accounts::session::AccountJson;
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

const CAPTION_LIMIT_REGULAR: usize = 200;
const CAPTION_LIMIT_PREMIUM: usize = 2048;
const TAG_SEPARATOR: &str = "\nㅤ\nㅤ\nㅤ\n";
const CHUNK_SIZE: usize = 512 * 1024;

#[derive(Deserialize, Clone)]
pub struct StoriesConfig {
    pub media_type: String,       // "photo" | "video"
    pub media_path: String,
    #[serde(default)]
    pub media_paths: Vec<String>, // list of files for batch mode
    #[serde(default)]
    pub distribute_mode: String,  // "all" | "unique"
    pub caption: String,
    pub tag_users: bool,
    pub tag_file_path: String,
    pub duration_seconds: i32,
    pub max_flood_wait: u64,
    #[serde(default = "default_privacy")]
    pub privacy: String,          // "all" | "contacts"
    #[serde(default = "default_max_stories")]
    pub max_stories_per_account: u32,
    #[serde(default)]
    pub stories_min: u32,         // random range: min stories per account (0 = use max_stories_per_account)
    #[serde(default)]
    pub stories_max: u32,
    #[serde(default)]
    pub delay_min: u32,           // delay between stories (seconds)
    #[serde(default)]
    pub delay_max: u32,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub output_links_path: String, // TXT file to save story links
}

fn default_privacy() -> String { "all".to_string() }
fn default_max_stories() -> u32 { 1 }

fn build_caption(base: &str, tags: &[String], is_premium: bool) -> Result<String, String> {
    let limit = if is_premium { CAPTION_LIMIT_PREMIUM } else { CAPTION_LIMIT_REGULAR };

    if !tags.is_empty() {
        let prefix = if base.is_empty() {
            String::new()
        } else {
            format!("{}{}", base, TAG_SEPARATOR)
        };

        if prefix.len() >= limit {
            return Err(t("stories_caption_too_long"));
        }

        let available = limit - prefix.len();
        let mut tag_str = String::new();
        for tag in tags {
            let mention = format!("@{} ", tag.trim_start_matches('@'));
            if tag_str.len() + mention.len() > available {
                break;
            }
            tag_str.push_str(&mention);
        }

        if tag_str.is_empty() {
            return Err(t("stories_caption_no_tag_fit"));
        }

        Ok(format!("{}{}", prefix, tag_str.trim_end()))
    } else if !base.is_empty() {
        if base.len() > limit {
            return Err(t_with("stories_caption_over_limit", &[("limit", &limit.to_string())]));
        }
        Ok(base.to_string())
    } else {
        Ok(String::new())
    }
}

fn calc_tags_per_account(caption: &str, tags: &[String], is_premium: bool) -> usize {
    let limit = if is_premium { CAPTION_LIMIT_PREMIUM } else { CAPTION_LIMIT_REGULAR };
    let prefix_len = if caption.is_empty() { 0 } else { caption.len() + TAG_SEPARATOR.len() };
    if prefix_len >= limit { return 0; }
    let available = limit - prefix_len;

    // count how many real tags fit
    let mut used = 0;
    let mut count = 0;
    for tag in tags {
        let mention = format!("@{} ", tag.trim_start_matches('@'));
        if used + mention.len() > available { break; }
        used += mention.len();
        count += 1;
    }
    count
}

#[tauri::command]
pub async fn stories_start(
    ids: Vec<String>,
    config: StoriesConfig,
    threads: Option<usize>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let concurrency = threads.unwrap_or(5).max(1).min(100);

    // read media file(s)
    let media_files: Vec<Arc<Vec<u8>>> = if !config.media_paths.is_empty() {
        let mut files = Vec::new();
        for path in &config.media_paths {
            let data = std::fs::read(path).map_err(|e| t_with("stories_read_error", &[("path", path.as_str()), ("error", &e.to_string())]))?;
            files.push(Arc::new(data));
        }
        files
    } else {
        let data = std::fs::read(&config.media_path)
            .map_err(|e| t_with("stories_read_media_error", &[("error", &e.to_string())]))?;
        vec![Arc::new(data)]
    };

    if media_files.is_empty() {
        return Err(t("stories_no_media"));
    }

    // read tags if enabled
    let all_tags: Vec<String> = if config.tag_users && !config.tag_file_path.is_empty() {
        let content = std::fs::read_to_string(&config.tag_file_path)
            .map_err(|e| t_with("stories_read_tags_error", &[("error", &e.to_string())]))?;
        content.lines()
            .map(|l| l.trim().trim_start_matches('@').to_string())
            .filter(|l| !l.is_empty())
            .collect()
    } else {
        Vec::new()
    };

    if config.tag_users && all_tags.is_empty() {
        return Err(t("stories_tags_empty"));
    }

    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue.register_task(
        task_id.clone(),
        "stories".to_string(),
        t_with("stories_task_name", &[("count", &ids.len().to_string())]),
    ).await;

    let config = Arc::new(config);
    let media_files = Arc::new(media_files);
    let all_tags = Arc::new(all_tags);
    let media_file_idx = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // prepare output links file
    let links_file: Option<Arc<std::sync::Mutex<std::fs::File>>> = if !config.output_links_path.is_empty() {
        match std::fs::File::create(&config.output_links_path) {
            Ok(f) => Some(Arc::new(std::sync::Mutex::new(f))),
            Err(_) => None,
        }
    } else { None };

    tokio::spawn(async move {
        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let total = ids.len();
        let mut handles = Vec::new();

        for (i, id) in ids.into_iter().enumerate() {
            if !token.load(Ordering::Relaxed) { break; }
            let sem = sem.clone();
            let config = config.clone();
            let media_files = media_files.clone();
            let media_file_idx = media_file_idx.clone();
            let all_tags = all_tags.clone();
            let links_file = links_file.clone();
            let app_clone = app.clone();
            let token_clone = token.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                if !token_clone.load(Ordering::Relaxed) { return; }

                // select media file(s) for this account
                let account_media = if config.distribute_mode == "unique" {
                    // unique: each account takes next file from queue
                    let idx = media_file_idx.fetch_add(1, Ordering::Relaxed) % media_files.len();
                    vec![media_files[idx].clone()]
                } else {
                    // all: use all files (or just the single one)
                    media_files.iter().cloned().collect()
                };

                let result = process_account(&id, &config, &account_media, &all_tags, i, total, &token_clone, links_file.as_ref()).await;
                match result {
                    Ok(msg) => {
                        let _ = app_clone.emit("stories-log", t_with("stories_done", &[("idx", &(i+1).to_string()), ("total", &total.to_string()), ("msg", &msg)]));
                    }
                    Err(e) => {
                        crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                        let _ = app_clone.emit("stories-log", t_with("stories_error", &[("idx", &(i+1).to_string()), ("total", &total.to_string()), ("error", &e)]));
                    }
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }
        let _ = app.emit("stories-log", t("done"));
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn stories_stop(
    task_id: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn process_account(
    account_id: &str,
    config: &StoriesConfig,
    media_files: &[Arc<Vec<u8>>],
    all_tags: &[String],
    index: usize,
    _total: usize,
    token: &Arc<AtomicBool>,
    links_file: Option<&Arc<std::sync::Mutex<std::fs::File>>>,
) -> Result<String, String> {
    let storage = get_storage_pub();
    let json_path = storage.json_path(account_id);

    let json = AccountJson::from_file(&json_path)
        .map_err(|e| format!("json: {e}"))?;

    let is_premium = json.is_premium;

    // determine tags for this account using real tag lengths
    let max_stories = if config.stories_min > 0 && config.stories_max > 0 {
        let lo = config.stories_min.min(config.stories_max);
        let hi = config.stories_min.max(config.stories_max);
        if lo == hi { lo as usize } else { (lo + rand::random::<u32>() % (hi - lo + 1)) as usize }
    } else {
        config.max_stories_per_account.max(1) as usize
    };
    let account_tags = if config.tag_users && !all_tags.is_empty() {
        let per_story = calc_tags_per_account(&config.caption, all_tags, is_premium);
        if per_story == 0 {
            return Err(t("stories_caption_too_long"));
        }
        let per_account = per_story * max_stories;
        let start = index * per_account;
        if start >= all_tags.len() {
            Vec::new()
        } else {
            let end = (start + per_account).min(all_tags.len());
            all_tags[start..end].to_vec()
        }
    } else {
        Vec::new()
    };

    // build caption
    let per_story_tags = calc_tags_per_account(&config.caption, &account_tags, is_premium);
    let stories_to_upload = if account_tags.is_empty() {
        max_stories
    } else if per_story_tags == 0 {
        return Err(t("stories_caption_too_long"));
    } else {
        ((account_tags.len() + per_story_tags - 1) / per_story_tags).min(max_stories)
    };

    let mut uploaded = 0u32;
    for story_idx in 0..stories_to_upload {
        let story_tags = if !account_tags.is_empty() && per_story_tags > 0 {
            let start = story_idx * per_story_tags;
            let end = (start + per_story_tags).min(account_tags.len());
            if start >= account_tags.len() { Vec::new() } else { account_tags[start..end].to_vec() }
        } else {
            Vec::new()
        };

        let spun_caption = crate::randomizer::spin_text(&config.caption);
        let caption = build_caption(&spun_caption, &story_tags, is_premium)?;

    // select media for this story
    let media_bytes = &media_files[story_idx % media_files.len()];

    let mut client = connect_account(account_id).await?;

    // upload file
    let file_id: i64 = rand::Rng::gen(&mut rand::thread_rng());
    let total_parts = ((media_bytes.len() + CHUNK_SIZE - 1) / CHUNK_SIZE) as i32;
    let is_big = media_bytes.len() >= 10 * 1024 * 1024;

    for part in 0..total_parts {
        if !token.load(Ordering::Relaxed) { return Ok(t("stories_stopped")); }
        let offset = part as usize * CHUNK_SIZE;
        let end = (offset + CHUNK_SIZE).min(media_bytes.len());
        let chunk = &media_bytes[offset..end];

        let req = if is_big {
            tl_gen::build_upload_saveBigFilePart(file_id, part, total_parts, chunk)
        } else {
            tl_gen::build_upload_saveFilePart(file_id, part, chunk)
        };

        let _resp = client.invoke(&req).await
            .map_err(|e| format!("upload part {}: {e}", part))?;
        let jitter = rand::random::<u64>() % 500;
        tokio::time::sleep(std::time::Duration::from_millis(500 + jitter)).await;
    }

    // build and send story request
    let file_name = if config.media_type == "photo" { "story.jpg" } else { "story.mp4" };
    let privacy_rule = match config.privacy.as_str() {
        "contacts" => tl::serialize_privacy_allow_contacts(),
        _ => tl::serialize_privacy_allow_all(),
    };
    let privacy_rules: &[&[u8]] = &[&privacy_rule];
    let random_id: i64 = rand::Rng::gen(&mut rand::thread_rng());
    let caption_opt = if caption.is_empty() { None } else { Some(caption.as_str()) };

    let story_req = if config.media_type == "photo" {
        if is_big {
            tl::build_send_photo_story_big(
                file_id, total_parts, file_name,
                caption_opt, Some(config.duration_seconds),
                config.pinned, privacy_rules, random_id,
            )
        } else {
            tl::build_send_photo_story(
                file_id, total_parts, file_name,
                caption_opt, Some(config.duration_seconds),
                config.pinned, privacy_rules, random_id,
            )
        }
    } else {
        tl::build_send_video_story(
            file_id, total_parts, file_name,
            15.0, 1080, 1920,
            caption_opt, Some(config.duration_seconds),
            config.pinned, privacy_rules, random_id,
        )
    };

    let result = client.invoke(&story_req).await;
    match result {
        Ok(resp) => {
            uploaded += 1;
            // save story link if we can extract the ID
            if let Some(links) = &links_file {
                // try to extract story_id from response updates
                if let Some(story_id) = extract_story_id_from_response(&resp) {
                    let username = &json.username;
                    if !username.is_empty() {
                        let link = format!("https://t.me/{}/s/{}", username, story_id);
                        if let Ok(mut f) = links.lock() {
                            use std::io::Write;
                            let _ = writeln!(f, "{}", link);
                        }
                    }
                }
            }
        }
        Err(e) => {
            if e.contains("PREMIUM_ACCOUNT_REQUIRED") {
                return Err(t("stories_premium_required"));
            }
            if e.contains("FLOOD_WAIT") {
                if let Some(secs) = extract_flood_wait(&e) {
                    if config.max_flood_wait > 0 && secs > config.max_flood_wait {
                        return Err(t_with("stories_flood_wait", &[("secs", &secs.to_string()), ("limit", &config.max_flood_wait.to_string())]));
                    }
                    interruptible_sleep(secs * 1000, token).await;
                    if !token.load(Ordering::Relaxed) { return Ok(t("stories_stopped")); }
                    if client.invoke(&story_req).await.is_ok() {
                        uploaded += 1;
                    }
                }
            } else {
                return Err(e);
            }
        }
    }

    // configurable delay between stories
    if story_idx + 1 < stories_to_upload {
        let delay_ms = if config.delay_min > 0 || config.delay_max > 0 {
            let lo = config.delay_min.min(config.delay_max) as u64;
            let hi = config.delay_min.max(config.delay_max) as u64;
            let val = if lo == hi { lo } else { lo + rand::random::<u64>() % (hi - lo + 1) };
            val * 1000
        } else { 500 };
        interruptible_sleep(delay_ms, token).await;
    }

    // surface a fatal session error even if the upload retry path swallowed it
    if let Some(fatal) = client.fatal_error() {
        return Err(fatal.to_string());
    }
    } // end stories loop

    let tagged = if account_tags.is_empty() { String::new() } else { t_with("stories_tags_suffix", &[("count", &account_tags.len().to_string())]) };
    Ok(t_with("stories_uploaded", &[("count", &uploaded.to_string()), ("tags", &tagged)]))
}

fn extract_flood_wait(err: &str) -> Option<u64> {
    // format: "FLOOD_WAIT_123" or "FLOOD_WAIT (123)"
    let err_upper = err.to_uppercase();
    if let Some(pos) = err_upper.find("FLOOD_WAIT") {
        let after = &err[pos + 10..];
        let digits: String = after.chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        digits.parse().ok()
    } else {
        None
    }
}

/// Try to extract story_id from the updates response after sendStory.
/// Looks for UpdateStoryID (0x1bf335b9) which contains the story id.
fn extract_story_id_from_response(data: &[u8]) -> Option<i32> {
    use byteorder::{LittleEndian, ReadBytesExt};
    use std::io::Cursor;

    let inner = tl_gen::unwrap_rpc(data).ok()?;
    let mut cursor = Cursor::new(inner.as_slice());
    let updates = tl_gen::TlUpdates::deserialize(&mut cursor).ok()?;
    match updates {
        tl_gen::TlUpdates::Updates { updates, .. } => {
            for upd_raw in &updates {
                if upd_raw.len() >= 8 {
                    let ctor = u32::from_le_bytes([upd_raw[0], upd_raw[1], upd_raw[2], upd_raw[3]]);
                    // UpdateStoryID
                    if ctor == 0x1bf335b9 {
                        let mut c = Cursor::new(&upd_raw[4..]);
                        if let Ok(id) = c.read_i32::<LittleEndian>() {
                            return Some(id);
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}
