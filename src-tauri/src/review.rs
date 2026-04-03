use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::watch;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDecision {
    pub session_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_decisions: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifications: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additions: Option<serde_json::Value>,
}

pub struct ReviewState {
    pending: HashMap<String, watch::Sender<Option<ReviewDecision>>>,
}

impl ReviewState {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }

    /// Register a pending review, returns a receiver to await the decision
    pub fn add_pending(&mut self, session_id: String) -> watch::Receiver<Option<ReviewDecision>> {
        // Clean up any existing pending review for this session
        self.pending.remove(&session_id);

        let (tx, rx) = watch::channel(None);
        self.pending.insert(session_id, tx);
        rx
    }

    /// Subscribe to an existing pending review. Returns None if no pending review exists.
    pub fn subscribe(&self, session_id: &str) -> Option<watch::Receiver<Option<ReviewDecision>>> {
        self.pending.get(session_id).map(|tx| tx.subscribe())
    }

    /// Resolve a pending review with a decision. Returns true if there was a pending review.
    /// Does NOT remove from map — keeps sender alive for late subscribers.
    pub fn resolve(&mut self, session_id: &str, decision: ReviewDecision) -> bool {
        if let Some(tx) = self.pending.get(session_id) {
            let _ = tx.send(Some(decision));
            true
        } else {
            false
        }
    }

    /// Dismiss a pending review (browser closed / timeout). Returns true if there was a pending review.
    /// Does NOT remove from map — keeps sender alive for late subscribers.
    pub fn dismiss(&mut self, session_id: &str) -> bool {
        if let Some(tx) = self.pending.get(session_id) {
            let _ = tx.send(Some(ReviewDecision {
                session_id: session_id.to_string(),
                status: "decision_received".to_string(),
                decision: Some("dismissed".to_string()),
                operation_decisions: None,
                comments: None,
                modifications: None,
                additions: None,
            }));
            true
        } else {
            false
        }
    }

    /// Remove a resolved entry from the pending map (cleanup after decision consumed).
    pub fn remove_resolved(&mut self, session_id: &str) {
        self.pending.remove(session_id);
    }

    pub fn has_pending(&self, session_id: &str) -> bool {
        self.pending.contains_key(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_decision(session_id: &str, status: &str, decision: Option<&str>) -> ReviewDecision {
        ReviewDecision {
            session_id: session_id.to_string(),
            status: status.to_string(),
            decision: decision.map(|s| s.to_string()),
            operation_decisions: None,
            comments: None,
            modifications: None,
            additions: None,
        }
    }

    #[test]
    fn subscribe_returns_receiver_that_gets_decisions_from_resolve() {
        let mut state = ReviewState::new();
        state.add_pending("s1".to_string());

        let rx = state.subscribe("s1").expect("subscribe should return Some");
        let decision = make_decision("s1", "decision_received", Some("approved"));
        state.resolve("s1", decision.clone());

        let val = rx.borrow().clone().expect("should have a decision");
        assert_eq!(val.session_id, "s1");
        assert_eq!(val.decision, Some("approved".to_string()));
    }

    #[test]
    fn subscribe_on_nonexistent_session_returns_none() {
        let state = ReviewState::new();
        assert!(state.subscribe("nonexistent").is_none());
    }

    #[test]
    fn multiple_subscribers_all_receive_decision() {
        let mut state = ReviewState::new();
        state.add_pending("s2".to_string());

        let rx1 = state.subscribe("s2").unwrap();
        let rx2 = state.subscribe("s2").unwrap();
        let rx3 = state.subscribe("s2").unwrap();

        let decision = make_decision("s2", "decision_received", Some("rejected"));
        state.resolve("s2", decision);

        for rx in [rx1, rx2, rx3] {
            let val = rx.borrow().clone().expect("should have decision");
            assert_eq!(val.decision, Some("rejected".to_string()));
        }
    }

    #[test]
    fn remove_resolved_cleans_up() {
        let mut state = ReviewState::new();
        state.add_pending("s3".to_string());
        assert!(state.has_pending("s3"));

        state.remove_resolved("s3");
        assert!(!state.has_pending("s3"));
        assert!(state.subscribe("s3").is_none());
    }

    #[test]
    fn decision_readable_via_borrow_after_resolve() {
        let mut state = ReviewState::new();
        let rx = state.add_pending("s4".to_string());

        // Before resolve, borrow returns None
        assert!(rx.borrow().is_none());

        let decision = make_decision("s4", "decision_received", Some("approved"));
        state.resolve("s4", decision);

        // After resolve, borrow returns the decision without needing changed()
        let val = rx.borrow().clone().expect("should have decision after resolve");
        assert_eq!(val.session_id, "s4");
        assert_eq!(val.status, "decision_received");
        assert_eq!(val.decision, Some("approved".to_string()));
    }

    #[tokio::test]
    async fn await_decision_via_watch_channel() {
        let mut state = ReviewState::new();
        state.add_pending("s5".to_string());
        let mut rx = state.subscribe("s5").unwrap();

        // Spawn a task that resolves after a short delay
        let decision = make_decision("s5", "decision_received", Some("approved"));
        let handle = tokio::spawn({
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                // We can't mutate state from here, so we test the channel directly
            }
        });

        // Resolve from this context instead
        state.resolve("s5", decision);

        // changed() should complete
        rx.changed().await.expect("should receive change");
        let val = rx.borrow().clone().expect("should have decision");
        assert_eq!(val.decision, Some("approved".to_string()));

        handle.await.unwrap();
    }

    #[test]
    fn dismiss_sends_dismissed_decision() {
        let mut state = ReviewState::new();
        let rx = state.add_pending("s6".to_string());

        state.dismiss("s6");

        let val = rx.borrow().clone().expect("should have decision after dismiss");
        assert_eq!(val.decision, Some("dismissed".to_string()));
        assert_eq!(val.status, "decision_received");
    }

    #[test]
    fn resolve_on_nonexistent_returns_false() {
        let mut state = ReviewState::new();
        let decision = make_decision("nope", "decision_received", Some("approved"));
        assert!(!state.resolve("nope", decision));
    }

    #[test]
    fn dismiss_on_nonexistent_returns_false() {
        let mut state = ReviewState::new();
        assert!(!state.dismiss("nope"));
    }
}
