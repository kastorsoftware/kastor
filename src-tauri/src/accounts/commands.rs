use std::path::PathBuf;
use serde::{Deserialize, Serialize};

use super::import::{self, ImportFormat};
use super::session::AccountJson;
use super::storage::AccountStorage;
use crate::checker::validate;
use crate::proxy::ProxyList;
use crate::i18n::{t, t_with};

pub use crate::check_and_mark_dead_session;

// cached scan results to avoid reading all json files every 3 seconds
static ACCOUNTS_CACHE: std::sync::LazyLock<std::sync::RwLock<AccountsCache>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(AccountsCache::default()));

#[derive(Default)]
struct AccountsCache {
    accounts: Vec<StoredAccount>,
    last_scan: Option<std::time::Instant>,
    dir_mtime: Option<std::time::SystemTime>,
}

impl AccountsCache {
    fn is_stale(&self, current_mtime: Option<std::time::SystemTime>) -> bool {
        match self.last_scan {
            None => return true,
            Some(t) if t.elapsed() < std::time::Duration::from_millis(500) => return false,
            _ => {}
        }
        current_mtime != self.dir_mtime
    }
}

fn get_cached_accounts(storage: &AccountStorage) -> Vec<StoredAccount> {
    let dir = storage.session_json_dir();
    let current_mtime = std::fs::metadata(&dir).ok().and_then(|m| m.modified().ok());
    {
        let cache = ACCOUNTS_CACHE.read().unwrap();
        if !cache.is_stale(current_mtime) && !cache.accounts.is_empty() {
            return cache.accounts.clone();
        }
    }
    let accounts = scan_accounts(storage);
    let mut cache = ACCOUNTS_CACHE.write().unwrap();
    cache.accounts = accounts.clone();
    cache.last_scan = Some(std::time::Instant::now());
    cache.dir_mtime = current_mtime;
    accounts
}

// invalidate cache (call after mutations like import, delete, validate)
pub fn invalidate_accounts_cache() {
    let mut cache = ACCOUNTS_CACHE.write().unwrap();
    cache.dir_mtime = None;
}

pub fn get_storage_pub() -> AccountStorage {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kastor")
        .join("accounts");
    AccountStorage::new(&base)
}

#[derive(Serialize, Clone)]
pub struct StoredAccount {
    pub id: String,
    pub phone: String,
    pub geo: String,
    pub status: String,
    pub aging: String,
    pub role: String,
    pub name: String,
    pub username: String,
    pub app_id: i32,
    pub proxy: Option<String>,
    pub two_fa: String,
    pub premium: String,
    pub user_id: i64,
}

#[derive(Serialize)]
pub struct ImportedAccount {
    pub id: String,
    pub success: bool,
    pub missing_json: bool,
    pub error: Option<String>,
    #[serde(default)]
    pub multi_account_split: bool,
}

#[derive(Serialize)]
pub struct ValidationResult {
    pub id: String,
    pub valid: bool,
    pub user_id: Option<i64>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub phone: Option<String>,
    pub username: Option<String>,
}

#[derive(Serialize)]
pub struct RealAccountsStats {
    pub total: u32,
    pub clean: u32,
    pub restricted: u32,
}

#[derive(Serialize)]
pub struct AccountsWithStats {
    pub accounts: Vec<StoredAccount>,
    pub stats: RealAccountsStats,
}

#[tauri::command]
pub fn phones_to_countries(phones: Vec<String>) -> Vec<String> {
    phones.iter().map(|p| super::geo::phone_to_country_code(p)).collect()
}

#[tauri::command]
pub async fn get_real_accounts() -> Vec<StoredAccount> {
    let storage = get_storage_pub();
    get_cached_accounts(&storage)
}

#[tauri::command]
pub async fn get_real_accounts_stats() -> RealAccountsStats {
    let storage = get_storage_pub();
    let accounts = get_cached_accounts(&storage);
    let total = accounts.len() as u32;
    let restricted = accounts.iter()
        .filter(|a| a.status != crate::i18n::t("status_clean") && a.status != crate::i18n::t("status_unchecked")
            && !a.status.starts_with(&crate::i18n::t("status_checking").trim_end_matches('.').to_string()))
        .count() as u32;
    RealAccountsStats { total, clean: total - restricted, restricted }
}

#[tauri::command]
pub async fn get_accounts_with_stats() -> AccountsWithStats {
    let storage = get_storage_pub();
    let accounts = get_cached_accounts(&storage);
    let total = accounts.len() as u32;
    let status_clean = crate::i18n::t("status_clean");
    let status_unchecked = crate::i18n::t("status_unchecked");
    let status_checking = crate::i18n::t("status_checking");
    let checking_prefix = status_checking.trim_end_matches('.');
    let restricted = accounts.iter()
        .filter(|a| {
            let s = &a.status;
            let is_clean = s == &status_clean || s == "Без ограничений" || s == "No restrictions";
            let is_unchecked = s == &status_unchecked || s == "Не проверен" || s == "Unchecked";
            let is_checking = s.starts_with(checking_prefix) || s.starts_with("Проверка") || s.starts_with("Checking");
            let is_tdata = s.starts_with("TData");
            !is_clean && !is_unchecked && !is_checking && !is_tdata
        })
        .count() as u32;
    AccountsWithStats {
        accounts,
        stats: RealAccountsStats { total, clean: total - restricted, restricted },
    }
}

