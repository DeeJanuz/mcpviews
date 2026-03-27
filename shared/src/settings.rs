use serde::{Deserialize, Serialize};

use crate::{config_path, RegistrySource};

/// Typed representation of ~/.mcpviews/config.json
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    /// Legacy single registry URL (read for migration, omitted on save)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_url: Option<String>,

    /// Configured registry sources
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry_sources: Vec<RegistrySource>,
}

impl Settings {
    /// Load settings from ~/.mcpviews/config.json, returning defaults if the file
    /// does not exist or cannot be parsed.
    pub fn load() -> Self {
        let path = config_path();
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Persist settings to ~/.mcpviews/config.json.
    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("Failed to write config: {}", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = Settings::default();
        assert!(settings.registry_url.is_none());
        assert!(settings.registry_sources.is_empty());
    }

    #[test]
    fn test_serde_roundtrip() {
        let settings = Settings {
            registry_url: None,
            registry_sources: vec![RegistrySource {
                name: "Test".to_string(),
                url: "https://example.com/registry.json".to_string(),
                enabled: true,
            }],
        };
        let json = serde_json::to_string(&settings).unwrap();
        let parsed: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.registry_sources.len(), 1);
        assert_eq!(parsed.registry_sources[0].name, "Test");
    }

    #[test]
    fn test_deserialize_empty_object() {
        let parsed: Settings = serde_json::from_str("{}").unwrap();
        assert!(parsed.registry_url.is_none());
        assert!(parsed.registry_sources.is_empty());
    }

    #[test]
    fn test_deserialize_with_legacy_registry_url() {
        let json = r#"{"registry_url": "https://example.com/reg.json"}"#;
        let parsed: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.registry_url,
            Some("https://example.com/reg.json".to_string())
        );
        assert!(parsed.registry_sources.is_empty());
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let settings = Settings {
            registry_url: None,
            registry_sources: vec![RegistrySource {
                name: "Saved".to_string(),
                url: "https://saved.example.com".to_string(),
                enabled: false,
            }],
        };
        let json = serde_json::to_string_pretty(&settings).unwrap();
        std::fs::write(&path, &json).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let loaded: Settings = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.registry_sources.len(), 1);
        assert_eq!(loaded.registry_sources[0].name, "Saved");
        assert!(!loaded.registry_sources[0].enabled);
    }

    #[test]
    fn test_skip_serializing_empty_fields() {
        let settings = Settings::default();
        let json = serde_json::to_string(&settings).unwrap();
        // Should not contain registry_url or registry_sources when empty/None
        assert!(!json.contains("registry_url"));
        assert!(!json.contains("registry_sources"));
    }
}
