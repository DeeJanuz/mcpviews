use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use crate::http_server::AsyncAppState;
use crate::mcp_tools::{ensure_registry_fresh, collect_plugin_auth_status};

/// Build registry entry list from cached entries, installed manifests, and auth status.
/// Pure function for testability — no async or state locks.
pub(crate) fn build_registry_entries(
    cached: &[mcpviews_shared::RegistryEntry],
    manifests: &[mcpviews_shared::PluginManifest],
    auth_status: &[Value],
    tag_filter: Option<&str>,
) -> Vec<Value> {
    cached
        .iter()
        .filter(|entry| {
            if let Some(tag) = tag_filter {
                entry.tags.iter().any(|t| t.to_lowercase() == tag.to_lowercase())
            } else {
                true
            }
        })
        .map(|entry| {
            let installed_manifest = manifests.iter().find(|m| m.name == entry.name);
            let is_installed = installed_manifest.is_some();
            let installed_version = installed_manifest.map(|m| m.version.clone());
            let update_available = installed_manifest.and_then(|m| {
                mcpviews_shared::newer_version(&m.version, &entry.version)
                    .map(|v| v.to_string())
            });

            // Find auth info from collected status
            let auth_info = auth_status.iter().find(|s| s["plugin"] == entry.name);
            let auth_type = auth_info.and_then(|s| s["auth_type"].as_str());
            let auth_configured = auth_info
                .and_then(|s| s["auth_configured"].as_bool())
                .unwrap_or(false);

            serde_json::json!({
                "name": entry.name,
                "description": entry.description,
                "version": entry.version,
                "author": entry.author,
                "tags": entry.tags,
                "download_url": entry.download_url,
                "installed": is_installed,
                "installed_version": installed_version,
                "auth_type": auth_type,
                "auth_configured": if is_installed { auth_configured } else { false },
                "update_available": update_available,
            })
        })
        .collect()
}

pub(crate) async fn call_list_registry(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    // Ensure registry is populated
    ensure_registry_fresh(state).await;

    let tag_filter = arguments
        .get("tag")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());

    let result = {
        let state_guard = state.lock().await;
        let cached = state_guard.inner.latest_registry.lock().unwrap();
        let registry = state_guard.inner.plugin_registry.lock().unwrap();
        let auth_status = collect_plugin_auth_status(&registry.manifests);

        let entries = build_registry_entries(
            &cached,
            &registry.manifests,
            &auth_status,
            tag_filter.as_deref(),
        );

        serde_json::json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&serde_json::json!({
                    "plugins": entries,
                    "total": entries.len(),
                })).unwrap()
            }]
        })
    };

    Ok(result)
}

