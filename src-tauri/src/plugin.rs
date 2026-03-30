use mcpviews_shared::{PluginAuth, PluginInfo, PluginManifest};
use mcpviews_shared::plugin_store::PluginStore;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use crate::http_server::AsyncAppState;
use crate::tool_cache::ToolCache;

/// OAuth refresh info extracted while holding the lock, used after dropping it.
pub(crate) struct OAuthRefreshInfo {
    pub plugin_name: String,
    pub token_url: String,
    pub client_id: Option<String>,
}

/// Result of looking up a plugin tool by prefixed name.
pub(crate) struct PluginToolResult {
    pub mcp_url: String,
    pub auth_header: Option<String>,
    pub unprefixed_name: String,
    pub oauth_info: Option<OAuthRefreshInfo>,
}

/// Attempt OAuth token refresh, returning "Bearer {token}" on success.
pub async fn try_refresh_oauth(
    oauth_info: &OAuthRefreshInfo,
    client: &reqwest::Client,
) -> Option<String> {
    match crate::auth::refresh_oauth_token(
        &oauth_info.plugin_name,
        &oauth_info.token_url,
        oauth_info.client_id.as_deref(),
        client,
    )
    .await
    {
        Ok(token) => {
            eprintln!(
                "[mcpviews] Auto-refreshed token for '{}'",
                oauth_info.plugin_name
            );
            Some(format!("Bearer {}", token))
        }
        Err(e) => {
            eprintln!(
                "[mcpviews] Token refresh failed for '{}': {}",
                oauth_info.plugin_name, e
            );
            None
        }
    }
}

pub struct PluginRegistry {
    pub manifests: Vec<PluginManifest>,
    pub tool_cache: ToolCache,
    store: PluginStore,
}

