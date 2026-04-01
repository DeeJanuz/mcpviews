use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Emitter, State};

use mcpviews_shared::{PluginAuth, PluginInfo, PluginManifest, RegistryEntry, RegistrySource};

use crate::renderer_scanner::RendererInfo;

use crate::review::ReviewDecision;
use crate::session::PreviewSession;
use crate::state::AppState;

#[tauri::command]
pub fn get_sessions(state: State<Arc<AppState>>) -> Vec<PreviewSession> {
    let sessions = state.sessions.lock().unwrap();
    sessions.get_all()
}

#[tauri::command]
pub fn submit_decision(
    session_id: String,
    decision: String,
    operation_decisions: Option<HashMap<String, String>>,
    comments: Option<HashMap<String, String>>,
    modifications: Option<HashMap<String, String>>,
    additions: Option<serde_json::Value>,
    state: State<Arc<AppState>>,
) -> Result<(), String> {
    // Update session state
    {
        let mut sessions = state.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(&session_id) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            session.decided_at = Some(now);
            session.decision = Some(decision.clone());
            session.operation_decisions = operation_decisions.clone();
        }
    }

    // Resolve the pending review (unblocks the HTTP response)
    let overall_decision = if operation_decisions.is_some() && decision != "accept" && decision != "reject" {
        "partial".to_string()
    } else {
        decision
    };

    let review_decision = ReviewDecision {
        session_id: session_id.clone(),
        status: "decision_received".to_string(),
        decision: Some(overall_decision),
        operation_decisions,
        comments,
        modifications,
        additions,
    };

    let mut reviews = state.reviews.lock().unwrap();
    reviews.resolve(&session_id, review_decision);

    Ok(())
}

#[tauri::command]
pub fn dismiss_session(session_id: String, state: State<Arc<AppState>>) -> Result<(), String> {
    // Remove session
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions.delete(&session_id);
    }

    // Dismiss any pending review
    {
        let mut reviews = state.reviews.lock().unwrap();
        reviews.dismiss(&session_id);
    }

    Ok(())
}

#[tauri::command]
pub fn get_health() -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "status": "ok"
    })
}

#[tauri::command]
pub fn list_plugins(state: State<'_, Arc<AppState>>) -> Vec<PluginInfo> {
    let registry = state.plugin_registry.lock().unwrap();
    let cached = state.latest_registry.lock().unwrap();
    registry.list_plugins_with_updates(&cached)
}

#[tauri::command]
pub fn install_plugin(
    manifest_json: String,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let manifest: PluginManifest = serde_json::from_str(&manifest_json)
        .map_err(|e| format!("Invalid manifest: {}", e))?;
    let mut registry = state.plugin_registry.lock().unwrap();
    registry.add_plugin(manifest)?;
    drop(registry);
    state.notify_tools_changed();
    Ok(())
}

#[tauri::command]
pub fn uninstall_plugin(name: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let mut registry = state.plugin_registry.lock().unwrap();
    registry.remove_plugin(&name)?;
    drop(registry);
    // Clean up any stored auth tokens for this plugin
    let _ = mcpviews_shared::token_store::remove_token(&mcpviews_shared::auth_dir(), &name);
    state.notify_tools_changed();
    Ok(())
}

#[tauri::command]
pub fn install_plugin_from_file(
    path: String,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read file: {}", e))?;
    let manifest: PluginManifest = serde_json::from_str(&content)
        .map_err(|e| format!("Invalid manifest: {}", e))?;
    let mut registry = state.plugin_registry.lock().unwrap();
    registry.add_plugin(manifest)?;
    drop(registry);
    state.notify_tools_changed();
    Ok(())
}

