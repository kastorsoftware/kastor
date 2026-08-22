// converter: any input format -> common (auth_key, dc_id, user_id) -> any output format
// readers/writers reused from session/pyro/tdata modules

pub mod pyro;
pub mod tdata;
pub mod telethon;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Semaphore;

use self::pyro::PyroSession;
use self::tdata as tdata_fmt;
use self::telethon::TelethonSession;
use crate::i18n::{t, t_with};
use crate::queue::TaskQueue;

pub fn dc_host_from_config(dc: i32) -> String {
    crate::get_app_config()
        .dc_addresses
        .get(&dc)
        .map(|s| s.split(':').next().unwrap_or(s).to_string())
        .unwrap_or_else(|| "149.154.167.51".to_string())
}

#[derive(Debug, Clone)]
pub struct CommonAccount {
    pub auth_key: Vec<u8>,
    pub dc_id: i32,
    pub user_id: i64,
    pub source_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Format {
    Telethon,
    Pyrogram,
    Tdata,
    TdataZip,
    AuthKey,
}

impl Format {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "telethon" => Ok(Format::Telethon),
            "pyrogram" => Ok(Format::Pyrogram),
            "tdata" => Ok(Format::Tdata),
            "tdata_zip" => Ok(Format::TdataZip),
            "authkey" => Ok(Format::AuthKey),
            other => Err(t_with("converter_unknown_format", &[("format", other)])),
        }
    }
}