#[tauri::command]
pub async fn import_accounts(paths: Vec<String>, format: String) -> Vec<ImportedAccount> {
    dbg_log!("import_accounts called: {} paths, format='{}'", paths.len(), format);
    let storage = get_storage_pub();
    let fmt = match format.as_str() {
        "tdata" => ImportFormat::Tdata,
        "telethon" => ImportFormat::Telethon,
        "pyrogram" => ImportFormat::Pyrogram,
        _ => ImportFormat::Telethon,
    };

    let mut results = Vec::new();

    // load existing auth_keys for dedup
    let existing_keys = collect_existing_auth_keys(&storage);

    for path_str in &paths {
        let path = PathBuf::from(path_str);
        dbg_log!("import_accounts processing: {:?}", path);

        let result = if path.is_dir() {
            match fmt {
                ImportFormat::Tdata => {
                    append_tdata_results(
                        import::import_tdata_tree(&path, &storage),
                        &storage,
                        &existing_keys,
                        &mut results,
                    );
                    continue;
                }
                _ => {
                    let session_file = find_session_in_dir(&path);
                    let json_file = find_json_in_dir(&path);
                    match session_file {
                        Some(sf) => import::import_session(&sf, json_file.as_deref(), &fmt, &storage),
                        None => Err("no .session file found in folder".to_string()),
                    }
                }
            }
        } else if path.extension().map(|e| e == "zip").unwrap_or(false) {
            if fmt == ImportFormat::Tdata {
                match import::import_tdata_archive(&path, &storage) {
                    Ok(import_results) => append_tdata_results(
                        import_results,
                        &storage,
                        &existing_keys,
                        &mut results,
                    ),
                    Err(e) => results.push(ImportedAccount {
                        id: String::new(),
                        success: false,
                        missing_json: false,
                        error: Some(e),
                        multi_account_split: false,
                    }),
                }
                continue;
            }
            import::import_from_zip(&path, &fmt, &storage)
        } else if path.extension().map(|e| e == "session").unwrap_or(false) {
            let json_path = path.with_extension("json");
            let json_ref = if json_path.exists() { Some(json_path.as_path()) } else { None };
            import::import_session(&path, json_ref, &fmt, &storage)
        } else if path.extension().map(|e| e == "json").unwrap_or(false) {
            let session_path = path.with_extension("session");
            if session_path.exists() {
                import::import_session(&session_path, Some(&path), &fmt, &storage)
            } else {
                Err("no companion .session file found".to_string())
            }
        } else {
            Err(format!("unsupported file: {}", path.display()))
        };

        match result {
            Ok(id) => {
                dbg_log!("import_accounts: SUCCESS id={}", id);
                // check for duplicate auth_key
                let session_path = storage.session_path(&id);
                let is_dupe = if let Ok(session) = super::session::TelethonSession::from_file(&session_path) {
                    existing_keys.contains(&session.auth_key)
                } else {
                    false
                };

                if is_dupe {
                    // remove the just-imported files
                    std::fs::remove_file(&session_path).ok();
                    std::fs::remove_file(storage.json_path(&id)).ok();
                    std::fs::remove_dir_all(storage.tdata_dir(&id)).ok();
                    dbg_log!("import_accounts: DUPLICATE, removed id={}", id);
                    results.push(ImportedAccount {
                        id: String::new(),
                        success: false,
                        missing_json: false,
                        error: Some("duplicate".to_string()),
                        multi_account_split: false,
                    });
                    continue;
                }

                // skip warning for tdata - it always generates json from tdata contents
                let missing = if fmt == ImportFormat::Tdata {
                    false
                } else {
                    let json_path = storage.json_path(&id);
                    if json_path.exists() {
                        match AccountJson::from_file(&json_path) {
                            Ok(j) => j.phone.is_empty() && j.first_name.is_empty(),
                            Err(_) => true,
                        }
                    } else {
                        true
                    }
                };
                results.push(ImportedAccount { id, success: true, missing_json: missing, error: None, multi_account_split: false });
            }
            Err(e) => {
                dbg_log!("import_accounts: FAILED: {}", e);
                results.push(ImportedAccount {
                    id: String::new(),
                    success: false,
                    missing_json: false,
                    error: Some(e),
                    multi_account_split: false,
                });
            }
        }
    }

    invalidate_accounts_cache();
    results
}

fn append_tdata_results(
    import_results: Vec<import::TdataImportResult>,
    storage: &AccountStorage,
    existing_keys: &[Vec<u8>],
    results: &mut Vec<ImportedAccount>,
) {
    for import_result in import_results {
        match import_result {
            Ok(ids) => {
                let is_multi = ids.len() > 1;
                for id in ids {
                    let session_path = storage.session_path(&id);
                    let is_dupe = super::session::TelethonSession::from_file(&session_path)
                        .map(|session| existing_keys.contains(&session.auth_key))
                        .unwrap_or(false);

                    if is_dupe {
                        std::fs::remove_file(&session_path).ok();
                        std::fs::remove_file(storage.json_path(&id)).ok();
                        std::fs::remove_dir_all(storage.tdata_dir(&id)).ok();
                        dbg_log!("import_accounts: DUPLICATE, removed id={}", id);
                        results.push(ImportedAccount {
                            id: String::new(),
                            success: false,
                            missing_json: false,
                            error: Some("duplicate".to_string()),
                            multi_account_split: false,
                        });
                    } else {
                        results.push(ImportedAccount {
                            id,
                            success: true,
                            missing_json: false,
                            error: None,
                            multi_account_split: is_multi,
                        });
                    }
                }
            }
            Err(e) => {
                dbg_log!("import_accounts: tdata import FAILED: {}", e);
                results.push(ImportedAccount {
                    id: String::new(),
                    success: false,
                    missing_json: false,
                    error: Some(e),
                    multi_account_split: false,
                });
            }
        }
    }
}

