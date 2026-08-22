// inviter: mass invite users to groups/channels
// Features:
// - normal mode: each account invites from its contacts or a username list
// - admin mode: main account grants invite-admin rights, workers invite, then rights are revoked
// - SQLite database for user list with statuses (persistent across restarts)
// - Batching: multiple users per InviteToChannel request
// - Force mode: retry until target invite count is reached
// - Multiple target groups with round-robin distribution
// - AutoStop rules (ban, spamblock, flood, sequential errors)
// - Post-invite verification via GetCommonChats
// - Statistics database

use rusqlite::Connection;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};

use crate::accounts::connect::connect_account;
use crate::i18n::{t, t_with};
use crate::mtproto::client::MtpClient;
use crate::mtproto::invite::resolve_channel_link;
use crate::mtproto::tl::{self, OnlineBucket};
use crate::mtproto::tl_gen;
use crate::queue::TaskQueue;

pub mod db;

async fn interruptible_sleep(ms: u64, token: &Arc<AtomicBool>) {
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

/// Emulates online activity during delay
async fn delay_online(client: &mut MtpClient, ms: u64, token: &Arc<AtomicBool>) {
    let online_req = tl_gen::build_account_updateStatus(false);
    let _ = client.invoke(&online_req).await;
    interruptible_sleep(ms, token).await;
    let offline_req = tl_gen::build_account_updateStatus(true);
    let _ = client.invoke(&offline_req).await;
}

#[derive(Deserialize, Clone)]
pub struct AutoStopRules {
    #[serde(default)]
    pub max_ban: u32,
    #[serde(default)]
    pub max_spamblock: u32,
    #[serde(default)]
    pub max_flood: u32,
    #[serde(default)]
    pub max_sequential_errors: u32,
}

impl Default for AutoStopRules {
    fn default() -> Self {
        Self {
            max_ban: 0,
            max_spamblock: 0,
            max_flood: 0,
            max_sequential_errors: 0,
        }
    }
}

#[derive(Deserialize, Clone)]
pub struct InviterConfig {
    pub mode: String,         // "normal" | "admin"
    pub targets: Vec<String>, // multiple target groups/channels (round-robin)
    #[serde(default)]
    pub target: String, // legacy single target (fallback)
    pub max_per_account: u32,
    pub batch_size: u32, // users per single InviteToChannel request
    pub delay_min: u32,
    pub delay_max: u32,
    pub delay_unit: String, // "seconds" | "minutes"
    pub max_flood_wait: u64,
    pub source_mode: String, // "contacts" | "usernames" | "database" | "phones"
    pub usernames_path: String,
    #[serde(default)]
    pub phones_path: String, // file with phone numbers (for source_mode "phones")
    pub online_filter: String, // "any" | "recent" | "week" | "month" | "long"
    pub admin_account_id: String,
    pub delay_after_admin: u32,
    pub revoke_admin_after: bool,
    #[serde(default = "default_true")]
    pub leave_after_work: bool,
    #[serde(default = "default_true")]
    pub check_users: bool,
    #[serde(default = "default_peer_flood_limit")]
    pub peer_flood_limit: u32,
    #[serde(default = "default_true")]
    pub force_mode: bool,
    #[serde(default)]
    pub autostop: AutoStopRules,
    #[serde(default = "default_true")]
    pub verify_after_invite: bool,
}

fn default_true() -> bool {
    true
}
fn default_peer_flood_limit() -> u32 {
    3
}

/// Shared autostop counters across all workers
struct AutoStopCounters {
    ban: AtomicUsize,
    spamblock: AtomicUsize,
    flood: AtomicUsize,
    sequential: AtomicUsize,
}

impl AutoStopCounters {
    fn new() -> Self {
        Self {
            ban: AtomicUsize::new(0),
            spamblock: AtomicUsize::new(0),
            flood: AtomicUsize::new(0),
            sequential: AtomicUsize::new(0),
        }
    }

    fn should_stop(&self, rules: &AutoStopRules) -> bool {
        if rules.max_ban > 0 && self.ban.load(Ordering::Relaxed) as u32 >= rules.max_ban {
            return true;
        }
        if rules.max_spamblock > 0
            && self.spamblock.load(Ordering::Relaxed) as u32 >= rules.max_spamblock
        {
            return true;
        }
        if rules.max_flood > 0 && self.flood.load(Ordering::Relaxed) as u32 >= rules.max_flood {
            return true;
        }
        if rules.max_sequential_errors > 0
            && self.sequential.load(Ordering::Relaxed) as u32 >= rules.max_sequential_errors
        {
            return true;
        }
        false
    }

    fn record_error(&self, error: &str) {
        if error.contains("USER_BANNED") || error.contains("AUTH_KEY_UNREGISTERED") {
            self.ban.fetch_add(1, Ordering::Relaxed);
        }
        if error.contains("PEER_FLOOD") || error.contains("USER_PRIVACY") {
            self.spamblock.fetch_add(1, Ordering::Relaxed);
        }
        if error.contains("FLOOD_WAIT") || error.contains("FLOOD") {
            self.flood.fetch_add(1, Ordering::Relaxed);
        }
        self.sequential.fetch_add(1, Ordering::Relaxed);
    }

    fn reset_sequential(&self) {
        self.sequential.store(0, Ordering::Relaxed);
    }
}

#[tauri::command]
pub async fn inviter_start(
    ids: Vec<String>,
    config: InviterConfig,
    threads: Option<usize>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if ids.is_empty() {
        return Err(t("inviter_no_accounts"));
    }
    let concurrency = threads.unwrap_or(5).max(1).min(100);
    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();

    let queue: tauri::State<'_, TaskQueue> = app.state();
    let token = queue
        .register_task(
            task_id.clone(),
            "inviter".to_string(),
            t_with("inviter_task_name", &[("count", &ids.len().to_string())]),
        )
        .await;

    // Resolve targets list
    let targets: Vec<String> = if !config.targets.is_empty() {
        config.targets.clone()
    } else if !config.target.is_empty() {
        vec![config.target.clone()]
    } else {
        return Err(t("inviter_no_target"));
    };

    // Initialize databases
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kastor")
        .join("inviter");
    std::fs::create_dir_all(&data_dir).ok();

    let users_db_path = data_dir.join(format!("{}_users.db", timestamp));
    let stats_db_path = data_dir.join(format!("{}_stats.db", timestamp));

    // Import usernames to database if source is "usernames"
    let usernames: Vec<String> =
        if config.source_mode == "usernames" && !config.usernames_path.is_empty() {
            std::fs::read_to_string(&config.usernames_path)
                .unwrap_or_default()
                .lines()
                .map(|l| l.trim().trim_start_matches('@').to_string())
                .filter(|l| !l.is_empty())
                .collect()
        } else {
            Vec::new()
        };

    // Import phone numbers to database if source is "phones"
    let phones: Vec<String> = if config.source_mode == "phones" && !config.phones_path.is_empty() {
        std::fs::read_to_string(&config.phones_path)
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().replace([' ', '-', '(', ')'], ""))
            .filter(|l| !l.is_empty() && l.len() >= 7)
            .collect()
    } else {
        Vec::new()
    };

    if config.source_mode == "usernames" || config.source_mode == "database" {
        let db_conn =
            db::init_users_db(&users_db_path).map_err(|e| format!("init users db: {e}"))?;
        if !usernames.is_empty() {
            let imported =
                db::import_usernames(&db_conn, &usernames).map_err(|e| format!("import: {e}"))?;
            let _ = app.emit(
                "inviter-log",
                t_with(
                    "inviter_imported_usernames",
                    &[("count", &imported.to_string())],
                ),
            );
        }
    }

    if config.source_mode == "phones" {
        let db_conn =
            db::init_users_db(&users_db_path).map_err(|e| format!("init users db: {e}"))?;
        if !phones.is_empty() {
            let imported =
                db::import_phones(&db_conn, &phones).map_err(|e| format!("import phones: {e}"))?;
            let _ = app.emit(
                "inviter-log",
                t_with(
                    "inviter_imported_phones",
                    &[("count", &imported.to_string())],
                ),
            );
        }
    }

    let _stats_conn =
        db::init_stats_db(&stats_db_path).map_err(|e| format!("init stats db: {e}"))?;
    let _ = app.emit(
        "inviter-log",
        t_with(
            "inviter_stats_db",
            &[("path", &stats_db_path.display().to_string())],
        ),
    );

    let config = Arc::new(config);
    let targets = Arc::new(targets);
    let users_db_path = Arc::new(users_db_path);
    let stats_db_path = Arc::new(stats_db_path);
    let autostop = Arc::new(AutoStopCounters::new());

    // Shared index for round-robin group assignment
    let group_idx = Arc::new(AtomicUsize::new(0));
    // Shared username index for non-db mode
    let username_idx = Arc::new(AtomicUsize::new(0));
    let usernames_arc = Arc::new(usernames);

    tokio::spawn(async move {
        let is_admin_mode = config.mode == "admin" && !config.admin_account_id.is_empty();
        let main_id = if is_admin_mode {
            Some(config.admin_account_id.clone())
        } else {
            None
        };

        let mut user_ids: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        let mut promoted: Vec<i64> = Vec::new();

        if is_admin_mode {
            let _ = app.emit("inviter-log", t("inviter_collecting_ids"));
            for id in &ids {
                if !token.load(Ordering::Relaxed) {
                    break;
                }
                match get_account_user_id(id).await {
                    Ok(uid) => {
                        user_ids.insert(id.clone(), uid);
                    }
                    Err(e) => {
                        let _ = app.emit(
                            "inviter-log",
                            t_with("inviter_uid_error", &[("id", id), ("error", &e)]),
                        );
                    }
                }
            }
            if let Some(ref mid) = main_id {
                match setup_main_account_inviter(
                    mid, &ids, &user_ids, &config, &targets, &app, &token,
                )
                .await
                {
                    Ok(p) => {
                        promoted = p;
                        let _ = app.emit(
                            "inviter-log",
                            t_with(
                                "inviter_admin_setup",
                                &[("count", &promoted.len().to_string())],
                            ),
                        );
                        if config.delay_after_admin > 0 {
                            interruptible_sleep(config.delay_after_admin as u64 * 1000, &token)
                                .await;
                        }
                    }
                    Err(e) => {
                        let _ = app.emit("inviter-log", format!("{}: {e}", t("error")));
                    }
                }
            }
        }
        let user_ids = Arc::new(user_ids);
        let promoted = Arc::new(promoted);
        let admin_promised = !promoted.is_empty();

        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let total = ids.len();
        let mut handles = Vec::new();

        for (i, id) in ids.into_iter().enumerate() {
            if !token.load(Ordering::Relaxed) {
                break;
            }
            if autostop.should_stop(&config.autostop) {
                let _ = app.emit("inviter-log", t("inviter_autostop"));
                break;
            }

            let is_main = main_id.as_ref() == Some(&id);
            if is_admin_mode && is_main {
                continue;
            }

            if is_admin_mode && !is_main {
                if let Some(ref mid) = main_id {
                    if !admin_promised {
                        if let Err(e) = try_promote_single_inviter(
                            mid, &id, &user_ids, &config, &targets, &app, &token,
                        )
                        .await
                        {
                            let _ = app.emit(
                                "inviter-log",
                                format!(
                                    "[{}/{}] {}",
                                    i + 1,
                                    total,
                                    t_with("inviter_admin_grant_error", &[("error", &e)])
                                ),
                            );
                            continue;
                        }
                    }
                }
            }

            // Round-robin group assignment
            let target_idx = group_idx.fetch_add(1, Ordering::Relaxed) % targets.len();
            let current_target = targets[target_idx].clone();

            let sem = sem.clone();
            let config = config.clone();
            let usernames_arc = usernames_arc.clone();
            let username_idx = username_idx.clone();
            let users_db_path = users_db_path.clone();
            let stats_db_path = stats_db_path.clone();
            let autostop = autostop.clone();
            let app_clone = app.clone();
            let token_clone = token.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                if !token_clone.load(Ordering::Relaxed) {
                    return;
                }
                if autostop.should_stop(&config.autostop) {
                    return;
                }

                let result = process_account(
                    &id,
                    i + 1,
                    total,
                    &current_target,
                    &config,
                    &usernames_arc,
                    &username_idx,
                    &users_db_path,
                    &stats_db_path,
                    &autostop,
                    &app_clone,
                    &token_clone,
                )
                .await;
                if let Err(e) = result {
                    crate::accounts::commands::check_and_mark_dead_session(&e, &id);
                    autostop.record_error(&e);
                    let _ = app_clone.emit(
                        "inviter-log",
                        format!("[{}/{}] {}: {}", i + 1, total, t("error"), e),
                    );
                }
            }));
        }
        for h in handles {
            let _ = h.await;
        }

        if is_admin_mode && config.revoke_admin_after {
            if let Some(ref mid) = main_id {
                let _ = revoke_all_admins_inviter(mid, &promoted, &config, &targets, &app).await;
            }
        }

        // Summary stats from DB
        if let Ok(stats_conn) = db::init_stats_db(&stats_db_path) {
            let done_count: u32 = stats_conn
                .query_row(
                    "SELECT COUNT(*) FROM invites WHERE status = 'done'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let _ = app.emit(
                "inviter-log",
                t_with(
                    "inviter_total_summary",
                    &[("count", &done_count.to_string())],
                ),
            );
        }

        let _ = app.emit("inviter-log", t("done"));
        let queue: tauri::State<'_, TaskQueue> = app.state();
        queue.finish_task(&task_id, true).await;
    });
    Ok(tid)
}

