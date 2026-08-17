// pyrogram session format (.session = SQLite)
// table: sessions(dc_id, api_id, test_mode, auth_key, date, user_id, is_bot)

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA: &str = "
CREATE TABLE sessions (
    dc_id     INTEGER PRIMARY KEY,
    api_id    INTEGER,
    test_mode INTEGER,
    auth_key  BLOB,
    date      INTEGER NOT NULL,
    user_id   INTEGER,
    is_bot    INTEGER
);
CREATE TABLE peers (
    id             INTEGER PRIMARY KEY,
    access_hash    INTEGER,
    type           INTEGER NOT NULL,
    username       TEXT,
    phone_number   TEXT,
    last_update_on INTEGER NOT NULL DEFAULT (CAST(STRFTIME('%s', 'now') AS INTEGER))
);
CREATE TABLE version (
    number INTEGER PRIMARY KEY
);
CREATE INDEX idx_peers_id ON peers (id);
CREATE INDEX idx_peers_username ON peers (username);
CREATE INDEX idx_peers_phone_number ON peers (phone_number);
CREATE TRIGGER trg_peers_last_update_on
    AFTER UPDATE
    ON peers
BEGIN
    UPDATE peers
    SET last_update_on = CAST(STRFTIME('%s', 'now') AS INTEGER)
    WHERE id = NEW.id;
END;
";

#[derive(Debug, Clone)]
pub struct PyroSession {
    pub dc_id: i32,
    pub api_id: i32,
    pub test_mode: bool,
    pub auth_key: Vec<u8>,
    pub user_id: i64,
    pub is_bot: bool,
}

impl PyroSession {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        dbg_log!("PyroSession::from_file {:?}", path);
        let conn = rusqlite::Connection::open(path)
            .map_err(|e| crate::i18n::t_with("converter_pyro_open_error", &[("error", &e.to_string())]))?;

        let mut stmt = conn
            .prepare("SELECT dc_id, COALESCE(api_id, 0), COALESCE(test_mode, 0), auth_key, COALESCE(user_id, 0), COALESCE(is_bot, 0) FROM sessions LIMIT 1")
            .map_err(|e| crate::i18n::t_with("converter_pyro_table_error", &[("error", &e.to_string())]))?;

        let session = stmt
            .query_row([], |row| {
                let auth_key_blob: Option<Vec<u8>> = row.get(3).ok();
                let auth_key = if let Some(blob) = auth_key_blob {
                    blob
                } else {
                    let hex_str: String = row.get(3)?;
                    hex_to_bytes(&hex_str).unwrap_or_default()
                };
                Ok(PyroSession {
                    dc_id: row.get(0)?,
                    api_id: row.get(1)?,
                    test_mode: row.get::<_, i32>(2)? != 0,
                    auth_key,
                    user_id: row.get(4)?,
                    is_bot: row.get::<_, i32>(5)? != 0,
                })
            })
            .map_err(|e| crate::i18n::t_with("converter_pyro_empty_table", &[("error", &e.to_string())]))?;

        if session.auth_key.len() != 256 {
            return Err(crate::i18n::t_with("converter_pyro_authkey_size", &[("bytes", &session.auth_key.len().to_string())]));
        }

        Ok(session)
    }

    pub fn to_file(&self, path: &Path) -> Result<(), String> {
        dbg_log!("PyroSession::to_file {:?}", path);
        if path.exists() { std::fs::remove_file(path).ok(); }

        let conn = rusqlite::Connection::open(path)
            .map_err(|e| crate::i18n::t_with("converter_pyro_create_error", &[("error", &e.to_string())]))?;

        conn.execute_batch(SCHEMA)
            .map_err(|e| crate::i18n::t_with("converter_pyro_schema_error", &[("error", &e.to_string())]))?;

        let date = SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64).unwrap_or(0);

        conn.execute(
            "INSERT INTO sessions (dc_id, api_id, test_mode, auth_key, date, user_id, is_bot) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                self.dc_id, self.api_id,
                if self.test_mode { 1 } else { 0 },
                self.auth_key, date, self.user_id,
                if self.is_bot { 1 } else { 0 },
            ],
        ).map_err(|e| crate::i18n::t_with("converter_pyro_write_error", &[("error", &e.to_string())]))?;

        conn.execute("INSERT INTO version (number) VALUES (3)", [])
            .map_err(|e| crate::i18n::t_with("converter_pyro_version_error", &[("error", &e.to_string())]))?;

        Ok(())
    }
}

fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    let hex = hex.trim();
    if hex.len() % 2 != 0 { return None; }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i+2], 16).ok())
        .collect()
}
