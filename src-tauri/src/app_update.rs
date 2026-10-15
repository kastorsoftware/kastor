use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::Emitter;
use tauri::Manager;

const RELEASE_API: &str = "https://api.github.com/repos/kastorsoftware/kastor/releases/latest";
const USER_AGENT: &str = "Kastor-updater";
const MAX_UPDATE_BYTES: u64 = 512 * 1024 * 1024;

fn update_agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into()
}

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
    let release: GithubRelease = update_agent(Duration::from_secs(20))
        .get(RELEASE_API)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("GitHub release request failed: {e}"))?
        .into_body()
        .read_json()
        .map_err(|e| format!("GitHub release response is invalid: {e}"))?;

    update_from_release(release, env!("CARGO_PKG_VERSION"))
}

fn update_from_release(
    release: GithubRelease,
    current_version: &str,
) -> Result<Option<UpdateInfo>, String> {
    if release.draft || release.prerelease {
        return Ok(None);
    }

    let version = release.tag_name.trim_start_matches('v');
    let current = semver::Version::parse(current_version)
        .map_err(|e| format!("current version is invalid: {e}"))?;
    let available =
        semver::Version::parse(version).map_err(|e| format!("release version is invalid: {e}"))?;
    if available <= current || !available.pre.is_empty() {
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

    let asset_prefix = format!(
        "https://github.com/kastorsoftware/kastor/releases/download/{}/",
        release.tag_name
    );
    if download_url != format!("{asset_prefix}{asset_name}")
        || checksum_url != format!("{asset_prefix}{checksum_name}")
    {
        return Err("update assets must belong to the official Kastor release".into());
    }

    Ok(Some(UpdateInfo {
        current_version: current_version.to_string(),
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
    ensure_idle(&app).await?;
    let update = state
        .0
        .lock()
        .map_err(|_| "update state lock poisoned".to_string())?
        .clone()
        .ok_or_else(|| "no checked update is available".to_string())?;
    let current_exe = std::env::current_exe().map_err(|e| format!("current exe path: {e}"))?;
    let download_target = current_exe.clone();
    let download_app = app.clone();
    let new_exe = tokio::task::spawn_blocking(move || {
        download_update(&update, &download_target, &download_app)
    })
    .await
    .map_err(|e| format!("update download task failed: {e}"))??;

    // A task can start while the update is downloading.
    ensure_idle(&app).await?;
    std::process::Command::new(&new_exe)
        .arg("--replace-previous")
        .arg(&current_exe)
        .spawn()
        .map_err(|e| format!("start downloaded update: {e}"))?;
    std::process::exit(0);
}

async fn ensure_idle(app: &tauri::AppHandle) -> Result<(), String> {
    if crate::get_active_task_count(app.state(), app.state()).await? > 0 {
        return Err("UPDATE_TASKS_ACTIVE".into());
    }
    Ok(())
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

    let checksum = update_agent(Duration::from_secs(20))
        .get(&update.checksum_url)
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

    let body = update_agent(Duration::from_secs(600))
        .get(&update.download_url)
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
        if downloaded_bytes > MAX_UPDATE_BYTES {
            return Err("update exceeds the maximum download size".into());
        }

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
    drop(file);

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
    let Ok(current) = std::env::current_exe().and_then(std::fs::canonicalize) else {
        return;
    };
    let Ok(previous) = std::fs::canonicalize(previous) else {
        return;
    };
    let fallback = std::fs::canonicalize(update_dir()).ok();
    if !allowed_previous_exe(&current, &previous, fallback.as_deref()) {
        dbg_log!("updater: rejected previous executable cleanup path");
        return;
    }
    // Windows may still hold the old executable; cleanup must not delay startup.
    if let Err(error) = std::thread::Builder::new()
        .name("update-cleanup".into())
        .spawn(move || {
            for _ in 0..480 {
                match std::fs::remove_file(&previous) {
                    Ok(()) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                    Err(_) => std::thread::sleep(Duration::from_millis(250)),
                }
            }
        })
    {
        dbg_log!("updater: could not start cleanup worker: {error}");
    }
}

fn allowed_previous_exe(current: &Path, previous: &Path, fallback: Option<&Path>) -> bool {
    let Some(name) = previous.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let known_name = name.eq_ignore_ascii_case("kastor.exe")
        || name
            .strip_prefix("Kastor-v")
            .and_then(|value| value.strip_suffix(".exe"))
            .is_some_and(|version| semver::Version::parse(version).is_ok());
    let expected_current = format!("Kastor-v{}.exe", env!("CARGO_PKG_VERSION"));
    known_name
        && current != previous
        && current.file_name().and_then(|name| name.to_str()) == Some(expected_current.as_str())
        && (current.parent() == previous.parent()
            || current
                .parent()
                .is_some_and(|parent| Some(parent) == fallback))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str) -> GithubRelease {
        GithubRelease {
            tag_name: format!("v{version}"),
            prerelease: false,
            draft: false,
            assets: [
                format!("Kastor-v{version}.exe"),
                format!("Kastor-v{version}.exe.sha256"),
            ]
            .into_iter()
            .map(|name| GithubAsset {
                browser_download_url: format!(
                    "https://github.com/kastorsoftware/kastor/releases/download/v{version}/{name}"
                ),
                name,
            })
            .collect(),
        }
    }

    #[test]
    fn selects_only_new_stable_releases_with_both_official_assets() {
        assert!(update_from_release(release("1.2.0"), "1.1.1")
            .unwrap()
            .is_some());
        assert!(update_from_release(release("1.1.1"), "1.1.1")
            .unwrap()
            .is_none());
        assert!(update_from_release(release("1.0.9"), "1.1.1")
            .unwrap()
            .is_none());
        assert!(update_from_release(release("1.2.0-beta.1"), "1.1.1")
            .unwrap()
            .is_none());
        let mut candidate = release("1.2.0");
        candidate.assets.pop();
        assert!(update_from_release(candidate, "1.1.1").is_err());
        let mut candidate = release("1.2.0");
        candidate.assets[0].browser_download_url = "https://example.com/update.exe".into();
        assert!(update_from_release(candidate, "1.1.1").is_err());
        let mut candidate = release("1.2.0");
        candidate.draft = true;
        assert!(update_from_release(candidate, "1.1.1").unwrap().is_none());
    }

    #[test]
    fn cleanup_rejects_arbitrary_files_and_unrelated_directories() {
        let root = std::env::temp_dir().join("kastor-cleanup-path-test");
        let current = root.join(format!("Kastor-v{}.exe", env!("CARGO_PKG_VERSION")));
        assert!(allowed_previous_exe(
            &current,
            &root.join("kastor.exe"),
            None
        ));
        assert!(allowed_previous_exe(
            &current,
            &root.join("Kastor-v1.0.9.exe"),
            None
        ));
        assert!(!allowed_previous_exe(
            &current,
            &root.join("document.txt"),
            None
        ));
        assert!(!allowed_previous_exe(
            &current,
            &root.join("other.exe"),
            None
        ));
        assert!(!allowed_previous_exe(
            &current,
            &root.join("Kastor-vgarbage.exe"),
            None
        ));
        assert!(!allowed_previous_exe(
            &current,
            &root.join("other/kastor.exe"),
            None
        ));
        assert!(!allowed_previous_exe(&current, &current, None));
        assert!(allowed_previous_exe(
            &current,
            &root.join("other/kastor.exe"),
            Some(&root)
        ));
    }
}