#[tauri::command]
pub async fn inviter_stop(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app.state();
    queue.stop_task(&task_id).await;
    Ok(())
}

async fn process_account(
    id: &str,
    idx: usize,
    total: usize,
    target: &str,
    config: &InviterConfig,
    usernames: &[String],
    username_idx: &AtomicUsize,
    users_db_path: &PathBuf,
    stats_db_path: &PathBuf,
    autostop: &Arc<AutoStopCounters>,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    let prefix = format!("[{}/{}]", idx, total);
    let mut client = connect_account(id).await?;
    run_invites(
        &mut client,
        id,
        &prefix,
        target,
        config,
        usernames,
        username_idx,
        users_db_path,
        stats_db_path,
        autostop,
        app,
        token,
    )
    .await
}

async fn run_invites(
    client: &mut MtpClient,
    account_id: &str,
    prefix: &str,
    target: &str,
    config: &InviterConfig,
    _usernames: &[String],
    _username_idx: &AtomicUsize,
    users_db_path: &PathBuf,
    stats_db_path: &PathBuf,
    autostop: &Arc<AutoStopCounters>,
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    let emit = |msg: String| {
        let _ = app.emit("inviter-log", format!("{} {}", prefix, msg));
    };

    client.set_log_target("inviter-log", app.clone());
    client.set_max_flood_wait(config.max_flood_wait);

    // Resolve target
    let resolved = resolve_channel_link(client, target).await?;
    emit(t_with(
        "inviter_target_resolved",
        &[("id", &resolved.channel_id.to_string()), ("target", target)],
    ));

    // Validate entity type — only groups allowed in normal mode
    if config.mode == "normal" && resolved.is_broadcast {
        return Err(t("inviter_not_a_group"));
    }

    // Open stats DB connection
    let stats_conn = db::init_stats_db(stats_db_path).ok();

    let mut total_invited = 0u32;
    let mut peer_flood_count = 0u32;
    let batch_size = config.batch_size.max(1).min(50) as usize;
    let is_admin_mode = config.mode == "admin";

    // Force mode loop — keeps retrying until target count reached
    loop {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        if autostop.should_stop(&config.autostop) {
            break;
        }

        // Get users to invite for this round
        let users_to_invite =
            if config.source_mode == "usernames" || config.source_mode == "database" {
                // DB-backed: get pending users, resolve them
                resolve_from_db(
                    client,
                    users_db_path,
                    config.max_per_account - total_invited,
                    app,
                    prefix,
                    token,
                )
                .await
            } else if config.source_mode == "phones" {
                // Phone numbers: import contacts then resolve
                resolve_phones(
                    client,
                    &config.phones_path,
                    users_db_path,
                    config.max_per_account - total_invited,
                    app,
                    prefix,
                    token,
                )
                .await
            } else {
                // Contacts mode
                get_filtered_contacts(
                    client,
                    &config.online_filter,
                    config.max_per_account - total_invited,
                )
                .await?
            };

        if users_to_invite.is_empty() {
            emit(t("inviter_no_users"));
            break;
        }

        emit(t_with(
            "inviter_queue_size",
            &[("count", &users_to_invite.len().to_string())],
        ));

        // Process in batches
        let batches: Vec<&[(i64, i64, String)]> = users_to_invite.chunks(batch_size).collect();

        for batch in batches {
            if !token.load(Ordering::Relaxed) {
                break;
            }
            if total_invited >= config.max_per_account {
                break;
            }
            if peer_flood_count >= config.peer_flood_limit {
                emit(t_with(
                    "inviter_peer_flood_limit",
                    &[
                        ("count", &peer_flood_count.to_string()),
                        ("limit", &config.peer_flood_limit.to_string()),
                    ],
                ));
                break;
            }
            if autostop.should_stop(&config.autostop) {
                break;
            }

            // Pre-invite check: filter users already in group
            let mut filtered_batch: Vec<&(i64, i64, String)> = Vec::new();
            for user in batch {
                if config.check_users {
                    let (uid, ah, _) = user;
                    if is_already_in_group(client, *uid, *ah, resolved.channel_id).await {
                        emit(t_with(
                            "inviter_already_in_group",
                            &[("uid", &uid.to_string())],
                        ));
                        // Update DB status
                        if let Ok(db_conn) = db::init_users_db(users_db_path) {
                            db::update_status(
                                &db_conn,
                                *uid,
                                &db::InviteUserStatus::AlreadyInGroup,
                            );
                        }
                        if let Some(ref sc) = stats_conn {
                            db::record_invite(
                                sc,
                                account_id,
                                resolved.channel_id,
                                target,
                                *uid,
                                &user.2,
                                "",
                                "",
                                "already_in_group",
                            );
                        }
                        continue;
                    }
                }
                filtered_batch.push(user);
            }

            if filtered_batch.is_empty() {
                continue;
            }

            if is_admin_mode {
                // Admin mode: invite one by one via editAdmin
                for user in &filtered_batch {
                    if !token.load(Ordering::Relaxed) {
                        break;
                    }
                    if total_invited >= config.max_per_account {
                        break;
                    }

                    let (uid, ah, uname) = user;
                    let admin_rights = tl_gen::serialize_chatAdminRights(
                        false, false, false, false, false, true, false, false, false, false, false,
                        false, false, false, false, false, false, false,
                    );
                    let channel_input =
                        tl_gen::serialize_input_channel(resolved.channel_id, resolved.access_hash);
                    let user_input = tl_gen::serialize_input_user(*uid, *ah);
                    let req = tl_gen::build_channels_editAdmin(
                        &channel_input,
                        &user_input,
                        &admin_rights,
                        None,
                    );

                    match client.invoke(&req).await {
                        Ok(_) => {
                            interruptible_sleep(2000, token).await;
                            let confirmed = if config.verify_after_invite {
                                is_already_in_group(client, *uid, *ah, resolved.channel_id).await
                            } else {
                                true
                            };

                            if confirmed {
                                total_invited += 1;
                                autostop.reset_sequential();
                                emit(t_with(
                                    "inviter_user_added",
                                    &[
                                        ("uid", &uid.to_string()),
                                        ("done", &total_invited.to_string()),
                                        ("max", &config.max_per_account.to_string()),
                                    ],
                                ));
                                if let Ok(db_conn) = db::init_users_db(users_db_path) {
                                    db::update_status(&db_conn, *uid, &db::InviteUserStatus::Done);
                                }
                                if let Some(ref sc) = stats_conn {
                                    db::record_invite(
                                        sc,
                                        account_id,
                                        resolved.channel_id,
                                        target,
                                        *uid,
                                        uname,
                                        "",
                                        "",
                                        "done",
                                    );
                                }
                            } else {
                                emit(t_with(
                                    "inviter_not_confirmed",
                                    &[("uid", &uid.to_string())],
                                ));
                                if let Some(ref sc) = stats_conn {
                                    db::record_invite(
                                        sc,
                                        account_id,
                                        resolved.channel_id,
                                        target,
                                        *uid,
                                        uname,
                                        "",
                                        "",
                                        "not_confirmed",
                                    );
                                }
                            }

                            // Revoke admin immediately
                            let no_rights = tl_gen::serialize_chatAdminRights(
                                false, false, false, false, false, false, false, false, false,
                                false, false, false, false, false, false, false, false, false,
                            );
                            let channel_input2 = tl_gen::serialize_input_channel(
                                resolved.channel_id,
                                resolved.access_hash,
                            );
                            let user_input2 = tl_gen::serialize_input_user(*uid, *ah);
                            let revoke_req = tl_gen::build_channels_editAdmin(
                                &channel_input2,
                                &user_input2,
                                &no_rights,
                                None,
                            );
                            if let Err(e) = client.invoke(&revoke_req).await {
                                emit(t_with(
                                    "inviter_revoke_admin_error",
                                    &[("uid", &uid.to_string()), ("error", &e)],
                                ));
                            }
                        }
                        Err(e) => {
                            handle_invite_error(
                                &e,
                                *uid,
                                uname,
                                account_id,
                                resolved.channel_id,
                                target,
                                &mut peer_flood_count,
                                autostop,
                                users_db_path,
                                &stats_conn,
                                &emit,
                            );
                            if crate::mtproto::is_fatal_session_error(&e) {
                                return Err(e);
                            }
                        }
                    }

                    let delay = compute_delay_jitter(config);
                    delay_online(client, delay, token).await;
                }
            } else {
                // Normal mode: batch InviteToChannel
                let input_users: Vec<Vec<u8>> = filtered_batch
                    .iter()
                    .map(|(uid, ah, _)| tl_gen::serialize_inputUser(*uid, *ah))
                    .collect();
                let input_refs: Vec<&[u8]> = input_users.iter().map(|u| u.as_slice()).collect();
                let channel_input =
                    tl_gen::serialize_inputChannel(resolved.channel_id, resolved.access_hash);
                let req = tl_gen::build_channels_inviteToChannel(&channel_input, &input_refs);

                match client.invoke(&req).await {
                    Ok(_) => {
                        // Verify each user in the batch
                        for user in &filtered_batch {
                            let (uid, ah, uname) = user;
                            let confirmed = if config.verify_after_invite {
                                is_already_in_group(client, *uid, *ah, resolved.channel_id).await
                            } else {
                                true
                            };

                            let status = if confirmed { "done" } else { "not_in_group" };
                            if confirmed {
                                total_invited += 1;
                                autostop.reset_sequential();
                                emit(t_with(
                                    "inviter_user_invited",
                                    &[
                                        ("uid", &uid.to_string()),
                                        ("done", &total_invited.to_string()),
                                        ("max", &config.max_per_account.to_string()),
                                    ],
                                ));
                            } else {
                                emit(t_with(
                                    "inviter_not_confirmed_after",
                                    &[("uid", &uid.to_string())],
                                ));
                            }

                            if let Ok(db_conn) = db::init_users_db(users_db_path) {
                                let s = if confirmed {
                                    db::InviteUserStatus::Done
                                } else {
                                    db::InviteUserStatus::Error("not_confirmed".into())
                                };
                                db::update_status(&db_conn, *uid, &s);
                            }
                            if let Some(ref sc) = stats_conn {
                                db::record_invite(
                                    sc,
                                    account_id,
                                    resolved.channel_id,
                                    target,
                                    *uid,
                                    uname,
                                    "",
                                    "",
                                    status,
                                );
                            }
                        }
                    }
                    Err(e) => {
                        if crate::mtproto::is_fatal_session_error(&e) {
                            return Err(e);
                        }
                        // Mark all batch users with the error
                        for user in &filtered_batch {
                            let (uid, _, uname) = user;
                            handle_invite_error(
                                &e,
                                *uid,
                                uname,
                                account_id,
                                resolved.channel_id,
                                target,
                                &mut peer_flood_count,
                                autostop,
                                users_db_path,
                                &stats_conn,
                                &emit,
                            );
                        }
                    }
                }

                let delay = compute_delay_jitter(config);
                delay_online(client, delay, token).await;
            }
        }

        // Force mode: check if we need to continue
        if !config.force_mode {
            break;
        }
        if total_invited >= config.max_per_account {
            break;
        }
        if peer_flood_count >= config.peer_flood_limit {
            break;
        }
        if autostop.should_stop(&config.autostop) {
            break;
        }

        // Reset taken users back to pending for another round
        if let Ok(db_conn) = db::init_users_db(users_db_path) {
            db::reset_taken_to_pending(&db_conn);
            let remaining_pending = db::count_by_status(&db_conn, "pending");
            if remaining_pending == 0 {
                emit(t("inviter_force_no_users"));
                break;
            }
            let still_needed = config.max_per_account - total_invited;
            emit(t_with(
                "inviter_force_remaining",
                &[
                    ("needed", &still_needed.to_string()),
                    ("pending", &remaining_pending.to_string()),
                ],
            ));
        } else {
            break;
        }
    }

    emit(t_with(
        "inviter_total_invited",
        &[("count", &total_invited.to_string())],
    ));

    // Leave channel after work if configured
    if config.leave_after_work && resolved.joined_now {
        let channel = tl_gen::serialize_input_channel(resolved.channel_id, resolved.access_hash);
        let leave_req = tl_gen::build_channels_leaveChannel(&channel);
        match client.invoke(&leave_req).await {
            Ok(_) => emit(t("inviter_left_channel")),
            Err(e) => emit(t_with("inviter_leave_error", &[("error", &e)])),
        }
    }

    if let Some(fatal) = client.fatal_error() {
        return Err(fatal.to_string());
    }
    Ok(())
}

