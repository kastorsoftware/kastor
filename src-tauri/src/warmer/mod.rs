// warmer: account warm-up via natural-looking telegram activity.
// each account runs in its own task with randomized action order and delays.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rand::Rng;
use rand::SeedableRng;
use serde::Deserialize;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex as TokioMutex;

use crate::accounts::connect::connect_account;
use crate::accounts::commands::get_storage_pub;
use crate::accounts::session::AccountJson;
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
        tokio::time::sleep(Duration::from_millis(chunk)).await;
        remaining -= chunk;
    }
}

mod words;

const REACTION_EMOJIS: &[&str] = &[
    "\u{1F44D}", "\u{2764}\u{FE0F}", "\u{1F525}", "\u{1F44F}",
    "\u{1F60D}", "\u{1F929}", "\u{1F64F}", "\u{1F4AF}",
];

const MESSAGE_EMOJIS: &[&str] = &[
    "\u{1F60A}", "\u{1F44D}", "\u{1F600}", "\u{263A}", "\u{1F913}", "\u{1F601}",
    "\u{1F44C}", "\u{1F9D0}", "\u{1F648}", "\u{1F60C}", "\u{1F609}", "\u{1F447}",
    "\u{1F449}", "\u{1F603}", "\u{1F604}", "\u{1F605}", "\u{1F643}", "\u{1F642}",
    "\u{1F60E}", "\u{1F60F}", "\u{1F914}", "\u{1F92D}", "\u{1F450}", "\u{1F91D}",
    "\u{1F91F}", "\u{270C}\u{FE0F}", "\u{270B}", "\u{1F64F}", "\u{1F430}",
    "\u{1F439}", "\u{1F42D}", "\u{1F431}", "\u{1F42F}", "\u{1F981}", "\u{1F42E}",
    "\u{1F437}", "\u{1F649}", "\u{1F435}", "\u{1F64A}", "\u{1F436}", "\u{2600}\u{FE0F}",
    "\u{1F697}", "\u{1F695}", "\u{1F699}", "\u{2705}", "\u{1F4B2}",
];

/// Returns a random emoji suffix (sometimes none, sometimes one, sometimes two)
fn random_emoji_suffix(rng: &mut impl Rng) -> String {
    let roll = rng.gen_range(0..1000u32);
    if roll < 300 {
        // single emoji
        let idx = rng.gen_range(0..MESSAGE_EMOJIS.len());
        format!(" {}", MESSAGE_EMOJIS[idx])
    } else if roll > 800 {
        // double emoji
        let i1 = rng.gen_range(0..MESSAGE_EMOJIS.len());
        let i2 = rng.gen_range(0..MESSAGE_EMOJIS.len());
        format!(" {} {}", MESSAGE_EMOJIS[i1], MESSAGE_EMOJIS[i2])
    } else {
        // no emoji
        String::new()
    }
}

// shared peer info for cross-account fake chats
#[derive(Clone, Debug)]
struct WarmPeer {
    user_id: i64,
    access_hash: i64,
    username: String,
    #[allow(dead_code)]
    phone: String,
    has_spamblock: bool,
}

type SharedPeers = Arc<TokioMutex<Vec<WarmPeer>>>;

#[derive(Deserialize, Clone, Debug)]
pub struct WarmerConfig {
    pub do_all: bool,
    pub search_random_words: bool,
    pub read_random_channels: bool,
    pub read_channels_react: bool,
    pub subscribe_channels: bool,
    pub fake_chats: bool,
    pub fake_chats_use_llm: bool,
    pub write_saved_messages: bool,
    pub rest_between_actions: bool,
    pub browse_group_members: bool,
    pub browse_group_avatars: bool,
    pub browse_group_add_contacts: bool,
    pub read_dialogs: bool,
    pub view_stories: bool,
    pub add_contacts_from_search: bool,
    pub cleanup_after: bool,
    #[serde(default)]
    pub append_emoji: bool,
    #[serde(default)]
    pub duration_minutes: u32,
    #[serde(default = "default_flood_wait")]
    pub max_flood_wait: u64, // 0 = unlimited
}

fn default_flood_wait() -> u64 { 60 }

impl WarmerConfig {
    fn is_enabled(&self, action: &str) -> bool {
        if self.do_all { return true; }
        match action {
            "search" => self.search_random_words,
            "read_channels" => self.read_random_channels,
            "react" => self.read_channels_react,
            "subscribe" => self.subscribe_channels,
            "fake_chats" => self.fake_chats,
            "saved" => self.write_saved_messages,
            "rest" => self.rest_between_actions,
            "group_members" => self.browse_group_members,
            "group_avatars" => self.browse_group_avatars,
            "group_contacts" => self.browse_group_add_contacts,
            "dialogs" => self.read_dialogs,
            "stories" => self.view_stories,
            "search_contacts" => self.add_contacts_from_search,
            "cleanup" => self.cleanup_after,
            _ => false,
        }
    }
}

#[tauri::command]
pub async fn warmer_start(
    ids: Vec<String>,
    config: WarmerConfig,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() {
        return Err(t("warmer_no_accounts"));
    }

    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "warmer".to_string(),
            t_with("warmer_task_name", &[("count", &ids.len().to_string())]),
        )
        .await;

    let cfg = Arc::new(config);
    tokio::spawn(async move {
        run(ids, cfg, &app, token).await;
        emit(&app, t("done"));
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });

    Ok(tid)
}