pub(crate) async fn call_start_plugin_auth(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    let plugin_name = arguments
        .get("plugin_name")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: plugin_name")?
        .to_string();

    let (auth, client) = {
        let state_guard = state.lock().await;
        let registry = state_guard.inner.plugin_registry.lock().unwrap();
        let auth = registry.resolve_plugin_auth(&plugin_name)?;
        let client = state_guard.inner.http_client.clone();
        (auth, client)
    };

    let result_text = match &auth {
        mcpviews_shared::PluginAuth::OAuth {
            client_id,
            auth_url,
            token_url,
            scopes,
        } => {
            match crate::auth::start_oauth_flow(
                &plugin_name,
                client_id.as_deref(),
                auth_url,
                token_url,
                scopes,
                &client,
            )
            .await
            {
                Ok(_token) => format!(
                    "OAuth authentication for '{}' completed successfully.",
                    plugin_name
                ),
                Err(e) => {
                    return Err(format!("OAuth flow failed for '{}': {}", plugin_name, e))
                }
            }
        }
        mcpviews_shared::PluginAuth::Bearer { token_env } => match std::env::var(token_env) {
            Ok(_) => format!(
                "Bearer token for '{}' is configured via env var '{}'.",
                plugin_name, token_env
            ),
            Err(_) => {
                return Err(format!(
                    "Environment variable '{}' is not set. Set it and restart.",
                    token_env
                ))
            }
        },
        mcpviews_shared::PluginAuth::ApiKey { key_env, .. } => {
            if let Some(env_var) = key_env {
                match std::env::var(env_var) {
                    Ok(_) => format!(
                        "API key for '{}' is configured via env var '{}'.",
                        plugin_name, env_var
                    ),
                    Err(_) => {
                        return Err(format!(
                            "Environment variable '{}' is not set. Set it and restart.",
                            env_var
                        ))
                    }
                }
            } else {
                return Err("No key_env configured for this plugin".to_string());
            }
        }
    };

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": result_text
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry_entry(name: &str) -> mcpviews_shared::RegistryEntry {
        mcpviews_shared::RegistryEntry {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            description: "Test".to_string(),
            author: Some("Test Author".to_string()),
            homepage: None,
            manifest: crate::test_utils::test_manifest(name),
            tags: vec!["test".to_string()],
            download_url: Some("https://example.com/plugin.zip".to_string()),
            manifest_url: None,
        }
    }

    fn test_registry_entry_with_version(
        name: &str,
        version: &str,
        tags: Vec<&str>,
    ) -> mcpviews_shared::RegistryEntry {
        mcpviews_shared::RegistryEntry {
            name: name.to_string(),
            version: version.to_string(),
            description: format!("Test plugin {}", name),
            author: Some("Test Author".to_string()),
            homepage: None,
            manifest: {
                let mut m = crate::test_utils::test_manifest(name);
                m.version = version.to_string();
                m
            },
            tags: tags.into_iter().map(|s| s.to_string()).collect(),
            download_url: Some("https://example.com/plugin.zip".to_string()),
            manifest_url: None,
        }
    }

    fn test_manifest_with_version(name: &str, version: &str) -> mcpviews_shared::PluginManifest {
        let mut m = crate::test_utils::test_manifest(name);
        m.version = version.to_string();
        m
    }

    #[test]
    fn test_build_registry_entries_empty() {
        let result = build_registry_entries(&[], &[], &[], None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_registry_entries_uninstalled_plugin() {
        let cached = vec![test_registry_entry("test-plugin")];
        let result = build_registry_entries(&cached, &[], &[], None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["name"], "test-plugin");
        assert_eq!(result[0]["installed"], false);
        assert_eq!(result[0]["auth_configured"], false);
        assert!(result[0]["installed_version"].is_null());
    }

    #[test]
    fn test_build_registry_entries_installed_plugin() {
        let cached = vec![test_registry_entry("test-plugin")];
        let manifests = vec![crate::test_utils::test_manifest("test-plugin")];
        let result = build_registry_entries(&cached, &manifests, &[], None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["installed"], true);
        assert_eq!(result[0]["installed_version"], "1.0.0");
    }

    #[test]
    fn test_build_registry_entries_tag_filter() {
        let cached = vec![
            test_registry_entry_with_version("plugin-a", "1.0.0", vec!["analytics"]),
            test_registry_entry_with_version("plugin-b", "1.0.0", vec!["data"]),
            test_registry_entry_with_version("plugin-c", "1.0.0", vec!["analytics", "data"]),
        ];
        let result = build_registry_entries(&cached, &[], &[], Some("analytics"));
        assert_eq!(result.len(), 2);
        let names: Vec<&str> = result.iter().map(|r| r["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"plugin-a"));
        assert!(names.contains(&"plugin-c"));
    }

    #[test]
    fn test_build_registry_entries_update_available() {
        let cached = vec![test_registry_entry_with_version(
            "test-plugin",
            "2.0.0",
            vec![],
        )];
        let manifests = vec![test_manifest_with_version("test-plugin", "1.0.0")];
        let result = build_registry_entries(&cached, &manifests, &[], None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["update_available"], "2.0.0");
    }

    #[test]
    fn test_build_registry_entries_no_update_when_same_version() {
        let cached = vec![test_registry_entry_with_version(
            "test-plugin",
            "1.0.0",
            vec![],
        )];
        let manifests = vec![test_manifest_with_version("test-plugin", "1.0.0")];
        let result = build_registry_entries(&cached, &manifests, &[], None);
        assert_eq!(result.len(), 1);
        assert!(result[0]["update_available"].is_null());
    }

    #[test]
    fn test_build_registry_entries_auth_status() {
        let cached = vec![test_registry_entry("test-plugin")];
        let manifests = vec![crate::test_utils::test_manifest("test-plugin")];
        let auth_status = vec![serde_json::json!({
            "plugin": "test-plugin",
            "auth_type": "bearer",
            "auth_configured": true,
        })];
        let result = build_registry_entries(&cached, &manifests, &auth_status, None);
        assert_eq!(result[0]["auth_type"], "bearer");
        assert_eq!(result[0]["auth_configured"], true);
    }

    #[test]
    fn test_build_registry_entries_auth_not_shown_when_uninstalled() {
        let cached = vec![test_registry_entry("test-plugin")];
        // Not installed — no manifests
        let auth_status = vec![serde_json::json!({
            "plugin": "test-plugin",
            "auth_type": "bearer",
            "auth_configured": true,
        })];
        let result = build_registry_entries(&cached, &[], &auth_status, None);
        // Even though auth_status says configured, uninstalled plugins show false
        assert_eq!(result[0]["auth_configured"], false);
    }
}
