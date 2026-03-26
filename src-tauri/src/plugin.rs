use mcp_mux_shared::{PluginAuth, PluginInfo, PluginManifest};
use mcp_mux_shared::plugin_store::PluginStore;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use crate::http_server::AsyncAppState;
use crate::tool_cache::ToolCache;

pub struct PluginRegistry {
    pub manifests: Vec<PluginManifest>,
    pub tool_cache: ToolCache,
}

impl PluginRegistry {
    /// Load all plugin manifests from ~/.mcp-mux/plugins/
    pub fn load_plugins() -> Self {
        let store = PluginStore::new();
        let manifests = match store.list() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[mcp-mux] Failed to load plugins: {}", e);
                return Self {
                    manifests: Vec::new(),
                    tool_cache: ToolCache::new(0),
                };
            }
        };

        for manifest in &manifests {
            eprintln!(
                "[mcp-mux] Loaded plugin: {} v{}",
                manifest.name, manifest.version
            );
        }

        let tool_cache = ToolCache::new(manifests.len());

        Self {
            manifests,
            tool_cache,
        }
    }

    /// Return indices of plugins whose tool cache is stale or empty
    pub fn stale_plugin_indices(&self) -> Vec<usize> {
        self.tool_cache
            .stale_indices(|i| self.manifests[i].mcp.is_some())
    }

    pub fn mark_refresh_pending(&mut self, idx: usize) {
        self.tool_cache.mark_pending(idx);
    }

    /// Refresh tool caches from plugin MCP backends
    pub async fn refresh_stale_plugins(
        state: &Arc<TokioMutex<AsyncAppState>>,
        client: &reqwest::Client,
    ) {
        // Collect info for plugins that need refresh
        let state_guard = state.lock().await;
        let to_refresh: Vec<(usize, String, Option<String>)> = {
            let registry = state_guard.inner.plugin_registry.lock().unwrap();
            let mut result = Vec::new();
            for i in 0..registry.manifests.len() {
                if registry.tool_cache.entries[i].refresh_pending {
                    if let Some(mcp) = &registry.manifests[i].mcp {
                        let auth = resolve_auth_header(&registry.manifests[i].name, &mcp.auth);
                        result.push((i, mcp.url.clone(), auth));
                    }
                }
            }
            result
        };
        drop(state_guard);

        for (idx, url, auth) in to_refresh {
            match fetch_plugin_tools(client, &url, auth.as_deref()).await {
                Ok(tools) => {
                    apply_tool_cache(state, idx, tools).await;
                }
                Err(e) => {
                    eprintln!("{}", e);
                    clear_refresh_pending(state, idx).await;
                }
            }
        }
    }

    /// Return all cached plugin tools
    pub fn all_tools(&self) -> Vec<Value> {
        self.tool_cache.all_tools()
    }

    /// Find which plugin handles a prefixed tool name.
    /// Returns (mcp_url, auth_header, unprefixed_name, renderer_map)
    pub fn find_plugin_for_tool(
        &self,
        prefixed_name: &str,
    ) -> Option<(String, Option<String>, String, HashMap<String, String>)> {
        let idx = self.tool_cache.tool_index.get(prefixed_name)?;
        let manifest = self.manifests.get(*idx)?;
        let mcp = manifest.mcp.as_ref()?;
        let unprefixed = prefixed_name.strip_prefix(&mcp.tool_prefix)?;
        let auth = resolve_auth_header(&manifest.name, &mcp.auth);

        Some((
            mcp.url.clone(),
            auth,
            unprefixed.to_string(),
            manifest.renderers.clone(),
        ))
    }

    /// Add a new plugin at runtime, persisting its manifest to disk.
    pub fn add_plugin(&mut self, manifest: PluginManifest) -> Result<(), String> {
        if self.manifests.iter().any(|m| m.name == manifest.name) {
            return Err(format!("Plugin '{}' is already installed", manifest.name));
        }

        let store = PluginStore::new();
        store.save(&manifest)?;

        eprintln!(
            "[mcp-mux] Installed plugin: {} v{}",
            manifest.name, manifest.version
        );

        self.manifests.push(manifest);
        self.tool_cache.push();

        Ok(())
    }

    /// Remove a plugin by name, deleting its manifest from disk.
    pub fn remove_plugin(&mut self, name: &str) -> Result<(), String> {
        let idx = self
            .manifests
            .iter()
            .position(|m| m.name == name)
            .ok_or_else(|| format!("Plugin '{}' not found", name))?;

        self.manifests.remove(idx);
        self.tool_cache.remove(idx);

        let store = PluginStore::new();
        // Ignore error if file already gone
        let _ = store.remove(name);

        self.tool_cache.rebuild_index();

        eprintln!("[mcp-mux] Uninstalled plugin: {}", name);
        Ok(())
    }

    /// Rebuild the tool_index from scratch based on current plugins and their cached tools.
    pub fn rebuild_tool_index(&mut self) {
        self.tool_cache.rebuild_index();
    }

    /// Return info about all loaded plugins.
    pub fn list_plugins(&self) -> Vec<PluginInfo> {
        self.manifests
            .iter()
            .enumerate()
            .map(|(i, manifest)| {
                let auth_type = manifest.mcp.as_ref().and_then(|m| {
                    m.auth.as_ref().map(|a| a.display_name().to_string())
                });
                let auth_configured = manifest
                    .mcp
                    .as_ref()
                    .and_then(|m| m.auth.as_ref())
                    .map(|a| a.is_configured(&manifest.name))
                    .unwrap_or(true); // no auth needed = considered "configured"
                PluginInfo {
                    name: manifest.name.clone(),
                    version: manifest.version.clone(),
                    has_mcp: manifest.mcp.is_some(),
                    auth_type,
                    auth_configured,
                    tool_count: self.tool_cache.tool_count(i),
                }
            })
            .collect()
    }
}

