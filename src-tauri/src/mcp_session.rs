use std::collections::HashMap;
use tokio::sync::broadcast;
use uuid::Uuid;

pub struct McpSession {
    pub tx: broadcast::Sender<String>,
}

pub struct McpSessionManager {
    sessions: HashMap<String, McpSession>,
}

impl McpSessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Creates a new session, returns (session_id, receiver)
    pub fn create_session(&mut self) -> (String, broadcast::Receiver<String>) {
        let session_id = Uuid::new_v4().to_string();
        let (tx, rx) = broadcast::channel(64);
        self.sessions.insert(session_id.clone(), McpSession { tx });
        (session_id, rx)
    }

    /// Get a receiver for an existing session (reconnect support)
    #[allow(dead_code)]
    pub fn subscribe(&self, session_id: &str) -> Option<broadcast::Receiver<String>> {
        self.sessions.get(session_id).map(|s| s.tx.subscribe())
    }

    /// Remove session explicitly
    pub fn remove_session(&mut self, session_id: &str) -> bool {
        self.sessions.remove(session_id).is_some()
    }

    /// Broadcast a notification to ALL active sessions
    pub fn broadcast(&self, notification_json: &str) {
        for session in self.sessions.values() {
            let _ = session.tx.send(notification_json.to_string());
        }
    }

    /// GC: remove sessions with no active receivers
    pub fn retain_active(&mut self) {
        self.sessions.retain(|_, s| s.tx.receiver_count() > 0);
    }

    /// Check if a session exists
    pub fn has_session(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }
}