#[tauri::command]
pub async fn import_from_authkey(auth_key_hex: String, dc_id: Option<i32>) -> Result<String, String> {
    let auth_key_bytes = hex_to_bytes(&auth_key_hex)
        .map_err(|_| t("acc_invalid_authkey"))?;

    if auth_key_bytes.len() != 256 {
        return Err(t_with("acc_authkey_len", &[("bytes", &auth_key_bytes.len().to_string())]));
    }

    // dedup: check if this auth_key already exists
    let storage = get_storage_pub();
    let existing = collect_existing_auth_keys(&storage);
    if existing.contains(&auth_key_bytes) {
        return Err("duplicate".to_string());
    }

    let resolved_dc = if let Some(dc) = dc_id {
        if dc < 1 || dc > 5 {
            return Err(t("acc_dc_range"));
        }
        dc
    } else {
        // probe all DCs to find the correct one
        let probe_proxy = crate::proxy::select_proxy_for_account(None)?;
        let mut auth_key = [0u8; 256];
        auth_key.copy_from_slice(&auth_key_bytes);

        let mut found_dc: Option<i32> = None;
        for dc in 1..=5 {
            let addr = dc_id_to_addr(dc);
            match crate::mtproto::client::MtpClient::connect(&addr, &auth_key, probe_proxy.as_ref()).await {
                Ok(mut client) => {
                    let dev = super::devices::generate_random_device();
                    let app_id = crate::get_app_config().app_id;
        match client.get_me(app_id, &dev.device, &dev.sdk, &dev.app_version, "en", "en").await {
                        Ok(_) => { found_dc = Some(dc); break; }
                        Err(_) => continue,
                    }
                }
                Err(_) => continue,
            }
        }
        found_dc.ok_or_else(|| t("acc_dc_detect_fail"))?
    };

    // assign the proxy that was used for probing to this account
    let assigned_proxy = {
        let proxy_list = ProxyList::load();
        proxy_list.get_random().map(|p| p.to_string_repr())
    };

    let id = uuid::Uuid::new_v4().to_string();

    let dc_addr = dc_id_to_addr(resolved_dc);
    let session = super::session::TelethonSession {
        dc_id: resolved_dc,
        server_address: dc_addr.split(':').next().unwrap_or("149.154.167.51").to_string(),
        port: 443,
        auth_key: auth_key_bytes.clone(),
    };

    session.to_file(&storage.session_path(&id))
        .map_err(|e| t_with("acc_session_write_error", &[("error", &e.to_string())]))?;

    let dev = super::devices::generate_random_device();
    let config = crate::get_app_config();
    let json = AccountJson {
        app_id: config.app_id,
        app_hash: config.app_hash.clone(),
        sdk: dev.sdk,
        device: dev.device,
        app_version: dev.app_version,
        lang_pack: "en".to_string(),
        system_lang_pack: "en-US".to_string(),
        proxy: assigned_proxy,
        ..Default::default()
    };
    json.to_file(&storage.json_path(&id))?;

    // generate tdata from session
    let tdata_account = crate::converter::tdata::TDataAccount {
        dc_id: resolved_dc,
        user_id: 0,
        auth_key: auth_key_bytes,
    };
    let tdata_dir = storage.tdata_dir(&id);
    crate::converter::tdata::write_tdata(&tdata_dir, &tdata_account)?;

    invalidate_accounts_cache();
    Ok(id)
}

fn dc_id_to_addr(dc: i32) -> String {
    crate::get_app_config()
        .dc_addresses
        .get(&dc)
        .cloned()
        .unwrap_or_else(|| "149.154.167.51:443".to_string())
}

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, ()> {
    let hex = hex.trim();
    if hex.len() % 2 != 0 { return Err(()); }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i+2], 16).map_err(|_| ()))
        .collect()
}