#[tauri::command]
pub async fn fetch_registry(
    registry_url: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<RegistryEntry>, String> {
    let client = state.http_client.clone();
    let entries = if let Some(url) = registry_url {
        // Specific URL provided (e.g. from legacy settings)
        crate::registry::fetch_registry(&client, &url).await?
    } else {
        // Use all configured sources
        let sources = mcpviews_shared::registry::get_registry_sources();
        mcpviews_shared::registry::fetch_all_registries(&client, &sources).await?
    };

    // Cache the latest registry entries
    {
        let mut cached = state.latest_registry.lock().unwrap();
        *cached = entries.clone();
    }

    Ok(entries)
}

#[tauri::command]
pub fn get_registry_sources() -> Vec<RegistrySource> {
    mcpviews_shared::registry::get_registry_sources()
}

#[tauri::command]
pub fn add_registry_source(name: String, url: String) -> Result<(), String> {
    let mut sources = mcpviews_shared::registry::get_registry_sources();
    if sources.iter().any(|s| s.url == url) {
        return Err("A source with this URL already exists".to_string());
    }
    sources.push(RegistrySource {
        name,
        url,
        enabled: true,
    });
    mcpviews_shared::registry::save_registry_sources(&sources)
}

#[tauri::command]
pub fn remove_registry_source(url: String) -> Result<(), String> {
    let mut sources = mcpviews_shared::registry::get_registry_sources();
    sources.retain(|s| s.url != url);
    mcpviews_shared::registry::save_registry_sources(&sources)
}

#[tauri::command]
pub fn toggle_registry_source(url: String) -> Result<(), String> {
    let mut sources = mcpviews_shared::registry::get_registry_sources();
    if let Some(source) = sources.iter_mut().find(|s| s.url == url) {
        source.enabled = !source.enabled;
    }
    mcpviews_shared::registry::save_registry_sources(&sources)
}

#[tauri::command]
pub async fn start_plugin_auth(
    plugin_name: String,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let auth = {
        let registry = state.plugin_registry.lock().unwrap();
        registry.resolve_plugin_auth(&plugin_name)?
    };

    let client = state.http_client.clone();

    match &auth {
        PluginAuth::OAuth {
            client_id,
            auth_url,
            token_url,
            scopes,
        } => {
            crate::auth::start_oauth_flow(
                &plugin_name,
                client_id.as_deref(),
                auth_url,
                token_url,
                scopes,
                &client,
            )
            .await
        }
        PluginAuth::Bearer { token_env } => std::env::var(token_env).map_err(|_| {
            format!(
                "Environment variable '{}' is not set. Set it and restart.",
                token_env
            )
        }),
        PluginAuth::ApiKey { key_env, .. } => {
            if let Some(env_var) = key_env {
                std::env::var(env_var).map_err(|_| {
                    format!(
                        "Environment variable '{}' is not set. Set it and restart.",
                        env_var
                    )
                })
            } else {
                Err("No key_env configured for this plugin".to_string())
            }
        }
    }
}

#[tauri::command]
pub async fn get_plugin_auth_header(
    plugin_name: String,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let auth = {
        let registry = state.plugin_registry.lock().unwrap();
        registry.resolve_plugin_auth(&plugin_name)?
    };

    // Try resolving from stored token (env var fallback for Bearer/ApiKey, stored file for OAuth)
    if let Some(header) = auth.resolve_header(&plugin_name) {
        return Ok(header);
    }

    // If OAuth with expired token, attempt refresh
    if let PluginAuth::OAuth {
        client_id,
        token_url,
        ..
    } = &auth
    {
        let client = state.http_client.clone();
        let token = crate::auth::refresh_oauth_token(
            &plugin_name,
            token_url,
            client_id.as_deref(),
            &client,
        )
        .await?;
        return Ok(format!("Bearer {}", token));
    }

    Err(format!("No token available for plugin '{}'", plugin_name))
}

#[tauri::command]
pub fn store_plugin_token(plugin_name: String, token: String) -> Result<(), String> {
    crate::auth::store_api_key(&plugin_name, &token)
}

#[tauri::command]
pub async fn install_plugin_from_registry(
    entry_json: String,
    state: State<'_, Arc<AppState>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let entry: RegistryEntry = serde_json::from_str(&entry_json)
        .map_err(|e| format!("Invalid registry entry: {}", e))?;

    state.install_or_update_from_entry(&entry).await?;

    state.notify_tools_changed();
    let _ = app_handle.emit("reload_renderers", ());

    Ok(())
}

#[tauri::command]
pub fn install_plugin_from_zip(
    path: String,
    state: State<'_, Arc<AppState>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let zip_path = std::path::Path::new(&path);
    let plugins_dir = mcpviews_shared::plugins_dir();
    let manifest = mcpviews_shared::package::install_from_local_zip(zip_path, &plugins_dir)?;

    let mut registry = state.plugin_registry.lock().unwrap();
    // Remove if already exists (for reinstall/update)
    // Only clear in-memory state — zip extraction already placed files on disk
    if registry.manifests.iter().any(|m| m.name == manifest.name) {
        let _ = registry.remove_plugin_in_memory(&manifest.name);
    }
    registry.add_plugin(manifest)?;
    drop(registry);

    state.notify_tools_changed();
    let _ = app_handle.emit("reload_renderers", ());

    Ok(())
}

#[tauri::command]
pub async fn reinstall_plugin(
    name: String,
    state: State<'_, Arc<AppState>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let entry = {
        let cached = state.latest_registry.lock().unwrap();
        cached.iter().find(|e| e.name == name).cloned()
    };

    if let Some(entry) = entry {
        state.install_or_update_from_entry(&entry).await?;
    } else {
        // For non-registry plugins, just reload from existing manifest
        let registry = state.plugin_registry.lock().unwrap();
        if !registry.manifests.iter().any(|m| m.name == name) {
            return Err(format!("Plugin '{}' not found", name));
        }
        drop(registry);
        // Plugin exists but not in registry - just notify to refresh
    }

    state.notify_tools_changed();
    let _ = app_handle.emit("reload_renderers", ());
    Ok(())
}