impl PluginRegistry {
    /// Load all plugin manifests using a provided PluginStore (useful for testing).
    pub fn load_plugins_with_store(store: PluginStore) -> Self {
        // Migrate legacy flat-file plugins to directory format
        if let Err(e) = store.migrate_legacy() {
            eprintln!("[mcpviews] Legacy plugin migration warning: {}", e);
        }
        let manifests = match store.list() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[mcpviews] Failed to load plugins: {}", e);
                return Self {
                    manifests: Vec::new(),
                    tool_cache: ToolCache::new(0),
                    store,
                };
            }
        };

        for manifest in &manifests {
            eprintln!(
                "[mcpviews] Loaded plugin: {} v{}",
                manifest.name, manifest.version
            );
        }

        let tool_cache = ToolCache::new(manifests.len());

        Self {
            manifests,
            tool_cache,
            store,
        }
    }

    pub fn find_plugin_by_name(&self, name: &str) -> Option<(usize, &PluginManifest)> {
        self.manifests.iter().enumerate().find(|(_, m)| m.name == name)
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
        let mut to_refresh: Vec<(usize, String, Option<String>, Option<OAuthRefreshInfo>)> = {
            let registry = state_guard.inner.plugin_registry.lock().unwrap();
            let mut result = Vec::new();
            for i in 0..registry.manifests.len() {
                if registry.tool_cache.entries[i].refresh_pending {
                    if let Some(mcp) = &registry.manifests[i].mcp {
                        let auth = resolve_auth_header(&registry.manifests[i].name, &mcp.auth);
                        let oauth_info = if auth.is_none() {
                            extract_oauth_refresh_info(&registry.manifests[i].name, &mcp.auth)
                        } else {
                            None
                        };
                        result.push((i, mcp.url.clone(), auth, oauth_info));
                    }
                }
            }
            result
        };
        drop(state_guard);

        // Attempt OAuth token refresh for entries where auth is None but OAuth info is present
        for entry in &mut to_refresh {
            if entry.2.is_none() {
                if let Some(oauth_info) = &entry.3 {
                    if let Some(bearer) = try_refresh_oauth(oauth_info, client).await {
                        entry.2 = Some(bearer);
                    }
                }
            }
        }

        for (idx, url, auth, _) in to_refresh {
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
    pub fn find_plugin_for_tool(
        &self,
        prefixed_name: &str,
    ) -> Option<PluginToolResult> {
        let idx = self.tool_cache.tool_index.get(prefixed_name)?;
        let manifest = self.manifests.get(*idx)?;
        let mcp = manifest.mcp.as_ref()?;
        let unprefixed = prefixed_name.strip_prefix(&mcp.tool_prefix)?;
        let auth = resolve_auth_header(&manifest.name, &mcp.auth);
        let oauth_info = if auth.is_none() {
            extract_oauth_refresh_info(&manifest.name, &mcp.auth)
        } else {
            None
        };

        Some(PluginToolResult {
            mcp_url: mcp.url.clone(),
            auth_header: auth,
            unprefixed_name: unprefixed.to_string(),
            oauth_info,
        })
    }

    /// Add a new plugin at runtime, persisting its manifest to disk.
    pub fn add_plugin(&mut self, manifest: PluginManifest) -> Result<(), String> {
        if self.manifests.iter().any(|m| m.name == manifest.name) {
            return Err(format!("Plugin '{}' is already installed", manifest.name));
        }

        self.store.save(&manifest)?;

        eprintln!(
            "[mcpviews] Installed plugin: {} v{}",
            manifest.name, manifest.version
        );

        self.manifests.push(manifest);
        self.tool_cache.push();

        Ok(())
    }

    /// Remove a plugin by name, deleting its manifest from disk.
    pub fn remove_plugin(&mut self, name: &str) -> Result<(), String> {
        self.remove_plugin_in_memory(name)?;

        // Ignore error if file already gone
        let _ = self.store.remove(name);

        eprintln!("[mcpviews] Uninstalled plugin: {}", name);
        Ok(())
    }

    /// Remove a plugin from in-memory state only (manifests vec + tool cache).
    /// Does NOT delete files from disk. Used by zip-based install paths where
    /// the extraction has already placed files on disk and we don't want to
    /// delete them before re-adding the plugin.
    pub fn remove_plugin_in_memory(&mut self, name: &str) -> Result<(), String> {
        let idx = self
            .manifests
            .iter()
            .position(|m| m.name == name)
            .ok_or_else(|| format!("Plugin '{}' not found", name))?;

        self.manifests.remove(idx);
        self.tool_cache.remove(idx);
        self.tool_cache.rebuild_index();

        Ok(())
    }

    /// Return info about all loaded plugins, checking for updates against registry.
    pub fn list_plugins_with_updates(&self, registry_entries: &[mcpviews_shared::RegistryEntry]) -> Vec<PluginInfo> {
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

                // Check for updates
                let update_available = registry_entries
                    .iter()
                    .find(|e| e.name == manifest.name)
                    .and_then(|e| {
                        let installed = semver::Version::parse(&manifest.version).ok()?;
                        let available = semver::Version::parse(&e.version).ok()?;
                        if available > installed {
                            Some(e.version.clone())
                        } else {
                            None
                        }
                    });

                PluginInfo {
                    name: manifest.name.clone(),
                    version: manifest.version.clone(),
                    has_mcp: manifest.mcp.is_some(),
                    auth_type,
                    auth_configured,
                    tool_count: self.tool_cache.tool_count(i),
                    update_available,
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
                "name": "mcpviews",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    });

    let mut req_builder = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .json(&init_req);
    if let Some(auth_val) = auth {
        req_builder = req_builder.header("Authorization", auth_val);
    }

    let resp = req_builder
        .send()
        .await
        .map_err(|e| format!("[mcpviews] Plugin initialize failed ({}): {}", url, e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "[mcpviews] Plugin initialize returned HTTP {}",
            resp.status()
        ));
    }

    // Send initialized notification
    let notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let mut notif_builder = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .json(&notif);
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
    let mut list_builder = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .json(&list_req);
    if let Some(auth_val) = auth {
        list_builder = list_builder.header("Authorization", auth_val);
    }

    let list_resp = list_builder
        .send()
        .await
        .map_err(|e| format!("[mcpviews] Plugin tools/list failed: {}", e))?;
    if !list_resp.status().is_success() {
        return Err(format!(
            "[mcpviews] Plugin tools/list returned HTTP {}",
            list_resp.status()
        ));
    }

    let body: Value = list_resp
        .json()
        .await
        .map_err(|e| format!("[mcpviews] Failed to parse tools/list response: {}", e))?;

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
        "[mcpviews] Refreshed {} tools from plugin '{}'",
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

/// Extract OAuth refresh info from a plugin's auth config, if it's an OAuth type.
fn extract_oauth_refresh_info(plugin_name: &str, auth: &Option<PluginAuth>) -> Option<OAuthRefreshInfo> {
    match auth.as_ref()? {
        PluginAuth::OAuth {
            client_id,
            token_url,
            ..
        } => Some(OAuthRefreshInfo {
            plugin_name: plugin_name.to_string(),
            token_url: token_url.clone(),
            client_id: client_id.clone(),
        }),
        _ => None,
    }
}