#[tauri::command]
pub async fn validate_accounts(ids: Vec<String>) -> Vec<ValidationResult> {
    let storage = get_storage_pub();
    let mut results = Vec::new();

    for id in &ids {
        let session_path = storage.session_path(id);
        let json_path = storage.json_path(id);

        if !session_path.exists() {
            results.push(ValidationResult {
                id: id.clone(),
                valid: false,
                user_id: None,
                first_name: None,
                last_name: None,
                phone: None,
                username: None,
            });
            continue;
        }

        let session = match super::session::TelethonSession::from_file(&session_path) {
            Ok(s) => s,
            Err(_) => {
                results.push(ValidationResult {
                    id: id.clone(),
                    valid: false,
                    user_id: None,
                    first_name: None,
                    last_name: None,
                    phone: None,
                    username: None,
                });
                continue;
            }
        };

        let json = if json_path.exists() {
            AccountJson::from_file(&json_path).unwrap_or_default()
        } else {
            let config = crate::get_app_config();
            AccountJson {
                app_id: config.app_id,
                app_hash: config.app_hash.clone(),
                ..Default::default()
            }
        };

        let proxy = match crate::proxy::select_proxy_for_account(json.proxy.as_deref()) {
            Ok(p) => p,
            Err(_) => {
                // no proxy and allow_no_proxy=false: skip with invalid result
                results.push(ValidationResult {
                    id: id.clone(),
                    valid: false,
                    user_id: None,
                    first_name: None,
                    last_name: None,
                    phone: None,
                    username: None,
                });
                continue;
            }
        };

        let (vr, _client) = validate::validate_account(&session, &json, proxy.as_ref()).await;

        // persist validation results
        let mut updated = json.clone();
        updated.validated = true;
        updated.valid = vr.valid;
        if let Some(ref phone) = vr.phone { updated.phone = phone.clone(); }
        if let Some(ref name) = vr.first_name { updated.first_name = name.clone(); }
        if let Some(ref last) = vr.last_name { updated.last_name = last.clone(); }
        if let Some(ref uname) = vr.username { updated.username = uname.clone(); }
        if let Some(uid) = vr.user_id { updated.user_id = uid; }
        updated.status = if vr.valid {
            crate::i18n::t("status_clean")
        } else {
            crate::i18n::t("status_invalid")
        };
        updated.last_check_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if vr.valid {
            updated.last_connect_date = chrono_now_iso();
        }
        let _ = updated.to_file(&json_path);

        // generate tdata if user_id was just determined and tdata doesn't exist yet
        if vr.valid && updated.user_id != 0 {
            let tdata_dir = storage.tdata_dir(&id);
            if !tdata_dir.join("key_datas").exists() {
                let tdata_acc = crate::converter::tdata::TDataAccount {
                    dc_id: session.dc_id,
                    user_id: updated.user_id,
                    auth_key: session.auth_key.clone(),
                };
                let _ = crate::converter::tdata::write_tdata(&tdata_dir, &tdata_acc);
            }
        }

        results.push(ValidationResult {
            id: id.clone(),
            valid: vr.valid,
            user_id: vr.user_id,
            first_name: vr.first_name,
            last_name: vr.last_name,
            phone: vr.phone,
            username: vr.username,
        });
    }

    results
}

// delete accounts by id - removes .session, .json, and tdata folder
#[tauri::command]
pub async fn delete_accounts(ids: Vec<String>) -> Result<u32, String> {
    let storage = get_storage_pub();
    let mut deleted = 0u32;

    for id in &ids {
        let session_path = storage.session_path(id);
        let json_path = storage.json_path(id);
        let tdata_path = storage.tdata_dir(id);

        let mut found = false;
        if session_path.exists() {
            std::fs::remove_file(&session_path).ok();
            found = true;
        }
        if json_path.exists() {
            std::fs::remove_file(&json_path).ok();
            found = true;
        }
        if tdata_path.exists() {
            std::fs::remove_dir_all(&tdata_path).ok();
            found = true;
        }
        if found {
            deleted += 1;
        }
    }

    dbg_log!("delete_accounts: deleted {} of {} requested", deleted, ids.len());
    invalidate_accounts_cache();
    Ok(deleted)
}

// proxy management commands
#[tauri::command]
pub async fn get_proxies() -> Vec<crate::proxy::ProxyConfig> {
    ProxyList::load().proxies
}

#[tauri::command]
pub async fn add_proxy(proxy_str: String) -> Result<String, String> {
    dbg_log!("add_proxy input='{}'", proxy_str);
    let config = crate::proxy::ProxyConfig::from_string(&proxy_str)?;
    let mut list = ProxyList::load();
    // check duplicate
    let is_dupe = list.proxies.iter().any(|p| {
        p.host == config.host && p.port == config.port
            && p.username == config.username && p.password == config.password
    });
    if is_dupe {
        return Err(t("acc_proxy_exists"));
    }
    let repr = config.to_string_repr();
    list.proxies.push(config);
    list.save()?;
    Ok(repr)
}

#[tauri::command]
pub async fn add_proxies_bulk(proxies_text: String) -> Result<(u32, u32), String> {
    dbg_log!("add_proxies_bulk {} lines", proxies_text.lines().count());
    let mut list = ProxyList::load();
    let mut count = 0u32;
    let mut dupes = 0u32;
    for line in proxies_text.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        match crate::proxy::ProxyConfig::from_string(line) {
            Ok(config) => {
                let is_dupe = list.proxies.iter().any(|p| {
                    p.host == config.host && p.port == config.port
                        && p.username == config.username && p.password == config.password
                });
                if is_dupe {
                    dupes += 1;
                    continue;
                }
                list.proxies.push(config);
                count += 1;
            }
            Err(e) => {
                dbg_log!("add_proxies_bulk: FAILED '{}': {}", line, e);
            }
        }
    }
    list.save()?;
    Ok((count, dupes))
}

#[tauri::command]
pub async fn remove_proxy(index: usize) -> Result<(), String> {
    let mut list = ProxyList::load();
    if index >= list.proxies.len() {
        return Err("index out of bounds".into());
    }
    let removed = list.proxies.remove(index);
    list.save()?;
    detach_proxy_from_accounts(&removed.to_string_repr());
    Ok(())
}