#[tauri::command]
pub async fn warmer_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn run(
    ids: Vec<String>,
    cfg: Arc<WarmerConfig>,
    app: &tauri::AppHandle,
    token: Arc<AtomicBool>,
) {
    let num = ids.len();
    emit(app, t_with("warmer_starting", &[("count", &num.to_string())]));

    let shared_peers: SharedPeers = Arc::new(TokioMutex::new(Vec::new()));

    let mut handles = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        if !token.load(Ordering::Relaxed) { break; }
        let id = id.clone();
        let cfg = cfg.clone();
        let app_clone = app.clone();
        let token_clone = token.clone();
        let peers = shared_peers.clone();
        let stagger_ms = (i as u64) * 500; // 0.5 sec stagger between accounts

        handles.push(tokio::spawn(async move {
            if stagger_ms > 0 {
                interruptible_sleep(stagger_ms, &token_clone).await;
            }
            let result = warm_account(&id, i + 1, num, &cfg, &app_clone, &token_clone, &peers).await;
            if let Err(e) = result {
                crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                let _ = app_clone.emit("warmer-log", t_with("warmer_acc_error", &[("idx", &(i+1).to_string()), ("total", &num.to_string()), ("error", &e)]));
            }
        }));
    }

    for h in handles { let _ = h.await; }
}

async fn warm_account(
    account_id: &str,
    idx: usize,
    total: usize,
    cfg: &WarmerConfig,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
    peers: &SharedPeers,
) -> Result<(), String> {
    let prefix = format!("[acc {}/{}]", idx, total);
    let storage = get_storage_pub();
    let json_path = storage.json_path(account_id);
    let json = if json_path.exists() {
        AccountJson::from_file(&json_path).unwrap_or_default()
    } else {
        AccountJson::default()
    };

    let mut client = connect_account(account_id).await?;

    let result = warm_session(&mut client, &prefix, cfg, app, token, &json, peers).await;
    // surface a fatal session error even if warm_session swallowed it mid-loop
    if let Some(fatal) = client.fatal_error() {
        return Err(fatal.to_string());
    }
    result
}

