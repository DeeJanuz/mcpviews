use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>, // unix timestamp
}

impl StoredToken {
    /// Check if this token has expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            now >= expires_at
        } else {
            false
        }
    }
}

/// Load a stored token from {dir}/{plugin_name}.json, returning None if missing, unparseable, or expired.
pub fn load_stored_token(dir: &Path, plugin_name: &str) -> Option<StoredToken> {
    let path = dir.join(format!("{}.json", plugin_name));
    let content = std::fs::read_to_string(&path).ok()?;
    let token: StoredToken = serde_json::from_str(&content).ok()?;

    if token.is_expired() {
        eprintln!(
            "[mcp-mux] Stored token for plugin '{}' has expired",
            plugin_name
        );
        return None;
    }

    Some(token)
}

/// Store a token to {dir}/{plugin_name}.json
pub fn store_token(dir: &Path, plugin_name: &str, token: &StoredToken) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Failed to create auth dir: {}", e))?;
    let path = dir.join(format!("{}.json", plugin_name));
    let json = serde_json::to_string_pretty(token)
        .map_err(|e| format!("Failed to serialize token: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to write token: {}", e))?;
    Ok(())
}

/// Check if a stored token file exists at {dir}/{plugin_name}.json
pub fn has_stored_token(dir: &Path, plugin_name: &str) -> bool {
    dir.join(format!("{}.json", plugin_name)).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stored_token_roundtrip() {
        let token = StoredToken {
            access_token: "test-token".to_string(),
            refresh_token: Some("refresh-123".to_string()),
            expires_at: Some(1700000000),
        };
        let json = serde_json::to_string(&token).unwrap();
        let parsed: StoredToken = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, "test-token");
        assert_eq!(parsed.refresh_token, Some("refresh-123".to_string()));
        assert_eq!(parsed.expires_at, Some(1700000000));
    }

    #[test]
    fn test_is_expired_false_for_future() {
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 3600;
        let token = StoredToken {
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: Some(future),
        };
        assert!(!token.is_expired());
    }

    #[test]
    fn test_is_expired_true_for_past() {
        let past = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 3600;
        let token = StoredToken {
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: Some(past),
        };
        assert!(token.is_expired());
    }

    #[test]
    fn test_is_expired_false_for_none() {
        let token = StoredToken {
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_at: None,
        };
        assert!(!token.is_expired());
    }

    #[test]
    fn test_load_stored_token_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-plugin.json");
        std::fs::write(
            &path,
            r#"{"access_token":"abc","refresh_token":null,"expires_at":null}"#,
        )
        .unwrap();

        let token = load_stored_token(dir.path(), "my-plugin");
        assert!(token.is_some());
        let token = token.unwrap();
        assert_eq!(token.access_token, "abc");
    }

    #[test]
    fn test_load_stored_token_expired() {
        let dir = tempfile::tempdir().unwrap();
        let past = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 3600;
        let path = dir.path().join("expired-plugin.json");
        std::fs::write(
            &path,
            format!(
                r#"{{"access_token":"abc","refresh_token":null,"expires_at":{}}}"#,
                past
            ),
        )
        .unwrap();

        let token = load_stored_token(dir.path(), "expired-plugin");
        assert!(token.is_none());
    }

    #[test]
    fn test_load_stored_token_missing() {
        let dir = tempfile::tempdir().unwrap();
        let token = load_stored_token(dir.path(), "nonexistent-plugin");
        assert!(token.is_none());
    }

    #[test]
    fn test_store_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let token = StoredToken {
            access_token: "roundtrip-tok".to_string(),
            refresh_token: Some("rt".to_string()),
            expires_at: None,
        };
        store_token(dir.path(), "rt-plugin", &token).unwrap();
        let loaded = load_stored_token(dir.path(), "rt-plugin").unwrap();
        assert_eq!(loaded.access_token, "roundtrip-tok");
        assert_eq!(loaded.refresh_token, Some("rt".to_string()));
    }

    #[test]
    fn test_has_stored_token() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!has_stored_token(dir.path(), "check-plugin"));

        let path = dir.path().join("check-plugin.json");
        std::fs::write(&path, "{}").unwrap();
        assert!(has_stored_token(dir.path(), "check-plugin"));
    }
}