/// Check if user is already in the target group via GetCommonChats
async fn is_already_in_group(
    client: &mut MtpClient,
    user_id: i64,
    access_hash: i64,
    channel_id: i64,
) -> bool {
    let user_input = tl_gen::serialize_input_user(user_id, access_hash);
    let common_req = tl_gen::build_messages_getCommonChats(&user_input, 0, 100);
    match client.invoke(&common_req).await {
        Ok(data) => {
            if let Ok(chats) = tl_gen::parse_messages_getCommonChats(&data) {
                let chat_list = match &chats {
                    tl_gen::TlMessagesChats::Chats { chats } => chats,
                    tl_gen::TlMessagesChats::Slice { chats, .. } => chats,
                };
                return chat_list.iter().any(|raw| {
                    if let Ok(chat) =
                        tl_gen::TlChat::deserialize(&mut std::io::Cursor::new(raw.as_slice()))
                    {
                        match chat {
                            tl_gen::TlChat::Channel { id, .. } => id == channel_id,
                            tl_gen::TlChat::Chat { id, .. } => id == channel_id,
                            _ => false,
                        }
                    } else {
                        false
                    }
                });
            }
            false
        }
        Err(_) => false,
    }
}

/// Handle invite error: update counters, DB, emit log
fn handle_invite_error(
    error: &str,
    user_id: i64,
    username: &str,
    account_id: &str,
    channel_id: i64,
    target: &str,
    peer_flood_count: &mut u32,
    autostop: &Arc<AutoStopCounters>,
    users_db_path: &PathBuf,
    stats_conn: &Option<Connection>,
    emit: &dyn Fn(String),
) {
    autostop.record_error(error);

    let db_status;
    let stat_status;

    if error.contains("PEER_FLOOD") {
        *peer_flood_count += 1;
        db_status = db::InviteUserStatus::PeerFlood;
        stat_status = "peer_flood";
        emit(t_with(
            "inviter_peer_flood_user",
            &[
                ("uid", &user_id.to_string()),
                ("count", &peer_flood_count.to_string()),
            ],
        ));
    } else if error.contains("USER_PRIVACY") || error.contains("USER_NOT_MUTUAL") {
        db_status = db::InviteUserStatus::Privacy;
        stat_status = "privacy";
        emit(t_with(
            "inviter_skip_user",
            &[("uid", &user_id.to_string()), ("error", error)],
        ));
    } else if error.contains("USER_CHANNELS_TOO_MUCH") {
        db_status = db::InviteUserStatus::Error("channels_too_much".into());
        stat_status = "channels_too_much";
        emit(t_with(
            "inviter_too_many_channels",
            &[("uid", &user_id.to_string())],
        ));
    } else {
        db_status = db::InviteUserStatus::Error(error.to_string());
        stat_status = "error";
        emit(t_with(
            "inviter_user_error",
            &[("uid", &user_id.to_string()), ("error", error)],
        ));
    }

    if let Ok(db_conn) = db::init_users_db(users_db_path) {
        db::update_status(&db_conn, user_id, &db_status);
    }
    if let Some(ref sc) = stats_conn {
        db::record_invite(
            sc,
            account_id,
            channel_id,
            target,
            user_id,
            username,
            "",
            "",
            stat_status,
        );
    }
}

