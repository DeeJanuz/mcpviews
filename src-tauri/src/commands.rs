use std::collections::HashMap;
use std::sync::Arc;
use tauri::State;

use mcp_mux_shared::{PluginAuth, PluginInfo, PluginManifest, RegistryEntry};

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
    registry.list_plugins()
}

#[tauri::command]
pub fn install_plugin(
    manifest_json: String,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let manifest: PluginManifest = serde_json::from_str(&manifest_json)
        .map_err(|e| format!("Invalid manifest: {}", e))?;
    let mut registry = state.plugin_registry.lock().unwrap();
    registry.add_plugin(manifest)
}

#[tauri::command]
pub fn uninstall_plugin(name: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let mut registry = state.plugin_registry.lock().unwrap();
    registry.remove_plugin(&name)
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
    registry.add_plugin(manifest)
}

#[tauri::command]
pub async fn fetch_registry(
    registry_url: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<RegistryEntry>, String> {
    let url = registry_url.unwrap_or_else(|| crate::registry::get_configured_registry_url());
    let client = state.http_client.clone();
    crate::registry::fetch_registry(&client, &url).await
}

#[tauri::command]
pub async fn start_plugin_auth(
    plugin_name: String,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let auth = {
        let registry = state.plugin_registry.lock().unwrap();
        let manifest = registry
            .manifests
            .iter()
            .find(|m| m.name == plugin_name)
            .ok_or_else(|| format!("Plugin '{}' not found", plugin_name))?;
        manifest
            .mcp
            .as_ref()
            .and_then(|m| m.auth.clone())
            .ok_or_else(|| format!("Plugin '{}' has no auth config", plugin_name))?
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
                client_id,
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
pub fn store_plugin_token(plugin_name: String, token: String) -> Result<(), String> {
    crate::auth::store_api_key(&plugin_name, &token)
}

#[tauri::command]
pub fn get_settings() -> Result<serde_json::Value, String> {
    let path = mcp_mux_shared::config_path();
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read config: {}", e))?;
    let config: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse config: {}", e))?;
    Ok(config)
}

#[tauri::command]
pub fn save_settings(settings: serde_json::Value) -> Result<(), String> {
    let path = mcp_mux_shared::config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;
    }
    let json = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write config: {}", e))?;
    Ok(())
}
