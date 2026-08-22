// boost / engagement: bot start, views, reactions, channel/group subscribe, addlist import

use rusqlite;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};

use crate::accounts::commands::get_storage_pub;
use crate::accounts::connect::connect_account;
use crate::accounts::session::AccountJson;
use crate::i18n::{t, t_with};
use crate::mtproto::client::MtpClient;
use crate::mtproto::text_parse;
use crate::mtproto::tl;
use crate::mtproto::tl_gen;
use crate::queue::TaskQueue;

// matches BoostMode discriminator emitted by frontend
#[derive(Deserialize, Clone)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum BoostConfig {
    Bot {
        bot_link: String,
        #[serde(default)]
        bot_links: Vec<String>, // list of bot links for batch mode
        use_referral: bool,
        ref_param: String,
        #[serde(default)]
        delete_after: bool,
        #[serde(default)]
        distribute_mode: String, // "all" (each account starts all bots) | "unique" (queue-based distribution)
        #[serde(default)]
        max_per_account: u32, // 0 = no limit
    },
    Views {
        post_link: String,
        is_private: bool,
        join_link: String,
        leave_after: bool,
        archive_after: bool,
        #[serde(default = "default_post_target")]
        post_target: String, // "specific" | "last_n"
        #[serde(default)]
        last_n_min: u32,
        #[serde(default)]
        last_n_max: u32,
    },
    Reactions {
        post_link: String,
        #[serde(default)]
        post_links: Vec<String>, // multiple post links from different channels
        is_private: bool,
        join_link: String,
        leave_after: bool,
        archive_after: bool,
        emoji_mode: String, // "random_positive" | "random_negative" | "specific" | "custom_list"
        specific_emoji: String,
        #[serde(default)]
        emoji_list: Vec<String>, // custom list of emoji for "custom_list" mode
        #[serde(default)]
        reactions_shuffle: bool, // shuffle emoji list order
        #[serde(default = "default_one")]
        reactions_per_post_min: u32, // how many different reactions per post
        #[serde(default = "default_one")]
        reactions_per_post_max: u32,
        #[serde(default)]
        reactions_delay_min: u32, // delay between reactions on same post (seconds)
        #[serde(default)]
        reactions_delay_max: u32,
        #[serde(default = "default_post_target")]
        post_target: String,
        #[serde(default)]
        last_n_min: u32,
        #[serde(default)]
        last_n_max: u32,
        #[serde(default = "default_true_boost")]
        view_after_each: bool, // call getMessagesViews after each reaction
        #[serde(default = "default_true_boost")]
        auto_join: bool, // auto-join channel if not a member
    },
    #[serde(rename = "subscribe-channel")]
    SubscribeChannel {
        join_link: String,
        archive_after: bool,
    },
    #[serde(rename = "subscribe-group")]
    SubscribeGroup {
        join_link: String,
        archive_after: bool,
    },
    #[serde(rename = "import-folder")]
    ImportFolder { links: Vec<String> },
}

fn default_post_target() -> String {
    "specific".to_string()
}
fn default_one() -> u32 {
    1
}
fn default_true_boost() -> bool {
    true
}

async fn rate_limit() {
    let jitter = rand::random::<u64>() % 500;
    tokio::time::sleep(std::time::Duration::from_millis(500 + jitter)).await;
}

const POSITIVE_EMOJIS: &[&str] = &[
    "\u{1F44D}",
    "\u{2764}\u{FE0F}",
    "\u{1F525}",
    "\u{1F970}",
    "\u{1F44F}",
    "\u{1F389}",
    "\u{1F929}",
    "\u{1F4AF}",
    "\u{26A1}",
    "\u{1F3C6}",
    "\u{1F60D}",
    "\u{1F91D}",
    "\u{1F64F}",
];
const NEGATIVE_EMOJIS: &[&str] = &[
    "\u{1F44E}",
    "\u{1F622}",
    "\u{1F4A9}",
    "\u{1F92E}",
    "\u{1F631}",
    "\u{1F92C}",
    "\u{1F494}",
    "\u{1F971}",
    "\u{1F921}",
    "\u{1F928}",
];