async fn warm_session(
    client: &mut MtpClient,
    prefix: &str,
    cfg: &WarmerConfig,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
    json: &AccountJson,
    peers: &SharedPeers,
) -> Result<(), String> {
    client.set_max_flood_wait(cfg.max_flood_wait);
    let my_user_id = if json.user_id > 0 { json.user_id } else { 0 };

    let emit_log = |msg: String| { let _ = app.emit("warmer-log", format!("{prefix} {msg}")); };
    let emit_action = |msg: String| { let _ = app.emit("warmer-log", format!("[action] {prefix} {msg}")); };
    let sleep_token = token.clone();
    let sleep_jitter = move |base_ms: u64, jitter_ms: u64| {
        let t = sleep_token.clone();
        async move {
            let ms = base_ms + rand::thread_rng().gen_range(0..jitter_ms.max(1));
            interruptible_sleep(ms, &t).await;
        }
    };

    if my_user_id == 0 {
        emit_log(t("warmer_no_uid_warning"));
    }

    // check spamblock: first from json, then via checker module
    let mut has_spamblock = !json.spamblock.is_empty() && json.spamblock != crate::i18n::t("status_clean");
    if has_spamblock {
        emit_log(t_with("warmer_spamblock_json", &[("status", &json.spamblock)]));
    } else {
        match crate::checker::checks::check_spambot(client).await {
            Ok(status) => {
                if status != crate::i18n::t("status_clean") {
                    has_spamblock = true;
                    emit_log(t_with("warmer_spamblock_detected", &[("status", &status)]));
                } else {
                    emit_log(t("warmer_no_spamblock"));
                }
            }
            Err(e) => {
                if e.contains("PEER_FLOOD") || e.contains("USER_BANNED") {
                    has_spamblock = true;
                    emit_log(t("warmer_spamblock_peer_flood"));
                }
            }
        }
    }
    sleep_jitter(2000, 1000).await;

    // register self in shared peers pool
    let mut rng = rand::rngs::StdRng::from_entropy();
    let mut temp_username_set = false;
    if my_user_id > 0 {
        let mut username = json.username.clone();
        // if still no username, set a temporary one for peer resolution
        if username.is_empty() {
            let temp = generate_temp_username(&mut rng);
            let req = tl::build_account_update_username(&temp);
            if client.invoke(&req).await.is_ok() {
                username = temp;
                temp_username_set = true;
                emit_log(t_with("warmer_temp_tag_set", &[("username", &username)]));
            }
            sleep_jitter(500, 300).await;
        }
        let phone = json.phone.clone();
        let mut pool = peers.lock().await;
        pool.push(WarmPeer {
            user_id: my_user_id,
            access_hash: 0,
            username,
            phone,
            has_spamblock,
        });
    }

    let mut action_count = 0u32;
    let mut subscribed_channels: Vec<(i64, i64)> = Vec::new();
    let mut saved_msg_ids: Vec<i32> = Vec::new();
    let mut added_contacts: Vec<(i64, i64)> = Vec::new();
    let mut fake_chat_peers: Vec<(i64, i64)> = Vec::new(); // (peer_id, access_hash) for cleanup
    let mut reactions_blocked_channels: HashSet<i64> = HashSet::new();
    let mut send_errors = 0u32;
    let start_time = Instant::now();
    let mut next_rest_at: u32 = rng.gen_range(50..100);

    // build weighted action pool
    let mut pool: Vec<&str> = Vec::new();
    if cfg.is_enabled("search") { for _ in 0..4 { pool.push("search"); } }
    if cfg.is_enabled("read_channels") { for _ in 0..3 { pool.push("read_channel"); } }
    if cfg.is_enabled("dialogs") { for _ in 0..2 { pool.push("dialogs"); } }
    if cfg.is_enabled("saved") { for _ in 0..2 { pool.push("saved"); } }
    if cfg.is_enabled("stories") { pool.push("stories"); }
    if cfg.is_enabled("subscribe") { pool.push("subscribe"); }
    if cfg.is_enabled("group_members") { pool.push("group_members"); }
    if cfg.is_enabled("fake_chats") && !has_spamblock { for _ in 0..4 { pool.push("fake_chat"); } }
    pool.push("read_telegram");

    if pool.is_empty() {
        emit_log(t("warmer_no_actions"));
        return Ok(());
    }

    let deadline = if cfg.duration_minutes > 0 {
        Some(start_time + Duration::from_secs(cfg.duration_minutes as u64 * 60))
    } else {
        None
    };
    emit_log(if cfg.duration_minutes > 0 {
        t_with("warmer_duration", &[("minutes", &cfg.duration_minutes.to_string())])
    } else {
        t("warmer_until_stop")
    });

    loop {
        if !token.load(Ordering::Relaxed) { break; }
        if let Some(dl) = deadline {
            if Instant::now() >= dl { break; }
        }

        let action = pool[rng.gen_range(0..pool.len())];

        match action {
            "saved" => {
                let phrase = words::random_phrase(&mut rng);
                let phrase = if cfg.append_emoji {
                    format!("{}{}", phrase, random_emoji_suffix(&mut rng))
                } else {
                    phrase
                };
                emit_action(t_with("warmer_saved", &[("text", &phrase)]));
                let req = tl::build_send_saved_message(&phrase);
                match client.invoke(&req).await {
                    Ok(data) => {
                        if let Some(msg_id) = tl::extract_first_new_message_id(&data) {
                            saved_msg_ids.push(msg_id);
                        } else {
                            emit_action(t("warmer_saved_no_msgid"));
                        }
                        // flush oldest batch if over limit
                        if saved_msg_ids.len() >= 200 {
                            let batch: Vec<i32> = saved_msg_ids.drain(..100).collect();
                            for &mid in &batch {
                                let del_req = tl::build_delete_messages(&[mid], false);
                                if let Err(e) = client.invoke(&del_req).await {
                                    emit_action(t_with("warmer_del_msg_error", &[("id", &mid.to_string()), ("error", &e)]));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        emit_action(t_with("warmer_saved_send_error", &[("error", &e)]));
                    }
                }
                sleep_jitter(2000, 1500).await;
            }
            "search" => {
                let word = words::random_search_word(&mut rng);
                emit_action(t_with("warmer_search", &[("word", &word)]));
                let req = tl::build_search_global(&word);
                if let Err(e) = client.invoke(&req).await {
                    emit_action(t_with("warmer_search_error", &[("error", &e)]));
                }
                sleep_jitter(2500, 2000).await;
            }
            "read_channel" => {
                let word = words::random_search_word(&mut rng);
                emit_action(t_with("warmer_read_channel", &[("word", &word)]));
                let req = tl::build_contacts_search(&word, 20);
                let channel = match client.invoke(&req).await {
                    Ok(data) => find_channel_in_search_results(&data),
                    Err(e) => { emit_action(t_with("warmer_contacts_search_error", &[("error", &e)])); None }
                };
                // fallback: resolve a known public channel
                let channel = if channel.is_none() {
                    let ch_name = words::random_public_channel(&mut rng);
                    let resolve_req = tl::build_resolve_username(ch_name);
                    match client.invoke(&resolve_req).await {
                        Ok(data) => match tl::parse_resolved_peer(&data) {
                            Ok(pair) => Some(pair),
                            Err(e) => { emit_action(format!("resolve parse: {e}")); None }
                        },
                        Err(e) => { emit_action(format!("resolve @{ch_name}: {e}")); None }
                    }
                } else {
                    channel
                };

                if let Some((ch_id, ch_hash)) = channel {
                    // get real message ids from channel history
                    let history_req = tl::build_get_history_channel(ch_id, ch_hash, 10);
                    let real_msg_ids: Vec<i32> = if let Ok(hist_data) = client.invoke(&history_req).await {
                        tl::parse_messages_structured(&hist_data)
                            .unwrap_or_default()
                            .iter()
                            .map(|m| m.id)
                            .filter(|id| *id > 0)
                            .collect()
                    } else {
                        Vec::new()
                    };
                    sleep_jitter(800, 600).await;

                    let read_count = rng.gen_range(2..8);
                    for _ in 0..read_count {
                        sleep_jitter(800, 600).await;
                        if cfg.is_enabled("react")
                            && !reactions_blocked_channels.contains(&ch_id)
                            && rng.gen_range(0..100) < 30
                            && !real_msg_ids.is_empty()
                        {
                            let emoji = REACTION_EMOJIS[rng.gen_range(0..REACTION_EMOJIS.len())];
                            let msg_id = real_msg_ids[rng.gen_range(0..real_msg_ids.len())];
                            let req = tl::build_send_reaction(
                                tl::INPUT_PEER_CHANNEL, ch_id, ch_hash, msg_id, emoji
                            );
                            match client.invoke(&req).await {
                                Ok(_) => { emit_action(t_with("warmer_reaction", &[("emoji", emoji), ("id", &msg_id.to_string())])); }
                                Err(e) => {
                                    if e.contains("CHAT_WRITE_FORBIDDEN") || e.contains("REACTION_EMPTY") {
                                        reactions_blocked_channels.insert(ch_id);
                                        emit_action(t_with("warmer_reactions_disabled", &[("id", &ch_id.to_string())]));
                                    }
                                }
                            }
                        }
                    }
                }
                sleep_jitter(1500, 1000).await;
            }
            "dialogs" => {
                emit_action(t("warmer_read_dialogs"));
                let req = tl::build_get_dialogs();
                if let Err(e) = client.invoke(&req).await {
                    emit_action(t_with("warmer_read_dialogs_error", &[("error", &e)]));
                }
                sleep_jitter(2000, 1000).await;
            }
            "stories" => {
                emit_action(t("warmer_view_stories"));
                let word = words::random_search_word(&mut rng);
                let search_req = tl::build_contacts_search(&word, 20);
                if let Ok(data) = client.invoke(&search_req).await {
                    if let Some((ch_id, ch_hash)) = find_channel_in_search_results(&data) {
                        // read stories from this channel using inputPeerChannel
                        let req = build_get_peer_stories(ch_id, ch_hash);
                        if let Ok(stories_data) = client.invoke(&req).await {
                            let story_ids = extract_story_ids_from_peer_stories(&stories_data);
                            let view_count = story_ids.len().min(5);
                            if view_count > 0 {
                                let read_req = build_read_stories_channel(ch_id, ch_hash, story_ids[view_count - 1]);
                                if let Err(e) = client.invoke(&read_req).await {
                                    emit_action(t_with("warmer_stories_read_error", &[("error", &e)]));
                                } else {
                                    emit_action(t_with("warmer_stories_viewed", &[("count", &view_count.to_string()), ("id", &ch_id.to_string())]));
                                }
                            }
                        }
                    }
                }
                sleep_jitter(1500, 800).await;
            }
            "subscribe" => {
                emit_action(t("warmer_subscribing"));
                let word = words::random_search_word(&mut rng);
                let req = tl::build_contacts_search(&word, 20);
                let channel = match client.invoke(&req).await {
                    Ok(data) => {
                        emit_action(t_with("warmer_search_result", &[("word", &word), ("len", &data.len().to_string())]));
                        find_channel_in_search_results(&data)
                    }
                    Err(e) => {
                        emit_action(t_with("warmer_contacts_search_error", &[("error", &e)]));
                        None
                    }
                };
                // fallback: resolve a known public channel by username
                let channel = if channel.is_none() {
                    let ch_name = words::random_public_channel(&mut rng);
                    emit_action(t_with("warmer_search_empty_fallback", &[("name", ch_name)]));
                    let resolve_req = tl::build_resolve_username(ch_name);
                    match client.invoke(&resolve_req).await {
                        Ok(data) => match tl::parse_resolved_peer(&data) {
                            Ok(pair) => Some(pair),
                            Err(e) => { emit_action(t_with("warmer_parse_resolve_error", &[("error", &e)])); None }
                        },
                        Err(e) => { emit_action(t_with("warmer_resolve_error", &[("name", ch_name), ("error", &e)])); None }
                    }
                } else {
                    channel
                };

                if let Some((ch_id, ch_hash)) = channel {
                    let join_req = tl::build_join_channel(ch_id, ch_hash);
                    match client.invoke(&join_req).await {
                        Ok(_) => {
                            if subscribed_channels.len() >= 100 {
                                let (old_id, old_hash) = subscribed_channels.remove(0);
                                let leave_req = tl::build_leave_channel(old_id, old_hash);
                                if let Err(e) = client.invoke(&leave_req).await {
                                    emit_action(t_with("warmer_unsubscribe_error", &[("id", &old_id.to_string()), ("error", &e)]));
                                }
                                sleep_jitter(300, 200).await;
                            }
                            subscribed_channels.push((ch_id, ch_hash));
                            emit_action(t_with("warmer_subscribed", &[("id", &ch_id.to_string())]));
                        }
                        Err(e) => {
                            emit_action(t_with("warmer_join_error", &[("id", &ch_id.to_string()), ("error", &e)]));
                        }
                    }
                } else {
                    emit_action(t("warmer_no_channel_found"));
                }
                sleep_jitter(3000, 2000).await;
            }
            "group_members" => {
                emit_action(t("warmer_group_members"));
                let word = words::random_search_word(&mut rng);
                let req = tl::build_contacts_search(&word, 20);
                if let Ok(data) = client.invoke(&req).await {
                    // try to find a megagroup (not broadcast channel)
                    if let Some((ch_id, ch_hash, is_megagroup)) = find_group_in_search_results(&data) {
                        if is_megagroup {
                            // get participants
                            let part_req = tl::build_channels_get_participants(
                                ch_id, ch_hash, tl::ParticipantsFilter::Recent, 0, 20
                            );
                            if let Ok(part_data) = client.invoke(&part_req).await {
                                if let Ok(batch) = tl::parse_channel_participants(&part_data) {
                                    emit_action(t_with("warmer_group_members_count", &[("id", &ch_id.to_string()), ("count", &batch.users.len().to_string())]));

                                    // optionally browse avatars
                                    if cfg.is_enabled("group_avatars") && !batch.users.is_empty() {
                                        let avatar_count = rng.gen_range(1..4).min(batch.users.len());
                                        for u in batch.users.iter().take(avatar_count) {
                                            if u.access_hash != 0 {
                                                let photo_req = build_get_user_photos(u.id, u.access_hash, 1);
                                                if let Err(e) = client.invoke(&photo_req).await {
                                                    emit_action(t_with("warmer_photo_error", &[("id", &u.id.to_string()), ("error", &e)]));
                                                }
                                                sleep_jitter(300, 200).await;
                                            }
                                        }
                                        emit_action(t_with("warmer_avatars_viewed", &[("count", &avatar_count.to_string())]));
                                    }

                                    // optionally add contacts from group
                                    if cfg.is_enabled("group_contacts") && !batch.users.is_empty() {
                                        let add_count = rng.gen_range(1..3).min(batch.users.len());
                                        for u in batch.users.iter().take(add_count) {
                                            if u.access_hash != 0 && !u.is_bot && !u.is_deleted && !u.is_self {
                                                if added_contacts.len() >= 100 {
                                                    let (old_id, old_hash) = added_contacts.remove(0);
                                                    let del_req = tl::build_contacts_delete_contacts(&[(old_id, old_hash)]);
                                                    if let Err(e) = client.invoke(&del_req).await {
                                                        emit_action(t_with("warmer_del_contact_error", &[("id", &old_id.to_string()), ("error", &e)]));
                                                    }
                                                    sleep_jitter(300, 200).await;
                                                }
                                                let name = words::random_phrase(&mut rng);
                                                let add_req = tl::build_add_contact(u.id, u.access_hash, &name, "", "");
                                                if client.invoke(&add_req).await.is_ok() {
                                                    added_contacts.push((u.id, u.access_hash));
                                                    emit_action(t_with("warmer_contact_added_group", &[("id", &u.id.to_string())]));
                                                }
                                                sleep_jitter(500, 300).await;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                sleep_jitter(2000, 1500).await;
            }
            "fake_chat" => {
                // pick a peer from the warming pool that is not us
                let target = {
                    let pool = peers.lock().await;
                    // prefer peers without spamblock, but accept any if none available
                    let mut candidates: Vec<&WarmPeer> = pool.iter()
                        .filter(|p| p.user_id != my_user_id && !p.has_spamblock)
                        .collect();
                    if candidates.is_empty() {
                        candidates = pool.iter()
                            .filter(|p| p.user_id != my_user_id)
                            .collect();
                    }
                    if candidates.is_empty() { None }
                    else { Some(candidates[rng.gen_range(0..candidates.len())].clone()) }
                };

                if let Some(target) = target {
                    if has_spamblock {
                        sleep_jitter(2000, 1000).await;
                        action_count += 1;
                        continue;
                    }

                    // resolve target's access_hash via username
                    let resolved = if !target.username.is_empty() {
                        let req = tl::build_resolve_username(&target.username);
                        match client.invoke(&req).await {
                            Ok(data) => tl::parse_resolved_peer(&data).ok(),
                            Err(e) => {
                                emit_action(t_with("warmer_resolve_username_error", &[("username", &target.username), ("error", &e)]));
                                None
                            }
                        }
                    } else {
                        None
                    };

                    // if resolve failed and we have phone, try importing contact
                    let resolved = if resolved.is_none() && !target.phone.is_empty() {
                        let name = generate_random_name(&mut rng);
                        let import_req = build_import_contact(&target.phone, &name);
                        match client.invoke(&import_req).await {
                            Ok(data) => extract_imported_user(&data),
                            Err(_) => None,
                        }
                    } else {
                        resolved
                    };

                    let (target_id, target_hash) = match resolved {
                        Some((id, hash)) => (id, hash),
                        None => (target.user_id, target.access_hash),
                    };

                    if target_hash == 0 {
                        emit_action(t_with("warmer_skip_fake_chat", &[("id", &target.user_id.to_string()), ("username", &target.username)]));
                    } else {
                        let text = if cfg.fake_chats_use_llm {
                            match crate::llm::complete(
                                "Ты обычный человек в мессенджере. Пиши коротко, по-русски, неформально. Просто веди непринуждённую беседу.",
                                "Напиши короткое сообщение другу о повседневной жизни.",
                            ) {
                                Ok(reply) => reply.trim().to_string(),
                                Err(e) => {
                                    emit_action(t_with("warmer_llm_error", &[("error", &e)]));
                                    words::random_phrase(&mut rng)
                                }
                            }
                        } else {
                            words::random_phrase(&mut rng)
                        };
                        let text = if cfg.append_emoji {
                            format!("{}{}", text, random_emoji_suffix(&mut rng))
                        } else {
                            text
                        };
                        emit_action(t_with("warmer_fake_chat_msg", &[("id", &target_id.to_string()), ("text", &text)]));
                        let random_id: i64 = rng.gen();
                        let req = tl::build_send_message(target_id, target_hash, &text, random_id);
                        match client.invoke(&req).await {
                            Ok(_) => {
                                fake_chat_peers.push((target_id, target_hash));
                                let contact_name = words::random_phrase(&mut rng);
                                let add_req = tl::build_add_contact(target_id, target_hash, &contact_name, "", "");
                                if client.invoke(&add_req).await.is_ok() {
                                    added_contacts.push((target_id, target_hash));
                                }
                            }
                            Err(e) => {
                                send_errors += 1;
                                emit_action(t_with("warmer_send_error", &[("error", &e)]));
                                if e.contains("PEER_FLOOD") || e.contains("USER_PRIVACY_RESTRICTED") {
                                    has_spamblock = true;
                                    let mut pool = peers.lock().await;
                                    if let Some(me) = pool.iter_mut().find(|p| p.user_id == my_user_id) {
                                        me.has_spamblock = true;
                                    }
                                    emit_log(t_with("warmer_spamblock_on_send", &[("error", &e)]));
                                } else if send_errors >= 5 {
                                    has_spamblock = true;
                                    emit_log(t("warmer_too_many_send_errors"));
                                }
                            }
                        }
                    }
                } else {
                    // no peers yet — fallback to saved messages
                    let text = words::random_phrase(&mut rng);
                    let text = if cfg.append_emoji {
                        format!("{}{}", text, random_emoji_suffix(&mut rng))
                    } else {
                        text
                    };
                    emit_action(t_with("warmer_fake_chat_no_peers", &[("text", &text)]));
                    let req = tl::build_send_saved_message(&text);
                    if let Ok(data) = client.invoke(&req).await {
                        if let Some(msg_id) = tl::extract_first_new_message_id(&data) {
                            saved_msg_ids.push(msg_id);
                        }
                    }
                }
                sleep_jitter(15000, 20000).await;
            }
            "read_telegram" => {
                emit_action(t("warmer_read_telegram"));
                let req = tl::build_get_history_peer(777000, 0, 5);
                if let Err(e) = client.invoke(&req).await {
                    emit_action(t_with("warmer_read_telegram_error", &[("error", &e)]));
                }
                sleep_jitter(2000, 1500).await;
            }
            "add_contact" => {
                emit_action(t("warmer_add_contact"));
                let word = words::random_search_word(&mut rng);
                let req = tl::build_contacts_search(&word, 20);
                match client.invoke(&req).await {
                    Ok(data) => {
                        if let Some((user_id, user_hash)) = find_user_in_search_results(&data) {
                            let name = words::random_phrase(&mut rng);
                            let add_req = tl::build_add_contact(user_id, user_hash, &name, "", "");
                            match client.invoke(&add_req).await {
                                Ok(_) => {
                                    if added_contacts.len() >= 100 {
                                        let (old_id, old_hash) = added_contacts.remove(0);
                                        let del_req = tl::build_contacts_delete_contacts(&[(old_id, old_hash)]);
                                        if let Err(e) = client.invoke(&del_req).await {
                                            emit_action(t_with("warmer_del_old_contact_error", &[("id", &old_id.to_string()), ("error", &e)]));
                                        }
                                        sleep_jitter(300, 200).await;
                                    }
                                    added_contacts.push((user_id, user_hash));
                                    emit_action(t_with("warmer_contact_added", &[("id", &user_id.to_string())]));
                                }
                                Err(e) => {
                                    emit_action(t_with("warmer_contact_add_error", &[("id", &user_id.to_string()), ("error", &e)]));
                                }
                            }
                        } else {
                            emit_action(t("warmer_search_no_users"));
                        }
                    }
                    Err(e) => {
                        emit_action(t_with("warmer_search_error", &[("error", &e)]));
                    }
                }
                sleep_jitter(3000, 2000).await;
            }
            _ => {}
        }

        action_count += 1;

        // rest between actions
        if cfg.is_enabled("rest") {
            sleep_jitter(1500, 1000).await;
            if action_count >= next_rest_at {
                let rest_sec = rng.gen_range(60..90);
                emit_log(t_with("warmer_rest", &[("seconds", &rest_sec.to_string())]));
                interruptible_sleep(rest_sec as u64 * 1000, token).await;
                next_rest_at = action_count + rng.gen_range(50..100);
            }
        }
    }

    // cleanup phase
    if cfg.is_enabled("cleanup") {
        emit_log(t_with("warmer_cleanup_start", &[("channels", &subscribed_channels.len().to_string()), ("msgs", &saved_msg_ids.len().to_string()), ("chats", &fake_chat_peers.len().to_string()), ("contacts", &added_contacts.len().to_string())]));

        if !subscribed_channels.is_empty() {
            emit_log(t_with("warmer_cleanup_unsub", &[("count", &subscribed_channels.len().to_string())]));
            for (ch_id, ch_hash) in &subscribed_channels {
                if !token.load(Ordering::Relaxed) { break; }
                let req = tl::build_leave_channel(*ch_id, *ch_hash);
                if let Err(e) = client.invoke(&req).await {
                    emit_log(t_with("warmer_cleanup_unsub_error", &[("id", &ch_id.to_string()), ("error", &e)]));
                }
                sleep_jitter(500, 300).await;
            }
        }
        // saved messages: fetch last N from history and delete by real IDs
        let saved_count = saved_msg_ids.len() as i32;
        if saved_count > 0 {
            emit_log(t_with("warmer_cleanup_saved", &[("count", &saved_count.to_string())]));
            let history_req = tl::build_get_history_self(saved_count.min(100));
            match client.invoke(&history_req).await {
                Ok(hist_data) => {
                    let msgs = tl::parse_messages_structured(&hist_data).unwrap_or_default();
                    let ids: Vec<i32> = msgs.iter().map(|m| m.id).filter(|id| *id > 0).collect();
                    if !ids.is_empty() {
                        emit_log(t_with("warmer_cleanup_saved_found", &[("count", &ids.len().to_string())]));
                        for &msg_id in &ids {
                            if !token.load(Ordering::Relaxed) { break; }
                            let del_req = tl::build_delete_messages(&[msg_id], false);
                            if let Err(e) = client.invoke(&del_req).await {
                                emit_log(t_with("warmer_del_msg_error", &[("id", &msg_id.to_string()), ("error", &e)]));
                            }
                            sleep_jitter(200, 100).await;
                        }
                    } else {
                        emit_log(t("warmer_cleanup_saved_no_ids"));
                    }
                }
                Err(e) => {
                    emit_log(t_with("warmer_cleanup_saved_history_error", &[("error", &e)]));
                }
            }
        }
        if !fake_chat_peers.is_empty() {
            emit_log(t_with("warmer_cleanup_chats", &[("count", &fake_chat_peers.len().to_string())]));
            for (peer_id, peer_hash) in &fake_chat_peers {
                if !token.load(Ordering::Relaxed) { break; }
                let req = tl::build_delete_history(*peer_id, *peer_hash);
                if let Err(e) = client.invoke(&req).await {
                    emit_log(t_with("warmer_cleanup_chat_error", &[("id", &peer_id.to_string()), ("error", &e)]));
                }
                sleep_jitter(500, 300).await;
            }
        }
        if !added_contacts.is_empty() {
            let valid_contacts: Vec<(i64, i64)> = added_contacts.iter()
                .filter(|(_, hash)| *hash != 0)
                .copied()
                .collect();
            if !valid_contacts.is_empty() {
                emit_log(t_with("warmer_cleanup_contacts", &[("valid", &valid_contacts.len().to_string()), ("total", &added_contacts.len().to_string())]));
                for chunk in valid_contacts.chunks(50) {
                    let req = tl::build_contacts_delete_contacts(chunk);
                    if let Err(e) = client.invoke(&req).await {
                        emit_log(t_with("warmer_cleanup_contacts_error", &[("error", &e)]));
                    }
                    sleep_jitter(300, 200).await;
                }
            } else {
                emit_log(t_with("warmer_cleanup_contacts_skip", &[("count", &added_contacts.len().to_string())]));
            }
        }
        // remove temporary username
        if temp_username_set {
            let req = tl::build_account_update_username("");
            if let Err(e) = client.invoke(&req).await {
                emit_log(t_with("warmer_cleanup_tag_error", &[("error", &e)]));
            } else {
                emit_log(t("warmer_cleanup_tag_done"));
            }
        }
    }

    emit_log(t_with("warmer_session_done", &[("count", &action_count.to_string())]));
    Ok(())
}

// heuristic: scan search results for a Channel constructor and extract id+access_hash
fn find_channel_in_search_results(data: &[u8]) -> Option<(i64, i64)> {
    // try gzip decompression first
    if data.len() >= 4 {
        let ctor = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if ctor == tl::GZIP_PACKED {
            if let Ok(decompressed) = tl::decompress_gzip(&data[5..]) {
                return find_channel_in_search_results(&decompressed);
            }
            let mut cursor = std::io::Cursor::new(data);
            let _ = byteorder::ReadBytesExt::read_u32::<byteorder::LittleEndian>(&mut cursor);
            if let Ok(compressed) = tl::deserialize_bytes(&mut cursor) {
                if let Ok(decompressed) = tl::decompress_gzip(&compressed) {
                    return find_channel_in_search_results(&decompressed);
                }
            }
        }
    }

    let mut i = 0usize;
    while i + 4 <= data.len() {
        let c = u32::from_le_bytes([data[i], data[i+1], data[i+2], data[i+3]]);
        if c == tl_gen::CHANNEL {
            let mut cursor = std::io::Cursor::new(&data[i..]);
            if let Ok(tl_gen::TlChat::Channel { id, access_hash: Some(ah), .. }) = tl_gen::TlChat::deserialize(&mut cursor) {
                if id > 0 && ah != 0 {
                    return Some((id, ah));
                }
            }
        }
        i += 4;
    }
    None
}

// find a User in search results by scanning for user constructors
fn find_user_in_search_results(data: &[u8]) -> Option<(i64, i64)> {
    if data.len() >= 4 {
        let ctor = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if ctor == tl::GZIP_PACKED {
            let mut cursor = std::io::Cursor::new(data);
            let _ = byteorder::ReadBytesExt::read_u32::<byteorder::LittleEndian>(&mut cursor);
            if let Ok(compressed) = tl::deserialize_bytes(&mut cursor) {
                if let Ok(decompressed) = tl::decompress_gzip(&compressed) {
                    return find_user_in_search_results(&decompressed);
                }
            }
        }
    }

    let mut i = 0usize;
    while i + 4 <= data.len() {
        let c = u32::from_le_bytes([data[i], data[i+1], data[i+2], data[i+3]]);
        if c == tl_gen::USER {
            let mut cursor = std::io::Cursor::new(&data[i..]);
            if let Ok(tl_gen::TlUser::User { id, access_hash: Some(ah), bot, .. }) = tl_gen::TlUser::deserialize(&mut cursor) {
                // skip bots (bot flag), keep regular users
                if id > 0 && ah != 0 && !bot {
                    return Some((id, ah));
                }
            }
        }
        i += 4;
    }
    None
}
fn generate_temp_username(rng: &mut impl Rng) -> String {
    let chars: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    let mut name = String::with_capacity(11);
    for _ in 0..5 { name.push(chars[rng.gen_range(0..chars.len())] as char); }
    name.push('_');
    for _ in 0..5 { name.push(chars[rng.gen_range(0..chars.len())] as char); }
    name
}

fn generate_random_name(rng: &mut impl Rng) -> String {
    let chars: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    let mut first = String::with_capacity(5);
    let mut last = String::with_capacity(5);
    for _ in 0..5 { first.push(chars[rng.gen_range(0..chars.len())] as char); }
    for _ in 0..5 { last.push(chars[rng.gen_range(0..chars.len())] as char); }
    format!("{first} {last}")
}

fn emit(app: &tauri::AppHandle, msg: String) {
    let _ = app.emit("warmer-log", msg);
}

// find a megagroup in search results: returns (id, access_hash, is_megagroup)
fn find_group_in_search_results(data: &[u8]) -> Option<(i64, i64, bool)> {
    if data.len() >= 4 {
        let ctor = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if ctor == tl::GZIP_PACKED {
            let mut cursor = std::io::Cursor::new(data);
            let _ = byteorder::ReadBytesExt::read_u32::<byteorder::LittleEndian>(&mut cursor);
            if let Ok(compressed) = tl::deserialize_bytes(&mut cursor) {
                if let Ok(decompressed) = tl::decompress_gzip(&compressed) {
                    return find_group_in_search_results(&decompressed);
                }
            }
        }
    }

    let mut i = 0usize;
    while i + 4 <= data.len() {
        let c = u32::from_le_bytes([data[i], data[i+1], data[i+2], data[i+3]]);
        if c == tl_gen::CHANNEL {
            let mut cursor = std::io::Cursor::new(&data[i..]);
            if let Ok(tl_gen::TlChat::Channel { id, access_hash: Some(ah), broadcast, megagroup, .. }) = tl_gen::TlChat::deserialize(&mut cursor) {
                if id > 0 && ah != 0 {
                    return Some((id, ah, megagroup && !broadcast));
                }
            }
        }
        i += 4;
    }
    None
}

// stories.getPeerStories#2c4ada50 peer:InputPeer = stories.PeerStories
fn build_get_peer_stories(channel_id: i64, access_hash: i64) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    tl_gen::build_stories_getPeerStories(&peer)
}

// extract story IDs from stories.peerStories response (scan for storyItem constructors)
fn extract_story_ids_from_peer_stories(data: &[u8]) -> Vec<i32> {
    let mut ids = Vec::new();
    let mut i = 0usize;
    while i + 12 <= data.len() {
        let c = u32::from_le_bytes([data[i], data[i+1], data[i+2], data[i+3]]);
        if c == tl_gen::STORY_ITEM && i + 12 <= data.len() {
            // flags(4) + id(4)
            let story_id = i32::from_le_bytes([data[i+8], data[i+9], data[i+10], data[i+11]]);
            if story_id > 0 {
                ids.push(story_id);
            }
        }
        i += 4;
    }
    ids.truncate(20);
    ids
}

fn build_read_stories_channel(channel_id: i64, access_hash: i64, max_id: i32) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    tl_gen::build_stories_readStories(&peer, max_id)
}

// photos.getUserPhotos for a specific user (not self)
fn build_get_user_photos(user_id: i64, access_hash: i64, limit: i32) -> Vec<u8> {
    let input_user = tl_gen::serialize_input_user(user_id, access_hash);
    tl_gen::build_photos_getUserPhotos(&input_user, 0, 0, limit)
}

// contacts.importContacts with a single inputPhoneContact
fn build_import_contact(phone: &str, first_name: &str) -> Vec<u8> {
    let client_id: i64 = rand::thread_rng().gen();
    let contact = tl_gen::serialize_inputPhoneContact(client_id, phone, first_name, "", None);
    tl_gen::build_contacts_importContacts(&[&contact])
}

fn extract_imported_user(data: &[u8]) -> Option<(i64, i64)> {
    let mut i = 0usize;
    while i + 4 <= data.len() {
        let c = u32::from_le_bytes([data[i], data[i+1], data[i+2], data[i+3]]);
        if c == tl_gen::USER {
            let mut cursor = std::io::Cursor::new(&data[i..]);
            if let Ok(tl_gen::TlUser::User { id, access_hash: Some(ah), .. }) = tl_gen::TlUser::deserialize(&mut cursor) {
                if id > 0 && ah != 0 {
                    return Some((id, ah));
                }
            }
        }
        i += 4;
    }
    None
}
