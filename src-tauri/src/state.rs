use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::Mutex as TokioMutex;
use tokio::time::Instant;

use mcpviews_shared::plugin_store::PluginStore;
use mcpviews_shared::RegistryEntry;

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
    plugin_store: PluginStore,
}

impl AppState {
    pub fn new() -> Self {
        let store = PluginStore::new();
        Self::new_with_store(store)
    }

    /// Create an AppState with a custom PluginStore (useful for testing without touching the real filesystem).
    pub fn new_with_store(store: PluginStore) -> Self {
        let registry = PluginRegistry::load_plugins_with_store(
            PluginStore::with_dir(store.dir().to_path_buf()),
        );
        Self {
            sessions: Mutex::new(SessionStore::new()),
            reviews: Mutex::new(ReviewState::new()),
            review_deadlines: Mutex::new(HashMap::new()),
            plugin_registry: Mutex::new(registry),
            http_client: reqwest::Client::new(),
            latest_registry: Mutex::new(Vec::new()),
            mcp_sessions: Mutex::new(McpSessionManager::new()),
            plugin_store: store,
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
        let store = PluginStore::with_dir(self.plugin_store.dir().to_path_buf());
        let new_registry = PluginRegistry::load_plugins_with_store(store);
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
    use crate::test_utils::{test_app_state, test_manifest};

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
