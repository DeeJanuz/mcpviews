use base64::Engine;
use mcpviews_shared::auth_dir;
use mcpviews_shared::token_store::StoredToken;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Start an OAuth authorization code flow:
/// 1. Spin up ephemeral Axum HTTP server on 127.0.0.1:0 (random port)
/// 2. Build auth URL with redirect_uri=http://localhost:{port}/callback
/// 3. Open user's browser to the auth URL
/// 4. Wait for callback with ?code=... parameter (with timeout)
/// 5. Exchange code for token at token_url
/// 6. Store the token
/// 7. Shut down the ephemeral server
/// Returns the access token on success
pub async fn start_oauth_flow(
    plugin_name: &str,
    client_id: Option<&str>,
    auth_url: &str,
    token_url: &str,
    scopes: &[String],
    http_client: &reqwest::Client,
) -> Result<String, String> {
    let (code_tx, code_rx) = tokio::sync::oneshot::channel::<String>();
    let code_tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(code_tx)));

    // Build the callback route
    let tx_clone = code_tx.clone();
    let app = axum::Router::new().route(
        "/callback",
        axum::routing::get(move |query: axum::extract::Query<HashMap<String, String>>| {
            let tx = tx_clone.clone();
            async move {
                if let Some(code) = query.get("code") {
                    let mut guard = tx.lock().await;
                    if let Some(sender) = guard.take() {
                        let _ = sender.send(code.clone());
                    }
                    axum::response::Html(
                        "<html><body><h1>Authorization successful!</h1><p>You can close this tab.</p></body></html>"
                            .to_string(),
                    )
                } else {
                    let error = query
                        .get("error")
                        .cloned()
                        .unwrap_or_else(|| "unknown error".to_string());
                    axum::response::Html(format!(
                        "<html><body><h1>Authorization failed</h1><p>{}</p></body></html>",
                        error
                    ))
                }
            }
        }),
    );

    // Bind to random port on localhost
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Failed to bind ephemeral server: {}", e))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| format!("Failed to get local address: {}", e))?;
    let port = local_addr.port();
    let redirect_uri = format!("http://localhost:{}/callback", port);

    // Generate PKCE code_verifier and code_challenge (S256)
    let verifier_bytes: [u8; 32] = {
        let u1 = uuid::Uuid::new_v4();
        let u2 = uuid::Uuid::new_v4();
        let mut bytes = [0u8; 32];
        bytes[..16].copy_from_slice(u1.as_bytes());
        bytes[16..].copy_from_slice(u2.as_bytes());
        bytes
    };
    let code_verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let hash = hasher.finalize();
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);

    // Build the authorization URL with proper encoding
    let scopes_joined = scopes.join(" ");
    let mut parsed_url = reqwest::Url::parse(auth_url)
        .map_err(|e| format!("Invalid auth_url '{}': {}", auth_url, e))?;
    if let Some(cid) = client_id {
        parsed_url.query_pairs_mut().append_pair("client_id", cid);
    }
    parsed_url
        .query_pairs_mut()
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &scopes_joined)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256");
    let authorization_url = parsed_url.to_string();

    // Start the server in a background task
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    // Open the browser
    open_browser(&authorization_url)?;

    // Wait for the authorization code with a 120-second timeout
    let code = tokio::time::timeout(std::time::Duration::from_secs(120), code_rx)
        .await
        .map_err(|_| "OAuth flow timed out after 120 seconds".to_string())?
        .map_err(|_| "OAuth callback channel closed unexpectedly".to_string())?;

    // Shut down the ephemeral server
    server_handle.abort();

    // Exchange the authorization code for a token
    let mut form_params: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_verifier", code_verifier.as_str()),
    ];
    if let Some(cid) = client_id {
        form_params.push(("client_id", cid));
    }

    let token_response = http_client
        .post(token_url)
        .form(&form_params)
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {}", e))?;

    if !token_response.status().is_success() {
        let status = token_response.status();
        let body = token_response.text().await.unwrap_or_default();
        return Err(format!(
            "Token exchange failed with HTTP {}: {}",
            status, body
        ));
    }

    let token_data: serde_json::Value = token_response
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    let access_token = token_data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Token response missing access_token".to_string())?
        .to_string();

    let refresh_token = token_data
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let expires_at = token_data.get("expires_in").and_then(|v| v.as_i64()).map(
        |expires_in| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
                + expires_in
        },
    );

    let stored = StoredToken {
        access_token: access_token.clone(),
        refresh_token,
        expires_at,
    };
    store_token(plugin_name, &stored)?;

    eprintln!("[mcpviews] OAuth flow completed for plugin '{}'", plugin_name);
    Ok(access_token)
}