pub async fn read_inputs(paths: &[PathBuf], format: Format, app: &AppHandle) -> Vec<CommonAccount> {
    let emit = |msg: String| {
        let _ = app.emit("converter-log", msg);
    };
    let mut out = Vec::new();

    for path in paths {
        match format {
            Format::Telethon => {
                let files = collect_files_with_ext(path, "session");
                for file in files {
                    match TelethonSession::from_file(&file) {
                        Ok(s) => {
                            let uid = TelethonSession::get_user_id(&file);
                            out.push(CommonAccount {
                                auth_key: s.auth_key,
                                dc_id: s.dc_id,
                                user_id: uid,
                                source_name: file_stem(&file),
                            });
                            emit(t_with(
                                "converter_telethon_read",
                                &[("path", &file.display().to_string())],
                            ));
                        }
                        Err(e) => emit(t_with(
                            "converter_telethon_error",
                            &[("path", &file.display().to_string()), ("error", &e)],
                        )),
                    }
                }
            }
            Format::Pyrogram => {
                let files = collect_files_with_ext(path, "session");
                for file in files {
                    match PyroSession::from_file(&file) {
                        Ok(s) => {
                            out.push(CommonAccount {
                                auth_key: s.auth_key,
                                dc_id: s.dc_id,
                                user_id: s.user_id,
                                source_name: file_stem(&file),
                            });
                            emit(t_with(
                                "converter_pyrogram_read",
                                &[("path", &file.display().to_string())],
                            ));
                        }
                        Err(e) => emit(t_with(
                            "converter_pyrogram_error",
                            &[("path", &file.display().to_string()), ("error", &e)],
                        )),
                    }
                }
            }
            Format::Tdata => {
                let tdata_dirs = collect_tdata_dirs(path);
                if tdata_dirs.is_empty() {
                    emit(t_with(
                        "converter_tdata_not_found",
                        &[("path", &path.display().to_string())],
                    ));
                }
                for dir in tdata_dirs {
                    match tdata_fmt::parse_tdata(&dir) {
                        Ok(accs) => {
                            let label = file_stem(&dir);
                            for (i, acc) in accs.iter().enumerate() {
                                let suffix = if accs.len() > 1 {
                                    format!("_acc{}", i + 1)
                                } else {
                                    String::new()
                                };
                                out.push(CommonAccount {
                                    auth_key: acc.auth_key.clone(),
                                    dc_id: acc.dc_id,
                                    user_id: acc.user_id,
                                    source_name: format!("{}{}", label, suffix),
                                });
                            }
                            emit(t_with(
                                "converter_tdata_read",
                                &[
                                    ("count", &accs.len().to_string()),
                                    ("path", &dir.display().to_string()),
                                ],
                            ));
                        }
                        Err(e) => emit(t_with(
                            "converter_tdata_error",
                            &[("path", &dir.display().to_string()), ("error", &e)],
                        )),
                    }
                }
            }
            Format::TdataZip => {
                let zips = collect_files_with_ext(path, "zip");
                for zip_file in zips {
                    let temp = std::env::temp_dir()
                        .join("combine_converter")
                        .join(uuid::Uuid::new_v4().to_string());
                    if let Err(e) = extract_zip(&zip_file, &temp) {
                        emit(t_with(
                            "converter_tdatazip_unpack_error",
                            &[("path", &zip_file.display().to_string()), ("error", &e)],
                        ));
                        continue;
                    }
                    let tdata_dirs = collect_tdata_dirs(&temp);
                    let label = file_stem(&zip_file);
                    for dir in tdata_dirs {
                        match tdata_fmt::parse_tdata(&dir) {
                            Ok(accs) => {
                                for (i, acc) in accs.iter().enumerate() {
                                    let suffix = if accs.len() > 1 {
                                        format!("_acc{}", i + 1)
                                    } else {
                                        String::new()
                                    };
                                    out.push(CommonAccount {
                                        auth_key: acc.auth_key.clone(),
                                        dc_id: acc.dc_id,
                                        user_id: acc.user_id,
                                        source_name: format!("{}{}", label, suffix),
                                    });
                                }
                                emit(t_with(
                                    "converter_tdatazip_read",
                                    &[
                                        ("count", &accs.len().to_string()),
                                        ("path", &zip_file.display().to_string()),
                                    ],
                                ));
                            }
                            Err(e) => emit(t_with(
                                "converter_tdatazip_error",
                                &[("path", &dir.display().to_string()), ("error", &e)],
                            )),
                        }
                    }
                }
            }
            Format::AuthKey => {
                let files = collect_files_with_ext(path, "txt");
                for file in files {
                    let content = match std::fs::read_to_string(&file) {
                        Ok(c) => c,
                        Err(e) => {
                            emit(t_with(
                                "converter_authkey_read_error",
                                &[
                                    ("path", &file.display().to_string()),
                                    ("error", &e.to_string()),
                                ],
                            ));
                            continue;
                        }
                    };
                    let mut count = 0usize;
                    for (line_num, raw_line) in content.lines().enumerate() {
                        let line = raw_line.trim();
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        let (key_part, dc_explicit) = split_authkey_dc(line);
                        let auth_key_bytes = match hex_to_bytes(key_part) {
                            Some(b) if b.len() == 256 => b,
                            _ => {
                                emit(t_with(
                                    "converter_authkey_invalid_hex",
                                    &[
                                        ("path", &file.display().to_string()),
                                        ("line", &(line_num + 1).to_string()),
                                    ],
                                ));
                                continue;
                            }
                        };
                        let dc_id = if let Some(dc) = dc_explicit {
                            dc
                        } else {
                            match probe_dc(&auth_key_bytes).await {
                                Some(dc) => dc,
                                None => {
                                    emit(t_with(
                                        "converter_authkey_no_dc",
                                        &[
                                            ("path", &file.display().to_string()),
                                            ("line", &(line_num + 1).to_string()),
                                        ],
                                    ));
                                    continue;
                                }
                            }
                        };
                        out.push(CommonAccount {
                            auth_key: auth_key_bytes,
                            dc_id,
                            user_id: 0,
                            source_name: format!("{}_{}", file_stem(&file), line_num + 1),
                        });
                        count += 1;
                    }
                    emit(t_with(
                        "converter_authkey_read",
                        &[
                            ("count", &count.to_string()),
                            ("path", &file.display().to_string()),
                        ],
                    ));
                }
            }
        }
    }

    out
}