#[tauri::command]
pub fn clear_plugin_auth(name: String) -> Result<(), String> {
    mcpviews_shared::token_store::remove_token(&mcpviews_shared::auth_dir(), &name)
}

#[tauri::command]
pub fn get_settings() -> Result<mcpviews_shared::settings::Settings, String> {
    Ok(mcpviews_shared::settings::Settings::load())
}

#[tauri::command]
pub fn save_settings(settings: mcpviews_shared::settings::Settings) -> Result<(), String> {
    settings.save()
}

#[tauri::command]
pub fn get_plugin_renderers() -> Vec<RendererInfo> {
    crate::renderer_scanner::scan_plugin_renderers()
}

#[tauri::command]
pub async fn update_plugin(
    name: String,
    state: State<'_, Arc<AppState>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let entry = {
        let cached = state.latest_registry.lock().unwrap();
        cached.iter().find(|e| e.name == name).cloned()
    }
    .ok_or_else(|| format!("Plugin '{}' not found in registry", name))?;

    // Version guard: only update if the registry version is actually newer
    {
        let registry = state.plugin_registry.lock().unwrap();
        if let Some(installed) = registry.manifests.iter().find(|m| m.name == name) {
            if mcpviews_shared::newer_version(&installed.version, &entry.version).is_none() {
                return Err(format!(
                    "Plugin '{}' is already up to date (version {})",
                    name, installed.version
                ));
            }
        }
    }

    state.install_or_update_from_entry(&entry).await?;

    state.notify_tools_changed();
    let _ = app_handle.emit("reload_renderers", ());

    Ok(())
}

#[tauri::command]
pub async fn save_file(
    app_handle: tauri::AppHandle,
    filename: String,
    content: String,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();

    app_handle
        .dialog()
        .file()
        .set_file_name(&filename)
        .add_filter("CSV", &["csv"])
        .add_filter("All Files", &["*"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });

    let path = rx.await.map_err(|_| "Save dialog cancelled unexpectedly".to_string())?;

    match path {
        Some(file_path) => {
            let p = file_path
                .as_path()
                .ok_or_else(|| "Save dialog returned a non-local path".to_string())?;
            std::fs::write(p, &content)
                .map_err(|e| format!("Failed to write file: {}", e))?;
            Ok(true)
        }
        None => Ok(false), // user cancelled
    }
}

#[tauri::command]
pub fn get_standalone_renderers(state: State<'_, Arc<AppState>>) -> Vec<serde_json::Value> {
    let registry = state.plugin_registry.lock().unwrap();
    let mut results = Vec::new();

    for manifest in registry.manifests.iter() {
        let standalone_renderers: Vec<serde_json::Value> = manifest
            .renderer_definitions
            .iter()
            .filter(|def| def.standalone)
            .map(|def| {
                serde_json::json!({
                    "name": def.name,
                    "label": def.standalone_label.as_deref().unwrap_or(&def.name),
                    "description": def.description,
                    "data_hint": def.data_hint,
                })
            })
            .collect();

        if !standalone_renderers.is_empty() {
            results.push(serde_json::json!({
                "plugin": manifest.name,
                "renderers": standalone_renderers,
            }));
        }
    }
    results
}

