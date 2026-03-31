use crate::{cache_dir, config_path, PluginManifest, RegistrySource, RemoteRegistry, RegistryEntry};

pub const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/DeeJanuz/mcpviews/master/registry/registry.json";

const CACHE_TTL_SECS: u64 = 3600; // 1 hour

/// Read the configured registry URL from config.json, falling back to DEFAULT_REGISTRY_URL
pub fn get_configured_registry_url() -> String {
    if let Ok(content) = std::fs::read_to_string(config_path()) {
        if let Ok(config) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(url) = config.get("registry_url").and_then(|v| v.as_str()) {
                return url.to_string();
            }
        }
    }
    DEFAULT_REGISTRY_URL.to_string()
}

/// Fetch registry entries, using a 1-hour disk cache
pub async fn fetch_registry(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<RegistryEntry>, String> {
    // Check cache first
    let cache_path = cache_dir().join("registry.json");
    if let Ok(metadata) = std::fs::metadata(&cache_path) {
        if let Ok(modified) = metadata.modified() {
            if modified
                .elapsed()
                .map(|d| d.as_secs())
                .unwrap_or(u64::MAX)
                < CACHE_TTL_SECS
            {
                if let Ok(content) = std::fs::read_to_string(&cache_path) {
                    if let Ok(registry) = serde_json::from_str::<RemoteRegistry>(&content) {
                        return Ok(registry.plugins);
                    }
                }
            }
        }
    }

    // Fetch from remote
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch registry: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Registry returned HTTP {}", resp.status()));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read registry response: {}", e))?;

    let registry: RemoteRegistry = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse registry: {}", e))?;

    // Write to cache
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = std::fs::write(&cache_path, &body);

    Ok(registry.plugins)
}

/// Read registry sources from config.json. Falls back to single registry_url or default.
pub fn get_registry_sources() -> Vec<RegistrySource> {
    if let Ok(content) = std::fs::read_to_string(config_path()) {
        if let Ok(config) = serde_json::from_str::<serde_json::Value>(&content) {
            // Check for new multi-source format first
            if let Some(sources) = config.get("registry_sources") {
                if let Ok(sources) = serde_json::from_value::<Vec<RegistrySource>>(sources.clone())
                {
                    if !sources.is_empty() {
                        return sources;
                    }
                }
            }
            // Fall back to single URL
            if let Some(url) = config.get("registry_url").and_then(|v| v.as_str()) {
                return vec![RegistrySource {
                    name: "Default".to_string(),
                    url: url.to_string(),
                    enabled: true,
                }];
            }
        }
    }
    vec![RegistrySource {
        name: "Default".to_string(),
        url: DEFAULT_REGISTRY_URL.to_string(),
        enabled: true,
    }]
}