fn split_authkey_dc(line: &str) -> (&str, Option<i32>) {
    if let Some((key, rest)) = line.rsplit_once(':') {
        if let Ok(dc) = rest.trim().parse::<i32>() {
            if (1..=5).contains(&dc) {
                return (key.trim(), Some(dc));
            }
        }
    }
    (line.trim(), None)
}

async fn probe_dc(auth_key_bytes: &[u8]) -> Option<i32> {
    let proxy = match crate::proxy::select_proxy_for_account(None) {
        Ok(p) => p,
        Err(_) => return None, // no proxy available and allow_no_proxy=false
    };
    let mut auth_key = [0u8; 256];
    auth_key.copy_from_slice(auth_key_bytes);

    let cfg = crate::get_app_config();
    for dc in 1..=5i32 {
        if let Some(addr) = cfg.dc_addresses.get(&dc) {
            if crate::mtproto::client::MtpClient::connect(addr, &auth_key, proxy.as_ref())
                .await
                .is_ok()
            {
                return Some(dc);
            }
        }
    }
    None
}

pub fn write_account(
    acc: &CommonAccount,
    output_dir: &Path,
    target: Format,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(output_dir).map_err(|e| format!("mkdir: {e}"))?;

    match target {
        Format::Telethon => {
            let path = output_dir.join(format!("{}.session", acc.source_name));
            let session = TelethonSession {
                dc_id: acc.dc_id,
                server_address: dc_host_from_config(acc.dc_id),
                port: 443,
                auth_key: acc.auth_key.clone(),
            };
            session.to_file(&path)?;
            Ok(path)
        }
        Format::Pyrogram => {
            let path = output_dir.join(format!("{}.session", acc.source_name));
            let session = PyroSession {
                dc_id: acc.dc_id,
                api_id: 2040,
                test_mode: false,
                auth_key: acc.auth_key.clone(),
                user_id: acc.user_id,
                is_bot: false,
            };
            session.to_file(&path)?;
            Ok(path)
        }
        Format::Tdata => {
            if acc.user_id == 0 {
                return Err(t("converter_no_userid"));
            }
            let phone_clean = acc.source_name.trim_start_matches('+');
            let dir = output_dir.join(format!("{}_tdata", phone_clean));
            let tdata_acc = tdata_fmt::TDataAccount {
                dc_id: acc.dc_id,
                user_id: acc.user_id,
                auth_key: acc.auth_key.clone(),
            };
            tdata_fmt::write_tdata(&dir, &tdata_acc)?;
            Ok(dir)
        }
        Format::TdataZip => Err(t("converter_tdatazip_unsupported_out")),
        Format::AuthKey => {
            // output_dir is actually the output file path for authkey format
            let line = format!("{}:{}\n", bytes_to_hex(&acc.auth_key), acc.dc_id);
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(output_dir)
                .map_err(|e| t_with("converter_open_file_error", &[("error", &e.to_string())]))?;
            file.write_all(line.as_bytes())
                .map_err(|e| t_with("converter_write_error", &[("error", &e.to_string())]))?;
            Ok(output_dir.to_path_buf())
        }
    }
}

pub fn add_to_panel(acc: &CommonAccount) -> Result<String, String> {
    let storage = crate::accounts::commands::get_storage_pub();

    // dedup by auth_key
    let existing = crate::accounts::commands::collect_existing_auth_keys_pub(&storage);
    if existing.contains(&acc.auth_key) {
        return Err("duplicate".to_string());
    }

    let id = uuid::Uuid::new_v4().to_string();

    let session = TelethonSession {
        dc_id: acc.dc_id,
        server_address: dc_host_from_config(acc.dc_id),
        port: 443,
        auth_key: acc.auth_key.clone(),
    };
    session.to_file(&storage.session_path(&id))?;

    let cfg = crate::get_app_config();
    let dev = crate::accounts::devices::generate_random_device();
    let json = crate::accounts::session::AccountJson {
        app_id: cfg.app_id,
        app_hash: cfg.app_hash.clone(),
        sdk: dev.sdk,
        device: dev.device,
        app_version: dev.app_version,
        lang_pack: "en".to_string(),
        system_lang_pack: "en-US".to_string(),
        user_id: acc.user_id,
        ..Default::default()
    };
    json.to_file(&storage.json_path(&id))?;

    // only write tdata if user_id is known
    if acc.user_id != 0 {
        let tdata_acc = tdata_fmt::TDataAccount {
            dc_id: acc.dc_id,
            user_id: acc.user_id,
            auth_key: acc.auth_key.clone(),
        };
        let _ = tdata_fmt::write_tdata(&storage.tdata_dir(&id), &tdata_acc);
    }

    Ok(id)
}