/// Collect invocable renderer definitions (those with invoke_schema) from plugin manifests.
pub fn collect_invocable_renderers(manifests: &[mcpviews_shared::PluginManifest]) -> Vec<serde_json::Value> {
    let mut results = Vec::new();
    for manifest in manifests {
        for def in &manifest.renderer_definitions {
            if def.invoke_schema.is_some() {
                results.push(serde_json::json!({
                    "name": def.name,
                    "description": def.description,
                    "display_mode": def.display_mode,
                    "invoke_schema": def.invoke_schema,
                    "url_patterns": def.url_patterns,
                    "plugin": manifest.name,
                }));
            }
        }
    }
    results
}

/// Return renderer definitions that have invoke_schema set (i.e., are invocable).
/// Used by the frontend invocation registry to know which renderers can be invoked.
#[tauri::command]
pub fn get_renderer_registry(state: State<'_, Arc<AppState>>) -> Vec<serde_json::Value> {
    let registry = state.plugin_registry.lock().unwrap();
    collect_invocable_renderers(&registry.manifests)
}

#[tauri::command]
pub fn set_native_theme(theme: String, window: tauri::Window) -> Result<(), String> {
    let native_theme = match theme.as_str() {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        _ => None,
    };
    window.set_theme(native_theme).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{test_app_state, test_manifest};

    fn test_registry_entry(name: &str) -> RegistryEntry {
        RegistryEntry {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            description: "Test plugin".to_string(),
            author: None,
            homepage: None,
            manifest: test_manifest(name),
            tags: vec![],
            download_url: None,
            manifest_url: None,
        }
    }

    #[test]
    fn test_get_health() {
        let health = get_health();
        assert_eq!(health["status"], "ok");
        assert!(health["version"].is_string());
    }

    #[test]
    fn test_get_registry_sources() {
        let sources = get_registry_sources();
        let _ = sources.len();
    }

    #[tokio::test]
    async fn test_install_from_entry_manifest_only() {
        let (state, _dir) = test_app_state();
        let entry = test_registry_entry("test-plugin");

        state.install_or_update_from_entry(&entry).await.unwrap();

        let registry = state.plugin_registry.lock().unwrap();
        assert_eq!(registry.manifests.len(), 1);
        assert_eq!(registry.manifests[0].name, "test-plugin");
    }

    #[tokio::test]
    async fn test_install_from_entry_replaces_existing() {
        let (state, _dir) = test_app_state();
        let entry = test_registry_entry("dup-plugin");

        state.install_or_update_from_entry(&entry).await.unwrap();
        state.install_or_update_from_entry(&entry).await.unwrap();

        let registry = state.plugin_registry.lock().unwrap();
        let count = registry.manifests.iter().filter(|m| m.name == "dup-plugin").count();
        assert_eq!(count, 1, "Should not have duplicate entries");
    }

    #[test]
    fn test_install_plugin_logic() {
        let (state, _dir) = test_app_state();
        let manifest = test_manifest("logic-test");
        let manifest_json = serde_json::to_string(&manifest).unwrap();

        let parsed: PluginManifest = serde_json::from_str(&manifest_json).unwrap();
        let mut registry = state.plugin_registry.lock().unwrap();
        registry.add_plugin(parsed).unwrap();
        drop(registry);

        let registry = state.plugin_registry.lock().unwrap();
        assert_eq!(registry.manifests.len(), 1);
        assert_eq!(registry.manifests[0].name, "logic-test");
    }

    #[test]
    fn test_uninstall_plugin_logic() {
        let (state, _dir) = test_app_state();

        {
            let mut registry = state.plugin_registry.lock().unwrap();
            registry.add_plugin(test_manifest("removeme")).unwrap();
            assert_eq!(registry.manifests.len(), 1);
        }

        {
            let mut registry = state.plugin_registry.lock().unwrap();
            registry.remove_plugin("removeme").unwrap();
        }

        let registry = state.plugin_registry.lock().unwrap();
        assert!(registry.manifests.is_empty(), "Plugin should be removed");
    }

    #[test]
    fn test_list_plugins_empty() {
        let (state, _dir) = test_app_state();
        let registry = state.plugin_registry.lock().unwrap();
        let cached = state.latest_registry.lock().unwrap();
        let plugins = registry.list_plugins_with_updates(&cached);
        assert!(plugins.is_empty());
    }

    #[tokio::test]
    async fn test_reinstall_plugin_from_registry() {
        let (state, _dir) = test_app_state();
        let entry = test_registry_entry("reinstall-me");

        // First install
        state.install_or_update_from_entry(&entry).await.unwrap();

        // Cache the registry entry (simulating fetch_registry)
        {
            let mut cached = state.latest_registry.lock().unwrap();
            cached.push(entry.clone());
        }

        // Reinstall logic (same as the command does, minus Tauri State wrapper)
        let found_entry = {
            let cached = state.latest_registry.lock().unwrap();
            cached.iter().find(|e| e.name == "reinstall-me").cloned()
        };
        assert!(found_entry.is_some());
        state.install_or_update_from_entry(&found_entry.unwrap()).await.unwrap();

        let registry = state.plugin_registry.lock().unwrap();
        let count = registry.manifests.iter().filter(|m| m.name == "reinstall-me").count();
        assert_eq!(count, 1, "Should have exactly one instance after reinstall");
    }

    #[tokio::test]
    async fn test_reinstall_plugin_not_in_registry() {
        let (state, _dir) = test_app_state();

        // Install a plugin directly (not via registry)
        let manifest = test_manifest("local-only");
        {
            let mut registry = state.plugin_registry.lock().unwrap();
            registry.add_plugin(manifest).unwrap();
        }

        // Registry cache is empty, so reinstall should not find it
        let found_entry = {
            let cached = state.latest_registry.lock().unwrap();
            cached.iter().find(|e| e.name == "local-only").cloned()
        };
        assert!(found_entry.is_none(), "Should not find local-only plugin in registry");
    }

    #[test]
    fn test_get_renderer_registry_logic() {
        let (state, _dir) = test_app_state();

        // Add a plugin with an invocable renderer
        let mut manifest = test_manifest("test-invocable");
        manifest.renderer_definitions.push(mcpviews_shared::RendererDef {
            name: "decision_detail".to_string(),
            description: "Decision detail".to_string(),
            scope: "universal".to_string(),
            tools: vec![],
            data_hint: None,
            rule: None,
            display_mode: Some(mcpviews_shared::DisplayMode::Drawer),
            invoke_schema: Some("{ id: string }".to_string()),
            url_patterns: vec!["/decisions/*".to_string()],
            standalone: false,
            standalone_label: None,
        });

        // Also add a non-invocable renderer (no invoke_schema)
        manifest.renderer_definitions.push(mcpviews_shared::RendererDef {
            name: "basic_view".to_string(),
            description: "Basic view".to_string(),
            scope: "tool".to_string(),
            tools: vec!["some_tool".to_string()],
            data_hint: None,
            rule: None,
            display_mode: None,
            invoke_schema: None,
            url_patterns: vec![],
            standalone: false,
            standalone_label: None,
        });

        {
            let mut registry = state.plugin_registry.lock().unwrap();
            registry.add_plugin(manifest).unwrap();
        }

        let registry = state.plugin_registry.lock().unwrap();
        let results = collect_invocable_renderers(&registry.manifests);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["name"], "decision_detail");
        assert_eq!(results[0]["display_mode"], "drawer");
        assert_eq!(results[0]["plugin"], "test-invocable");
    }

    #[tokio::test]
    async fn test_version_guard_prevents_downgrade() {
        let (state, _dir) = test_app_state();

        // Install a plugin at version 2.0.0
        let mut manifest = test_manifest("guarded-plugin");
        manifest.version = "2.0.0".to_string();
        state.install_plugin_from_manifest(manifest, false).unwrap();

        // Create a registry entry at version 1.0.0 (older)
        let entry = test_registry_entry("guarded-plugin");
        {
            let mut cached = state.latest_registry.lock().unwrap();
            cached.push(entry);
        }

        // Simulate the version guard logic from update_plugin
        let result = {
            let cached = state.latest_registry.lock().unwrap();
            let entry = cached.iter().find(|e| e.name == "guarded-plugin").unwrap();
            let registry = state.plugin_registry.lock().unwrap();
            let installed = registry.manifests.iter().find(|m| m.name == "guarded-plugin").unwrap();
            let installed_ver = semver::Version::parse(&installed.version).ok();
            let available_ver = semver::Version::parse(&entry.version).ok();
            if let (Some(iv), Some(av)) = (installed_ver, available_ver) {
                if av <= iv {
                    Err(format!(
                        "Plugin '{}' is already up to date (version {})",
                        "guarded-plugin", installed.version
                    ))
                } else {
                    Ok(())
                }
            } else {
                Ok(())
            }
        };

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already up to date"));
    }
}
