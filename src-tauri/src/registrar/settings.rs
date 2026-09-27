use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;

const FORMAT_VERSION: u8 = 1;
const PBKDF2_ITERATIONS: u32 = 600_000;
static SETTINGS_FILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Serialize)]
pub struct RegistrarSettingsState {
    pub configured: bool,
}

#[derive(Serialize, Deserialize)]
struct EncryptedSettings {
    version: u8,
    salt: String,
    nonce: String,
    ciphertext: String,
}

fn path() -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("kastor")
        .join("registrar-settings.v1.json")
}

fn validate_password(password: &str) -> Result<(), String> {
    if password.chars().count() < 8 || !password.chars().any(|character| character.is_ascii_digit()) {
        return Err("registrar password must contain at least 8 characters and one digit".to_string());
    }
    Ok(())
}

fn derive_key(password: &str, salt: &[u8]) -> [u8; 32] {
    let mut key = [0_u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, PBKDF2_ITERATIONS, &mut key);
    key
}

#[tauri::command]
pub fn registrar_settings_state() -> RegistrarSettingsState {
    let _guard = SETTINGS_FILE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let target = path();
    let _ = recover_settings_file(&target);
    RegistrarSettingsState { configured: target.is_file() }
}

#[tauri::command]
pub fn registrar_settings_unlock(password: String) -> Result<Value, String> {
    validate_password(&password)?;
    let _guard = SETTINGS_FILE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let target = path();
    recover_settings_file(&target)?;
    let content = std::fs::read_to_string(target).map_err(|_| "registrar settings are not configured".to_string())?;
    let file: EncryptedSettings = serde_json::from_str(&content).map_err(|_| "registrar settings file is damaged".to_string())?;
    if file.version != FORMAT_VERSION {
        return Err("unsupported registrar settings format".to_string());
    }
    let salt = STANDARD.decode(file.salt).map_err(|_| "registrar settings file is damaged".to_string())?;
    let nonce = STANDARD.decode(file.nonce).map_err(|_| "registrar settings file is damaged".to_string())?;
    let ciphertext = STANDARD.decode(file.ciphertext).map_err(|_| "registrar settings file is damaged".to_string())?;
    if salt.len() != 16 || nonce.len() != 12 { return Err("registrar settings file is damaged".to_string()); }
    let key = derive_key(&password, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| "cannot initialize registrar settings encryption".to_string())?;
    let plaintext = cipher.decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| "incorrect registrar password or damaged settings".to_string())?;
    serde_json::from_slice(&plaintext).map_err(|_| "registrar settings file is damaged".to_string())
}

#[tauri::command]
pub fn registrar_settings_save(password: String, settings: Value) -> Result<(), String> {
    validate_password(&password)?;
    let _guard = SETTINGS_FILE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !settings.is_object() { return Err("invalid registrar settings".to_string()); }
    let plaintext = serde_json::to_vec(&settings).map_err(|_| "cannot serialize registrar settings".to_string())?;
    let mut salt = [0_u8; 16];
    let mut nonce = [0_u8; 12];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce);
    let key = derive_key(&password, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| "cannot initialize registrar settings encryption".to_string())?;
    let ciphertext = cipher.encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|_| "cannot encrypt registrar settings".to_string())?;
    let file = EncryptedSettings { version: FORMAT_VERSION, salt: STANDARD.encode(salt), nonce: STANDARD.encode(nonce), ciphertext: STANDARD.encode(ciphertext) };
    let content = serde_json::to_vec(&file).map_err(|_| "cannot serialize registrar settings".to_string())?;
    let target = path();
    recover_settings_file(&target)?;
    let parent = target.parent().ok_or("invalid registrar settings path")?;
    std::fs::create_dir_all(parent).map_err(|_| "cannot create registrar settings directory".to_string())?;
    let temporary = target.with_extension("tmp");
    std::fs::write(&temporary, content).map_err(|_| "cannot save registrar settings".to_string())?;
    replace_settings_file(&temporary, &target)
}

fn replace_settings_file(temporary: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    if !target.exists() {
        return std::fs::rename(temporary, target)
            .map_err(|_| "cannot save registrar settings".to_string());
    }
    let backup = target.with_extension("bak");
    if backup.exists() {
        return Err("registrar settings backup already exists".to_string());
    }
    std::fs::rename(target, &backup)
        .map_err(|_| "cannot back up registrar settings".to_string())?;
    if std::fs::rename(temporary, target).is_err() {
        std::fs::rename(&backup, target)
            .map_err(|_| "cannot restore registrar settings backup".to_string())?;
        return Err("cannot save registrar settings".to_string());
    }
    let _ = std::fs::remove_file(backup);
    Ok(())
}

fn recover_settings_file(target: &std::path::Path) -> Result<(), String> {
    let backup = target.with_extension("bak");
    if !backup.exists() {
        return Ok(());
    }
    if target.exists() {
        let _ = std::fs::remove_file(backup);
        return Ok(());
    }
    std::fs::rename(backup, target)
        .map_err(|_| "cannot restore registrar settings backup".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_replacement_preserves_existing_settings() {
        let directory = std::env::temp_dir().join(format!("registrar-settings-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let target = directory.join("settings.json");
        std::fs::write(&target, b"previous").unwrap();
        assert!(replace_settings_file(&directory.join("missing.tmp"), &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"previous");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn recovers_backup_after_interrupted_replacement() {
        let directory = std::env::temp_dir().join(format!("registrar-settings-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let target = directory.join("settings.json");
        std::fs::write(target.with_extension("bak"), b"previous").unwrap();
        recover_settings_file(&target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"previous");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