#[tauri::command]
pub async fn clear_proxies() -> Result<(), String> {
    let mut list = ProxyList::load();
    let reprs: Vec<String> = list.proxies.iter().map(|p| p.to_string_repr()).collect();
    list.proxies.clear();
    list.save()?;
    for repr in &reprs {
        detach_proxy_from_accounts(repr);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_proxy_count() -> u32 {
    ProxyList::load().proxies.len() as u32
}

// validate selected proxies (or all if ids is empty)
#[tauri::command]
pub async fn validate_proxies(ids: Vec<String>, threads: Option<usize>) -> Vec<(String, bool)> {
    let mut list = crate::proxy::ProxyList::load();
    let target_ids: Vec<String> = if ids.is_empty() {
        list.proxies.iter().map(|p| p.id.clone()).collect()
    } else {
        ids
    };
    let concurrency = threads.unwrap_or(10).max(1).min(1000);
    crate::proxy::validate_proxies_batch(&mut list, &target_ids, concurrency).await
}

// remove multiple proxies by id
#[tauri::command]
pub async fn remove_proxies(ids: Vec<String>) -> Result<u32, String> {
    let mut list = ProxyList::load();
    let before = list.proxies.len();
    let removed_reprs: Vec<String> = list.proxies.iter()
        .filter(|p| ids.contains(&p.id))
        .map(|p| p.to_string_repr())
        .collect();
    list.proxies.retain(|p| !ids.contains(&p.id));
    let removed = (before - list.proxies.len()) as u32;
    list.save()?;
    for repr in &removed_reprs {
        detach_proxy_from_accounts(repr);
    }
    Ok(removed)
}

// get path to proxy txt file for manual editing
#[tauri::command]
pub async fn get_proxy_txt_path(proxy_type: String) -> Result<String, String> {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("kastor");
    std::fs::create_dir_all(&base).ok();
    let filename = format!("proxies_{}.txt", proxy_type.to_lowercase());
    let path = base.join(&filename);
    // create file if not exists
    if !path.exists() {
        let header = format!("# {} proxy - one per line\n# format: ip:port or ip:port:login:password or ip:port@login:password\n\n", proxy_type);
        std::fs::write(&path, header).ok();
    }
    Ok(path.to_string_lossy().to_string())
}

// reload proxies from txt file (replaces all proxies of this type)
#[tauri::command]
pub async fn import_proxies_from_txt(proxy_type: String) -> Result<(u32, u32), String> {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("kastor");
    let filename = format!("proxies_{}.txt", proxy_type.to_lowercase());
    let path = base.join(&filename);

    if !path.exists() {
        return Ok((0, 0));
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| t_with("acc_read_file_error", &[("error", &e.to_string())]))?;

    let mut list = ProxyList::load();
    let mut count = 0u32;

    let ptype = match proxy_type.to_lowercase().as_str() {
        "socks4" => crate::proxy::ProxyType::Socks4,
        "socks5" => crate::proxy::ProxyType::Socks5,
        "https" => crate::proxy::ProxyType::Https,
        _ => crate::proxy::ProxyType::Socks5,
    };

    // remove existing proxies of this type before re-importing
    list.proxies.retain(|p| p.proxy_type != ptype);

    let mut skipped_dupes = 0u32;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }

        // parse line and override type
        if let Ok(mut config) = crate::proxy::ProxyConfig::from_string(line) {
            config.proxy_type = ptype.clone();
            // check for duplicate (same host:port:user:pass)
            let is_dupe = list.proxies.iter().any(|p| {
                p.host == config.host && p.port == config.port
                    && p.username == config.username && p.password == config.password
            });
            if is_dupe {
                skipped_dupes += 1;
                continue;
            }
            list.proxies.push(config);
            count += 1;
        }
    }

    list.save()?;
    dbg_log!("import_proxies_from_txt: imported {} proxies, skipped {} dupes from {:?}", count, skipped_dupes, path);
    Ok((count, skipped_dupes))
}

