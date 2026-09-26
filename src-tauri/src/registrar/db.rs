use std::path::PathBuf;

use rusqlite::{params, Connection};

pub struct RegistrarDatabase {
    connection: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationRecord {
    pub phone: String,
    pub provider: String,
    pub activation_id: Option<String>,
    pub status: String,
    pub account_id: Option<String>,
    pub error: Option<String>,
}

impl RegistrarDatabase {
    pub fn create(task_id: &str) -> Result<(Self, PathBuf), String> {
        validate_task_id(task_id)?;
        let directory = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("kastor")
            .join("registrar");
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("create registrar directory: {error}"))?;
        let path = directory.join(format!("{task_id}.db"));
        let database = Self::open(&path)?;
        Ok((database, path))
    }

    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        let connection = Connection::open(path).map_err(|error| format!("open registrar database: {error}"))?;
        connection
            .execute_batch(
                "
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                CREATE TABLE IF NOT EXISTS registrations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    phone TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    activation_id TEXT,
                    status TEXT NOT NULL,
                    account_id TEXT,
                    error TEXT,
                    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
                );
                CREATE INDEX IF NOT EXISTS idx_registrations_status ON registrations(status);
                CREATE INDEX IF NOT EXISTS idx_registrations_phone ON registrations(phone);
                ",
            )
            .map_err(|error| format!("create registrar database tables: {error}"))?;
        Ok(Self { connection })
    }

    pub fn insert(&self, record: &RegistrationRecord) -> Result<i64, String> {
        self.connection
            .execute(
                "INSERT INTO registrations (phone, provider, activation_id, status, account_id, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    record.phone,
                    record.provider,
                    record.activation_id,
                    record.status,
                    record.account_id,
                    record.error,
                ],
            )
            .map_err(|error| format!("insert registrar result: {error}"))?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn update(&self, id: i64, record: &RegistrationRecord) -> Result<(), String> {
        let changed = self
            .connection
            .execute(
                "UPDATE registrations
                 SET phone = ?1, provider = ?2, activation_id = ?3, status = ?4,
                     account_id = ?5, error = ?6, updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?7",
                params![
                    record.phone,
                    record.provider,
                    record.activation_id,
                    record.status,
                    record.account_id,
                    record.error,
                    id,
                ],
            )
            .map_err(|error| format!("update registrar result: {error}"))?;
        if changed == 0 {
            return Err("registrar result was not found".to_string());
        }
        Ok(())
    }
}

fn validate_task_id(task_id: &str) -> Result<(), String> {
    if task_id.is_empty()
        || task_id.len() > 64
        || !task_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("invalid registrar task identifier".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_updates_registration_result() {
        let database = RegistrarDatabase::open(std::path::Path::new(":memory:")).unwrap();
        let mut record = RegistrationRecord {
            phone: "12025550100".to_string(),
            provider: "5sim".to_string(),
            activation_id: Some("123".to_string()),
            status: "waiting_code".to_string(),
            account_id: None,
            error: None,
        };
        let id = database.insert(&record).unwrap();
        record.status = "registered".to_string();
        record.account_id = Some("account-id".to_string());
        database.update(id, &record).unwrap();
    }
}
