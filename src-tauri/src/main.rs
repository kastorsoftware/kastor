// prevents console window on windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod debug;
mod accounts;
mod audio;
mod auto_reply;
mod boost;
mod bot_parser;
mod botcreator;
mod channelcreator;
mod checker;
mod cloner;
mod converter;
mod first_comment;
mod forwarder;
mod global_search;
pub mod i18n;
mod interceptor;
mod inviter;
mod link_checker;
mod llm;
mod mailing;
mod masslooking;
mod mtproto;
mod parser;
mod proxy;
mod queue;
mod randomizer;
mod reporter;
mod settings;
mod stories;
mod username_checker;
mod warmer;

use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::Manager;

use crate::i18n::t;
use queue::TaskQueue;

static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

pub fn get_app_handle() -> Option<&'static tauri::AppHandle> {
    APP_HANDLE.get()
}

#[derive(Clone)]
pub struct AppConfig {
    pub app_id: i32,
    pub app_hash: String,
    pub dc_addresses: HashMap<i32, String>,
    pub dead_session_markers: Vec<String>,
    pub frozen_session_markers: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut dc_addresses = HashMap::new();
        dc_addresses.insert(1, "149.154.175.53:443".to_string());
        dc_addresses.insert(2, "149.154.167.51:443".to_string());
        dc_addresses.insert(3, "149.154.175.100:443".to_string());
        dc_addresses.insert(4, "149.154.167.91:443".to_string());
        dc_addresses.insert(5, "91.108.56.130:443".to_string());
        Self {
            app_id: 2040,
            app_hash: "b18441a1ff607e10a989891a5462e627".to_string(),
            dc_addresses,
            dead_session_markers: vec![
                "AUTH_KEY_UNREGISTERED".to_string(),
                "SESSION_REVOKED".to_string(),
                "USER_DEACTIVATED".to_string(),
                "AUTH_KEY_DUPLICATED".to_string(),
                "SESSION_EXPIRED".to_string(),
                "PHONE_NUMBER_BANNED".to_string(),
            ],
            frozen_session_markers: vec![
                "USER_DEACTIVATED_BAN".to_string(),
                "FROZEN_METHOD_INVALID".to_string(),
            ],
        }
    }
}

static APP_CONFIG: OnceLock<Arc<Mutex<AppConfig>>> = OnceLock::new();

fn get_app_config_arc() -> &'static Arc<Mutex<AppConfig>> {
    APP_CONFIG.get_or_init(|| Arc::new(Mutex::new(AppConfig::default())))
}

/// Returns a snapshot of the current AppConfig.
/// Used throughout the codebase wherever config fields are needed.
pub fn get_app_config() -> AppConfig {
    get_app_config_arc().lock().unwrap().clone()
}

pub fn update_app_config(app_id: i32, app_hash: String, dc_addresses: HashMap<i32, String>) {
    let config = get_app_config_arc();
    let mut c = config.lock().unwrap();
    c.app_id = app_id;
    c.app_hash = app_hash;
    if !dc_addresses.is_empty() {
        c.dc_addresses = dc_addresses;
    }
    dbg_log!(
        "app_config updated: app_id={}, dc_count={}",
        c.app_id,
        c.dc_addresses.len()
    );
}

pub fn check_and_mark_dead_session(error: &str, id: &str) -> bool {
    let config = get_app_config();
    let storage = accounts::commands::get_storage_pub();
    let json_path = storage.json_path(id);

    for marker in &config.frozen_session_markers {
        if error.contains(marker) {
            if let Ok(mut json) = accounts::session::AccountJson::from_file(&json_path) {
                json.status = crate::i18n::t("status_frozen");
                let _ = json.to_file(&json_path);
                accounts::commands::invalidate_accounts_cache();
            }
            return true;
        }
    }
    for marker in &config.dead_session_markers {
        if error.contains(marker) {
            if let Ok(mut json) = accounts::session::AccountJson::from_file(&json_path) {
                json.status = crate::i18n::t("status_invalid");
                let _ = json.to_file(&json_path);
                accounts::commands::invalidate_accounts_cache();
            }
            return true;
        }
    }
    false
}

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize, Clone)]
pub struct DashboardStats {
    accounts: u32,
    proxies: u32,
}

#[derive(Serialize, Clone)]
pub struct QuickAction {
    id: String,
    label: String,
    icon: String,
}

pub struct AppState {
    pub validating_ids: Vec<String>,
}

#[tauri::command]
fn get_version() -> String {
    dbg_log!("get_version -> {}", APP_VERSION);
    APP_VERSION.to_string()
}

