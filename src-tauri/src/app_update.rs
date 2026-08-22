use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::Emitter;

const RELEASE_API: &str = "https://api.github.com/repos/kastorsoftware/kastor/releases/latest";
const USER_AGENT: &str = "Kastor-updater";

#[derive(Clone, Serialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub version: String,
    pub download_url: String,
    pub checksum_url: String,
}

pub struct UpdateState(pub Mutex<Option<UpdateInfo>>);

#[derive(Clone, Serialize)]
struct DownloadProgress {
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    prerelease: bool,
    draft: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[tauri::command]
pub async fn check_for_update(
    state: tauri::State<'_, UpdateState>,
) -> Result<Option<UpdateInfo>, String> {
    let update = tokio::task::spawn_blocking(check_for_update_sync)
        .await
        .map_err(|e| format!("update check task failed: {e}"))??;
    *state
        .0
        .lock()
        .map_err(|_| "update state lock poisoned".to_string())? = update.clone();
    Ok(update)
}

fn check_for_update_sync() -> Result<Option<UpdateInfo>, String> {
    let release: GithubRelease = ureq::get(RELEASE_API)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("GitHub release request failed: {e}"))?
        .into_body()
        .read_json()
        .map_err(|e| format!("GitHub release response is invalid: {e}"))?;

    if release.draft || release.prerelease {
        return Ok(None);
    }

    let version = release.tag_name.trim_start_matches('v');
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|e| format!("current version is invalid: {e}"))?;
    let available =
        semver::Version::parse(version).map_err(|e| format!("release version is invalid: {e}"))?;
    if available <= current {
        return Ok(None);
    }

    let asset_name = format!("Kastor-v{version}.exe");
    let checksum_name = format!("{asset_name}.sha256");
    let download_url = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| format!("release asset {asset_name} is missing"))?;
    let checksum_url = release
        .assets
        .iter()
        .find(|asset| asset.name == checksum_name)
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| format!("release checksum {checksum_name} is missing"))?;

    Ok(Some(UpdateInfo {
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        version: version.to_string(),
        download_url,
        checksum_url,
    }))
}

#[tauri::command]
pub async fn download_and_apply_update(
    state: tauri::State<'_, UpdateState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let update = state
        .0
        .lock()
        .map_err(|_| "update state lock poisoned".to_string())?
        .clone()
        .ok_or_else(|| "no checked update is available".to_string())?;
    let current_exe = std::env::current_exe().map_err(|e| format!("current exe path: {e}"))?;
    let download_target = current_exe.clone();
    let new_exe = tokio::task::spawn_blocking(move || download_update(&update, &download_target, &app))
        .await
        .map_err(|e| format!("update download task failed: {e}"))??;

    std::process::Command::new(&new_exe)
        .arg("--replace-previous")
        .arg(&current_exe)
        .spawn()
        .map_err(|e| format!("start downloaded update: {e}"))?;
    std::process::exit(0);
}

fn download_update(
    update: &UpdateInfo,
    current_exe: &Path,
    app: &tauri::AppHandle,
) -> Result<PathBuf, String> {
    let target_dir = writable_parent(current_exe).unwrap_or_else(update_dir);
    std::fs::create_dir_all(&target_dir).map_err(|e| format!("create update directory: {e}"))?;
    let target = target_dir.join(format!("Kastor-v{}.exe", update.version));
    let temporary = target.with_extension("exe.part");

    let checksum = ureq::get(&update.checksum_url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("download update checksum: {e}"))?
        .into_body()
        .read_to_string()
        .map_err(|e| format!("read update checksum: {e}"))?;
    let expected = checksum
        .split_whitespace()
        .next()
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| "update checksum format is invalid".to_string())?;

    let body = ureq::get(&update.download_url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("download update: {e}"))?
        .into_body();
    let total_bytes = body.content_length();
    let mut reader = body.into_reader();
    let mut file = std::fs::File::create(&temporary).map_err(|e| format!("create update: {e}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded_bytes = 0u64;
    let mut last_reported_percent = None;
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("read update: {e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("write update: {e}"))?;
        hasher.update(&buffer[..read]);
        downloaded_bytes += read as u64;

        let percent = total_bytes.map(|total| {
            if total == 0 {
                100
            } else {
                ((downloaded_bytes.saturating_mul(100) / total).min(100)) as u8
            }
        });
        if percent != last_reported_percent {
            let _ = app.emit(
                "update-download-progress",
                DownloadProgress {
                    downloaded_bytes,
                    total_bytes,
                },
            );
            last_reported_percent = percent;
        }
    }
    file.flush().map_err(|e| format!("flush update: {e}"))?;

    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err("downloaded update checksum does not match".into());
    }

    let _ = app.emit(
        "update-download-progress",
        DownloadProgress {
            downloaded_bytes,
            total_bytes,
        },
    );
    std::fs::rename(&temporary, &target).map_err(|e| format!("finalize update: {e}"))?;
    Ok(target)
}

fn writable_parent(current_exe: &Path) -> Option<PathBuf> {
    let parent = current_exe.parent()?.to_path_buf();
    let probe = parent.join(format!(".kastor-write-test-{}", std::process::id()));
    std::fs::write(&probe, []).ok()?;
    std::fs::remove_file(probe).ok()?;
    Some(parent)
}

fn update_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kastor")
        .join("updates")
}

pub fn apply_previous_cleanup_from_args() {
    let args: Vec<_> = std::env::args_os().collect();
    let Some(index) = args.iter().position(|arg| arg == "--replace-previous") else {
        return;
    };
    let Some(previous) = args.get(index + 1).map(PathBuf::from) else {
        return;
    };

    for _ in 0..480 {
        match std::fs::remove_file(&previous) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => std::thread::sleep(Duration::from_millis(250)),
        }
    }
}