#[tauri::command]
pub async fn boost_start(
    ids: Vec<String>,
    config: BoostConfig,
    threads: Option<usize>,
    max_flood_wait: Option<u64>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let concurrency = threads.unwrap_or(5).max(1).min(100);
    let max_flood_wait = max_flood_wait.unwrap_or(0);

    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "boost".to_string(),
            t_with("boost_task_name", &[("count", &ids.len().to_string())]),
        )
        .await;

    let config = Arc::new(config);

    tokio::spawn(async move {
        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let total = ids.len();
        let mut handles = Vec::new();
        for (i, id) in ids.into_iter().enumerate() {
            if !token.load(Ordering::Relaxed) {
                break;
            }
            let sem = sem.clone();
            let config = config.clone();
            let app_clone = app.clone();
            let token_clone = token.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                if !token_clone.load(Ordering::Relaxed) {
                    return;
                }

                let result = process_account(
                    &id,
                    i + 1,
                    total,
                    &config,
                    max_flood_wait,
                    &app_clone,
                    &token_clone,
                )
                .await;
                if let Err(e) = result {
                    crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                    let _ = app_clone.emit(
                        "boost-log",
                        format!("[{}/{}] {}: {}", i + 1, total, t("error"), e),
                    );
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }

        let _ = app.emit("boost-log", t("done"));

        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn boost_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn process_account(
    id: &str,
    idx: usize,
    total: usize,
    config: &BoostConfig,
    max_flood_wait: u64,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    let mut client = connect_account(id).await?;
    client.set_log_target("boost-log", app.clone());
    client.set_max_flood_wait(max_flood_wait);

    let storage = get_storage_pub();
    let json_path = storage.json_path(id);
    let json = if json_path.exists() {
        AccountJson::from_file(&json_path).unwrap_or_default()
    } else {
        AccountJson::default()
    };

    let phone = json.phone.clone();
    let prefix = format!(
        "[{}/{}] +{}",
        idx,
        total,
        if phone.is_empty() { "?" } else { &phone }
    );

    let result = match config {
        BoostConfig::Bot {
            bot_link,
            bot_links,
            use_referral,
            ref_param,
            delete_after,
            distribute_mode,
            max_per_account,
        } => {
            // build effective list of bots
            let mut links: Vec<&str> = bot_links.iter().map(|s| s.as_str()).collect();
            if links.is_empty() && !bot_link.is_empty() {
                links.push(bot_link.as_str());
            }
            if links.is_empty() {
                return Err(t("boost_no_bot_links"));
            }

            let is_unique = distribute_mode == "unique";
            let limit = if *max_per_account > 0 {
                *max_per_account as usize
            } else {
                links.len()
            };

            if is_unique {
                // unique mode: this account takes bots from shared index
                // since we process accounts independently and don't share state here,
                // in unique mode each account starts bot_link (single) — the multi-bot
                // unique distribution is handled externally via bot_links being pre-sliced
                // For simplicity: each account processes up to max_per_account bots
                // starting from idx-based offset
                let start_offset = ((idx - 1) * limit) % links.len();
                let mut started = 0;
                for i in 0..links.len() {
                    if started >= limit {
                        break;
                    }
                    if !token.load(Ordering::Relaxed) {
                        break;
                    }
                    let link = links[(start_offset + i) % links.len()];
                    if let Err(e) = run_bot_activation(
                        &mut client,
                        &prefix,
                        link,
                        *use_referral,
                        ref_param,
                        *delete_after,
                        app,
                        token,
                    )
                    .await
                    {
                        emit(
                            app,
                            t_with(
                                "boost_bot_error",
                                &[("prefix", &prefix), ("link", link), ("error", &e)],
                            ),
                        );
                        if crate::mtproto::is_fatal_session_error(&e) {
                            return Err(e);
                        }
                    }
                    started += 1;
                    if started < limit {
                        interruptible_sleep_boost(1500 + (rand::random::<u64>() % 1500), token)
                            .await;
                    }
                }
                Ok(())
            } else {
                // mass mode: each account starts all bots (up to limit)
                let mut started = 0;
                for link in &links {
                    if started >= limit {
                        break;
                    }
                    if !token.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Err(e) = run_bot_activation(
                        &mut client,
                        &prefix,
                        link,
                        *use_referral,
                        ref_param,
                        *delete_after,
                        app,
                        token,
                    )
                    .await
                    {
                        emit(
                            app,
                            t_with(
                                "boost_bot_error",
                                &[("prefix", &prefix), ("link", link), ("error", &e)],
                            ),
                        );
                        if crate::mtproto::is_fatal_session_error(&e) {
                            return Err(e);
                        }
                    }
                    started += 1;
                    if started < limit {
                        interruptible_sleep_boost(1500 + (rand::random::<u64>() % 1500), token)
                            .await;
                    }
                }
                Ok(())
            }
        }
        BoostConfig::Views {
            post_link,
            is_private,
            join_link,
            leave_after,
            archive_after,
            post_target,
            last_n_min,
            last_n_max,
        } => {
            run_views(
                &mut client,
                &prefix,
                post_link,
                *is_private,
                join_link,
                *leave_after,
                *archive_after,
                post_target,
                *last_n_min,
                *last_n_max,
                app,
                token,
            )
            .await
        }
        BoostConfig::Reactions {
            post_link,
            post_links,
            is_private,
            join_link,
            leave_after,
            archive_after,
            emoji_mode,
            specific_emoji,
            emoji_list,
            reactions_shuffle,
            reactions_per_post_min,
            reactions_per_post_max,
            reactions_delay_min,
            reactions_delay_max,
            post_target,
            last_n_min,
            last_n_max,
            view_after_each,
            auto_join,
        } => {
            run_reactions(
                &mut client,
                &prefix,
                post_link,
                post_links,
                *is_private,
                join_link,
                *leave_after,
                *archive_after,
                emoji_mode,
                specific_emoji,
                emoji_list,
                *reactions_shuffle,
                *reactions_per_post_min,
                *reactions_per_post_max,
                *reactions_delay_min,
                *reactions_delay_max,
                post_target,
                *last_n_min,
                *last_n_max,
                *view_after_each,
                *auto_join,
                app,
                token,
            )
            .await
        }
        BoostConfig::SubscribeChannel {
            join_link,
            archive_after,
        } => {
            run_subscribe(
                &mut client,
                &prefix,
                join_link,
                *archive_after,
                app,
                token,
                false,
            )
            .await
        }
        BoostConfig::SubscribeGroup {
            join_link,
            archive_after,
        } => {
            run_subscribe(
                &mut client,
                &prefix,
                join_link,
                *archive_after,
                app,
                token,
                true,
            )
            .await
        }
        BoostConfig::ImportFolder { links } => {
            run_import_folders(&mut client, &prefix, links, app, token).await
        }
    };

    if let Some(fatal) = client.fatal_error() {
        return Err(fatal.to_string());
    }
    result
}

fn emit(app: &tauri::AppHandle, msg: String) {
    let _ = app.emit("boost-log", msg);
}

// === bot activation ===
async fn run_bot_activation(
    client: &mut MtpClient,
    prefix: &str,
    bot_link: &str,
    use_referral: bool,
    ref_param: &str,
    delete_after: bool,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    if !token.load(Ordering::Relaxed) {
        return Ok(());
    }
    let username = bot_link
        .trim()
        .trim_start_matches("https://t.me/")
        .trim_start_matches("http://t.me/")
        .trim_start_matches("t.me/")
        .trim_start_matches('@')
        .split('?')
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    if username.is_empty() {
        return Err(t("boost_empty_bot_link"));
    }

    // payload from explicit ref_param or extracted from ?start=...
    let payload = if use_referral && !ref_param.is_empty() {
        ref_param.trim().to_string()
    } else {
        extract_start_payload(bot_link).unwrap_or_default()
    };

    let resolve_req = tl::build_resolve_username(username);
    let resolve_data = client
        .invoke(&resolve_req)
        .await
        .map_err(|e| format!("resolve {}: {e}", username))?;
    let (bot_id, bot_access_hash) =
        tl::parse_resolved_peer(&resolve_data).map_err(|e| format!("parse bot peer: {e}"))?;

    // unblock in case the user previously blocked it
    let unblock_req = tl::build_unblock_peer(bot_id, bot_access_hash);
    if let Err(e) = client.invoke(&unblock_req).await {
        dbg_log!("разблокировка бота @{} не удалась: {e}", username);
    }

    let start_text = if payload.is_empty() {
        "/start".to_string()
    } else {
        format!("/start {}", payload)
    };
    let random_id: i64 = rand::random();
    let req = tl::build_send_message(bot_id, bot_access_hash, &start_text, random_id);
    client
        .invoke(&req)
        .await
        .map_err(|e| format!("send /start: {e}"))?;
    rate_limit().await;

    if payload.is_empty() {
        emit(
            app,
            t_with(
                "boost_bot_activated",
                &[("prefix", prefix), ("username", username)],
            ),
        );
    } else {
        emit(
            app,
            t_with(
                "boost_bot_activated_ref",
                &[
                    ("prefix", prefix),
                    ("username", username),
                    ("payload", &payload),
                ],
            ),
        );
    }

    if delete_after {
        if !token.load(Ordering::Relaxed) {
            return Ok(());
        }
        let del_req = tl::build_delete_history(bot_id, bot_access_hash);
        let _ = client.invoke(&del_req).await;
        rate_limit().await;
        let block_req = tl::build_block_peer(bot_id, bot_access_hash);
        let _ = client.invoke(&block_req).await;
        emit(
            app,
            t_with(
                "boost_bot_blocked",
                &[("prefix", prefix), ("username", username)],
            ),
        );
    }

    Ok(())
}

fn extract_start_payload(link: &str) -> Option<String> {
    // https://t.me/bot?start=foo
    let q = link.split_once('?')?.1;
    for kv in q.split('&') {
        if let Some(("start", v)) = kv.split_once('=') {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

// === views ===
async fn run_views(
    client: &mut MtpClient,
    prefix: &str,
    post_link: &str,
    is_private: bool,
    join_link: &str,
    leave_after: bool,
    archive_after: bool,
    post_target: &str,
    last_n_min: u32,
    last_n_max: u32,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    if !token.load(Ordering::Relaxed) {
        return Ok(());
    }
    let (channel_label, channel_id, access_hash, joined, msg_ids) = resolve_channel_and_messages(
        client,
        post_link,
        is_private,
        join_link,
        post_target,
        last_n_min,
        last_n_max,
    )
    .await?;

    if msg_ids.is_empty() {
        return Err(t("boost_no_posts_to_view"));
    }

    let req = tl::build_get_messages_views_channel(channel_id, access_hash, &msg_ids, true);
    client
        .invoke(&req)
        .await
        .map_err(|e| format!("getMessagesViews: {e}"))?;
    emit(
        app,
        t_with(
            "boost_views_done",
            &[
                ("prefix", prefix),
                ("count", &msg_ids.len().to_string()),
                ("channel", &channel_label),
            ],
        ),
    );

    let mut joined_now = joined;
    if !is_private && (leave_after || archive_after) {
        if let Err(e) = client
            .invoke(&tl::build_join_channel(channel_id, access_hash))
            .await
        {
            emit(
                app,
                t_with("boost_join_failed", &[("prefix", prefix), ("error", &e)]),
            );
        } else {
            joined_now = true;
        }
    }
    post_subscribe_actions(
        client,
        prefix,
        channel_id,
        access_hash,
        joined_now,
        leave_after,
        archive_after,
        app,
    )
    .await;
    Ok(())
}

// === reactions (also bumps views) ===
async fn run_reactions(
    client: &mut MtpClient,
    prefix: &str,
    post_link: &str,
    post_links: &[String],
    is_private: bool,
    join_link: &str,
    leave_after: bool,
    archive_after: bool,
    emoji_mode: &str,
    specific_emoji: &str,
    emoji_list: &[String],
    reactions_shuffle: bool,
    reactions_per_post_min: u32,
    reactions_per_post_max: u32,
    reactions_delay_min: u32,
    reactions_delay_max: u32,
    post_target: &str,
    last_n_min: u32,
    last_n_max: u32,
    view_after_each: bool,
    auto_join: bool,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    if !token.load(Ordering::Relaxed) {
        return Ok(());
    }

    // Build list of (channel_id, access_hash, msg_ids, label, joined) tuples
    struct TargetInfo {
        label: String,
        channel_id: i64,
        access_hash: i64,
        msg_ids: Vec<i32>,
        joined: bool,
    }

    let mut targets: Vec<TargetInfo> = Vec::new();

    // If post_links has multiple entries, resolve each separately
    let links_to_process: Vec<&str> = if !post_links.is_empty() {
        post_links.iter().map(|s| s.as_str()).collect()
    } else if !post_link.is_empty() {
        vec![post_link]
    } else {
        return Err(t("boost_no_reaction_links"));
    };

    for link in &links_to_process {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let link = link.trim();
        if link.is_empty() {
            continue;
        }

        match resolve_channel_and_messages(
            client,
            link,
            is_private,
            join_link,
            post_target,
            last_n_min,
            last_n_max,
        )
        .await
        {
            Ok((label, channel_id, access_hash, joined, msg_ids)) => {
                if msg_ids.is_empty() {
                    emit(
                        app,
                        t_with(
                            "boost_no_posts_for",
                            &[("prefix", prefix), ("label", &label)],
                        ),
                    );
                    continue;
                }
                // Auto-join if not a member
                let actual_joined = if auto_join && !joined {
                    match client
                        .invoke(&tl::build_join_channel(channel_id, access_hash))
                        .await
                    {
                        Ok(_) => {
                            emit(
                                app,
                                t_with("boost_joined", &[("prefix", prefix), ("label", &label)]),
                            );
                            true
                        }
                        Err(e) => {
                            emit(
                                app,
                                t_with(
                                    "boost_join_failed_label",
                                    &[("prefix", prefix), ("label", &label), ("error", &e)],
                                ),
                            );
                            joined
                        }
                    }
                } else {
                    joined
                };
                targets.push(TargetInfo {
                    label,
                    channel_id,
                    access_hash,
                    msg_ids,
                    joined: actual_joined,
                });
            }
            Err(e) => {
                emit(
                    app,
                    t_with(
                        "boost_resolve_error",
                        &[("prefix", prefix), ("link", link), ("error", &e)],
                    ),
                );
            }
        }
    }

    if targets.is_empty() {
        return Err(t("boost_no_posts_for_reactions"));
    }

    // Build emoji pool
    let mut emoji_pool: Vec<String> = if emoji_mode == "custom_list" && !emoji_list.is_empty() {
        emoji_list.to_vec()
    } else {
        // use the existing pick_emoji logic to generate a pool
        Vec::new()
    };
    if reactions_shuffle && !emoji_pool.is_empty() {
        // Fisher-Yates shuffle
        for i in (1..emoji_pool.len()).rev() {
            let j = rand::random::<usize>() % (i + 1);
            emoji_pool.swap(i, j);
        }
    }

    // SQLite for reactions results
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("kastor")
        .join("reactions");
    std::fs::create_dir_all(&data_dir).ok();
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let db_path = data_dir.join(format!("{}_reactions.db", timestamp));
    let reactions_db = init_reactions_db(&db_path);
    if let Ok(ref _db) = reactions_db {
        emit(
            app,
            t_with(
                "boost_reactions_db",
                &[("prefix", prefix), ("path", &db_path.display().to_string())],
            ),
        );
    }

    let mut total_ok = 0u32;
    let mut total_errs = 0u32;
    let mut emoji_idx = 0usize;

    for target in &targets {
        if !token.load(Ordering::Relaxed) {
            break;
        }

        // Initial views bump
        let views_req = tl::build_get_messages_views_channel(
            target.channel_id,
            target.access_hash,
            &target.msg_ids,
            true,
        );
        let _ = client.invoke(&views_req).await;

        for &msg_id in &target.msg_ids {
            if !token.load(Ordering::Relaxed) {
                break;
            }

            // Determine how many reactions for this post
            let min_r = reactions_per_post_min.max(1);
            let max_r = reactions_per_post_max.max(min_r);
            let num_reactions = if min_r == max_r {
                min_r
            } else {
                min_r + (rand::random::<u32>() % (max_r - min_r + 1))
            };

            for reaction_idx in 0..num_reactions {
                if !token.load(Ordering::Relaxed) {
                    break;
                }

                // Pick emoji
                let emoji = if !emoji_pool.is_empty() {
                    emoji_pool[emoji_idx % emoji_pool.len()].clone()
                } else {
                    pick_emoji(emoji_mode, specific_emoji)
                };
                emoji_idx += 1;

                if emoji.is_empty() {
                    emit(app, t_with("boost_no_emoji_skip", &[("prefix", prefix)]));
                    break;
                }

                let req = tl::build_send_reaction_channel(
                    target.channel_id,
                    target.access_hash,
                    msg_id,
                    Some(&emoji),
                    false,
                );
                let status = match client.invoke(&req).await {
                    Ok(_) => {
                        total_ok += 1;
                        emit(
                            app,
                            t_with(
                                "boost_reaction_progress",
                                &[
                                    ("prefix", prefix),
                                    ("emoji", &emoji),
                                    ("label", &target.label),
                                    ("msg_id", &msg_id.to_string()),
                                    ("idx", &(reaction_idx + 1).to_string()),
                                    ("ok", &total_ok.to_string()),
                                    ("total", &(total_ok + total_errs).to_string()),
                                ],
                            ),
                        );
                        "done"
                    }
                    Err(e) => {
                        if crate::mtproto::is_fatal_session_error(&e) {
                            return Err(e);
                        }
                        if e.contains("REACTIONS_TOO_MANY") {
                            emit(
                                app,
                                t_with(
                                    "boost_reaction_limit",
                                    &[
                                        ("prefix", prefix),
                                        ("label", &target.label),
                                        ("msg_id", &msg_id.to_string()),
                                    ],
                                ),
                            );
                            "too_many"
                        } else {
                            total_errs += 1;
                            emit(
                                app,
                                t_with(
                                    "boost_reaction_error",
                                    &[
                                        ("prefix", prefix),
                                        ("label", &target.label),
                                        ("msg_id", &msg_id.to_string()),
                                        ("error", &e),
                                    ],
                                ),
                            );
                            "error"
                        }
                    }
                };

                // Record to DB
                if let Ok(ref db) = reactions_db {
                    record_reaction(
                        db,
                        prefix,
                        &format!("{}/{}", target.label, msg_id),
                        &emoji,
                        status,
                    );
                }

                // View after each reaction
                if view_after_each {
                    let view_req = tl::build_get_messages_views_channel(
                        target.channel_id,
                        target.access_hash,
                        &[msg_id],
                        true,
                    );
                    let _ = client.invoke(&view_req).await;
                }

                // Delay between reactions on same post
                if reaction_idx + 1 < num_reactions {
                    let delay_ms = if reactions_delay_min == 0 && reactions_delay_max == 0 {
                        500u64 // minimal default
                    } else {
                        let lo = reactions_delay_min.min(reactions_delay_max) as u64 * 1000;
                        let hi = reactions_delay_min.max(reactions_delay_max) as u64 * 1000;
                        if lo == hi {
                            lo
                        } else {
                            lo + (rand::random::<u64>() % (hi - lo + 1))
                        }
                    };
                    interruptible_sleep_boost(delay_ms, token).await;
                }
            }

            // Delay between posts
            rate_limit().await;
        }
    }

    emit(
        app,
        t_with(
            "boost_reactions_done",
            &[
                ("prefix", prefix),
                ("ok", &total_ok.to_string()),
                ("errs", &total_errs.to_string()),
            ],
        ),
    );

    // Leave channels if configured
    if leave_after || archive_after {
        for target in &targets {
            if target.joined {
                post_subscribe_actions(
                    client,
                    prefix,
                    target.channel_id,
                    target.access_hash,
                    true,
                    leave_after,
                    archive_after,
                    app,
                )
                .await;
            }
        }
    }

    if total_ok == 0 && total_errs > 0 {
        return Err(t("boost_no_reactions_sent"));
    }
    Ok(())
}

// resolves the channel and returns the message ids to act on.
// for "specific" target the post_link must be t.me/<channel>/<msg_id>;
// for "last_n" target post_link is a public channel link or @username and we
// pull a random N from [min..=max] of the latest broadcast posts.
async fn resolve_channel_and_messages(
    client: &mut MtpClient,
    post_link: &str,
    is_private: bool,
    join_link: &str,
    post_target: &str,
    last_n_min: u32,
    last_n_max: u32,
) -> Result<(String, i64, i64, bool, Vec<i32>), String> {
    if post_target == "last_n" || post_target == "all" || post_target == "pin" {
        let target_count = if post_target == "all" {
            500u32
        } else {
            let min = last_n_min.max(1);
            let max = last_n_max.max(min);
            let count = if min == max {
                min
            } else {
                min + (rand::random::<u32>() % (max - min + 1))
            };
            count.min(100)
        };
        let request_limit = if post_target == "all" || post_target == "pin" {
            500i32
        } else {
            (target_count.saturating_mul(2))
                .max(target_count + 5)
                .min(100) as i32
        };

        let (label, channel_id, access_hash, joined) =
            resolve_channel_target(client, post_link, is_private, join_link).await?;

        let req = if post_target == "pin" {
            // Use search with InputMessagesFilterPinned to get only pinned messages
            tl::build_search_channel_pinned(channel_id, access_hash, request_limit)
        } else {
            tl::build_get_history_channel(channel_id, access_hash, request_limit)
        };
        let data = client
            .invoke(&req)
            .await
            .map_err(|e| format!("getHistory/search: {e}"))?;
        let msgs =
            tl::parse_messages_structured(&data).map_err(|e| format!("parse history: {e}"))?;
        let ids: Vec<i32> = msgs
            .iter()
            .filter(|m| m.id > 0 && !m.is_service)
            .take(target_count as usize)
            .map(|m| m.id)
            .collect();
        Ok((label, channel_id, access_hash, joined, ids))
    } else {
        let (channel_username, msg_id) =
            text_parse::parse_post_link(post_link).ok_or_else(|| t("boost_invalid_post_link"))?;
        let (channel_id, access_hash, joined) =
            resolve_post_target(client, &channel_username, is_private, join_link).await?;
        Ok((
            format!("{}/{}", channel_username, msg_id),
            channel_id,
            access_hash,
            joined,
            vec![msg_id],
        ))
    }
}

// resolve a channel given its public link (e.g. https://t.me/channel) or
// @username, joining via invite link if private. used by last_n flow where
// post_link is the channel itself.
async fn resolve_channel_target(
    client: &mut MtpClient,
    channel_link: &str,
    is_private: bool,
    join_link: &str,
) -> Result<(String, i64, i64, bool), String> {
    if is_private {
        let (id, hash, joined, title) = join_by_link_with_title(client, join_link, false).await?;
        return Ok((
            if title.is_empty() {
                "private".to_string()
            } else {
                title
            },
            id,
            hash,
            joined,
        ));
    }
    let username = channel_link
        .trim()
        .trim_start_matches("https://t.me/")
        .trim_start_matches("http://t.me/")
        .trim_start_matches("t.me/")
        .trim_start_matches('@')
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    if username.is_empty() {
        return Err(t("boost_empty_channel_link"));
    }
    let resolve_req = tl::build_resolve_username(username);
    let resolve_data = client
        .invoke(&resolve_req)
        .await
        .map_err(|e| format!("resolve {}: {e}", username))?;
    let (id, hash) =
        tl::parse_resolved_peer(&resolve_data).map_err(|e| format!("parse channel: {e}"))?;
    Ok((username.to_string(), id, hash, false))
}

fn pick_emoji(mode: &str, specific: &str) -> String {
    // thread_rng() is !Send, so we wrap usage in a non-async block
    match mode {
        "random_positive" => {
            use rand::seq::SliceRandom;
            let mut rng = rand::thread_rng();
            POSITIVE_EMOJIS
                .choose(&mut rng)
                .map(|s| s.to_string())
                .unwrap_or_default()
        }
        "random_negative" => {
            use rand::seq::SliceRandom;
            let mut rng = rand::thread_rng();
            NEGATIVE_EMOJIS
                .choose(&mut rng)
                .map(|s| s.to_string())
                .unwrap_or_default()
        }
        _ => specific.trim().to_string(),
    }
}

// === channel/group subscribe ===
async fn run_subscribe(
    client: &mut MtpClient,
    prefix: &str,
    link: &str,
    archive_after: bool,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
    is_group: bool,
) -> Result<(), String> {
    if !token.load(Ordering::Relaxed) {
        return Ok(());
    }
    let (channel_id, access_hash, joined) = join_by_link(client, link, is_group).await?;
    emit(
        app,
        t_with(
            "boost_join_link_done",
            &[("prefix", prefix), ("link", link)],
        ),
    );

    if archive_after && joined {
        let req = tl::build_edit_peer_folder_channel(channel_id, access_hash, 1);
        if let Err(e) = client.invoke(&req).await {
            emit(
                app,
                t_with("boost_archive_error", &[("prefix", prefix), ("error", &e)]),
            );
        } else {
            emit(app, t_with("boost_archived", &[("prefix", prefix)]));
        }
    }
    Ok(())
}

// === folder import (addlist) ===
async fn run_import_folders(
    client: &mut MtpClient,
    prefix: &str,
    links: &[String],
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    if !token.load(Ordering::Relaxed) {
        return Ok(());
    }
    if links.is_empty() {
        return Err(t("boost_empty_folder_links"));
    }

    let mut ok = 0u32;
    let mut errs = 0u32;
    for link in links {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let slug = match text_parse::parse_invite_link(link) {
            Some((kind, slug)) if kind == "addlist" => slug,
            _ => {
                emit(
                    app,
                    t_with("boost_not_addlist", &[("prefix", prefix), ("link", link)]),
                );
                errs += 1;
                continue;
            }
        };

        let check_req = tl::build_check_chatlist_invite(&slug);
        let check_data = match client.invoke(&check_req).await {
            Ok(d) => d,
            Err(e) => {
                if crate::mtproto::is_fatal_session_error(&e) {
                    return Err(e);
                }
                emit(
                    app,
                    t_with(
                        "boost_check_invite_error",
                        &[("prefix", prefix), ("slug", &slug), ("error", &e)],
                    ),
                );
                errs += 1;
                continue;
            }
        };

        let (title, peers_blob) = match tl::parse_chatlist_invite_as_input_peers(&check_data) {
            Ok(x) => x,
            Err(e) => {
                emit(
                    app,
                    t_with(
                        "boost_parse_invite_error",
                        &[("prefix", prefix), ("slug", &slug), ("error", &e)],
                    ),
                );
                errs += 1;
                continue;
            }
        };

        let join_req = tl::build_join_chatlist_invite(&slug, &peers_blob);
        match client.invoke(&join_req).await {
            Ok(_) => {
                let pretty = if title.is_empty() {
                    slug.clone()
                } else {
                    title
                };
                emit(
                    app,
                    t_with(
                        "boost_folder_imported",
                        &[("prefix", prefix), ("name", &pretty)],
                    ),
                );
                ok += 1;
            }
            Err(e) => {
                emit(
                    app,
                    t_with(
                        "boost_join_invite_error",
                        &[("prefix", prefix), ("slug", &slug), ("error", &e)],
                    ),
                );
                errs += 1;
            }
        }
        rate_limit().await;
    }

    if errs > 0 && ok == 0 {
        return Err(t_with(
            "boost_no_folders_imported",
            &[("count", &links.len().to_string())],
        ));
    }
    Ok(())
}

// resolves the channel that owns a post link, joining it if necessary.
// returns (channel_id, access_hash, joined_now)
async fn resolve_post_target(
    client: &mut MtpClient,
    channel_username: &str,
    is_private: bool,
    join_link: &str,
) -> Result<(i64, i64, bool), String> {
    if is_private {
        // private channel: must join via invite first; resolveUsername won't work
        return join_by_link(client, join_link, false).await;
    }

    let resolve_req = tl::build_resolve_username(channel_username);
    let resolve_data = client
        .invoke(&resolve_req)
        .await
        .map_err(|e| format!("resolve {}: {e}", channel_username))?;
    let (id, hash) =
        tl::parse_resolved_peer(&resolve_data).map_err(|e| format!("parse channel: {e}"))?;
    Ok((id, hash, false))
}

// joins a channel/group by its public username, t.me/+invite or t.me/joinchat link
// returns (channel_id, access_hash, joined_now, title)
async fn join_by_link_with_title(
    client: &mut MtpClient,
    link: &str,
    is_group: bool,
) -> Result<(i64, i64, bool, String), String> {
    let (kind, body) = text_parse::parse_invite_link(link)
        .ok_or_else(|| t_with("boost_parse_link_error", &[("link", link)]))?;

    match kind {
        "private" => {
            let req = tl::build_import_chat_invite(&body);
            let data = client
                .invoke(&req)
                .await
                .map_err(|e| format!("importChatInvite: {e}"))?;
            let (id, hash) = tl::parse_created_channel(&data)
                .map_err(|e| format!("parse joined channel: {e}"))?;
            // try to extract title from Updates response
            let title = extract_title_from_updates(&data).unwrap_or_default();
            Ok((id, hash, true, title))
        }
        "public" => {
            let resolve_req = tl::build_resolve_username(&body);
            let resolve_data = client
                .invoke(&resolve_req)
                .await
                .map_err(|e| format!("resolve {}: {e}", body))?;
            let (id, hash) = tl::parse_resolved_peer(&resolve_data)
                .map_err(|e| format!("parse channel: {e}"))?;
            if hash == 0 {
                if is_group {
                    let req = tl::build_add_chat_user(id);
                    client
                        .invoke(&req)
                        .await
                        .map_err(|e| format!("addChatUser: {e}"))?;
                    return Ok((id, 0, true, String::new()));
                }
                return Err(t("boost_sub_username_failed"));
            }
            let join_req = tl::build_join_channel(id, hash);
            client
                .invoke(&join_req)
                .await
                .map_err(|e| format!("joinChannel: {e}"))?;
            Ok((id, hash, true, String::new()))
        }
        "addlist" => Err(t("boost_addlist_unsupported")),
        _ => Err(t_with("boost_unknown_link_type", &[("kind", kind)])),
    }
}

async fn join_by_link(
    client: &mut MtpClient,
    link: &str,
    is_group: bool,
) -> Result<(i64, i64, bool), String> {
    join_by_link_with_title(client, link, is_group)
        .await
        .map(|(id, hash, joined, _)| (id, hash, joined))
}

fn extract_title_from_updates(data: &[u8]) -> Option<String> {
    let inner = tl_gen::unwrap_rpc(data).ok()?;
    let updates = tl_gen::deserialize_tl_obj::<tl_gen::TlUpdates>(&inner).ok()?;
    let chats = match updates {
        tl_gen::TlUpdates::Updates { chats, .. } => chats,
        tl_gen::TlUpdates::Combined { chats, .. } => chats,
        _ => return None,
    };
    for chat_raw in &chats {
        if let Ok(tl_gen::TlChat::Channel { title, .. }) =
            tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(chat_raw)
        {
            return Some(title);
        }
        if let Ok(tl_gen::TlChat::Chat { title, .. }) =
            tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(chat_raw)
        {
            return Some(title);
        }
    }
    None
}

// post-action: leave or archive the channel after work is done
async fn post_subscribe_actions(
    client: &mut MtpClient,
    prefix: &str,
    channel_id: i64,
    access_hash: i64,
    joined: bool,
    leave_after: bool,
    archive_after: bool,
    app: &tauri::AppHandle,
) {
    if !joined {
        return;
    }
    if leave_after {
        let req = tl::build_leave_channel(channel_id, access_hash);
        match client.invoke(&req).await {
            Ok(_) => emit(app, t_with("boost_left_channel", &[("prefix", prefix)])),
            Err(e) => emit(
                app,
                t_with("boost_leave_error", &[("prefix", prefix), ("error", &e)]),
            ),
        }
    } else if archive_after {
        let req = tl::build_edit_peer_folder_channel(channel_id, access_hash, 1);
        match client.invoke(&req).await {
            Ok(_) => emit(app, t_with("boost_archived", &[("prefix", prefix)])),
            Err(e) => emit(
                app,
                t_with("boost_archive_error", &[("prefix", prefix), ("error", &e)]),
            ),
        }
    }
}

// ─── Reactions helpers ─────────────────────────────────────────────────────

async fn interruptible_sleep_boost(ms: u64, token: &Arc<AtomicBool>) {
    let mut remaining = ms;
    while remaining > 0 {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let chunk = remaining.min(200);
        tokio::time::sleep(std::time::Duration::from_millis(chunk)).await;
        remaining -= chunk;
    }
}

fn init_reactions_db(path: &std::path::PathBuf) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(path).map_err(|e| {
        t_with(
            "boost_reactions_db_open_error",
            &[("error", &e.to_string())],
        )
    })?;
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS reactions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account TEXT DEFAULT '',
            target TEXT DEFAULT '',
            emoji TEXT DEFAULT '',
            status TEXT DEFAULT '',
            reacted_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_reactions_status ON reactions(status);
    ",
    )
    .map_err(|e| format!("create tables: {e}"))?;
    Ok(conn)
}

fn record_reaction(
    conn: &rusqlite::Connection,
    account: &str,
    target: &str,
    emoji: &str,
    status: &str,
) {
    conn.execute(
        "INSERT INTO reactions (account, target, emoji, status) VALUES (?1,?2,?3,?4)",
        rusqlite::params![account, target, emoji, status],
    )
    .ok();
}