#[tauri::command]
async fn check_telegram_connectivity() -> bool {
    use std::time::Duration;
    use tokio::net::TcpStream;

    let targets: &[(&str, u16)] = &[
        // Telegram DC IPs (DC1-DC5)
        ("149.154.175.53", 443),
        ("149.154.167.51", 443),
        ("149.154.175.100", 443),
        ("149.154.167.91", 443),
        ("91.108.56.130", 443),
        // telegram.org & t.me resolve
        ("telegram.org", 443),
        ("t.me", 443),
    ];

    let timeout = Duration::from_secs(15);
    let mut ok = 0u32;
    let mut total = 0u32;

    let mut handles = Vec::new();
    for &(host, port) in targets {
        let addr = format!("{}:{}", host, port);
        handles.push(tokio::spawn(async move {
            matches!(
                tokio::time::timeout(timeout, TcpStream::connect(&addr)).await,
                Ok(Ok(_))
            )
        }));
    }

    for h in handles {
        total += 1;
        if let Ok(result) = h.await {
            if result {
                ok += 1;
            }
        }
    }

    dbg_log!("check_telegram_connectivity: {}/{} reachable", ok, total);
    ok > 0
}

#[tauri::command]
fn open_url(url: String) {
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", &url])
        .spawn();
}

#[derive(Serialize)]
struct InstalledTools {
    beekeeper: bool,
    notepadpp: bool,
}

#[tauri::command]
fn check_installed_tools() -> InstalledTools {
    let beekeeper = is_installed_beekeeper();
    let notepadpp = is_installed_notepadpp();
    dbg_log!(
        "check_installed_tools: beekeeper={}, notepadpp={}",
        beekeeper,
        notepadpp
    );
    InstalledTools {
        beekeeper,
        notepadpp,
    }
}

fn is_installed_beekeeper() -> bool {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let p = std::path::PathBuf::from(local)
            .join("Programs")
            .join("beekeeper-studio")
            .join("Beekeeper Studio.exe");
        if p.exists() {
            return true;
        }
    }
    if let Some(pf) = std::env::var_os("PROGRAMFILES") {
        let p = std::path::PathBuf::from(pf)
            .join("Beekeeper Studio")
            .join("Beekeeper Studio.exe");
        if p.exists() {
            return true;
        }
    }
    false
}

fn is_installed_notepadpp() -> bool {
    if let Some(pf) = std::env::var_os("PROGRAMFILES") {
        let p = std::path::PathBuf::from(pf)
            .join("Notepad++")
            .join("notepad++.exe");
        if p.exists() {
            return true;
        }
    }
    if let Some(pf) = std::env::var_os("PROGRAMFILES(X86)") {
        let p = std::path::PathBuf::from(pf)
            .join("Notepad++")
            .join("notepad++.exe");
        if p.exists() {
            return true;
        }
    }
    false
}

