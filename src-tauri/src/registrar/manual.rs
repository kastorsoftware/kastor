use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::State;
use tokio::sync::Mutex;

use crate::registrar::flow::{
    RegistrationCodeRequest, RegistrationIdentity, RegistrationOutcome, RegistrationSession,
};

const SESSION_TTL: Duration = Duration::from_secs(15 * 60);

pub struct RegistrationSessions {
    sessions: Arc<Mutex<HashMap<String, StoredSession>>>,
}

struct StoredSession {
    session: RegistrationSession,
    created_at: Instant,
}

impl RegistrationSessions {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn insert(&self, session: RegistrationSession) -> String {
        let mut sessions = self.sessions.lock().await;
        purge_expired(&mut sessions);
        let id = uuid::Uuid::new_v4().to_string();
        sessions.insert(
            id.clone(),
            StoredSession {
                session,
                created_at: Instant::now(),
            },
        );
        id
    }

    async fn take(&self, id: &str) -> Result<StoredSession, String> {
        let mut sessions = self.sessions.lock().await;
        purge_expired(&mut sessions);
        sessions
            .remove(id)
            .ok_or_else(|| "registration session was not found or expired".to_string())
    }

    async fn restore(&self, id: String, session: StoredSession) {
        self.sessions.lock().await.insert(id, session);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ManualCodeRequestResult {
    pub session_id: String,
    pub code_type: String,
}

#[tauri::command]
pub async fn registrar_manual_send_code(
    request: RegistrationCodeRequest,
    sessions: State<'_, RegistrationSessions>,
) -> Result<ManualCodeRequestResult, String> {
    let (session, sent_code) = RegistrationSession::start(request).await?;
    let session_id = sessions.insert(session).await;
    Ok(ManualCodeRequestResult {
        session_id,
        code_type: sent_code.code_type,
    })
}

#[tauri::command]
pub async fn registrar_manual_complete(
    session_id: String,
    code: String,
    identity: RegistrationIdentity,
    sessions: State<'_, RegistrationSessions>,
) -> Result<RegistrationOutcome, String> {
    let mut stored = sessions.take(&session_id).await?;
    match stored.session.complete(&code, &identity, false).await {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            sessions.restore(session_id, stored).await;
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn registrar_manual_resend_code(
    session_id: String,
    sessions: State<'_, RegistrationSessions>,
) -> Result<String, String> {
    let mut stored = sessions.take(&session_id).await?;
    let result = stored
        .session
        .resend()
        .await
        .map(|sent_code| sent_code.code_type);
    sessions.restore(session_id, stored).await;
    result
}

#[tauri::command]
pub async fn registrar_manual_cancel(
    session_id: String,
    sessions: State<'_, RegistrationSessions>,
) -> Result<(), String> {
    let mut stored = sessions.take(&session_id).await?;
    match stored.session.cancel().await {
        Ok(()) => Ok(()),
        Err(error) => {
            sessions.restore(session_id, stored).await;
            Err(error)
        }
    }
}

fn purge_expired(sessions: &mut HashMap<String, StoredSession>) {
    sessions.retain(|_, stored| stored.created_at.elapsed() < SESSION_TTL);
}
