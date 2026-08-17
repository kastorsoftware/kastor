use std::path::{Path, PathBuf};
use std::fs;
use zip::ZipArchive;

use super::session::{AccountJson, TelethonSession};
use super::storage::AccountStorage;
use crate::converter;
use crate::converter::pyro::PyroSession;

#[derive(Debug, Clone, PartialEq)]
pub enum ImportFormat {
    Tdata,
    Telethon,
    Pyrogram,
}

fn default_json() -> AccountJson {
    let dev = super::devices::generate_random_device();
    let config = crate::get_app_config();
    AccountJson {
        app_id: config.app_id,
        app_hash: config.app_hash.clone(),
        sdk: dev.sdk,
        device: dev.device,
        app_version: dev.app_version,
        lang_pack: "en".to_string(),
        system_lang_pack: "en-US".to_string(),
        ..Default::default()
    }
}

// save a CommonAccount into the panel storage, return id
fn persist_common_account(
    acc: &converter::CommonAccount,
    storage: &AccountStorage,
    store_tdata_source: Option<&Path>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();

    // write telethon session
    converter::write_account(acc, &storage.session_json_dir(), converter::Format::Telethon)
        .map_err(|e| format!("session write: {e}"))?;
    // rename from source_name.session to id.session
    let written = storage.session_json_dir().join(format!("{}.session", acc.source_name));
    let target = storage.session_path(&id);
    if written != target {
        fs::rename(&written, &target).map_err(|e| format!("rename session: {e}"))?;
    }

    // write json with device info
    let mut json = default_json();
    json.user_id = acc.user_id;
    json.to_file(&storage.json_path(&id))?;

    // generate tdata for dual storage
    let tdata_acc = crate::converter::tdata::TDataAccount {
        dc_id: acc.dc_id,
        user_id: acc.user_id,
        auth_key: acc.auth_key.clone(),
    };
    let _ = crate::converter::tdata::write_tdata(&storage.tdata_dir(&id), &tdata_acc);

    // optionally copy raw tdata source
    if let Some(src) = store_tdata_source {
        let dest = storage.tdata_dir(&id);
        if !dest.exists() {
            fs::create_dir_all(&dest).ok();
            copy_dir_recursive(src, &dest).ok();
        }
    }

    Ok(id)
}

// import telethon/pyrogram session+json pair
pub fn import_session(
    session_file: &Path,
    json_file: Option<&Path>,
    format: &ImportFormat,
    storage: &AccountStorage,
) -> Result<String, String> {
    // read auth_key via converter's format reader
    let mut acc = match format {
        ImportFormat::Pyrogram => {
            let pyro = PyroSession::from_file(session_file)?;
            converter::CommonAccount {
                auth_key: pyro.auth_key,
                dc_id: pyro.dc_id,
                user_id: pyro.user_id,
                source_name: uuid::Uuid::new_v4().to_string(),
            }
        }
        _ => {
            let session = TelethonSession::from_file(session_file)?;
            let uid = TelethonSession::get_user_id(session_file);
            converter::CommonAccount {
                auth_key: session.auth_key,
                dc_id: session.dc_id,
                user_id: uid,
                source_name: uuid::Uuid::new_v4().to_string(),
            }
        }
    };

    let id = uuid::Uuid::new_v4().to_string();

    let cfg = crate::get_app_config();
    let server_address = cfg.dc_addresses.get(&acc.dc_id)
        .map(|s| s.split(':').next().unwrap_or(s).to_string())
        .unwrap_or_else(|| "149.154.167.51".to_string());
    // write session via converter
    let session = TelethonSession {
        dc_id: acc.dc_id,
        server_address,
        port: 443,
        auth_key: acc.auth_key.clone(),
    };
    session.to_file(&storage.session_path(&id))?;

    // handle json
    if let Some(jf) = json_file {
        fs::copy(jf, storage.json_path(&id))
            .map_err(|e| format!("failed to copy json: {e}"))?;
        if let Ok(mut json) = AccountJson::from_file(&storage.json_path(&id)) {
            let mut changed = false;
            if json.device.is_empty() || json.sdk.is_empty() || json.app_version.is_empty() {
                let dev = super::devices::generate_random_device();
                if json.device.is_empty() { json.device = dev.device; changed = true; }
                if json.sdk.is_empty() { json.sdk = dev.sdk; changed = true; }
                if json.app_version.is_empty() { json.app_version = dev.app_version; changed = true; }
            }
            if json.app_id == 0 { json.app_id = crate::get_app_config().app_id; changed = true; }
            if json.app_hash.is_empty() { json.app_hash = crate::get_app_config().app_hash.clone(); changed = true; }
            if json.lang_pack.is_empty() { json.lang_pack = "en".to_string(); changed = true; }
            if json.system_lang_pack.is_empty() { json.system_lang_pack = "en-US".to_string(); changed = true; }
            // use user_id from json if session didn't have it
            if acc.user_id == 0 && json.user_id != 0 {
                acc.user_id = json.user_id;
            }
            if changed { let _ = json.to_file(&storage.json_path(&id)); }
        }
    } else {
        let mut json = default_json();
        json.user_id = acc.user_id;
        json.to_file(&storage.json_path(&id))?;
    }

    // generate tdata for dual storage
    let tdata_account = crate::converter::tdata::TDataAccount {
        dc_id: acc.dc_id,
        user_id: acc.user_id,
        auth_key: acc.auth_key,
    };
    if let Err(e) = crate::converter::tdata::write_tdata(&storage.tdata_dir(&id), &tdata_account) {
        dbg_log!("import_session: tdata generation failed (non-fatal): {}", e);
    }

    Ok(id)
}