#[tauri::command]
fn get_stats() -> DashboardStats {
    dbg_log!("get_stats called");
    let storage = accounts::commands::get_storage_pub();
    let accounts = std::fs::read_dir(storage.session_json_dir())
        .map(|e| {
            e.flatten()
                .filter(|f| {
                    f.path()
                        .extension()
                        .map(|x| x == "session")
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0) as u32;
    let proxies = proxy::ProxyList::load().proxies.len() as u32;

    dbg_log!("get_stats accounts={} proxies={}", accounts, proxies);
    DashboardStats { accounts, proxies }
}

#[tauri::command]
fn get_quick_actions() -> Vec<QuickAction> {
    vec![
        QuickAction {
            id: "mailing".into(),
            label: t("quick_mailing"),
            icon: "Send".into(),
        },
        QuickAction {
            id: "checker".into(),
            label: t("quick_checker"),
            icon: "ShieldCheck".into(),
        },
        QuickAction {
            id: "inviter".into(),
            label: t("quick_inviter"),
            icon: "UserPlus".into(),
        },
        QuickAction {
            id: "parser".into(),
            label: t("quick_parser"),
            icon: "Database".into(),
        },
    ]
}

#[tauri::command]
fn show_window(window: tauri::Window) {
    dbg_log!("show_window called");
    if let Some(w) = window.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[tauri::command]
async fn get_task_queue(
    queue: tauri::State<'_, TaskQueue>,
) -> Result<Vec<queue::TaskInfo>, String> {
    Ok(queue.get_tasks().await)
}

#[tauri::command]
async fn get_queue_stats(queue: tauri::State<'_, TaskQueue>) -> Result<(u32, u32), String> {
    let queued = queue.queue_size().await;
    let running = queue.running_count().await;
    Ok((queued, running))
}

#[tauri::command]
fn get_validating_ids(state: tauri::State<'_, Arc<Mutex<AppState>>>) -> Vec<String> {
    let ids = state.lock().unwrap().validating_ids.clone();
    dbg_log!("get_validating_ids -> {} ids", ids.len());
    ids
}

fn main() {
    dbg_log!("=== Kastor v{} starting (DEBUG BUILD) ===", APP_VERSION);

    let state = Arc::new(Mutex::new(AppState {
        validating_ids: Vec::new(),
    }));

    let task_queue = TaskQueue::new(5);

    // startup dedup: remove sessions with duplicate auth_keys
    let removed = accounts::commands::dedup_sessions_by_auth_key();
    if removed > 0 {
        dbg_log!(
            "startup: removed {} duplicate sessions (by auth_key)",
            removed
        );
    }

    dbg_log!("main: initializing tauri...");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(state)
        .manage(task_queue)
        .manage(accounts::auth_login::AuthSessions::new())
        .invoke_handler(tauri::generate_handler![
            i18n::set_locale,
            get_version,
            open_url,
            check_telegram_connectivity,
            check_installed_tools,
            get_stats,
            get_quick_actions,
            show_window,
            accounts::commands::open_accounts_folder,
            accounts::commands::open_file_in_editor,
            accounts::commands::launch_telegram,
            get_task_queue,
            get_queue_stats,
            get_validating_ids,
            settings::get_settings,
            settings::save_settings,
            settings::patch_settings,
            queue::validate::enqueue_validate,
            accounts::commands::import_accounts,
            accounts::commands::import_from_authkey,
            accounts::commands::validate_accounts,
            accounts::commands::get_real_accounts,
            accounts::commands::get_real_accounts_stats,
            accounts::commands::get_accounts_with_stats,
            accounts::commands::get_proxies,
            accounts::commands::add_proxy,
            accounts::commands::add_proxies_bulk,
            accounts::commands::remove_proxy,
            accounts::commands::clear_proxies,
            accounts::commands::get_proxy_count,
            accounts::commands::validate_proxies,
            accounts::commands::remove_proxies,
            accounts::commands::get_proxy_txt_path,
            accounts::commands::import_proxies_from_txt,
            accounts::commands::get_roles,
            accounts::commands::add_role,
            accounts::commands::delete_role,
            accounts::commands::assign_role,
            accounts::commands::delete_accounts,
            accounts::commands::distribute_proxies,
            accounts::commands::get_proxy_distribution_info,
            accounts::commands::check_accounts_have_proxy,
            accounts::commands::set_account_two_fa,
            accounts::commands::phones_to_countries,
            checker::commands::checker_scan_folder,
            checker::commands::checker_start,
            checker::commands::checker_sort_results,
            converter::converter_start,
            converter::converter_stop,
            accounts::commands::get_authkey_txt_path,
            accounts::commands::read_authkey_txt,
            accounts::commands::get_file_mtime,
            checker::nft::fetch_nft_preview,
            proxy::enqueue_validate_proxies,
            accounts::auth_login::auth_send_code,
            accounts::auth_login::auth_sign_in,
            accounts::auth_login::auth_check_password,
            accounts::reauth::reauth_accounts,
            accounts::actions::account_actions_start,
            accounts::actions::account_actions_stop,
            reporter::reporter_start,
            reporter::reporter_stop,
            bot_parser::bot_parser_start,
            bot_parser::bot_parser_stop,
            botcreator::create_bots_start,
            botcreator::create_bots_stop,
            channelcreator::create_channels_start,
            channelcreator::create_channels_stop,
            boost::boost_start,
            boost::boost_stop,
            stories::stories_start,
            stories::stories_stop,
            cloner::runner::cloner_start,
            cloner::runner::cloner_stop,
            parser::parser_start,
            parser::parser_stop,
            parser::user_lookup::user_lookup_start,
            parser::user_lookup::user_lookup_stop,
            randomizer::randomize_text,
            username_checker::username_checker_start,
            username_checker::username_checker_stop,
            link_checker::link_checker_start,
            link_checker::link_checker_stop,
            global_search::global_search_start,
            global_search::global_search_stop,
            interceptor::interceptor_start,
            interceptor::interceptor_stop,
            forwarder::forwarder_start,
            forwarder::forwarder_stop,
            llm::llm_get_models,
            llm::llm_detect_api_type,
            warmer::warmer_start,
            warmer::warmer_stop,
            auto_reply::auto_reply_start,
            auto_reply::auto_reply_stop,
            first_comment::first_comment_start,
            first_comment::first_comment_stop,
            inviter::inviter_start,
            inviter::inviter_stop,
            masslooking::masslooking_start,
            masslooking::masslooking_stop,
            mailing::mailing_start,
            mailing::mailing_stop,
            queue::stop_task,
        ])
        .setup(|app| {
            let _ = APP_HANDLE.set(app.handle().clone());

            // periodic dedup by user_id every 60s
            let _handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    let removed = accounts::commands::dedup_sessions_by_user_id();
                    if removed > 0 {
                        dbg_log!(
                            "periodic dedup: removed {} duplicate sessions (by user_id)",
                            removed
                        );
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