fn scan_accounts(storage: &AccountStorage) -> Vec<StoredAccount> {
    let dir = storage.session_json_dir();
    let mut accounts = Vec::new();
    let mut auto_roles: Vec<(String, String)> = Vec::new();

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return accounts,
    };

    let roles = load_roles();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "session").unwrap_or(false) {
            let id = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let json_path = storage.json_path(&id);

            let json = if json_path.exists() {
                AccountJson::from_file(&json_path).unwrap_or_default()
            } else {
                AccountJson::default()
            };

            let status = if json.validated {
                if json.status.is_empty() {
                    if json.valid { crate::i18n::t("status_clean") } else { crate::i18n::t("status_invalid") }
                } else {
                    normalize_status(&json.status)
                }
            } else if !json.spamblock.is_empty() && json.spamblock != "free" {
                match json.spamblock.as_str() {
                    "geo" | "geonumber" => crate::i18n::t("status_geo_spam"),
                    "frozen" => crate::i18n::t("status_frozen"),
                    "permanent" | "spamblock" => crate::i18n::t("status_perm_spam"),
                    _ => crate::i18n::t("status_unchecked"),
                }
            } else {
                crate::i18n::t("status_unchecked")
            };

            let name = if !json.first_name.is_empty() {
                if json.last_name.is_empty() {
                    json.first_name.clone()
                } else {
                    format!("{} {}", json.first_name, json.last_name)
                }
            } else {
                String::new()
            };

            let aging = if json.register_time > 0 {
                calc_aging_from_timestamp(json.register_time)
            } else {
                calc_aging(&path)
            };
            let role = if !json.role.is_empty() {
                json.role.clone()
            } else {
                roles.get(&id).cloned().unwrap_or_default()
            };
            let geo = super::geo::phone_to_country_code(&json.phone);

            let two_fa = if !json.two_fa.is_empty() {
                json.two_fa.clone()
            } else {
                String::new()
            };

            let premium = if json.is_premium {
                if json.premium_expiry.is_empty() {
                    t("acc_2fa_present")
                } else {
                    t_with("acc_2fa_until", &[("date", &json.premium_expiry)])
                }
            } else {
                String::new()
            };

            accounts.push(StoredAccount {
                id: id.clone(),
                phone: json.phone.clone(),
                geo,
                status,
                aging,
                role: role.clone(),
                name,
                username: json.username.clone(),
                app_id: json.app_id,
                proxy: json.proxy.clone(),
                two_fa,
                premium,
                user_id: json.user_id,
            });

            // auto-assign role from json if not already assigned
            if !json.role.is_empty() && !roles.contains_key(&id) {
                auto_roles.push((id.clone(), json.role.clone()));
            }
        }
    }

    // persist auto-created roles
    if !auto_roles.is_empty() {
        let mut data = load_roles_data();
        for (acc_id, role_name) in &auto_roles {
            if !data.roles.contains(role_name) {
                data.roles.push(role_name.clone());
            }
            data.assignments.insert(acc_id.clone(), role_name.clone());
        }
        save_roles_data(&data);
    }

    // also scan tdatas dir for unconverted accounts
    let tdata_dir = storage.tdatas_dir();
    if let Ok(entries) = std::fs::read_dir(&tdata_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let id = entry.file_name().to_string_lossy().to_string();
                if !storage.session_path(&id).exists() {
                    let aging = calc_aging(&entry.path());
                    let role = roles.get(&id).cloned().unwrap_or_default();
                    accounts.push(StoredAccount {
                        id,
                        phone: String::new(),
                        geo: String::new(),
                        status: t("status_tdata"),
                        aging,
                        role,
                        name: String::new(),
                        username: String::new(),
                        app_id: 0,
                        proxy: None,
                        two_fa: String::new(),
                        premium: String::new(),
                        user_id: 0,
                    });
                }
            }
        }
    }

    accounts
}

/// Normalize a stored status string (possibly in the wrong language) to the current locale.
fn normalize_status(stored: &str) -> String {
    use crate::i18n::t;
    // Map known status values (both languages) to their i18n key
    match stored {
        "Без ограничений" | "No restrictions" => t("status_clean"),
        "Невалид" | "Invalid" => t("status_invalid"),
        "Заморожен" | "Frozen" => t("status_frozen"),
        "Вечный спамблок" | "Permanent spamblock" => t("status_perm_spam"),
        "Не проверен" | "Unchecked" => t("status_unchecked"),
        "TData (не конвертирован)" | "TData (not converted)" => t("status_tdata"),
        s if s.starts_with("Спамблок по ГЕО") || s.starts_with("Geo spamblock") => t("status_geo_spam"),
        s if s.starts_with("Проверка") || s.starts_with("Checking") => t("status_checking"),
        _ => stored.to_string(),
    }
}

fn calc_aging_from_timestamp(register_time: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let elapsed_secs = (now - register_time).max(0) as u64;
    let days = elapsed_secs / 86400;
    if days >= 365 {
        let years = days / 365;
        let months = (days % 365) / 30;
        if months > 0 { t_with("acc_aging_years_months", &[("years", &years.to_string()), ("months", &months.to_string())]) } else { t_with("acc_aging_years", &[("years", &years.to_string())]) }
    } else if days >= 30 {
        let months = days / 30;
        t_with("acc_aging_months", &[("months", &months.to_string())])
    } else {
        t("acc_aging_less_month")
    }
}

fn calc_aging(path: &std::path::Path) -> String {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return String::new(),
    };
    let created = match metadata.created().or_else(|_| metadata.modified()) {
        Ok(t) => t,
        Err(_) => return String::new(),
    };
    let elapsed = match created.elapsed() {
        Ok(d) => d,
        Err(_) => return String::new(),
    };

    let days = elapsed.as_secs() / 86400;
    if days >= 365 {
        let years = days / 365;
        let months = (days % 365) / 30;
        if months > 0 {
            t_with("acc_aging_years_months", &[("years", &years.to_string()), ("months", &months.to_string())])
        } else {
            t_with("acc_aging_years", &[("years", &years.to_string())])
        }
    } else if days >= 30 {
        let months = days / 30;
        t_with("acc_aging_months", &[("months", &months.to_string())])
    } else {
        t("acc_aging_less_month")
    }
}

fn chrono_now_iso() -> String {
    // simple iso-ish timestamp without chrono dependency
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // just store as unix timestamp string - frontend can format
    now.to_string()
}

// roles stored in a separate json file
fn roles_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kastor")
        .join("roles.json")
}

#[derive(Serialize, Deserialize, Default)]
struct RolesData {
    roles: Vec<String>,
    assignments: std::collections::HashMap<String, String>,
}

fn load_roles_data() -> RolesData {
    let path = roles_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        RolesData::default()
    }
}

fn save_roles_data(data: &RolesData) {
    let path = roles_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Ok(content) = serde_json::to_string_pretty(data) {
        std::fs::write(&path, content).ok();
    }
}