pub async fn run_conversion(
    paths: Vec<String>,
    from_format: Format,
    to_format: Format,
    output_dir: PathBuf,
    add_to_panel_flag: bool,
    threads: u32,
    app: &AppHandle,
    token: &AtomicBool,
) {
    let emit = |msg: String| {
        let _ = app.emit("converter-log", msg);
    };

    if matches!(to_format, Format::TdataZip) {
        emit(t("converter_tdatazip_unsupported_target"));
        let _ = app.emit("converter-done", "0/0");
        return;
    }

    // authkey -> authkey special case: just ensure dc_id is present
    if from_format == Format::AuthKey && to_format == Format::AuthKey {
        run_authkey_to_authkey(&paths, &output_dir, threads, &app, token).await;
        return;
    }

    let path_bufs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    emit(t_with(
        "converter_reading_paths",
        &[
            ("count", &path_bufs.len().to_string()),
            ("format", &format!("{:?}", from_format)),
        ],
    ));

    let accounts = read_inputs(&path_bufs, from_format, &app).await;
    emit(t_with(
        "converter_accounts_found",
        &[("count", &accounts.len().to_string())],
    ));

    if accounts.is_empty() {
        let _ = app.emit("converter-done", "0/0");
        return;
    }

    // for authkey output, ensure parent dir exists; for others, create output dir
    if to_format == Format::AuthKey {
        if let Some(parent) = output_dir.parent() {
            std::fs::create_dir_all(parent).ok();
        }
    } else {
        if let Err(e) = std::fs::create_dir_all(&output_dir) {
            emit(t_with(
                "converter_outdir_error",
                &[("error", &e.to_string())],
            ));
            let _ = app.emit("converter-done", "0/0");
            return;
        }
    }

    let success = Arc::new(AtomicU32::new(0));
    let errors = Arc::new(AtomicU32::new(0));
    let concurrency = (threads.max(1).min(1000)) as usize;
    let sem = Arc::new(Semaphore::new(concurrency));

    let mut handles = Vec::new();
    let total = accounts.len();
    for (i, acc) in accounts.into_iter().enumerate() {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let sem = sem.clone();
        let app = app.clone();
        let out = output_dir.clone();
        let success = success.clone();
        let errors = errors.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();

            let res = tokio::task::spawn_blocking({
                let acc = acc.clone();
                let out = out.clone();
                move || write_account(&acc, &out, to_format)
            })
            .await
            .unwrap_or_else(|e| Err(format!("join error: {e}")));

            match res {
                Ok(p) => {
                    success.fetch_add(1, Ordering::Relaxed);
                    let _ = app.emit(
                        "converter-log",
                        format!(
                            "[{}/{}] {} -> {}",
                            i + 1,
                            total,
                            t("converter_ok_short"),
                            p.display()
                        ),
                    );
                    let _ = app.emit("converter-stats", "ok");

                    if add_to_panel_flag {
                        match add_to_panel(&acc) {
                            Ok(id) => {
                                let _ = app.emit(
                                    "converter-log",
                                    t_with("converter_added_to_panel", &[("id", &id.to_string())]),
                                );
                            }
                            Err(e) => {
                                let _ = app.emit(
                                    "converter-log",
                                    t_with("converter_not_added_to_panel", &[("error", &e)]),
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    errors.fetch_add(1, Ordering::Relaxed);
                    let _ = app.emit(
                        "converter-log",
                        format!(
                            "[{}/{}] {} ({}): {}",
                            i + 1,
                            total,
                            t("error"),
                            acc.source_name,
                            e
                        ),
                    );
                    let _ = app.emit("converter-stats", "err");
                }
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    if add_to_panel_flag {
        crate::accounts::commands::invalidate_accounts_cache();
    }

    let s = success.load(Ordering::Relaxed);
    let e = errors.load(Ordering::Relaxed);
    let _ = app.emit(
        "converter-log",
        t_with(
            "converter_summary",
            &[("ok", &s.to_string()), ("err", &e.to_string())],
        ),
    );
    let _ = app.emit("converter-done", format!("{}/{}", s, e));
}

// helpers

fn file_stem(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "account".to_string())
}

fn collect_files_with_ext(path: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if path.is_file() {
        if path.extension().map(|e| e == ext).unwrap_or(false) {
            out.push(path.to_path_buf());
        }
        return out;
    }
    if !path.is_dir() {
        return out;
    }
    walk_files(path, &mut |p| {
        if p.extension().map(|e| e == ext).unwrap_or(false) {
            out.push(p.to_path_buf());
        }
    });
    out
}

fn walk_files(dir: &Path, cb: &mut dyn FnMut(&Path)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_files(&p, cb);
        } else {
            cb(&p);
        }
    }
}

fn collect_tdata_dirs(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if path.is_dir() && is_tdata_root(path) {
        out.push(path.to_path_buf());
        return out;
    }
    walk_dirs(path, &mut |p| {
        if is_tdata_root(p) {
            out.push(p.to_path_buf());
        }
    });
    out
}

fn is_tdata_root(p: &Path) -> bool {
    if !p.is_dir() {
        return false;
    }
    for name in &["key_datas", "key_data1", "key_data0"] {
        if p.join(name).exists() {
            return true;
        }
    }
    let sub = p.join("tdata");
    if sub.is_dir() {
        for name in &["key_datas", "key_data1", "key_data0"] {
            if sub.join(name).exists() {
                return true;
            }
        }
    }
    false
}

fn walk_dirs(dir: &Path, cb: &mut dyn FnMut(&Path)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if is_tdata_root(&p) {
                cb(&p);
            } else {
                walk_dirs(&p, cb);
            }
        }
    }
}

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    if size > 500 * 1024 * 1024 {
        return Err("zip > 500 MB".into());
    }
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if total > 1024 * 1024 * 1024 {
            return Err(t("converter_unpack_over_limit"));
        }
        let out = dest.join(entry.mangled_name());
        if entry.is_dir() {
            std::fs::create_dir_all(&out).ok();
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
            let written = std::io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
            total += written;
        }
    }
    Ok(())
}

