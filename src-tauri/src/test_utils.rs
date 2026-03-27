use std::sync::Arc;

use mcpviews_shared::plugin_store::PluginStore;

use crate::state::AppState;

pub fn test_manifest(name: &str) -> mcpviews_shared::PluginManifest {
    mcpviews_shared::PluginManifest {
        name: name.to_string(),
        version: "1.0.0".to_string(),
        renderers: std::collections::HashMap::new(),
        mcp: None,
        renderer_definitions: vec![],
        tool_rules: std::collections::HashMap::new(),
        no_auto_push: vec![],
    }
}

pub fn test_app_state() -> (Arc<AppState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = PluginStore::with_dir(dir.path().to_path_buf());
    (Arc::new(AppState::new_with_store(store)), dir)
}