fn load_roles() -> std::collections::HashMap<String, String> {
    load_roles_data().assignments
}

#[tauri::command]
pub async fn get_roles() -> Vec<String> {
    load_roles_data().roles
}

#[tauri::command]
pub async fn add_role(name: String) {
    let mut data = load_roles_data();
    if !data.roles.contains(&name) {
        data.roles.push(name);
        save_roles_data(&data);
    }
}

#[tauri::command]
pub async fn delete_role(name: String) {
    let mut data = load_roles_data();
    data.roles.retain(|r| r != &name);
    data.assignments.retain(|_, v| v != &name);
    save_roles_data(&data);
}

#[tauri::command]
pub async fn assign_role(ids: Vec<String>, role: String) {
    let mut data = load_roles_data();
    for id in ids {
        if role.is_empty() {
            data.assignments.remove(&id);
        } else {
            data.assignments.insert(id, role.clone());
        }
    }
    save_roles_data(&data);
    invalidate_accounts_cache();
}

// distribute proxies across accounts
// mode: "skip" = leave accounts without proxy, "reuse" = reuse proxies cyclically
#[tauri::command]
pub async fn distribute_proxies(mode: String) -> Result<u32, String> {
    let storage = get_storage_pub();
    let accounts = scan_accounts(&storage);
    let proxy_list = ProxyList::load();

    if proxy_list.proxies.is_empty() {
        return Err(t("acc_no_proxies_distribute"));
    }

    let mut assigned = 0u32;
    let proxy_count = proxy_list.proxies.len();

    // redistribute mode: clear all proxies first, then assign cyclically
    if mode == "redistribute" || mode == "clear_proxies" {
        for acc in &accounts {
            let json_path = storage.json_path(&acc.id);
            if let Ok(mut json) = AccountJson::from_file(&json_path) {
                if json.proxy.is_some() {
                    json.proxy = None;
                    let _ = json.to_file(&json_path);
                }
            }
        }
        if mode == "clear_proxies" {
            return Ok(0);
        }
    }

    let mut unassigned_idx = 0usize;
    for acc in &accounts {
        let json_path = storage.json_path(&acc.id);
        let mut json = if json_path.exists() {
            AccountJson::from_file(&json_path).unwrap_or_default()
        } else {
            continue;
        };

        if json.proxy.is_some() && mode != "redistribute" { continue; }

        let proxy_idx = if mode == "reuse" || mode == "redistribute" {
            Some(unassigned_idx % proxy_count)
        } else {
            // "skip" mode
            if unassigned_idx < proxy_count { Some(unassigned_idx) } else { None }
        };

        if let Some(idx) = proxy_idx {
            let px = &proxy_list.proxies[idx];
            json.proxy = Some(px.to_string_repr());
            let _ = json.to_file(&json_path);
            assigned += 1;
        }
        unassigned_idx += 1;
    }

    dbg_log!("distribute_proxies: mode={} assigned={}", mode, assigned);
    invalidate_accounts_cache();
    Ok(assigned)
}

// get distribution info (how many accounts, how many proxies, how many unassigned)
#[tauri::command]
pub async fn get_proxy_distribution_info() -> (u32, u32, u32) {
    let storage = get_storage_pub();
    let accounts = scan_accounts(&storage);
    let proxy_count = ProxyList::load().proxies.len() as u32;

    let mut unassigned = 0u32;
    for acc in &accounts {
        let json_path = storage.json_path(&acc.id);
        if json_path.exists() {
            if let Ok(json) = AccountJson::from_file(&json_path) {
                if json.proxy.is_none() { unassigned += 1; }
            }
        }
    }

    (accounts.len() as u32, proxy_count, unassigned)
}

// check if specific accounts have proxy assigned
#[tauri::command]
pub async fn check_accounts_have_proxy(ids: Vec<String>) -> bool {
    let storage = get_storage_pub();
    for id in &ids {
        let json_path = storage.json_path(id);
        if json_path.exists() {
            if let Ok(json) = AccountJson::from_file(&json_path) {
                if json.proxy.is_none() { return false; }
            } else {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}

#[tauri::command]
pub async fn set_account_two_fa(id: String, two_fa: String) -> Result<(), String> {
    let storage = get_storage_pub();
    let json_path = storage.json_path(&id);
    let mut json = if json_path.exists() {
        AccountJson::from_file(&json_path).map_err(|e| format!("read json: {e}"))?
    } else {
        return Err("account not found".into());
    };
    json.two_fa = two_fa;
    json.to_file(&json_path).map_err(|e| format!("write json: {e}"))?;
    invalidate_accounts_cache();
    Ok(())
}

fn find_session_in_dir(dir: &PathBuf) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        if p.extension().map(|x| x == "session").unwrap_or(false) {
            Some(p)
        } else {
            None
        }
    })
}

fn find_json_in_dir(dir: &PathBuf) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        if p.extension().map(|x| x == "json").unwrap_or(false) {
            Some(p)
        } else {
            None
        }
    })
}

fn collect_existing_auth_keys(storage: &AccountStorage) -> Vec<Vec<u8>> {
    let dir = storage.session_json_dir();
    let mut keys = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "session").unwrap_or(false) {
                if let Ok(session) = super::session::TelethonSession::from_file(&path) {
                    keys.push(session.auth_key);
                }
            }
        }
    }
    keys
}

