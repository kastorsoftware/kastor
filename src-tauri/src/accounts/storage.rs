use std::fs;
use std::path::{Path, PathBuf};

use super::session::AccountJson;

pub struct AccountStorage {
    base: PathBuf,
}

impl AccountStorage {
    pub fn new(base: &Path) -> Self {
        let storage = Self {
            base: base.to_path_buf(),
        };
        fs::create_dir_all(storage.session_json_dir()).ok();
        fs::create_dir_all(storage.tdatas_dir()).ok();
        storage
    }

    pub fn session_json_dir(&self) -> PathBuf {
        self.base.join("session_json")
    }

    pub fn tdatas_dir(&self) -> PathBuf {
        self.base.join("tdatas")
    }

    pub fn session_path(&self, id: &str) -> PathBuf {
        self.session_json_dir().join(format!("{id}.session"))
    }

    pub fn json_path(&self, id: &str) -> PathBuf {
        self.session_json_dir().join(format!("{id}.json"))
    }

    pub fn tdata_dir(&self, id: &str) -> PathBuf {
        self.tdatas_dir().join(id)
    }

    pub fn export_dir(&self) -> PathBuf {
        self.base.parent().unwrap_or(&self.base).join("export")
    }

    pub fn create_export_dir(&self) -> Result<PathBuf, String> {
        let root = self.export_dir();
        fs::create_dir_all(&root).map_err(|e| format!("create export directory: {e}"))?;
        // Each operation owns a new directory; previous exports are user data.
        let destination = root.join(format!("export-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&destination).map_err(|e| format!("create export directory: {e}"))?;
        Ok(destination)
    }

    pub fn contains_phone(&self, phone: &str) -> Result<bool, String> {
        let expected = phone_digits(phone);
        if expected.is_empty() {
            return Ok(false);
        }
        let entries = fs::read_dir(self.session_json_dir())
            .map_err(|_| "Failed to inspect stored accounts")?;
        for entry in entries {
            let entry = entry.map_err(|_| "Failed to inspect stored accounts")?;
            let path = entry.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "session")
            {
                continue;
            }
            let metadata = entry
                .metadata()
                .map_err(|_| "Failed to inspect stored accounts")?;
            if !metadata.is_file() {
                continue;
            }
            let json_path = path.with_extension("json");
            match fs::read(&json_path) {
                Ok(bytes) => {
                    if let Ok(account) = serde_json::from_slice::<AccountJson>(&bytes) {
                        if phone_digits(&account.phone) == expected {
                            return Ok(true);
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("Failed to inspect stored accounts".into()),
            }
        }
        Ok(false)
    }
}

fn phone_digits(phone: &str) -> String {
    phone.chars().filter(char::is_ascii_digit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_do_not_replace_previous_results() {
        let root =
            std::env::temp_dir().join(format!("kastor-export-test-{}", uuid::Uuid::new_v4()));
        let storage = AccountStorage::new(&root.join("accounts"));
        let first = storage.create_export_dir().unwrap();
        fs::write(first.join("account.session"), b"previous export").unwrap();
        let second = storage.create_export_dir().unwrap();
        assert_ne!(first, second);
        assert_eq!(
            fs::read(first.join("account.session")).unwrap(),
            b"previous export"
        );
        assert!(second.starts_with(root.join("export")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_session_phone_matches_across_formatting() {
        let directory =
            std::env::temp_dir().join(format!("account-phone-test-{}", uuid::Uuid::new_v4()));
        let storage = AccountStorage::new(&directory);
        let id = uuid::Uuid::new_v4().to_string();
        fs::write(storage.json_path(&id), br#"{"phone":"+1 (234) 567-8901"}"#).unwrap();
        assert!(!storage.contains_phone("12345678901").unwrap());
        fs::write(storage.session_path(&id), b"session fixture").unwrap();
        assert!(storage.contains_phone("+1 234 567 8901").unwrap());
        assert!(!storage.contains_phone("12345678902").unwrap());
        fs::remove_dir_all(directory).unwrap();
    }
}