// import from zip containing tdata or session files
pub fn import_from_zip(
    zip_path: &Path,
    format: &ImportFormat,
    storage: &AccountStorage,
) -> Result<String, String> {
    let file = fs::File::open(zip_path)
        .map_err(|e| format!("failed to open zip: {e}"))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| format!("failed to read zip: {e}"))?;

    let id = uuid::Uuid::new_v4().to_string();
    let temp_dir = std::env::temp_dir().join(format!("combine_import_{id}"));
    fs::create_dir_all(&temp_dir).ok();

    archive.extract(&temp_dir)
        .map_err(|e| format!("failed to extract zip: {e}"))?;

    let result = match format {
        ImportFormat::Tdata => import_tdata_from_dir(&temp_dir, storage),
        _ => import_session_from_dir(&temp_dir, format, storage),
    };

    fs::remove_dir_all(&temp_dir).ok();
    result
}

// import tdata folder directly (supports multi-account tdata)
pub fn import_tdata_folder(
    tdata_path: &Path,
    storage: &AccountStorage,
) -> Result<Vec<String>, String> {
    dbg_log!("import::import_tdata_folder {:?}", tdata_path);

    let accounts = crate::converter::tdata::parse_tdata(tdata_path)
        .map_err(|e| {
            dbg_log!("import::import_tdata_folder parse_tdata FAILED: {}", e);
            e
        })?;

    if accounts.is_empty() {
        return Err("no accounts found in tdata".to_string());
    }

    let mut ids = Vec::new();

    for (i, acc) in accounts.iter().enumerate() {
        let common = converter::CommonAccount {
            auth_key: acc.auth_key.clone(),
            dc_id: acc.dc_id,
            user_id: acc.user_id,
            source_name: uuid::Uuid::new_v4().to_string(),
        };

        // store raw tdata only for first account
        let tdata_src = if i == 0 { Some(tdata_path) } else { None };
        let id = persist_common_account(&common, storage, tdata_src)?;

        dbg_log!("import::import_tdata_folder saved id={} dc_id={} user_id={}",
            id, acc.dc_id, acc.user_id);
        ids.push(id);
    }

    Ok(ids)
}

fn import_session_from_dir(
    dir: &Path,
    format: &ImportFormat,
    storage: &AccountStorage,
) -> Result<String, String> {
    let session_file = find_file_with_ext(dir, "session")
        .ok_or_else(|| "no .session file found in archive".to_string())?;
    let json_file = find_file_with_ext(dir, "json");
    import_session(&session_file, json_file.as_deref(), format, storage)
}

fn import_tdata_from_dir(
    dir: &Path,
    storage: &AccountStorage,
) -> Result<String, String> {
    let tdata_dir = if dir.join("tdata").exists() {
        dir.join("tdata")
    } else {
        dir.to_path_buf()
    };

    let ids = import_tdata_folder(&tdata_dir, storage)?;
    ids.into_iter().next().ok_or_else(|| "no accounts imported".to_string())
}

fn find_file_with_ext(dir: &Path, ext: &str) -> Option<PathBuf> {
    walkdir(dir)
        .into_iter()
        .find(|p| p.extension().map(|e| e == ext).unwrap_or(false))
}

fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                results.push(path);
            } else if path.is_dir() {
                results.extend(walkdir(&path));
            }
        }
    }
    results
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("mkdir failed: {e}"))?;
    for entry in fs::read_dir(src).map_err(|e| format!("readdir failed: {e}"))? {
        let entry = entry.map_err(|e| format!("entry error: {e}"))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("copy failed: {e}"))?;
        }
    }
    Ok(())
}