/// Save registry sources to config.json (preserving other config fields)
pub fn save_registry_sources(sources: &[RegistrySource]) -> Result<(), String> {
    let path = config_path();
    let mut config = if let Ok(content) = std::fs::read_to_string(&path) {
        serde_json::from_str::<serde_json::Value>(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    config["registry_sources"] = serde_json::to_value(sources)
        .map_err(|e| format!("Failed to serialize sources: {}", e))?;

    // Remove legacy registry_url if present
    if let Some(obj) = config.as_object_mut() {
        obj.remove("registry_url");
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config dir: {}", e))?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap())
        .map_err(|e| format!("Failed to write config: {}", e))?;

    Ok(())
}

/// Fetch from all enabled registry sources, merge results (last source wins on name conflict)
pub async fn fetch_all_registries(
    client: &reqwest::Client,
    sources: &[RegistrySource],
) -> Result<Vec<RegistryEntry>, String> {
    let mut all_entries: Vec<RegistryEntry> = Vec::new();
    let mut seen_names: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut any_success = false;

    for source in sources {
        if !source.enabled {
            continue;
        }

        // Use per-source cache file
        match fetch_registry_with_cache(client, &source.url, &source.name).await {
            Ok(entries) => {
                any_success = true;
                for entry in entries {
                    if let Some(idx) = seen_names.get(&entry.name) {
                        // Replace with newer version if available
                        all_entries[*idx] = entry;
                    } else {
                        seen_names.insert(entry.name.clone(), all_entries.len());
                        all_entries.push(entry);
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "[mcpviews] Failed to fetch registry '{}': {}",
                    source.name, e
                );
            }
        }
    }

    if !any_success && !sources.is_empty() {
        // All remote sources failed — fall back to bundled registry
        eprintln!("[mcpviews] All remote registry sources failed, using bundled registry");
        let bundled = include_str!("bundled_registry.json");
        let registry: RemoteRegistry = serde_json::from_str(bundled)
            .expect("bundled_registry.json is compiled-in and must always parse; this is a build-time bug");
        return Ok(registry.plugins);
    }

    // Resolve remote manifest URLs for entries that have manifest_url set
    let all_entries = resolve_manifest_urls(client, all_entries).await;

    Ok(all_entries)
}

/// Fetch with per-source cache
async fn fetch_registry_with_cache(
    client: &reqwest::Client,
    url: &str,
    source_name: &str,
) -> Result<Vec<RegistryEntry>, String> {
    // Per-source cache file based on a simple hash of the URL
    let hash = url
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    let cache_path = cache_dir().join(format!("registry-{:x}.json", hash));

    // Check cache
    if let Ok(metadata) = std::fs::metadata(&cache_path) {
        if let Ok(modified) = metadata.modified() {
            if modified
                .elapsed()
                .map(|d| d.as_secs())
                .unwrap_or(u64::MAX)
                < CACHE_TTL_SECS
            {
                if let Ok(content) = std::fs::read_to_string(&cache_path) {
                    if let Ok(registry) = serde_json::from_str::<RemoteRegistry>(&content) {
                        return Ok(registry.plugins);
                    }
                }
            }
        }
    }

    // Fetch from remote
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch '{}': {}", source_name, e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "'{}' returned HTTP {}",
            source_name,
            resp.status()
        ));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response from '{}': {}", source_name, e))?;
    let registry: RemoteRegistry = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse registry '{}': {}", source_name, e))?;

    // Write to cache
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = std::fs::write(&cache_path, &body);

    Ok(registry.plugins)
}

/// For each entry with a `manifest_url`, fetch the remote manifest.json and
/// update the entry's `manifest`, `version`, and `download_url` fields.
/// On fetch failure, log a warning and leave the entry unchanged.
pub async fn resolve_manifest_urls(
    client: &reqwest::Client,
    entries: Vec<RegistryEntry>,
) -> Vec<RegistryEntry> {
    use futures::future::join_all;

    let tasks: Vec<_> = entries
        .into_iter()
        .map(|entry| {
            let client = client.clone();
            async move {
                let url = match &entry.manifest_url {
                    Some(url) => url.clone(),
                    None => return entry,
                };

                match fetch_remote_manifest(&client, &url).await {
                    Ok(remote_manifest) => {
                        let mut entry = entry;
                        // Update version from remote manifest
                        entry.version = remote_manifest.version.clone();
                        // If the remote manifest has a download_url, set it on the entry
                        // (overrides any existing entry-level download_url)
                        if remote_manifest.download_url.is_some() {
                            entry.download_url = remote_manifest.download_url.clone();
                        }
                        // Replace inline manifest with the fetched one
                        entry.manifest = remote_manifest;
                        entry
                    }
                    Err(e) => {
                        eprintln!(
                            "[mcpviews] Failed to fetch manifest from '{}': {}",
                            url, e
                        );
                        entry
                    }
                }
            }
        })
        .collect();

    join_all(tasks).await
}

/// Fetch and parse a remote manifest.json from the given URL.
async fn fetch_remote_manifest(
    client: &reqwest::Client,
    url: &str,
) -> Result<PluginManifest, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    serde_json::from_str::<PluginManifest>(&body)
        .map_err(|e| format!("Failed to parse manifest: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_registry_url_is_valid_https() {
        assert!(DEFAULT_REGISTRY_URL.starts_with("https://"));
    }

    #[test]
    fn test_get_configured_registry_url_returns_default_when_no_config() {
        // config_path() points to ~/.mcpviews/config.json which likely doesn't exist in CI
        // If it does exist and has a registry_url, this test just verifies we get a non-empty string
        let url = get_configured_registry_url();
        assert!(!url.is_empty());
        assert!(url.starts_with("https://"));
    }

    fn test_registry_entry(name: &str) -> RegistryEntry {
        RegistryEntry {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            description: "Test plugin".to_string(),
            author: None,
            homepage: None,
            manifest: PluginManifest {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                renderers: std::collections::HashMap::new(),
                mcp: None,
                renderer_definitions: vec![],
                tool_rules: std::collections::HashMap::new(),
                no_auto_push: vec![],
                registry_index: None,
                download_url: None,
            },
            tags: vec![],
            download_url: None,
            manifest_url: None,
        }
    }

    #[tokio::test]
    async fn test_resolve_manifest_urls_no_manifest_url() {
        let client = reqwest::Client::new();
        let entry = test_registry_entry("test-plugin");
        let entries = vec![entry.clone()];
        let resolved = resolve_manifest_urls(&client, entries).await;
        assert_eq!(resolved.len(), 1);
        // Should be unchanged
        assert_eq!(resolved[0].version, "1.0.0");
        assert!(resolved[0].manifest_url.is_none());
    }

    #[tokio::test]
    async fn test_resolve_manifest_urls_fetch_failure_falls_back() {
        let client = reqwest::Client::new();
        let mut entry = test_registry_entry("test-plugin");
        entry.manifest_url = Some("https://invalid.example.com/nonexistent/manifest.json".to_string());

        let entries = vec![entry];
        let resolved = resolve_manifest_urls(&client, entries).await;
        assert_eq!(resolved.len(), 1);
        // Should fall back to original version on failure
        assert_eq!(resolved[0].version, "1.0.0");
        assert_eq!(resolved[0].manifest.version, "1.0.0");
    }
}