/// Resolve usernames from DB (take pending, resolve via API, update DB)
async fn resolve_from_db(
    client: &mut MtpClient,
    users_db_path: &PathBuf,
    max: u32,
    app: &tauri::AppHandle,
    prefix: &str,
    token: &Arc<AtomicBool>,
) -> Vec<(i64, i64, String)> {
    let db_conn = match db::init_users_db(users_db_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let pending = db::get_pending_users(&db_conn, max as usize);
    let mut out = Vec::new();

    for user in pending {
        if !token.load(Ordering::Relaxed) {
            break;
        }

        // Mark as taken
        db::mark_taken(&db_conn, user.user_id);

        // If user_id is negative (temp), need to resolve username
        if user.user_id < 0 && !user.username.is_empty() {
            let req = tl::build_resolve_username(&user.username);
            match client.invoke(&req).await {
                Ok(data) => {
                    if let Ok((id, hash)) = tl::parse_resolved_peer(&data) {
                        db::update_resolved(&db_conn, user.user_id, id, hash, "", "");
                        db::mark_taken(&db_conn, id);
                        out.push((id, hash, user.username.clone()));
                    } else {
                        db::update_status(&db_conn, user.user_id, &db::InviteUserStatus::NotUser);
                        let _ = app.emit(
                            "inviter-log",
                            format!(
                                "{} {}",
                                prefix,
                                t_with("inviter_parse_error", &[("username", &user.username)])
                            ),
                        );
                    }
                }
                Err(e) => {
                    if e.contains("FLOOD") {
                        db::update_status(&db_conn, user.user_id, &db::InviteUserStatus::FloodWait);
                    } else {
                        db::update_status(&db_conn, user.user_id, &db::InviteUserStatus::NotUser);
                    }
                    let _ = app.emit(
                        "inviter-log",
                        format!(
                            "{} {}",
                            prefix,
                            t_with(
                                "inviter_resolve_error",
                                &[("username", &user.username), ("error", &e)]
                            )
                        ),
                    );
                }
            }
            interruptible_sleep(300, token).await;
        } else if user.user_id > 0 && user.access_hash != 0 {
            out.push((user.user_id, user.access_hash, user.username.clone()));
        }
    }
    out
}

/// Resolve phone numbers via contacts.importContacts, then return user_id + access_hash
async fn resolve_phones(
    client: &mut MtpClient,
    _phones_path: &str,
    users_db_path: &PathBuf,
    max: u32,
    app: &tauri::AppHandle,
    prefix: &str,
    token: &Arc<AtomicBool>,
) -> Vec<(i64, i64, String)> {
    let db_conn = match db::init_users_db(users_db_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let pending = db::get_pending_phones(&db_conn, max as usize);
    if pending.is_empty() {
        // fallback: also try regular pending (already resolved)
        return resolve_from_db(client, users_db_path, max, app, prefix, token).await;
    }

    let mut out = Vec::new();

    // Import phones in batches of 100 via contacts.importContacts
    let batch_size = 100;
    for chunk in pending.chunks(batch_size) {
        if !token.load(Ordering::Relaxed) {
            break;
        }

        // Mark as taken
        for user in chunk {
            db::mark_taken(&db_conn, user.user_id);
        }

        // Build importContacts request
        let contacts: Vec<Vec<u8>> = chunk
            .iter()
            .enumerate()
            .map(|(_i, u)| {
                let phone = &u.username; // phone stored in username field
                tl_gen::serialize_inputPhoneContact(
                    (u.user_id.abs()) as i64, // client_id
                    phone,
                    "", // first_name (empty, just importing)
                    "", // last_name
                    None,
                )
            })
            .collect();

        let contact_refs: Vec<&[u8]> = contacts.iter().map(|c| c.as_slice()).collect();
        let req = tl_gen::build_contacts_importContacts(&contact_refs);

        match client.invoke(&req).await {
            Ok(data) => {
                if let Ok(result) = tl_gen::parse_contacts_importContacts(&data) {
                    // Parse users to get user_id + access_hash
                    for raw in &result.users {
                        if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
                            if let tl_gen::TlUser::User {
                                id,
                                access_hash,
                                phone,
                                ..
                            } = user
                            {
                                let ah = access_hash.unwrap_or(0);
                                let phone_str = phone.unwrap_or_default();
                                // Find matching pending entry and update DB
                                for pending_user in chunk {
                                    if pending_user.username == phone_str
                                        || pending_user.username.ends_with(&phone_str)
                                        || phone_str.ends_with(&pending_user.username)
                                    {
                                        db::update_resolved(
                                            &db_conn,
                                            pending_user.user_id,
                                            id,
                                            ah,
                                            "",
                                            "",
                                        );
                                        db::mark_taken(&db_conn, id);
                                        out.push((id, ah, phone_str.clone()));
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    // Mark phones that weren't resolved as NotFound
                    let resolved_phones: std::collections::HashSet<String> =
                        out.iter().map(|(_, _, p)| p.clone()).collect();
                    for pending_user in chunk {
                        if !resolved_phones.contains(&pending_user.username) {
                            // Check if this phone was already resolved (might match partially)
                            let found = out.iter().any(|(_, _, p)| {
                                p == &pending_user.username
                                    || p.ends_with(&pending_user.username)
                                    || pending_user.username.ends_with(p.as_str())
                            });
                            if !found {
                                db::update_status(
                                    &db_conn,
                                    pending_user.user_id,
                                    &db::InviteUserStatus::NotUser,
                                );
                            }
                        }
                    }

                    let _ = app.emit(
                        "inviter-log",
                        format!(
                            "{} {}",
                            prefix,
                            t_with(
                                "inviter_import_contacts",
                                &[
                                    ("found", &result.users.len().to_string()),
                                    ("total", &chunk.len().to_string())
                                ]
                            )
                        ),
                    );
                }
            }
            Err(e) => {
                let _ = app.emit(
                    "inviter-log",
                    format!(
                        "{} {}",
                        prefix,
                        t_with("inviter_import_contacts_error", &[("error", &e)])
                    ),
                );
                // Mark all as flood_wait if flood error
                for pending_user in chunk {
                    if e.contains("FLOOD") {
                        db::update_status(
                            &db_conn,
                            pending_user.user_id,
                            &db::InviteUserStatus::FloodWait,
                        );
                    }
                }
            }
        }

        interruptible_sleep(500, token).await;

        if out.len() >= max as usize {
            break;
        }
    }

    // Delete imported contacts to clean up (optional, reduces footprint)
    // We skip this for now — Telegram auto-cleans imported contacts

    out.truncate(max as usize);
    out
}

async fn get_filtered_contacts(
    client: &mut MtpClient,
    filter: &str,
    max: u32,
) -> Result<Vec<(i64, i64, String)>, String> {
    let req = tl::build_contacts_get_contacts();
    let data = client
        .invoke(&req)
        .await
        .map_err(|e| format!("getContacts: {e}"))?;
    let contacts = tl::parse_contacts_response_with_status(&data).unwrap_or_default();

    let filtered: Vec<(i64, i64, String)> = if filter == "any" || filter.is_empty() {
        contacts
            .into_iter()
            .map(|(id, ah, _)| (id, ah, String::new()))
            .collect()
    } else {
        contacts
            .into_iter()
            .filter(|(_, _, bucket)| match filter {
                "recent" => *bucket == OnlineBucket::Recent,
                "week" => matches!(*bucket, OnlineBucket::Recent | OnlineBucket::Week),
                "month" => matches!(
                    *bucket,
                    OnlineBucket::Recent | OnlineBucket::Week | OnlineBucket::Month
                ),
                "long" => *bucket == OnlineBucket::Long,
                _ => true,
            })
            .map(|(id, ah, _)| (id, ah, String::new()))
            .collect()
    };
    Ok(filtered.into_iter().take(max as usize).collect())
}

fn compute_delay_jitter(config: &InviterConfig) -> u64 {
    let min = config.delay_min as u64;
    let max = config.delay_max.max(config.delay_min) as u64;
    let base = if min == max {
        min
    } else {
        min + (rand::random::<u64>() % (max - min + 1))
    };
    let multiplier = match config.delay_unit.as_str() {
        "minutes" => 60_000,
        _ => 1_000,
    };
    let ms = base * multiplier;
    let jitter = (ms as f64 * 0.2) as u64;
    if jitter == 0 {
        return ms;
    }
    let offset = rand::random::<u64>() % (jitter * 2 + 1);
    ms.saturating_sub(jitter) + offset
}

async fn get_account_user_id(id: &str) -> Result<i64, String> {
    let storage = crate::accounts::commands::get_storage_pub();
    let json_path = storage.json_path(id);
    let json = if json_path.exists() {
        crate::accounts::session::AccountJson::from_file(&json_path).unwrap_or_default()
    } else {
        crate::accounts::session::AccountJson::default()
    };
    if json.user_id > 0 {
        return Ok(json.user_id);
    }
    let mut client = connect_account(id).await?;
    let cfg = crate::get_app_config();
    let app_id = if json.app_id == 0 {
        cfg.app_id
    } else {
        json.app_id
    };
    let dev = crate::accounts::devices::generate_random_device();
    let get_me = crate::mtproto::tl::build_get_me_request(
        app_id,
        &dev.device,
        &dev.sdk,
        &dev.app_version,
        "en",
        "en",
    );
    let resp = client
        .invoke(&get_me)
        .await
        .map_err(|e| format!("get_me: {e}"))?;
    let info =
        crate::mtproto::tl::parse_users_response(&resp).map_err(|e| format!("parse me: {e}"))?;
    Ok(info.id)
}

async fn setup_main_account_inviter(
    main_id: &str,
    worker_ids: &[String],
    user_ids: &std::collections::HashMap<String, i64>,
    config: &InviterConfig,
    targets: &[String],
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<Vec<i64>, String> {
    let emit = |msg: String| {
        let _ = app.emit(
            "inviter-log",
            format!("{} {}", t("inviter_main_prefix"), msg),
        );
    };

    let mut client = connect_account(main_id).await?;
    client.set_max_flood_wait(config.max_flood_wait);

    // Promote on first target (main admin resolves all targets for simplicity)
    let target = targets.first().ok_or(t("inviter_no_target_groups_err"))?;
    let dest = resolve_channel_link(&mut client, target)
        .await
        .map_err(|e| format!("dest: {e}"))?;
    emit(t_with(
        "inviter_target_channel_info",
        &[
            ("id", &dest.channel_id.to_string()),
            ("title", &dest.title_hint),
        ],
    ));

    let admin_rights = tl_gen::serialize_chatAdminRights(
        false, false, false, false, false, true, false, false, false, false, false, false, false,
        false, false, false, false, false,
    );
    let mut promoted: Vec<i64> = Vec::new();

    for wid in worker_ids {
        if wid == main_id {
            continue;
        }
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let user_id = match user_ids.get(wid) {
            Some(&uid) => uid,
            None => {
                emit(t_with("inviter_unknown_uid_skip", &[("id", wid)]));
                continue;
            }
        };
        let channel_input = tl_gen::serialize_input_channel(dest.channel_id, dest.access_hash);
        let user_input = tl_gen::serialize_input_user(user_id, 0);
        let req =
            tl_gen::build_channels_editAdmin(&channel_input, &user_input, &admin_rights, None);
        match client.invoke(&req).await {
            Ok(_) => {
                emit(t_with(
                    "inviter_admin_granted_msg",
                    &[("uid", &user_id.to_string())],
                ));
                promoted.push(user_id);
            }
            Err(e) => emit(t_with(
                "inviter_admin_error_msg",
                &[("uid", &user_id.to_string()), ("error", &e)],
            )),
        }
    }

    Ok(promoted)
}

async fn try_promote_single_inviter(
    main_id: &str,
    worker_id: &str,
    user_ids: &std::collections::HashMap<String, i64>,
    config: &InviterConfig,
    targets: &[String],
    app: &tauri::AppHandle,
    token: &Arc<AtomicBool>,
) -> Result<(), String> {
    if !token.load(Ordering::Relaxed) {
        return Ok(());
    }
    let user_id = user_ids
        .get(worker_id)
        .copied()
        .ok_or(t("inviter_no_uid_err"))?;
    let emit = |msg: String| {
        let _ = app.emit(
            "inviter-log",
            format!("{} {}", t("inviter_main_prefix"), msg),
        );
    };

    let mut client = connect_account(main_id).await?;
    client.set_max_flood_wait(config.max_flood_wait);

    let target = targets.first().ok_or(t("inviter_no_target_groups_err"))?;
    let dest = resolve_channel_link(&mut client, target)
        .await
        .map_err(|e| format!("dest: {e}"))?;
    let admin_rights = tl_gen::serialize_chatAdminRights(
        false, false, false, false, false, true, false, false, false, false, false, false, false,
        false, false, false, false, false,
    );
    let channel_input = tl_gen::serialize_input_channel(dest.channel_id, dest.access_hash);
    let user_input = tl_gen::serialize_input_user(user_id, 0);
    let req = tl_gen::build_channels_editAdmin(&channel_input, &user_input, &admin_rights, None);
    match client.invoke(&req).await {
        Ok(_) => {
            emit(t_with(
                "inviter_admin_granted_msg",
                &[("uid", &user_id.to_string())],
            ));
            Ok(())
        }
        Err(e) => {
            emit(t_with(
                "inviter_admin_error_msg",
                &[("uid", &user_id.to_string()), ("error", &e)],
            ));
            Err(e)
        }
    }
}

async fn revoke_all_admins_inviter(
    main_id: &str,
    promoted: &[i64],
    _config: &InviterConfig,
    targets: &[String],
    app: &tauri::AppHandle,
) -> Result<(), String> {
    let emit = |msg: String| {
        let _ = app.emit(
            "inviter-log",
            format!("{} {}", t("inviter_main_prefix"), msg),
        );
    };

    if promoted.is_empty() {
        emit(t("inviter_revoke_nobody_msg"));
        return Ok(());
    }

    let main_user_id = match get_account_user_id(main_id).await {
        Ok(uid) => uid,
        Err(e) => {
            emit(t_with("inviter_main_uid_error_msg", &[("error", &e)]));
            return Ok(());
        }
    };

    let Ok(mut client) = connect_account(main_id).await else {
        return Ok(());
    };
    let target = targets.first().ok_or(t("inviter_no_target_groups_err"))?;
    let Ok(dest) = resolve_channel_link(&mut client, target).await else {
        return Ok(());
    };
    let no_rights = tl_gen::serialize_chatAdminRights(
        false, false, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false, false, false,
    );

    emit(t("inviter_revoking_msg"));

    let promoted_set: std::collections::HashSet<i64> = promoted.iter().copied().collect();
    let mut revoked = 0u32;
    for user_id in &promoted_set {
        if *user_id == main_user_id {
            continue;
        }
        let channel_input = tl_gen::serialize_input_channel(dest.channel_id, dest.access_hash);
        let user_input = tl_gen::serialize_input_user(*user_id, 0);
        let req = tl_gen::build_channels_editAdmin(&channel_input, &user_input, &no_rights, None);
        match client.invoke(&req).await {
            Ok(_) => revoked += 1,
            Err(e) => emit(t_with(
                "inviter_revoke_error_msg",
                &[("uid", &user_id.to_string()), ("error", &e)],
            )),
        }
    }

    emit(t_with(
        "inviter_revoked_msg",
        &[
            ("done", &revoked.to_string()),
            ("total", &promoted_set.len().saturating_sub(1).to_string()),
        ],
    ));
    Ok(())
}