/// Perform the MCP initialize -> notifications/initialized -> tools/list handshake,
/// returning the raw tool definitions on success.
async fn fetch_plugin_tools(
    client: &reqwest::Client,
    url: &str,
    auth: Option<&str>,
) -> Result<Vec<Value>, String> {
    // Initialize handshake
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {
                "name": "mcp-mux",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    });

    let mut req_builder = client.post(url).json(&init_req);
    if let Some(auth_val) = auth {
        req_builder = req_builder.header("Authorization", auth_val);
    }

    let resp = req_builder
        .send()
        .await
        .map_err(|e| format!("[mcp-mux] Plugin initialize failed ({}): {}", url, e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "[mcp-mux] Plugin initialize returned HTTP {}",
            resp.status()
        ));
    }

    // Send initialized notification
    let notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let mut notif_builder = client.post(url).json(&notif);
    if let Some(auth_val) = auth {
        notif_builder = notif_builder.header("Authorization", auth_val);
    }
    let _ = notif_builder.send().await;

    // List tools
    let list_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    });
    let mut list_builder = client.post(url).json(&list_req);
    if let Some(auth_val) = auth {
        list_builder = list_builder.header("Authorization", auth_val);
    }

    let list_resp = list_builder
        .send()
        .await
        .map_err(|e| format!("[mcp-mux] Plugin tools/list failed: {}", e))?;
    if !list_resp.status().is_success() {
        return Err(format!(
            "[mcp-mux] Plugin tools/list returned HTTP {}",
            list_resp.status()
        ));
    }

    let body: Value = list_resp
        .json()
        .await
        .map_err(|e| format!("[mcp-mux] Failed to parse tools/list response: {}", e))?;

    Ok(body
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default())
}

/// Apply fetched tools to the plugin cache: prefix names, update tool_index, set timestamps.
async fn apply_tool_cache(
    state: &Arc<TokioMutex<AsyncAppState>>,
    idx: usize,
    tools: Vec<Value>,
) {
    let state_guard = state.lock().await;
    let mut registry = state_guard.inner.plugin_registry.lock().unwrap();

    let prefix = registry
        .manifests
        .get(idx)
        .and_then(|m| m.mcp.as_ref())
        .map(|m| m.tool_prefix.clone())
        .unwrap_or_default();

    registry.tool_cache.apply(idx, &prefix, tools);

    let tool_count = registry.tool_cache.tool_count(idx);
    let plugin_name = registry
        .manifests
        .get(idx)
        .map(|m| m.name.clone())
        .unwrap_or_default();

    eprintln!(
        "[mcp-mux] Refreshed {} tools from plugin '{}'",
        tool_count, plugin_name
    );
}

async fn clear_refresh_pending(state: &Arc<TokioMutex<AsyncAppState>>, idx: usize) {
    let state_guard = state.lock().await;
    let mut registry = state_guard.inner.plugin_registry.lock().unwrap();
    registry.tool_cache.clear_pending(idx);
}

fn resolve_auth_header(plugin_name: &str, auth: &Option<PluginAuth>) -> Option<String> {
    auth.as_ref()?.resolve_header(plugin_name)
}
