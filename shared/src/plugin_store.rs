use crate::{plugins_dir, PluginManifest};
use std::path::PathBuf;

pub struct PluginStore {
    dir: PathBuf,
}

impl PluginStore {
    /// Create a new PluginStore using the default plugins directory (~/.mcpviews/plugins/)
    pub fn new() -> Self {
        Self {
            dir: plugins_dir(),
        }
    }

    /// Create a PluginStore with a custom directory (useful for testing)
    pub fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Return a reference to the plugin directory path.
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Return the plugin directory path for a given plugin name
    pub fn plugin_dir(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// List all installed plugin manifests (supports both directory and legacy flat-file formats)
    pub fn list(&self) -> Result<Vec<PluginManifest>, String> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let entries = std::fs::read_dir(&self.dir)
            .map_err(|e| format!("Failed to read plugins directory: {}", e))?;

        let mut plugins = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();

            // Directory-based plugin: {name}/manifest.json
            if path.is_dir() {
                let manifest_path = path.join("manifest.json");
                if manifest_path.exists() {
                    match std::fs::read_to_string(&manifest_path) {
                        Ok(content) => match serde_json::from_str::<PluginManifest>(&content) {
                            Ok(manifest) => plugins.push(manifest),
                            Err(e) => {
                                eprintln!("[mcpviews] Failed to parse plugin {:?}: {}", manifest_path, e);
                            }
                        },
                        Err(e) => {
                            eprintln!("[mcpviews] Failed to read plugin {:?}: {}", manifest_path, e);
                        }
                    }
                }
            }
            // Legacy flat-file plugin: {name}.json
            else if path.extension().and_then(|e| e.to_str()) == Some("json") {
                match std::fs::read_to_string(&path) {
                    Ok(content) => match serde_json::from_str::<PluginManifest>(&content) {
                        Ok(manifest) => plugins.push(manifest),
                        Err(e) => {
                            eprintln!("[mcpviews] Failed to parse plugin {:?}: {}", path, e);
                        }
                    },
                    Err(e) => {
                        eprintln!("[mcpviews] Failed to read plugin {:?}: {}", path, e);
                    }
                }
            }
        }

