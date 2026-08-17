use std::path::{Path, PathBuf};
use std::fs;

pub struct AccountStorage {
    base: PathBuf,
}

impl AccountStorage {
    pub fn new(base: &Path) -> Self {
        let storage = Self { base: base.to_path_buf() };
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
}
