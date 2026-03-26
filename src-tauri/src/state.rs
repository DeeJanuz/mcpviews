use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::Mutex as TokioMutex;
use tokio::time::Instant;

use mcp_mux_shared::RegistryEntry;

use crate::mcp_session::McpSessionManager;
use crate::plugin::PluginRegistry;
use crate::review::ReviewState;
use crate::session::SessionStore;

pub struct AppState {
    pub sessions: Mutex<SessionStore>,
    pub reviews: Mutex<ReviewState>,
    /// Maps session_id -> (deadline, original_timeout_secs)
    pub review_deadlines: Mutex<HashMap<String, (Arc<TokioMutex<Instant>>, u64)>>,
    pub plugin_registry: Mutex<PluginRegistry>,
    pub http_client: reqwest::Client,
    pub latest_registry: Mutex<Vec<RegistryEntry>>,
    pub mcp_sessions: Mutex<McpSessionManager>,
    /// Optional override for the plugins directory (used in tests).
    plugins_dir_override: Option<PathBuf>,
}

impl AppState {
    pub fn new() -> Self {
        let registry = PluginRegistry::load_plugins();
        Self {
            sessions: Mutex::new(SessionStore::new()),
            reviews: Mutex::new(ReviewState::new()),
            review_deadlines: Mutex::new(HashMap::new()),
            plugin_registry: Mutex::new(registry),
            http_client: reqwest::Client::new(),
            latest_registry: Mutex::new(Vec::new()),
            mcp_sessions: Mutex::new(McpSessionManager::new()),
            plugins_dir_override: None,
        }
    }

    /// Create an AppState with a custom PluginStore (useful for testing without touching the real filesystem).
    pub fn new_with_store(store: mcp_mux_shared::plugin_store::PluginStore) -> Self {
        let dir = store.dir().to_path_buf();
        let registry = PluginRegistry::load_plugins_with_store(store);
        Self {
            sessions: Mutex::new(SessionStore::new()),
            reviews: Mutex::new(ReviewState::new()),
            review_deadlines: Mutex::new(HashMap::new()),
            plugin_registry: Mutex::new(registry),
            http_client: reqwest::Client::new(),
            latest_registry: Mutex::new(Vec::new()),
            mcp_sessions: Mutex::new(McpSessionManager::new()),
            plugins_dir_override: Some(dir),
        }
    }

    /// Broadcast a tools/list_changed notification to all connected MCP SSE sessions.
    pub fn notify_tools_changed(&self) {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/tools/list_changed"
        })
        .to_string();
        let sessions = self.mcp_sessions.lock().unwrap();
        sessions.broadcast(&notification);
    }

    /// Reload all plugins from disk and broadcast a tools/list_changed notification
    /// to all connected MCP SSE sessions.
    pub fn reload_plugins(&self) {
        let new_registry = if let Some(dir) = &self.plugins_dir_override {
            let store = mcp_mux_shared::plugin_store::PluginStore::with_dir(dir.clone());
            PluginRegistry::load_plugins_with_store(store)
        } else {
            PluginRegistry::load_plugins()
        };
        {
            let mut registry = self.plugin_registry.lock().unwrap();
            *registry = new_registry;
        }
        self.notify_tools_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn test_app_state() -> (Arc<AppState>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = mcp_mux_shared::plugin_store::PluginStore::with_dir(dir.path().to_path_buf());
        (Arc::new(AppState::new_with_store(store)), dir)
    }

    fn test_manifest(name: &str) -> mcp_mux_shared::PluginManifest {
        mcp_mux_shared::PluginManifest {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            renderers: std::collections::HashMap::new(),
            mcp: None,
            renderer_definitions: vec![],
            tool_rules: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_new_with_store() {
        let (state, _dir) = test_app_state();
        let registry = state.plugin_registry.lock().unwrap();
        assert!(registry.manifests.is_empty(), "Fresh temp dir should have no plugins");
    }

    #[test]
    fn test_notify_tools_changed_no_sessions() {
        let (state, _dir) = test_app_state();
        // Should not panic even with no connected MCP sessions
        state.notify_tools_changed();
    }

    #[test]
    fn test_reload_plugins() {
        let (state, dir) = test_app_state();

        // Verify initially empty
        {
            let registry = state.plugin_registry.lock().unwrap();
            assert!(registry.manifests.is_empty());
        }

        // Write a plugin manifest to the temp dir on disk
        let plugin_dir = dir.path().join("reload-test");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let manifest = test_manifest("reload-test");
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        std::fs::write(plugin_dir.join("manifest.json"), &json).unwrap();

        // Reload and verify the plugin appears
        state.reload_plugins();
        {
            let registry = state.plugin_registry.lock().unwrap();
            assert_eq!(registry.manifests.len(), 1);
            assert_eq!(registry.manifests[0].name, "reload-test");
        }
    }
}