        Ok(plugins)
    }

    /// Load a specific plugin manifest by name (prefers directory format, falls back to legacy)
    pub fn load(&self, name: &str) -> Result<PluginManifest, String> {
        // Try directory format first: {name}/manifest.json
        let dir_path = self.dir.join(name).join("manifest.json");
        if dir_path.exists() {
            let content = std::fs::read_to_string(&dir_path)
                .map_err(|e| format!("Failed to read plugin '{}': {}", name, e))?;
            return serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse plugin '{}': {}", name, e));
        }

        // Fall back to legacy flat-file format: {name}.json
        let legacy_path = self.dir.join(format!("{}.json", name));
        let content = std::fs::read_to_string(&legacy_path)
            .map_err(|e| format!("Failed to read plugin '{}': {}", name, e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse plugin '{}': {}", name, e))
    }

    /// Save a plugin manifest to disk (directory format: {name}/manifest.json)
    pub fn save(&self, manifest: &PluginManifest) -> Result<(), String> {
        let plugin_dir = self.dir.join(&manifest.name);
        std::fs::create_dir_all(&plugin_dir)
            .map_err(|e| format!("Failed to create plugin directory: {}", e))?;

        let path = plugin_dir.join("manifest.json");
        let json = serde_json::to_string_pretty(manifest)
            .map_err(|e| format!("Failed to serialize manifest: {}", e))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("Failed to write plugin file: {}", e))?;

        Ok(())
    }

    /// Remove a plugin from disk (supports both directory and legacy formats)
    pub fn remove(&self, name: &str) -> Result<(), String> {
        // Try directory format first
        let plugin_dir = self.dir.join(name);
        if plugin_dir.is_dir() {
            std::fs::remove_dir_all(&plugin_dir)
                .map_err(|e| format!("Failed to remove plugin '{}': {}", name, e))?;
            return Ok(());
        }

        // Fall back to legacy flat-file format
        let legacy_path = self.dir.join(format!("{}.json", name));
        if legacy_path.exists() {
            std::fs::remove_file(&legacy_path)
                .map_err(|e| format!("Failed to remove plugin '{}': {}", name, e))?;
            return Ok(());
        }

        Err(format!("Plugin '{}' is not installed", name))
    }

    /// Check if a plugin is installed (supports both directory and legacy formats)
    pub fn exists(&self, name: &str) -> bool {
        self.dir.join(name).join("manifest.json").exists()
            || self.dir.join(format!("{}.json", name)).exists()
    }

    /// Migrate legacy flat-file plugins ({name}.json) to directory format ({name}/manifest.json)
    pub fn migrate_legacy(&self) -> Result<(), String> {
        if !self.dir.exists() {
            return Ok(());
        }

        let entries = std::fs::read_dir(&self.dir)
            .map_err(|e| format!("Failed to read plugins directory: {}", e))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
                let name = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };

                let plugin_dir = self.dir.join(&name);
                let new_path = plugin_dir.join("manifest.json");

                // Skip if directory format already exists
                if new_path.exists() {
                    continue;
                }

                std::fs::create_dir_all(&plugin_dir)
                    .map_err(|e| format!("Failed to create directory for plugin '{}': {}", name, e))?;

                std::fs::rename(&path, &new_path)
                    .map_err(|e| format!("Failed to migrate plugin '{}': {}", name, e))?;

                eprintln!("[mcpviews] Migrated plugin '{}' to directory format", name);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginManifest;

    fn test_manifest(name: &str) -> PluginManifest {
        PluginManifest {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            renderers: std::collections::HashMap::new(),
            mcp: None,
            renderer_definitions: vec![],
            tool_rules: std::collections::HashMap::new(),
            no_auto_push: vec![],
        }
    }

    #[test]
    fn test_list_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::with_dir(dir.path().to_path_buf());
        let plugins = store.list().unwrap();
        assert!(plugins.is_empty());
    }

    #[test]
    fn test_save_then_list() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::with_dir(dir.path().to_path_buf());
        let manifest = test_manifest("test-plugin");
        store.save(&manifest).unwrap();
        let plugins = store.list().unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "test-plugin");
    }

    #[test]
    fn test_save_then_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::with_dir(dir.path().to_path_buf());
        let manifest = test_manifest("my-plugin");
        store.save(&manifest).unwrap();
        let loaded = store.load("my-plugin").unwrap();
        assert_eq!(loaded.name, "my-plugin");
        assert_eq!(loaded.version, "1.0.0");
    }

    #[test]
    fn test_exists_after_save() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::with_dir(dir.path().to_path_buf());
        assert!(!store.exists("foo"));
        store.save(&test_manifest("foo")).unwrap();
        assert!(store.exists("foo"));
    }

    #[test]
    fn test_remove_deletes_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::with_dir(dir.path().to_path_buf());
        store.save(&test_manifest("bar")).unwrap();
        assert!(store.exists("bar"));
        store.remove("bar").unwrap();
        assert!(!store.exists("bar"));
    }

    #[test]
    fn test_remove_nonexistent_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::with_dir(dir.path().to_path_buf());
        let result = store.remove("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_migrate_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::with_dir(dir.path().to_path_buf());

        // Create a legacy flat-file plugin
        let manifest = test_manifest("foo");
        let legacy_path = dir.path().join("foo.json");
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        std::fs::write(&legacy_path, &json).unwrap();

        // Verify legacy file exists
        assert!(legacy_path.exists());

        // Migrate
        store.migrate_legacy().unwrap();

        // Verify directory format exists and legacy file is gone
        assert!(dir.path().join("foo").join("manifest.json").exists());
        assert!(!legacy_path.exists());

        // Verify the content is preserved
        let loaded = store.load("foo").unwrap();
        assert_eq!(loaded.name, "foo");
    }

    #[test]
    fn test_list_mixed_formats() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::with_dir(dir.path().to_path_buf());

        // Create a directory-based plugin
        let dir_plugin = dir.path().join("dir-plugin");
        std::fs::create_dir_all(&dir_plugin).unwrap();
        let manifest1 = test_manifest("dir-plugin");
        let json1 = serde_json::to_string_pretty(&manifest1).unwrap();
        std::fs::write(dir_plugin.join("manifest.json"), &json1).unwrap();

        // Create a legacy flat-file plugin
        let manifest2 = test_manifest("legacy-plugin");
        let json2 = serde_json::to_string_pretty(&manifest2).unwrap();
        std::fs::write(dir.path().join("legacy-plugin.json"), &json2).unwrap();

        // list() should find both
        let plugins = store.list().unwrap();
        assert_eq!(plugins.len(), 2);
        let names: Vec<&str> = plugins.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"dir-plugin"));
        assert!(names.contains(&"legacy-plugin"));
    }

    #[test]
    fn test_load_prefers_directory() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::with_dir(dir.path().to_path_buf());

        // Create both formats for the same plugin name
        // Directory version has version "2.0.0"
        let plugin_dir = dir.path().join("foo");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let mut dir_manifest = test_manifest("foo");
        dir_manifest.version = "2.0.0".to_string();
        let json_dir = serde_json::to_string_pretty(&dir_manifest).unwrap();
        std::fs::write(plugin_dir.join("manifest.json"), &json_dir).unwrap();

        // Legacy version has version "1.0.0"
        let legacy_manifest = test_manifest("foo");
        let json_legacy = serde_json::to_string_pretty(&legacy_manifest).unwrap();
        std::fs::write(dir.path().join("foo.json"), &json_legacy).unwrap();

        // load() should prefer directory version
        let loaded = store.load("foo").unwrap();
        assert_eq!(loaded.version, "2.0.0");
    }

    #[test]
    fn test_remove_directory_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::with_dir(dir.path().to_path_buf());

        // Save creates directory format
        store.save(&test_manifest("removeme")).unwrap();
        assert!(dir.path().join("removeme").join("manifest.json").exists());

        // Remove should delete the directory
        store.remove("removeme").unwrap();
        assert!(!dir.path().join("removeme").exists());
        assert!(!store.exists("removeme"));
    }

    #[test]
    fn test_plugin_dir_helper() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::with_dir(dir.path().to_path_buf());
        let expected = dir.path().join("my-plugin");
        assert_eq!(store.plugin_dir("my-plugin"), expected);
    }
}
