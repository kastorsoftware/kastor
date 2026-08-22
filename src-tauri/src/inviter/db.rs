// inviter/db.rs — SQLite database for invite user list + statistics

use rusqlite::{params, Connection};
use std::path::PathBuf;
use crate::i18n::t_with;

/// Status values for users in the invite database
#[derive(Debug, Clone, PartialEq)]
pub enum InviteUserStatus {
    Pending,
    Taken,         // currently being processed
    Done,          // successfully invited
    AlreadyInGroup,
    NotUser,       // not a user type (bot, channel, etc.)
    FloodWait,
    PeerFlood,
    Privacy,       // USER_PRIVACY_RESTRICTED
    Error(String),
}

impl InviteUserStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::Taken => "taken",
            Self::Done => "done",
            Self::AlreadyInGroup => "already_in_group",
            Self::NotUser => "not_user",
            Self::FloodWait => "flood_wait",
            Self::PeerFlood => "peer_flood",
            Self::Privacy => "privacy",
            Self::Error(_) => "error",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "taken" => Self::Taken,
            "done" => Self::Done,
            "already_in_group" => Self::AlreadyInGroup,
            "not_user" => Self::NotUser,
            "flood_wait" => Self::FloodWait,
            "peer_flood" => Self::PeerFlood,
            "privacy" => Self::Privacy,
            other => Self::Error(other.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct InviteUser {
    pub user_id: i64,
    pub access_hash: i64,
    pub username: String,
    pub first_name: String,
    pub last_name: String,
    pub status: InviteUserStatus,
}

/// Initialize the invite users database
pub fn init_users_db(path: &PathBuf) -> Result<Connection, String> {
    let conn = Connection::open(path)
        .map_err(|e| t_with("inviter_db_open_error", &[("error", &e.to_string())]))?;
    conn.execute_batch("
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS users (
            user_id INTEGER PRIMARY KEY,
            access_hash INTEGER DEFAULT 0,
            username TEXT DEFAULT '',
            first_name TEXT DEFAULT '',
            last_name TEXT DEFAULT '',
            status TEXT DEFAULT 'pending',
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_invite_users_status ON users(status);
    ").map_err(|e| t_with("inviter_db_create_tables", &[("error", &e.to_string())]))?;
    Ok(conn)
}

/// Import usernames into the database, skipping duplicates
pub fn import_usernames(conn: &Connection, usernames: &[String]) -> Result<usize, String> {
    let mut inserted = 0usize;
    let tx = conn.unchecked_transaction()
        .map_err(|e| format!("begin tx: {e}"))?;
    {
        let mut next_temp_id = tx.query_row(
            "SELECT MIN(user_id) FROM users WHERE user_id < 0",
            [],
            |row| row.get::<_, Option<i64>>(0),
        ).map_err(|e| format!("read temporary id: {e}"))?
            .map(|id| id.saturating_sub(1))
            .unwrap_or(-1);
        let mut stmt = tx.prepare(
            "INSERT INTO users (user_id, username, status)
             SELECT ?1, ?2, 'pending'
             WHERE NOT EXISTS (SELECT 1 FROM users WHERE username = ?2)"
        ).map_err(|e| format!("prepare: {e}"))?;
        // User IDs are unknown at this point. Allocate from the negative range
        // without reusing IDs from a previous import.
        for uname in usernames {
            let temp_id = next_temp_id;
            next_temp_id = next_temp_id.saturating_sub(1);
            if stmt.execute(params![temp_id, uname]).is_ok() {
                inserted += 1;
            }
        }
    }
    tx.commit().map_err(|e| format!("commit: {e}"))?;
    Ok(inserted)
}

/// Get next batch of pending users for processing
pub fn get_pending_users(conn: &Connection, limit: usize) -> Vec<InviteUser> {
    let mut stmt = conn.prepare(
        "SELECT user_id, access_hash, username, first_name, last_name, status FROM users WHERE status = 'pending' LIMIT ?1"
    ).unwrap();
    stmt.query_map(params![limit as u32], |row| {
        Ok(InviteUser {
            user_id: row.get(0)?,
            access_hash: row.get(1)?,
            username: row.get(2)?,
            first_name: row.get(3)?,
            last_name: row.get(4)?,
            status: InviteUserStatus::from_str(&row.get::<_, String>(5)?),
        })
    }).unwrap().filter_map(|r| r.ok()).collect()
}

/// Count users by status
pub fn count_by_status(conn: &Connection, status: &str) -> u32 {
    conn.query_row(
        "SELECT COUNT(*) FROM users WHERE status = ?1",
        params![status],
        |row| row.get(0),
    ).unwrap_or(0)
}

/// Count total users
#[allow(dead_code)]
pub fn count_total(conn: &Connection) -> u32 {
    conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0)).unwrap_or(0)
}

/// Mark a user as taken (being processed)
pub fn mark_taken(conn: &Connection, user_id: i64) {
    conn.execute(
        "UPDATE users SET status = 'taken', updated_at = CURRENT_TIMESTAMP WHERE user_id = ?1",
        params![user_id],
    ).ok();
}

/// Update user after resolve (set real user_id, access_hash, names)
pub fn update_resolved(conn: &Connection, old_id: i64, user_id: i64, access_hash: i64, first_name: &str, last_name: &str) {
    let Ok(tx) = conn.unchecked_transaction() else { return; };
    let result = tx.execute(
        "INSERT INTO users (user_id, access_hash, username, first_name, last_name, status)
         SELECT ?1, ?2, username, ?3, ?4, 'taken' FROM users WHERE user_id = ?5
         ON CONFLICT(user_id) DO UPDATE SET
             access_hash = excluded.access_hash,
             first_name = CASE WHEN excluded.first_name != '' THEN excluded.first_name ELSE users.first_name END,
             last_name = CASE WHEN excluded.last_name != '' THEN excluded.last_name ELSE users.last_name END",
        params![user_id, access_hash, first_name, last_name, old_id],
    );
    if result.is_ok() {
        let _ = tx.execute("DELETE FROM users WHERE user_id = ?1", params![old_id]);
        let _ = tx.commit();
    }
}

/// Update user status
pub fn update_status(conn: &Connection, user_id: i64, status: &InviteUserStatus) {
    let status_str = match status {
        InviteUserStatus::Error(msg) => format!("error:{}", msg),
        other => other.as_str().to_string(),
    };
    conn.execute(
        "UPDATE users SET status = ?1, updated_at = CURRENT_TIMESTAMP WHERE user_id = ?2",
        params![status_str, user_id],
    ).ok();
}

/// Reset "taken" users back to "pending" (for force mode restart)
pub fn reset_taken_to_pending(conn: &Connection) {
    conn.execute("UPDATE users SET status = 'pending' WHERE status = 'taken'", []).ok();
}

// ─── Statistics database ───────────────────────────────────────────────────

/// Initialize the statistics database
pub fn init_stats_db(path: &PathBuf) -> Result<Connection, String> {
    let conn = Connection::open(path)
        .map_err(|e| t_with("inviter_stats_db_open_error", &[("error", &e.to_string())]))?;
    conn.execute_batch("
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS invites (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id TEXT DEFAULT '',
            group_id INTEGER DEFAULT 0,
            group_link TEXT DEFAULT '',
            user_id INTEGER DEFAULT 0,
            username TEXT DEFAULT '',
            first_name TEXT DEFAULT '',
            last_name TEXT DEFAULT '',
            status TEXT DEFAULT '',
            invited_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_invites_account ON invites(account_id);
        CREATE INDEX IF NOT EXISTS idx_invites_status ON invites(status);
    ").map_err(|e| t_with("inviter_stats_db_create_tables", &[("error", &e.to_string())]))?;
    Ok(conn)
}

/// Record an invite attempt in the statistics database
pub fn record_invite(
    conn: &Connection,
    account_id: &str,
    group_id: i64,
    group_link: &str,
    user_id: i64,
    username: &str,
    first_name: &str,
    last_name: &str,
    status: &str,
) {
    conn.execute(
        "INSERT INTO invites (account_id, group_id, group_link, user_id, username, first_name, last_name, status) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![account_id, group_id, group_link, user_id, username, first_name, last_name, status],
    ).ok();
}

/// Import phone numbers into the database
pub fn import_phones(conn: &Connection, phones: &[String]) -> Result<usize, String> {
    let mut inserted = 0usize;
    let tx = conn.unchecked_transaction()
        .map_err(|e| format!("begin tx: {e}"))?;
    {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO users (user_id, username, status) VALUES (?1, ?2, 'pending')"
        ).map_err(|e| format!("prepare: {e}"))?;
        for (i, phone) in phones.iter().enumerate() {
            let temp_id = -(i as i64 + 10000); // negative temp id (offset to avoid collision with username imports)
            // Store phone number in the username field temporarily (will be resolved later)
            if stmt.execute(params![temp_id, phone]).is_ok() {
                inserted += 1;
            }
        }
    }
    tx.commit().map_err(|e| format!("commit: {e}"))?;
    Ok(inserted)
}

/// Get pending phone entries (those with negative user_id starting from -10000)
pub fn get_pending_phones(conn: &Connection, limit: usize) -> Vec<InviteUser> {
    let mut stmt = conn.prepare(
        "SELECT user_id, access_hash, username, first_name, last_name, status FROM users WHERE status = 'pending' AND user_id <= -10000 LIMIT ?1"
    ).unwrap();
    stmt.query_map(params![limit as u32], |row| {
        Ok(InviteUser {
            user_id: row.get(0)?,
            access_hash: row.get(1)?,
            username: row.get(2)?, // this is actually the phone number
            first_name: row.get(3)?,
            last_name: row.get(4)?,
            status: InviteUserStatus::from_str(&row.get::<_, String>(5)?),
        })
    }).unwrap().filter_map(|r| r.ok()).collect()
}