pub fn collect_existing_auth_keys_pub(storage: &AccountStorage) -> Vec<Vec<u8>> {
    collect_existing_auth_keys(storage)
}

// remove duplicate sessions (same auth_key), keeping the first found
pub fn dedup_sessions_by_auth_key() -> u32 {
    let storage = get_storage_pub();
    let dir = storage.session_json_dir();
    let mut seen: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    let mut removed = 0u32;

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .into_iter().flatten().flatten()
        .filter(|e| e.path().extension().map(|x| x == "session").unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let path = entry.path();
        if let Ok(session) = super::session::TelethonSession::from_file(&path) {
            if !seen.insert(session.auth_key) {
                // duplicate — remove session + json + tdata
                let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                std::fs::remove_file(&path).ok();
                std::fs::remove_file(storage.json_path(&stem)).ok();
                std::fs::remove_dir_all(storage.tdata_dir(&stem)).ok();
                removed += 1;
                dbg_log!("dedup_sessions_by_auth_key: removed duplicate id={}", stem);
            }
        }
    }

    if removed > 0 {
        invalidate_accounts_cache();
    }
    removed
}

// remove duplicate sessions (same user_id in json), keeping the newest (by file mtime)
pub fn dedup_sessions_by_user_id() -> u32 {
    let storage = get_storage_pub();
    let dir = storage.session_json_dir();
    let mut uid_map: std::collections::HashMap<i64, (String, std::time::SystemTime)> = std::collections::HashMap::new();
    let mut removed = 0u32;

    let entries: Vec<_> = std::fs::read_dir(&dir)
        .into_iter().flatten().flatten()
        .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
        .collect();

    for entry in entries {
        let path = entry.path();
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        if let Ok(json) = super::session::AccountJson::from_file(&path) {
            if json.user_id == 0 { continue; }
            let mtime = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

            if let Some((existing_stem, existing_mtime)) = uid_map.get(&json.user_id) {
                // keep the newer one
                let (to_remove, to_keep_stem) = if mtime > *existing_mtime {
                    (existing_stem.clone(), stem.clone())
                } else {
                    (stem.clone(), existing_stem.clone())
                };
                std::fs::remove_file(storage.session_path(&to_remove)).ok();
                std::fs::remove_file(storage.json_path(&to_remove)).ok();
                std::fs::remove_dir_all(storage.tdata_dir(&to_remove)).ok();
                removed += 1;
                dbg_log!("dedup_sessions_by_user_id: removed duplicate uid={} id={}", json.user_id, to_remove);
                uid_map.insert(json.user_id, (to_keep_stem, mtime.max(*existing_mtime)));
            } else {
                uid_map.insert(json.user_id, (stem, mtime));
            }
        }
    }

    if removed > 0 {
        invalidate_accounts_cache();
    }
    removed
}

fn detach_proxy_from_accounts(proxy_repr: &str) {
    let storage = get_storage_pub();
    let dir = storage.session_json_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(mut json) = super::session::AccountJson::from_file(&path) {
                    if json.proxy.as_deref() == Some(proxy_repr) {
                        json.proxy = None;
                        let _ = json.to_file(&path);
                    }
                }
            }
        }
    }
}

#[tauri::command]
pub async fn launch_telegram(account_id: String) -> Result<String, String> {
    dbg_log!("launch_telegram id={}", account_id);

    let storage = get_storage_pub();
    let json_path = storage.json_path(&account_id);
    let proxy_url = if json_path.exists() {
        super::session::AccountJson::from_file(&json_path)
            .ok()
            .and_then(|j| j.proxy)
            .filter(|p| !p.is_empty())
    } else {
        None
    };

    super::browser::BrowserInstance::open_telegram_web(account_id.clone(), proxy_url).await?;
    Ok(account_id)
}

#[tauri::command]
pub fn get_file_mtime(path: String) -> Option<u64> {
    std::fs::metadata(&path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

pub fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn open_accounts_folder() {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("kastor")
        .join("accounts");
    dbg_log!("open_accounts_folder {:?}", base);
    std::fs::create_dir_all(&base).ok();
    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("explorer").arg(&base).spawn(); }
}

#[tauri::command]
pub fn open_file_in_editor(path: String) {
    dbg_log!("open_file_in_editor {:?}", path);
    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("cmd").args(["/c", "start", "", &path]).spawn(); }
}

#[tauri::command]
pub fn get_authkey_txt_path() -> Result<String, String> {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("kastor");
    std::fs::create_dir_all(&base).ok();
    let path = base.join("import_authkeys.txt");
    if !path.exists() {
        let header = "# AuthKey Import\n\
                      # 1 line = 1 auth key\n\
                      #\n\
                      # Supported formats:\n\
                      #   authkey_hex              (512 hex chars, DC will be auto-detected)\n\
                      #   authkey_hex:dc_id        (512 hex chars : DC number 1-5)\n\
                      #\n\
                      # Delete these comments and paste your keys below:\n\n";
        std::fs::write(&path, header).ok();
    }
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn read_authkey_txt(path: String) -> Result<Vec<String>, String> {
    let content = std::fs::read_to_string(&path)
        .map_err(|e| t_with("acc_read_file_error", &[("error", &e.to_string())]))?;
    let keys: Vec<String> = content.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    Ok(keys)
}