/// Open a URL in the user's default browser using platform-specific commands
fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();

    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();

    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let result: Result<std::process::Child, std::io::Error> =
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Unsupported platform"));

    result.map_err(|e| format!("Failed to open browser: {}", e))?;
    Ok(())
}

/// Store an OAuth token to ~/.mcpviews/auth/{plugin_name}.json
pub fn store_token(plugin_name: &str, token: &StoredToken) -> Result<(), String> {
    mcpviews_shared::token_store::store_token(&auth_dir(), plugin_name, token)
}


/// Attempt to refresh an expired OAuth token using the refresh_token grant.
/// Returns the new access token on success, storing the refreshed token to disk.
pub async fn refresh_oauth_token(
    plugin_name: &str,
    token_url: &str,
    client_id: Option<&str>,
    http_client: &reqwest::Client,
) -> Result<String, String> {
    let auth_dir = mcpviews_shared::auth_dir();
    let stored = mcpviews_shared::token_store::load_stored_token_unvalidated(&auth_dir, plugin_name)
        .ok_or_else(|| format!("No stored token for plugin '{}'", plugin_name))?;

    let refresh_token = stored
        .refresh_token
        .ok_or_else(|| format!("No refresh_token available for plugin '{}'", plugin_name))?;

    let mut form_params: Vec<(&str, &str)> = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
    ];
    if let Some(cid) = client_id {
        form_params.push(("client_id", cid));
    }

    let response = http_client
        .post(token_url)
        .form(&form_params)
        .send()
        .await
        .map_err(|e| format!("Token refresh request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Token refresh failed with HTTP {}: {}",
            status, body
        ));
    }

    let token_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse refresh response: {}", e))?;

    let access_token = token_data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Refresh response missing access_token".to_string())?
        .to_string();

    let new_refresh_token = token_data
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or(Some(refresh_token)); // Keep old refresh_token if server doesn't return new one

    let expires_at = token_data
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .map(|expires_in| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
                + expires_in
        });

    let new_stored = StoredToken {
        access_token: access_token.clone(),
        refresh_token: new_refresh_token,
        expires_at,
    };
    store_token(plugin_name, &new_stored)?;

    eprintln!(
        "[mcpviews] Refreshed OAuth token for plugin '{}'",
        plugin_name
    );
    Ok(access_token)
}

/// Store a simple API key or bearer token (not OAuth)
pub fn store_api_key(plugin_name: &str, key: &str) -> Result<(), String> {
    let token = StoredToken {
        access_token: key.to_string(),
        refresh_token: None,
        expires_at: None,
    };
    store_token(plugin_name, &token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stored_token_serde_roundtrip() {
        let token = StoredToken {
            access_token: "test-token".to_string(),
            refresh_token: Some("refresh-123".to_string()),
            expires_at: Some(1700000000),
        };
        let json = serde_json::to_string(&token).unwrap();
        let parsed: StoredToken = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, "test-token");
        assert_eq!(parsed.refresh_token, Some("refresh-123".to_string()));
        assert_eq!(parsed.expires_at, Some(1700000000));
    }

    #[test]
    fn test_stored_token_without_optional_fields() {
        let token = StoredToken {
            access_token: "test-token".to_string(),
            refresh_token: None,
            expires_at: None,
        };
        let json = serde_json::to_string(&token).unwrap();
        let parsed: StoredToken = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, "test-token");
        assert!(parsed.refresh_token.is_none());
        assert!(parsed.expires_at.is_none());
    }

    #[test]
    fn test_expired_token_detected() {
        let past = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 3600;

        let token = StoredToken {
            access_token: "expired-token".to_string(),
            refresh_token: None,
            expires_at: Some(past),
        };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test-plugin.json");
        let json = serde_json::to_string_pretty(&token).unwrap();
        std::fs::write(&path, json).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let stored: StoredToken = serde_json::from_str(&content).unwrap();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        assert!(stored.expires_at.unwrap() < now, "Token should be expired");
    }

    #[test]
    fn test_valid_token_not_expired() {
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 3600;

        let token = StoredToken {
            access_token: "valid-token".to_string(),
            refresh_token: None,
            expires_at: Some(future),
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        assert!(token.expires_at.unwrap() > now, "Token should not be expired");
    }
}
