pub mod package;
pub mod plugin_store;
pub mod registry;
pub mod token_store;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub renderers: HashMap<String, String>,
    pub mcp: Option<PluginMcpConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginMcpConfig {
    pub url: String,
    pub auth: Option<PluginAuth>,
    pub tool_prefix: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginAuth {
    Bearer {
        token_env: String,
    },
    ApiKey {
        #[serde(default = "default_api_key_header")]
        header_name: String,
        key_env: Option<String>,
    },
    OAuth {
        #[serde(default)]
        client_id: Option<String>,
        auth_url: String,
        token_url: String,
        #[serde(default)]
        scopes: Vec<String>,
    },
}

fn default_api_key_header() -> String {
    "X-API-Key".to_string()
}

impl fmt::Display for PluginAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

impl PluginAuth {
    pub fn display_name(&self) -> &'static str {
        match self {
            PluginAuth::Bearer { .. } => "bearer",
            PluginAuth::ApiKey { .. } => "api_key",
            PluginAuth::OAuth { .. } => "oauth",
        }
    }

    /// Check if auth is configured for a plugin (uses default auth_dir).
    pub fn is_configured(&self, plugin_name: &str) -> bool {
        self.is_configured_with_auth_dir(plugin_name, &auth_dir())
    }

    /// Check if auth is configured, with a custom auth directory (for testing).
    pub fn is_configured_with_auth_dir(&self, plugin_name: &str, dir: &std::path::Path) -> bool {
        if token_store::has_stored_token(dir, plugin_name) {
            return true;
        }
        // For Bearer/ApiKey: also check env var as fallback
        match self {
            PluginAuth::Bearer { token_env } => std::env::var(token_env).is_ok(),
            PluginAuth::ApiKey { key_env, .. } => {
                key_env
                    .as_ref()
                    .map(|e| std::env::var(e).is_ok())
                    .unwrap_or(false)
            }
            PluginAuth::OAuth { .. } => false, // OAuth only uses stored tokens
        }
    }

    /// Resolve the auth header value for this auth config.
    /// For Bearer/ApiKey: checks stored token first, then falls back to env var.
    /// For OAuth: reads stored token from auth_dir(), returns "Bearer {token}"
    pub fn resolve_header(&self, plugin_name: &str) -> Option<String> {
        self.resolve_header_with_auth_dir(plugin_name, &auth_dir())
    }

    /// Resolve the auth header with a custom auth directory (for testing).
    pub fn resolve_header_with_auth_dir(
        &self,
        plugin_name: &str,
        dir: &std::path::Path,
    ) -> Option<String> {
        match self {
            PluginAuth::Bearer { token_env } => {
                // Check stored token first
                if let Some(stored) = token_store::load_stored_token(dir, plugin_name) {
                    return Some(format!("Bearer {}", stored.access_token));
                }
                // Fall back to env var
                match std::env::var(token_env) {
                    Ok(token) => Some(format!("Bearer {}", token)),
                    Err(_) => {
                        eprintln!("[mcp-mux] Auth env var '{}' not set", token_env);
                        None
                    }
                }
            }
            PluginAuth::ApiKey {
                header_name,
                key_env,
            } => {
                // Check stored token first
                if let Some(stored) = token_store::load_stored_token(dir, plugin_name) {
                    return Some(format!("{}:{}", header_name, stored.access_token));
                }
                // Fall back to env var
                if let Some(env_var) = key_env {
                    match std::env::var(env_var) {
                        Ok(key) => Some(format!("{}:{}", header_name, key)),
                        Err(_) => {
                            eprintln!("[mcp-mux] Auth env var '{}' not set", env_var);
                            None
                        }
                    }
                } else {
                    None
                }
            }
            PluginAuth::OAuth { .. } => {
                let stored = token_store::load_stored_token(dir, plugin_name)?;
                Some(format!("Bearer {}", stored.access_token))
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RemoteRegistry {
    pub version: String,
    pub plugins: Vec<RegistryEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegistryEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: Option<String>,
    pub homepage: Option<String>,
    pub manifest: PluginManifest,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub download_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySource {
    pub name: String,
    pub url: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub has_mcp: bool,
    pub auth_type: Option<String>,
    pub auth_configured: bool,
    pub tool_count: usize,
    pub update_available: Option<String>,
}

pub fn plugins_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mcp-mux")
        .join("plugins")
}

pub fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mcp-mux")
        .join("config.json")
}

pub fn auth_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mcp-mux")
        .join("auth")
}

pub fn cache_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mcp-mux")
        .join("cache")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_name_bearer() {
        let auth = PluginAuth::Bearer {
            token_env: "MY_TOKEN".to_string(),
        };
        assert_eq!(auth.display_name(), "bearer");
    }

    #[test]
    fn test_display_name_api_key() {
        let auth = PluginAuth::ApiKey {
            header_name: "X-API-Key".to_string(),
            key_env: None,
        };
        assert_eq!(auth.display_name(), "api_key");
    }

    #[test]
    fn test_display_name_oauth() {
        let auth = PluginAuth::OAuth {
            client_id: Some("id".to_string()),
            auth_url: "https://example.com/auth".to_string(),
            token_url: "https://example.com/token".to_string(),
            scopes: vec![],
        };
        assert_eq!(auth.display_name(), "oauth");
    }

    #[test]
    fn test_is_configured_with_stored_token() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("test-plugin.json");
        std::fs::write(
            &token_path,
            r#"{"access_token":"tok123","refresh_token":null,"expires_at":null}"#,
        )
        .unwrap();

        let auth = PluginAuth::Bearer {
            token_env: "NONEXISTENT_ENV_VAR_12345".to_string(),
        };
        // is_configured should return true when a stored token file exists
        assert!(auth.is_configured_with_auth_dir("test-plugin", dir.path()));
    }

    #[test]
    fn test_is_configured_bearer_env_fallback() {
        let dir = tempfile::tempdir().unwrap();
        // No stored token file, but env var is set
        std::env::set_var("TEST_BEARER_TOKEN_XYZ", "some-token");
        let auth = PluginAuth::Bearer {
            token_env: "TEST_BEARER_TOKEN_XYZ".to_string(),
        };
        assert!(auth.is_configured_with_auth_dir("no-stored-token-plugin", dir.path()));
        std::env::remove_var("TEST_BEARER_TOKEN_XYZ");
    }

    #[test]
    fn test_is_configured_bearer_neither() {
        let dir = tempfile::tempdir().unwrap();
        let auth = PluginAuth::Bearer {
            token_env: "NONEXISTENT_ENV_VAR_99999".to_string(),
        };
        assert!(!auth.is_configured_with_auth_dir("missing-plugin", dir.path()));
    }

    #[test]
    fn test_is_configured_apikey_env_fallback() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("TEST_API_KEY_XYZ", "some-key");
        let auth = PluginAuth::ApiKey {
            header_name: "X-API-Key".to_string(),
            key_env: Some("TEST_API_KEY_XYZ".to_string()),
        };
        assert!(auth.is_configured_with_auth_dir("no-stored-apikey-plugin", dir.path()));
        std::env::remove_var("TEST_API_KEY_XYZ");
    }

    #[test]
    fn test_is_configured_apikey_no_env() {
        let dir = tempfile::tempdir().unwrap();
        let auth = PluginAuth::ApiKey {
            header_name: "X-API-Key".to_string(),
            key_env: None,
        };
        assert!(!auth.is_configured_with_auth_dir("no-apikey-plugin", dir.path()));
    }

    #[test]
    fn test_is_configured_oauth_no_stored_token() {
        let dir = tempfile::tempdir().unwrap();
        let auth = PluginAuth::OAuth {
            client_id: Some("id".to_string()),
            auth_url: "https://example.com/auth".to_string(),
            token_url: "https://example.com/token".to_string(),
            scopes: vec![],
        };
        assert!(!auth.is_configured_with_auth_dir("no-oauth-plugin", dir.path()));
    }

    #[test]
    fn test_resolve_header_bearer_stored_token_first() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("bearer-plugin.json");
        std::fs::write(
            &token_path,
            r#"{"access_token":"stored-tok","refresh_token":null,"expires_at":null}"#,
        )
        .unwrap();

        std::env::set_var("TEST_BEARER_RESOLVE_ENV", "env-tok");
        let auth = PluginAuth::Bearer {
            token_env: "TEST_BEARER_RESOLVE_ENV".to_string(),
        };
        // Should prefer stored token over env var
        let header = auth.resolve_header_with_auth_dir("bearer-plugin", dir.path());
        assert_eq!(header, Some("Bearer stored-tok".to_string()));
        std::env::remove_var("TEST_BEARER_RESOLVE_ENV");
    }

    #[test]
    fn test_resolve_header_bearer_env_fallback() {
        let dir = tempfile::tempdir().unwrap();
        // No stored token
        std::env::set_var("TEST_BEARER_RESOLVE_FB", "env-tok-fb");
        let auth = PluginAuth::Bearer {
            token_env: "TEST_BEARER_RESOLVE_FB".to_string(),
        };
        let header = auth.resolve_header_with_auth_dir("no-stored-bearer", dir.path());
        assert_eq!(header, Some("Bearer env-tok-fb".to_string()));
        std::env::remove_var("TEST_BEARER_RESOLVE_FB");
    }

    #[test]
    fn test_resolve_header_apikey_stored_token_first() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("apikey-plugin.json");
        std::fs::write(
            &token_path,
            r#"{"access_token":"stored-key","refresh_token":null,"expires_at":null}"#,
        )
        .unwrap();

        std::env::set_var("TEST_APIKEY_RESOLVE_ENV", "env-key");
        let auth = PluginAuth::ApiKey {
            header_name: "X-API-Key".to_string(),
            key_env: Some("TEST_APIKEY_RESOLVE_ENV".to_string()),
        };
        let header = auth.resolve_header_with_auth_dir("apikey-plugin", dir.path());
        assert_eq!(header, Some("X-API-Key:stored-key".to_string()));
        std::env::remove_var("TEST_APIKEY_RESOLVE_ENV");
    }

    #[test]
    fn test_plugin_info_has_auth_configured_field() {
        let info = PluginInfo {
            name: "test".to_string(),
            version: "1.0".to_string(),
            has_mcp: true,
            auth_type: Some("bearer".to_string()),
            auth_configured: true,
            tool_count: 0,
            update_available: None,
        };
        assert!(info.auth_configured);

        let info2 = PluginInfo {
            name: "test2".to_string(),
            version: "1.0".to_string(),
            has_mcp: true,
            auth_type: Some("oauth".to_string()),
            auth_configured: false,
            tool_count: 0,
            update_available: None,
        };
        assert!(!info2.auth_configured);
    }

    #[test]
    fn test_display_impl() {
        let auth = PluginAuth::Bearer {
            token_env: "MY_TOKEN".to_string(),
        };
        assert_eq!(format!("{}", auth), "bearer");

        let auth = PluginAuth::ApiKey {
            header_name: "X-API-Key".to_string(),
            key_env: None,
        };
        assert_eq!(format!("{}", auth), "api_key");

        let auth = PluginAuth::OAuth {
            client_id: Some("id".to_string()),
            auth_url: "https://example.com/auth".to_string(),
            token_url: "https://example.com/token".to_string(),
            scopes: vec![],
        };
        assert_eq!(format!("{}", auth), "oauth");
    }

    #[test]
    fn test_serde_roundtrip_bearer() {
        let auth = PluginAuth::Bearer {
            token_env: "MY_SECRET_TOKEN".to_string(),
        };
        let json = serde_json::to_string(&auth).unwrap();
        let parsed: PluginAuth = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.display_name(), "bearer");
        if let PluginAuth::Bearer { token_env } = parsed {
            assert_eq!(token_env, "MY_SECRET_TOKEN");
        } else {
            panic!("Expected Bearer variant");
        }
    }

    #[test]
    fn test_serde_roundtrip_api_key_default_header() {
        let auth = PluginAuth::ApiKey {
            header_name: default_api_key_header(),
            key_env: Some("MY_KEY".to_string()),
        };
        let json = serde_json::to_string(&auth).unwrap();
        let parsed: PluginAuth = serde_json::from_str(&json).unwrap();
        if let PluginAuth::ApiKey {
            header_name,
            key_env,
        } = parsed
        {
            assert_eq!(header_name, "X-API-Key");
            assert_eq!(key_env, Some("MY_KEY".to_string()));
        } else {
            panic!("Expected ApiKey variant");
        }
    }

    #[test]
    fn test_serde_roundtrip_oauth() {
        let auth = PluginAuth::OAuth {
            client_id: Some("client123".to_string()),
            auth_url: "https://example.com/auth".to_string(),
            token_url: "https://example.com/token".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
        };
        let json = serde_json::to_string(&auth).unwrap();
        let parsed: PluginAuth = serde_json::from_str(&json).unwrap();
        if let PluginAuth::OAuth {
            client_id,
            auth_url,
            token_url,
            scopes,
        } = parsed
        {
            assert_eq!(client_id, Some("client123".to_string()));
            assert_eq!(auth_url, "https://example.com/auth");
            assert_eq!(token_url, "https://example.com/token");
            assert_eq!(scopes, vec!["read", "write"]);
        } else {
            panic!("Expected OAuth variant");
        }
    }
}
