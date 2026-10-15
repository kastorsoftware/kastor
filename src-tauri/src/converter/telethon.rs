// telethon session format (.session = SQLite)
// table: sessions(dc_id, server_address, port, auth_key)

use std::path::Path;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TelethonSession {
    pub dc_id: i32,
    pub server_address: String,
    pub port: i32,
    pub auth_key: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_read_is_read_only_and_rejects_invalid_databases() {
        let directory = std::env::temp_dir().join(format!("kastor-session-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("account.session");
        assert!(TelethonSession::from_file(&path).is_err());
        assert!(!path.exists());
        std::fs::write(&path, b"not a sqlite session").unwrap();
        assert!(TelethonSession::from_file(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not a sqlite session");
        std::fs::remove_file(&path).unwrap();
        let session = TelethonSession { dc_id: 2, server_address: "149.154.167.51".into(), port: 443, auth_key: vec![1; 256] };
        session.to_file(&path).unwrap();
        let restored = TelethonSession::from_file(&path).unwrap();
        assert_eq!(restored.auth_key, session.auth_key);
        assert_eq!(restored.dc_id, session.dc_id);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }
}

impl TelethonSession {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        dbg_log!("TelethonSession::from_file {:?}", path);

        let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| crate::i18n::t_with("converter_telethon_open_session_error", &[("error", &e.to_string())]))?;

        let mut stmt = conn
            .prepare("SELECT dc_id, server_address, port, auth_key FROM sessions LIMIT 1")
            .map_err(|e| {
                crate::i18n::t_with(
                    "converter_telethon_table_error",
                    &[("error", &e.to_string())],
                )
            })?;

        let session = stmt
            .query_row([], |row| {
                Ok(TelethonSession {
                    dc_id: row.get(0)?,
                    server_address: row.get(1)?,
                    port: row.get(2)?,
                    auth_key: row.get(3)?,
                })
            })
            .map_err(|e| {
                crate::i18n::t_with(
                    "converter_telethon_empty_error",
                    &[("error", &e.to_string())],
                )
            })?;

        dbg_log!(
            "TelethonSession: dc_id={} addr={}:{} key_len={}",
            session.dc_id,
            session.server_address,
            session.port,
            session.auth_key.len()
        );

        Ok(session)
    }

    pub fn get_user_id(path: &Path) -> i64 {
        let conn = match rusqlite::Connection::open(path) {
            Ok(c) => c,
            Err(_) => return 0,
        };

        if let Ok(mut stmt) = conn.prepare("SELECT id FROM entities WHERE phone IS NOT NULL AND phone != 0 AND id > 0 ORDER BY date DESC LIMIT 1") {
            if let Ok(uid) = stmt.query_row([], |row| row.get::<_, i64>(0)) {
                if uid > 0 { return uid; }
            }
        }

        if let Ok(mut stmt) = conn.prepare(
            "SELECT id FROM entities WHERE id > 0 AND id < 99999999999 ORDER BY id DESC LIMIT 1",
        ) {
            if let Ok(uid) = stmt.query_row([], |row| row.get::<_, i64>(0)) {
                if uid > 0 {
                    return uid;
                }
            }
        }

        0
    }

    pub fn to_file(&self, path: &Path) -> Result<(), String> {
        dbg_log!("TelethonSession::to_file {:?}", path);

        let conn = rusqlite::Connection::open(path).map_err(|e| {
            crate::i18n::t_with(
                "converter_telethon_create_error",
                &[("error", &e.to_string())],
            )
        })?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS version (version integer primary key);
             CREATE TABLE IF NOT EXISTS sessions (
                 dc_id integer primary key, server_address text,
                 port integer, auth_key blob, takeout_id integer
             );
             CREATE TABLE IF NOT EXISTS entities (
                 id integer primary key, hash integer not null,
                 username text, phone integer, name text, date integer
             );
             CREATE TABLE IF NOT EXISTS sent_files (
                 md5_digest blob, file_size integer, type integer,
                 id integer, hash integer,
                 primary key(md5_digest, file_size, type)
             );
             CREATE TABLE IF NOT EXISTS update_state (
                 id integer primary key, pts integer, qts integer,
                 date integer, seq integer
             );",
        )
        .map_err(|e| {
            crate::i18n::t_with(
                "converter_telethon_schema_error",
                &[("error", &e.to_string())],
            )
        })?;

        conn.execute("INSERT OR REPLACE INTO version VALUES (7)", [])
            .map_err(|e| {
                crate::i18n::t_with(
                    "converter_telethon_version_error",
                    &[("error", &e.to_string())],
                )
            })?;

        conn.execute(
            "INSERT OR REPLACE INTO sessions (dc_id, server_address, port, auth_key, takeout_id) VALUES (?1, ?2, ?3, ?4, NULL)",
            rusqlite::params![self.dc_id, self.server_address, self.port, self.auth_key],
        ).map_err(|e| crate::i18n::t_with("converter_telethon_write_error", &[("error", &e.to_string())]))?;

        Ok(())
    }
}
