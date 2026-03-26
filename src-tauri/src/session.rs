use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const SESSION_TTL: Duration = Duration::from_secs(30 * 60); // 30 minutes

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSession {
    pub session_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub tool_args: serde_json::Value,
    pub content_type: String,
    pub data: serde_json::Value,
    #[serde(default)]
    pub meta: serde_json::Value,
    #[serde(default)]
    pub review_required: bool,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_decisions: Option<HashMap<String, String>>,
}

struct SessionEntry {
    session: PreviewSession,
    inserted_at: Instant,
}

pub struct SessionStore {
    entries: HashMap<String, SessionEntry>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn set(&mut self, session: PreviewSession) {
        let id = session.session_id.clone();
        self.entries.insert(
            id,
            SessionEntry {
                session,
                inserted_at: Instant::now(),
            },
        );
    }

    pub fn get(&self, id: &str) -> Option<&PreviewSession> {
        self.entries.get(id).map(|e| &e.session)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut PreviewSession> {
        self.entries.get_mut(id).map(|e| &mut e.session)
    }

    pub fn get_all(&self) -> Vec<PreviewSession> {
        self.entries.values().map(|e| e.session.clone()).collect()
    }

    pub fn delete(&mut self, id: &str) -> Option<PreviewSession> {
        self.entries.remove(id).map(|e| e.session)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Remove sessions older than TTL, return count removed
    pub fn gc(&mut self) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|_, e| e.inserted_at.elapsed() < SESSION_TTL);
        before - self.entries.len()
    }

}