// authkey -> authkey: read lines, add dc_id where missing, write to output
async fn run_authkey_to_authkey(
    paths: &[String],
    output_file: &Path,
    threads: u32,
    app: &AppHandle,
    token: &AtomicBool,
) {
    let emit = |msg: String| {
        let _ = app.emit("converter-log", msg);
    };

    let mut lines_needing_dc: Vec<(String, usize)> = Vec::new(); // (hex, line_num)
    let mut complete_lines: Vec<String> = Vec::new();
    let mut total_lines = 0usize;

    for path_str in paths {
        if !token.load(Ordering::Relaxed) {
            return;
        }
        let content = match std::fs::read_to_string(path_str) {
            Ok(c) => c,
            Err(e) => {
                emit(t_with(
                    "converter_read_error",
                    &[("path", path_str), ("error", &e.to_string())],
                ));
                continue;
            }
        };
        for (i, raw_line) in content.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            total_lines += 1;
            let (key_part, dc_explicit) = split_authkey_dc(line);
            if let Some(bytes) = hex_to_bytes(key_part) {
                if bytes.len() == 256 {
                    if let Some(dc) = dc_explicit {
                        complete_lines.push(format!("{}:{}", key_part, dc));
                    } else {
                        lines_needing_dc.push((key_part.to_string(), i + 1));
                    }
                } else {
                    emit(t_with(
                        "converter_invalid_authkey_len",
                        &[
                            ("line", &(i + 1).to_string()),
                            ("bytes", &bytes.len().to_string()),
                        ],
                    ));
                }
            } else {
                emit(t_with(
                    "converter_invalid_hex_line",
                    &[("line", &(i + 1).to_string())],
                ));
            }
        }
    }

    if lines_needing_dc.is_empty() && !complete_lines.is_empty() {
        emit(t_with(
            "converter_all_complete",
            &[("count", &complete_lines.len().to_string())],
        ));
    } else if !lines_needing_dc.is_empty() {
        emit(t_with(
            "converter_detecting_dc",
            &[
                ("count", &lines_needing_dc.len().to_string()),
                ("threads", &threads.to_string()),
            ],
        ));
    }

    // probe DC for lines that need it
    let concurrency = threads.max(1).min(1000) as usize;
    let sem = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::new();

    for (hex, line_num) in lines_needing_dc {
        if !token.load(Ordering::Relaxed) {
            break;
        }
        let sem = sem.clone();
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let bytes = hex_to_bytes(&hex).unwrap();
            match probe_dc(&bytes).await {
                Some(dc) => {
                    let _ = app.emit(
                        "converter-log",
                        t_with(
                            "converter_line_dc",
                            &[("line", &line_num.to_string()), ("dc", &dc.to_string())],
                        ),
                    );
                    let _ = app.emit("converter-stats", "ok");
                    Some(format!("{}:{}", hex, dc))
                }
                None => {
                    let _ = app.emit(
                        "converter-log",
                        t_with("converter_line_no_dc", &[("line", &line_num.to_string())]),
                    );
                    let _ = app.emit("converter-stats", "err");
                    None
                }
            }
        }));
        let jitter = rand::random::<u64>() % 500;
        tokio::time::sleep(std::time::Duration::from_millis(500 + jitter)).await;
    }

    for h in handles {
        if let Ok(Some(line)) = h.await {
            complete_lines.push(line);
        }
    }

    // write output
    if let Some(parent) = output_file.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let content = complete_lines.join("\n") + "\n";
    match std::fs::write(output_file, &content) {
        Ok(_) => emit(t_with(
            "converter_lines_written",
            &[
                ("count", &complete_lines.len().to_string()),
                ("path", &output_file.display().to_string()),
            ],
        )),
        Err(e) => emit(t_with(
            "converter_write_error_short",
            &[("error", &e.to_string())],
        )),
    }

    let _ = app.emit(
        "converter-done",
        format!(
            "{}/{}",
            complete_lines.len(),
            total_lines - complete_lines.len()
        ),
    );
}

fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    let hex = hex.trim();
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[tauri::command]
pub async fn converter_start(
    paths: Vec<String>,
    from_format: String,
    to_format: String,
    output_dir: String,
    add_to_panel: bool,
    threads: u32,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    dbg_log!(
        "converter_start: {} paths, {} -> {}, threads={}",
        paths.len(),
        from_format,
        to_format,
        threads
    );

    let from = Format::parse(&from_format)?;
    let to = Format::parse(&to_format)?;
    let out = std::path::PathBuf::from(&output_dir);

    let queue: tauri::State<'_, TaskQueue> = app_handle.state();
    let token = queue
        .register_task(
            "converter".to_string(),
            "converter".to_string(),
            t("converter_task_name"),
        )
        .await;

    tokio::spawn(async move {
        run_conversion(
            paths,
            from,
            to,
            out,
            add_to_panel,
            threads,
            &app_handle,
            &token,
        )
        .await;
        let queue: tauri::State<'_, TaskQueue> = app_handle.state();
        queue.finish_task(&"converter".to_string(), true).await;
    });

    Ok(())
}

#[tauri::command]
pub async fn converter_stop(app_handle: tauri::AppHandle) -> Result<(), String> {
    let queue: tauri::State<'_, TaskQueue> = app_handle.state();
    queue.stop_task(&"converter".to_string()).await;
    Ok(())
}
