// tauri command wrappers for checker functionality

use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

use super::runner::{self, CheckerOptions};
use crate::queue::TaskQueue;
use crate::accounts::commands::copy_dir_recursive;
use crate::i18n::t_with;

#[tauri::command]
pub fn checker_scan_folder(path: String) -> u32 {
    dbg_log!("checker_scan_folder: {:?}", path);
    let root = Path::new(&path);
    if !root.exists() || !root.is_dir() {
        return 0;
    }
    let mut paths = Vec::new();
    runner::collect_tdata_paths(root, &mut paths);
    paths.len() as u32
}

#[tauri::command]
pub async fn checker_start(folders: Vec<String>, options: CheckerOptions, app_handle: AppHandle) -> Vec<String> {
    dbg_log!("checker_start: {} folders", folders.len());

    let total = {
        let mut paths = Vec::new();
        for folder in &folders {
            runner::collect_tdata_paths(Path::new(folder), &mut paths);
        }
        paths.len()
    };

    let queue: tauri::State<'_, TaskQueue> = app_handle.state();
    let task_id = uuid::Uuid::new_v4().to_string();
    let app_clone = app_handle.clone();

    queue.enqueue(
        task_id.clone(),
        "checker".to_string(),
        t_with("checker_task_name", &[("count", &total.to_string())]),
        move || {
            let app = app_clone;
            Box::pin(async move {
                runner::run_checker(folders, options, app).await;
                Ok(())
            })
        },
    );

    vec![]
}

#[derive(serde::Deserialize)]
pub struct CheckerAccountForSort {
    source_path: String,
    spamblock: String,
    nft_tags: Vec<String>,
    phone888: bool,
    channels: Vec<serde_json::Value>,
    nft_gifts: Vec<String>,
    premium: bool,
    seed_found: bool,
    pass_files: Vec<String>,
    #[serde(default)]
    channel_balances: Vec<serde_json::Value>,
    #[serde(default)]
    id: i64,
}

#[tauri::command]
pub async fn checker_sort_results(accounts: Vec<serde_json::Value>, dest_path: String) -> Result<(), String> {
    let dest = Path::new(&dest_path);
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let canonical_dest = std::fs::canonicalize(dest).map_err(|e| e.to_string())?;

    let categories: Vec<(&str, Box<dyn Fn(&CheckerAccountForSort) -> bool + Send + Sync>)> = vec![
        ("With_NFT_Tags", Box::new(|a: &CheckerAccountForSort| !a.nft_tags.is_empty())),
        ("With_888", Box::new(|a: &CheckerAccountForSort| a.phone888)),
        ("With_Channels", Box::new(|a: &CheckerAccountForSort| !a.channels.is_empty())),
        ("With_NFT_Gifts", Box::new(|a: &CheckerAccountForSort| !a.nft_gifts.is_empty())),
        ("Premium", Box::new(|a: &CheckerAccountForSort| a.premium)),
        ("With_Seed", Box::new(|a: &CheckerAccountForSort| a.seed_found)),
        ("With_Pass_Files", Box::new(|a: &CheckerAccountForSort| !a.pass_files.is_empty())),
        ("With_Channel_Balances", Box::new(|a: &CheckerAccountForSort| !a.channel_balances.is_empty())),
        ("Temp_Spamblock", Box::new(|a: &CheckerAccountForSort| a.spamblock == "temp_geo")),
        ("Perm_Spamblock", Box::new(|a: &CheckerAccountForSort| a.spamblock == "perm")),
        ("Frozen", Box::new(|a: &CheckerAccountForSort| a.spamblock == "frozen")),
        // Valid_No_Restrictions: only clean accounts that don't have any special attributes
        ("Valid_No_Restrictions", Box::new(|a: &CheckerAccountForSort| {
            a.spamblock == "none"
                && a.nft_tags.is_empty()
                && !a.phone888
                && a.channels.is_empty()
                && a.nft_gifts.is_empty()
                && !a.premium
                && !a.seed_found
                && a.pass_files.is_empty()
                && a.channel_balances.is_empty()
        })),
    ];

    let parsed: Vec<CheckerAccountForSort> = accounts.iter().filter_map(|v| {
        serde_json::from_value(v.clone()).ok()
    }).collect();

    for account in &parsed {
        let source = PathBuf::from(&account.source_path);
        if source.exists() {
            let canonical_source = std::fs::canonicalize(&source).map_err(|e| e.to_string())?;
            if canonical_dest.starts_with(&canonical_source) {
                return Err("sort destination must not be inside a source account directory".into());
            }
        }
    }

    for (folder_name, predicate) in &categories {
        let matching: Vec<&CheckerAccountForSort> = parsed.iter().filter(|a| predicate(a)).collect();
        if matching.is_empty() { continue; }

        let cat_dir = dest.join(folder_name);
        let sessions_dir = cat_dir.join("sessions");
        let tdatas_dir = cat_dir.join("tdatas");
        std::fs::create_dir_all(&sessions_dir).ok();
        std::fs::create_dir_all(&tdatas_dir).ok();

        for acc in matching {
            let src = Path::new(&acc.source_path);
            if !src.exists() { continue; }

            let is_session = src.extension().map(|e| e == "session").unwrap_or(false);

            if is_session {
                // Copy session to sessions/
                let file_name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
                let _ = std::fs::copy(src, sessions_dir.join(&file_name));

                // Convert session → tdata in tdatas/
                let session_result = crate::accounts::session::TelethonSession::from_file(src)
                    .map(|s| (s.auth_key, s.dc_id));
                if let Ok((auth_key, dc_id)) = session_result {
                    let stem = src.file_stem().unwrap_or_default().to_string_lossy().to_string();
                    let user_id = acc.id;
                    if user_id != 0 {
                        let tdata_acc = crate::converter::tdata::TDataAccount {
                            dc_id,
                            user_id,
                            auth_key,
                        };
                        let tdata_out = tdatas_dir.join(format!("{}_tdata", stem.trim_start_matches('+')));
                        let _ = crate::converter::tdata::write_tdata(&tdata_out, &tdata_acc);
                    }
                }
            } else {
                // src is a tdata directory — copy to tdatas/
                // figure out a name: if src ends with "tdata", use parent folder name
                let tdata_name = if src.file_name().map(|n| n == "tdata").unwrap_or(false) {
                    // copy the parent (which contains tdata/)
                    let parent = src.parent().unwrap_or(src);
                    let name = parent.file_name().unwrap_or_default().to_string_lossy().to_string();
                    let dest_tdata = tdatas_dir.join(&name);
                    let _ = copy_dir_recursive(parent, &dest_tdata);
                    name
                } else {
                    let name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
                    let dest_tdata = tdatas_dir.join(&name);
                    let _ = copy_dir_recursive(src, &dest_tdata);
                    name
                };

                // Convert tdata → session in sessions/
                if let Ok(tdata_accs) = crate::converter::tdata::parse_tdata(src) {
                    for tdata_acc in &tdata_accs {
                        let session_name = if tdata_accs.len() == 1 {
                            format!("{}.session", tdata_name)
                        } else {
                            format!("{}_{}.session", tdata_name, tdata_acc.user_id)
                        };
                        let session = crate::accounts::session::TelethonSession {
                            dc_id: tdata_acc.dc_id,
                            server_address: crate::converter::dc_host_from_config(tdata_acc.dc_id),
                            port: 443,
                            auth_key: tdata_acc.auth_key.clone(),
                        };
                        let _ = session.to_file(&sessions_dir.join(&session_name));
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("explorer").arg(dest).spawn(); }

    Ok(())
}
