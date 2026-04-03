use axum::{
    extract::{Extension, Json, Query},
    http::{HeaderMap, Method, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Mutex as TokioMutex;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tower_http::cors::{Any, CorsLayer};

use crate::mcp;
use crate::plugin::PluginRegistry;
use crate::review::ReviewDecision;
use crate::session::PreviewSession;
use crate::state::AppState;

/// Shared state wrapper for async axum handlers (needs tokio::Mutex, not std::Mutex)
pub struct AsyncAppState {
    pub inner: Arc<AppState>,
    pub app_handle: AppHandle,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRequest {
    pub tool_name: String,
    #[serde(default)]
    pub tool_args: Option<serde_json::Value>,
    pub result: PushResult,
    #[serde(default)]
    pub review_required: Option<bool>,
    #[serde(default, rename = "openBrowser")]
    pub _open_browser: Option<bool>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PushResult {
    pub data: serde_json::Value,
    #[serde(default)]
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResponse {
    pub session_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_decisions: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifications: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additions: Option<serde_json::Value>,
}

impl From<ReviewDecision> for PushResponse {
    fn from(d: ReviewDecision) -> Self {
        PushResponse {
            session_id: d.session_id,
            status: d.status,
            decision: d.decision,
            operation_decisions: d.operation_decisions,
            comments: d.comments,
            modifications: d.modifications,
            additions: d.additions,
        }
    }
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    version: String,
    port: u16,
    uptime_seconds: u64,
    started_at: String,
}

/// Result of executing a push operation
pub enum ExecutePushResult {
    Stored { session_id: String },
    Pending { session_id: String },
    Decision(PushResponse),
}

/// Store a push session (emit to WebView, register review if needed) but do NOT block.
/// For reviews, returns `Pending { session_id }`.
/// For non-reviews, returns `Stored { session_id }`.
pub async fn store_push(
    state: &Arc<TokioMutex<AsyncAppState>>,
    tool_name: String,
    tool_args: Option<serde_json::Value>,
    data: serde_json::Value,
    meta: Option<serde_json::Value>,
    review_required: bool,
    timeout_secs: u64,
    session_id: Option<String>,
) -> ExecutePushResult {
    let session_id = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Resolve content_type through plugin renderer maps (tool_name -> renderer_name).
    // Must lock state (tokio) then plugin_registry (std) and drop before any await.
    let content_type = {
        let state_guard = state.lock().await;
        let registry = state_guard.inner.plugin_registry.lock().unwrap();
        resolve_content_type(&registry, &tool_name)
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let session = PreviewSession {
        session_id: session_id.clone(),
        tool_name,
        tool_args: tool_args.unwrap_or(serde_json::Value::Object(Default::default())),
        content_type,
        data,
        meta: meta.unwrap_or(serde_json::Value::Object(Default::default())),
        review_required,
        timeout_secs: if review_required { Some(timeout_secs) } else { None },
        created_at: now,
        decided_at: None,
        decision: None,
        operation_decisions: None,
    };

    let state_guard = state.lock().await;

    {
        let mut sessions = state_guard.inner.sessions.lock().unwrap();
        sessions.set(session.clone());
    }

    // Emit to WebView
    let _ = state_guard.app_handle.emit("push_preview", &session);

    // Show and focus the window
    if let Some(window) = state_guard.app_handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }

    if review_required {
        // Register pending review and set up deadline
        {
            let mut reviews = state_guard.inner.reviews.lock().unwrap();
            reviews.add_pending(session_id.clone());
        }

        let deadline = Arc::new(TokioMutex::new(
            tokio::time::Instant::now() + Duration::from_secs(timeout_secs),
        ));
        {
            let mut deadlines = state_guard.inner.review_deadlines.lock().unwrap();
            deadlines.insert(session_id.clone(), (deadline.clone(), timeout_secs));
        }
        drop(state_guard);

        ExecutePushResult::Pending { session_id }
    } else {
        drop(state_guard);
        ExecutePushResult::Stored { session_id }
    }
}

/// Subscribe to a watch channel and block until the user submits a decision or the deadline expires.
/// Called by the MCP `await_review` tool (via `call_await_review`) and by `execute_push` for
/// HTTP `/api/push` backward compatibility. The deadline resets to the full timeout on each call,
/// so agents can reconnect after a transport timeout without losing the review session.
pub async fn await_decision(
    state: &Arc<TokioMutex<AsyncAppState>>,
    session_id: &str,
) -> ExecutePushResult {
    // Subscribe to the watch channel
    let mut rx = {
        let state_guard = state.lock().await;
        let reviews = state_guard.inner.reviews.lock().unwrap();
        match reviews.subscribe(session_id) {
            Some(rx) => rx,
            None => {
                return ExecutePushResult::Decision(ReviewDecision {
                    session_id: session_id.to_string(),
                    status: "error".to_string(),
                    decision: Some("not_found".to_string()),
                    operation_decisions: None,
                    comments: None,
                    modifications: None,
                    additions: None,
                }.into());
            }
        }
    };

    // Check if decision already arrived
    {
        let current = rx.borrow().clone();
        if let Some(decision) = current {
            // Clean up
            let state_guard = state.lock().await;
            let mut deadlines = state_guard.inner.review_deadlines.lock().unwrap();
            deadlines.remove(session_id);
            drop(deadlines);
            let mut reviews = state_guard.inner.reviews.lock().unwrap();
            reviews.remove_resolved(session_id);
            return ExecutePushResult::Decision(decision.into());
        }
    }

    // Get the deadline arc
    let deadline = {
        let state_guard = state.lock().await;
        let deadlines = state_guard.inner.review_deadlines.lock().unwrap();
        match deadlines.get(session_id) {
            Some((dl, _)) => dl.clone(),
            None => {
                // No deadline means review already cleaned up
                return ExecutePushResult::Decision(ReviewDecision {
                    session_id: session_id.to_string(),
                    status: "error".to_string(),
                    decision: Some("expired".to_string()),
                    operation_decisions: None,
                    comments: None,
                    modifications: None,
                    additions: None,
                }.into());
            }
        }
    };

    let session_id_owned = session_id.to_string();

    // Reset deadline to full timeout from now — await_review may arrive long
    // after store_push created the original deadline (e.g. after a transport
    // timeout + reconnect).
    {
        let timeout_secs = {
            let state_guard = state.lock().await;
            let deadlines = state_guard.inner.review_deadlines.lock().unwrap();
            deadlines.get(&session_id_owned).map(|(_, t)| *t)
        };
        if let Some(t) = timeout_secs {
            let mut dl = deadline.lock().await;
            *dl = tokio::time::Instant::now() + Duration::from_secs(t);
        }
    }

    // Resettable timeout loop
    let result = loop {
        let current_deadline = *deadline.lock().await;
        tokio::select! {
            changed = rx.changed() => {
                match changed {
                    Ok(()) => {
                        let val = rx.borrow().clone();
                        if let Some(decision) = val {
                            break Some(decision);
                        }
                        // None means spurious wake, continue
                    }
                    Err(_) => {
                        // Sender dropped
                        break None;
                    }
                }
            }
            _ = tokio::time::sleep_until(current_deadline) => {
                let now = tokio::time::Instant::now();
                let dl = *deadline.lock().await;
                if dl > now {
                    eprintln!("[mcpviews] Review {}: deadline was bumped, continuing", session_id_owned);
                    continue; // deadline was bumped by heartbeat
                }
                eprintln!("[mcpviews] Review {}: truly expired", session_id_owned);
                break None; // truly expired
            }
        }
    };

    // Clean up deadline entry
    {
        let state_guard = state.lock().await;
        let mut deadlines = state_guard.inner.review_deadlines.lock().unwrap();
        deadlines.remove(&session_id_owned);
    }

    match result {
        Some(decision) => {
            let state_guard = state.lock().await;
            let mut reviews = state_guard.inner.reviews.lock().unwrap();
            reviews.remove_resolved(&session_id_owned);
            ExecutePushResult::Decision(decision.into())
        }
        None => {
            // Timeout or channel dropped — dismiss
            let state_guard = state.lock().await;
            let mut reviews = state_guard.inner.reviews.lock().unwrap();
            reviews.dismiss(&session_id_owned);
            reviews.remove_resolved(&session_id_owned);
            ExecutePushResult::Decision(ReviewDecision {
                session_id: session_id_owned,
                status: "decision_received".to_string(),
                decision: Some("dismissed".to_string()),
                operation_decisions: None,
                comments: None,
                modifications: None,
                additions: None,
            }.into())
        }
    }
}

/// Core push logic shared by HTTP `/api/push` and MCP `push_content` tools.
/// For non-reviews, calls `store_push` and returns `Stored`.
/// For reviews, composes `store_push` + `await_decision` (blocking until the user decides).
/// Note: The MCP `push_review` tool uses `store_push` directly (non-blocking) and returns
/// immediately; the agent then calls `await_review` which calls `await_decision` separately.
/// This function is used by the HTTP `/api/push` endpoint which still does the blocking
/// compose for backward compatibility.
pub async fn execute_push(
    state: &Arc<TokioMutex<AsyncAppState>>,
    tool_name: String,
    tool_args: Option<serde_json::Value>,
    data: serde_json::Value,
    meta: Option<serde_json::Value>,
    review_required: bool,
    timeout_secs: u64,
    session_id: Option<String>,
) -> ExecutePushResult {
    let result = store_push(state, tool_name, tool_args, data, meta, review_required, timeout_secs, session_id).await;
    match result {
        ExecutePushResult::Pending { ref session_id } => await_decision(state, session_id).await,
        other => other,
    }
}

static START_TIME: std::sync::OnceLock<(std::time::Instant, String)> = std::sync::OnceLock::new();

fn get_start_info() -> &'static (std::time::Instant, String) {
    START_TIME.get_or_init(|| {
        (
            std::time::Instant::now(),
            chrono::Utc::now().to_rfc3339(),
        )
    })
}

async fn health_handler() -> impl IntoResponse {
    let (start_instant, started_at) = get_start_info();
    Json(HealthResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        port: 4200,
        uptime_seconds: start_instant.elapsed().as_secs(),
        started_at: started_at.clone(),
    })
}

async fn push_handler(
    Extension(state): Extension<Arc<TokioMutex<AsyncAppState>>>,
    Json(push_req): Json<PushRequest>,
) -> impl IntoResponse {
    let review_required = push_req.review_required.unwrap_or(false);
    let timeout_secs = push_req.timeout.unwrap_or(120);

    let result = execute_push(
        &state,
        push_req.tool_name,
        push_req.tool_args,
        push_req.result.data,
        push_req.result.meta,
        review_required,
        timeout_secs,
        push_req.session_id,
    )
    .await;

    match result {
        ExecutePushResult::Stored { session_id } => (
            StatusCode::CREATED,
            Json(PushResponse {
                session_id,
                status: "stored".to_string(),
                decision: None,
                operation_decisions: None,
                comments: None,
                modifications: None,
                additions: None,
            }),
        ),
        ExecutePushResult::Decision(resp) => {
            let status_code = if resp.decision.as_deref() == Some("dismissed") {
                StatusCode::REQUEST_TIMEOUT
            } else {
                StatusCode::OK
            };
            (status_code, Json(resp))
        }
        ExecutePushResult::Pending { .. } => {
            unreachable!("execute_push never returns Pending directly")
        }
    }
}

#[derive(Debug, Deserialize)]
struct HeartbeatRequest {
    session_id: Option<String>,
}

async fn heartbeat_handler(
    Extension(state): Extension<Arc<TokioMutex<AsyncAppState>>>,
    body: axum::body::Bytes,
) -> StatusCode {
    let req: HeartbeatRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    let session_id = match req.session_id {
        Some(id) => id,
        None => return StatusCode::BAD_REQUEST,
    };
    let state_guard = state.lock().await;
    let entry = {
        let deadlines = state_guard.inner.review_deadlines.lock().unwrap();
        deadlines.get(&session_id).cloned()
    };
    drop(state_guard);
    match entry {
        Some((deadline, timeout_secs)) => {
            let mut dl = deadline.lock().await;
            *dl = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
            eprintln!("[mcpviews] Heartbeat OK for session {} (reset to {}s)", session_id, timeout_secs);
            StatusCode::OK
        }
        None => {
            eprintln!("[mcpviews] Heartbeat 404 for session {}", session_id);
            StatusCode::NOT_FOUND
        }
    }
}

async fn mcp_sse_handler(
    headers: HeaderMap,
    Extension(state): Extension<Arc<TokioMutex<AsyncAppState>>>,
) -> Result<impl IntoResponse, StatusCode> {
    // Verify Accept header
    let accept = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !accept.contains("text/event-stream") {
        return Err(StatusCode::NOT_ACCEPTABLE);
    }

    let state_guard = state.lock().await;
    let (session_id, rx) = {
        let mut sessions = state_guard.inner.mcp_sessions.lock().unwrap();
        sessions.create_session()
    };
    drop(state_guard);

    let stream = BroadcastStream::new(rx)
        .filter_map(|result: Result<String, _>| result.ok())
        .map(|data| -> Result<Event, Infallible> { Ok(Event::default().data(data)) });

    let sse = Sse::new(stream).keep_alive(KeepAlive::default());

    Ok(([("mcp-session-id", session_id)], sse))
}

async fn mcp_post_handler(
    headers: HeaderMap,
    Extension(state): Extension<Arc<TokioMutex<AsyncAppState>>>,
    body: String,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let mcp_session_id = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // If session header present, verify it exists
    if let Some(ref session_id) = mcp_session_id {
        let state_guard = state.lock().await;
        let exists = {
            let sessions = state_guard.inner.mcp_sessions.lock().unwrap();
            sessions.has_session(session_id)
        };
        drop(state_guard);
        if !exists {
            return Err(StatusCode::NOT_FOUND);
        }
    }
    let (status, value) = mcp::mcp_handler(state, body, mcp_session_id).await;
    Ok((status, Json(value)))
}

async fn mcp_delete_handler(
    headers: HeaderMap,
    Extension(state): Extension<Arc<TokioMutex<AsyncAppState>>>,
) -> StatusCode {
    let session_id = match headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
    {
        Some(id) => id.to_string(),
        None => return StatusCode::BAD_REQUEST,
    };
    let state_guard = state.lock().await;
    let removed = {
        let mut sessions = state_guard.inner.mcp_sessions.lock().unwrap();
        sessions.remove_session(&session_id)
    };
    if removed {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn reload_plugins_handler(
    Extension(state): Extension<Arc<TokioMutex<AsyncAppState>>>,
) -> StatusCode {
    let state_guard = state.lock().await;
    state_guard.inner.reload_plugins();
    StatusCode::OK
}

// ---------------------------------------------------------------------------
// Mock OAuth endpoints – satisfies Claude Code's HTTP transport auth handshake
// without requiring real authentication.
// ---------------------------------------------------------------------------

const BASE_URL: &str = "http://localhost:4200";

/// GET /.well-known/oauth-protected-resource  (RFC 9728)
async fn oauth_protected_resource() -> impl IntoResponse {
    Json(serde_json::json!({
        "resource": BASE_URL,
        "authorization_servers": [BASE_URL]
    }))
}

/// GET /.well-known/oauth-authorization-server  (RFC 8414)
async fn oauth_authorization_server() -> impl IntoResponse {
    Json(serde_json::json!({
        "issuer": BASE_URL,
        "authorization_endpoint": format!("{}/oauth/authorize", BASE_URL),
        "token_endpoint": format!("{}/oauth/token", BASE_URL),
        "registration_endpoint": format!("{}/oauth/register", BASE_URL),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"]
    }))
}

/// POST /oauth/register – dynamic client registration (mock)
#[derive(Deserialize, Default)]
struct RegisterRequest {
    #[serde(default)]
    redirect_uris: Vec<String>,
    // Ignore all other fields from the request body
    #[serde(flatten)]
    _extra: serde_json::Value,
}

async fn oauth_register(
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let req: RegisterRequest = serde_json::from_slice(&body).unwrap_or_default();
    Json(serde_json::json!({
        "client_id": "mcpviews-mock-client",
        "client_name": "MCPViews Mock Client",
        "redirect_uris": req.redirect_uris,
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none"
    }))
}

/// GET /oauth/authorize – immediately redirects with a mock auth code (302 Found)
async fn oauth_authorize(
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    let redirect_uri = params
        .get("redirect_uri")
        .ok_or((StatusCode::BAD_REQUEST, "missing redirect_uri"))?;
    let state = params.get("state").map(|s| s.as_str()).unwrap_or("");

    let sep = if redirect_uri.contains('?') { "&" } else { "?" };
    let location = format!(
        "{}{}code=mcpviews-mock-code&state={}",
        redirect_uri, sep, state
    );
    Ok((
        StatusCode::FOUND,
        [(axum::http::header::LOCATION, location)],
    ))
}

/// POST /oauth/token – returns a mock access token
async fn oauth_token() -> impl IntoResponse {
    Json(serde_json::json!({
        "access_token": "mcpviews-mock-token",
        "token_type": "bearer",
        "expires_in": 86400,
        "scope": "mcp"
    }))
}

pub async fn start_http_server(app_state: Arc<AppState>, app_handle: AppHandle, std_listener: std::net::TcpListener) {
    eprintln!("[mcpviews] Starting HTTP server on :4200");
    let _ = get_start_info(); // Initialize start time

    let async_state = Arc::new(TokioMutex::new(AsyncAppState {
        inner: app_state.clone(),
        app_handle,
    }));

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers(Any)
        .expose_headers(["mcp-session-id".parse::<axum::http::HeaderName>().unwrap()]);

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/api/push", post(push_handler))
        .route("/api/heartbeat", post(heartbeat_handler))
        .route("/api/reload-plugins", post(reload_plugins_handler))
        .route(
            "/mcp",
            get(mcp_sse_handler)
                .post(mcp_post_handler)
                .delete(mcp_delete_handler),
        )
        // Mock OAuth endpoints – Claude Code's HTTP transport probes these during
        // connection setup.  We return valid metadata so the handshake completes
        // instantly without real authentication.
        .route("/.well-known/oauth-protected-resource", get(oauth_protected_resource))
        .route("/.well-known/oauth-authorization-server", get(oauth_authorization_server))
        .route("/oauth/register", post(oauth_register))
        .route("/oauth/authorize", get(oauth_authorize))
        .route("/oauth/token", post(oauth_token))
        .layer(cors)
        .layer(Extension(async_state));

    // Start GC task
    let gc_state = app_state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let mut sessions = gc_state.sessions.lock().unwrap();
            sessions.gc();
            drop(sessions);
            // Clean up stale deadlines
            let mut deadlines = gc_state.review_deadlines.lock().unwrap();
            let reviews = gc_state.reviews.lock().unwrap();
            deadlines.retain(|id, _| reviews.has_pending(id));
            drop(deadlines);
            drop(reviews);
            // GC MCP SSE sessions with no active receivers
            let mut mcp_sessions = gc_state.mcp_sessions.lock().unwrap();
            mcp_sessions.retain_active();
        }
    });

    let listener = tokio::net::TcpListener::from_std(std_listener)
        .expect("Failed to convert std listener to tokio listener");
    eprintln!("[mcpviews] HTTP server listening on :4200");
    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("[mcpviews] HTTP server error: {}", e);
    }
}

/// Resolve a tool_name to a content_type (renderer name) by searching all plugin
/// manifest renderer maps. Falls back to `tool_name` if no mapping is found.
fn resolve_content_type(registry: &PluginRegistry, tool_name: &str) -> String {
    for manifest in &registry.manifests {
        if let Some(renderer_name) = manifest.renderers.get(tool_name) {
            return renderer_name.clone();
        }
    }
    tool_name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::PluginRegistry;
    use crate::review::ReviewDecision;
    use mcpviews_shared::PluginManifest;

    fn empty_registry() -> (PluginRegistry, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = mcpviews_shared::plugin_store::PluginStore::with_dir(dir.path().to_path_buf());
        let registry = PluginRegistry::load_plugins_with_store(store);
        (registry, dir)
    }

    fn manifest_with_renderers(name: &str, renderers: HashMap<String, String>) -> PluginManifest {
        PluginManifest {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            renderers,
            mcp: None,
            renderer_definitions: vec![],
            tool_rules: HashMap::new(),
            no_auto_push: vec![],
            registry_index: None,
            download_url: None,
            prompt_definitions: vec![],
            plugin_rules: vec![],
        }
    }

    #[test]
    fn test_resolve_content_type_with_mapping() {
        let (mut registry, _dir) = empty_registry();
        let mut renderers = HashMap::new();
        renderers.insert("search_codebase".to_string(), "search_results".to_string());
        registry.add_plugin(manifest_with_renderers("test-plugin", renderers)).unwrap();

        let result = resolve_content_type(&registry, "search_codebase");
        assert_eq!(result, "search_results");
    }

    #[test]
    fn test_resolve_content_type_falls_back_to_tool_name() {
        let (registry, _dir) = empty_registry();
        let result = resolve_content_type(&registry, "unknown_tool");
        assert_eq!(result, "unknown_tool");
    }

    #[test]
    fn test_resolve_content_type_no_match_in_manifests() {
        let (mut registry, _dir) = empty_registry();
        let mut renderers = HashMap::new();
        renderers.insert("other_tool".to_string(), "other_renderer".to_string());
        registry.add_plugin(manifest_with_renderers("test-plugin", renderers)).unwrap();

        let result = resolve_content_type(&registry, "search_codebase");
        assert_eq!(result, "search_codebase");
    }

    // -----------------------------------------------------------------------
    // Mock OAuth endpoint tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_oauth_protected_resource_response() {
        let resp = oauth_protected_resource().await;
        let json = resp.into_response();
        let body = axum::body::to_bytes(json.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["resource"], "http://localhost:4200");
        assert_eq!(v["authorization_servers"][0], "http://localhost:4200");
    }

    #[tokio::test]
    async fn test_oauth_authorization_server_response() {
        let resp = oauth_authorization_server().await;
        let json = resp.into_response();
        let body = axum::body::to_bytes(json.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["issuer"], "http://localhost:4200");
        assert_eq!(v["authorization_endpoint"], "http://localhost:4200/oauth/authorize");
        assert_eq!(v["token_endpoint"], "http://localhost:4200/oauth/token");
        assert_eq!(v["registration_endpoint"], "http://localhost:4200/oauth/register");
        assert_eq!(v["response_types_supported"][0], "code");
        assert_eq!(v["grant_types_supported"][0], "authorization_code");
        assert_eq!(v["grant_types_supported"][1], "refresh_token");
        assert_eq!(v["code_challenge_methods_supported"][0], "S256");
        assert_eq!(v["token_endpoint_auth_methods_supported"][0], "none");
    }

    #[tokio::test]
    async fn test_oauth_register_echoes_redirect_uris() {
        let body_bytes = axum::body::Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "redirect_uris": ["http://localhost:9999/callback"],
                "client_name": "test"
            })).unwrap()
        );
        let resp = oauth_register(body_bytes).await;
        let json = resp.into_response();
        let body = axum::body::to_bytes(json.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["client_id"], "mcpviews-mock-client");
        assert_eq!(v["redirect_uris"][0], "http://localhost:9999/callback");
    }

    #[tokio::test]
    async fn test_oauth_register_empty_body() {
        let body_bytes = axum::body::Bytes::from(b"{}".to_vec());
        let resp = oauth_register(body_bytes).await;
        let json = resp.into_response();
        let body = axum::body::to_bytes(json.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["client_id"], "mcpviews-mock-client");
        assert!(v["redirect_uris"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_oauth_authorize_redirects() {
        let mut params = HashMap::new();
        params.insert("redirect_uri".to_string(), "http://localhost:9999/cb".to_string());
        params.insert("state".to_string(), "abc123".to_string());
        let result = oauth_authorize(Query(params)).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_response();
        assert_eq!(resp.status(), StatusCode::FOUND);
        let location = resp.headers().get("location").unwrap().to_str().unwrap();
        assert!(location.contains("code=mcpviews-mock-code"));
        assert!(location.contains("state=abc123"));
        assert!(location.starts_with("http://localhost:9999/cb?"));
    }

    #[tokio::test]
    async fn test_oauth_authorize_missing_redirect_uri() {
        let params: HashMap<String, String> = HashMap::new();
        let result = oauth_authorize(Query(params)).await;
        match result {
            Err((status, _msg)) => assert_eq!(status, StatusCode::BAD_REQUEST),
            Ok(_) => panic!("expected Err for missing redirect_uri"),
        }
    }

    #[tokio::test]
    async fn test_oauth_token_response() {
        let resp = oauth_token().await;
        let json = resp.into_response();
        let body = axum::body::to_bytes(json.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["access_token"], "mcpviews-mock-token");
        assert_eq!(v["token_type"], "bearer");
        assert_eq!(v["expires_in"], 86400);
        assert_eq!(v["scope"], "mcp");
    }

    #[test]
    fn test_resolve_content_type_multiple_plugins() {
        let (mut registry, _dir) = empty_registry();

        let mut renderers1 = HashMap::new();
        renderers1.insert("tool_a".to_string(), "renderer_a".to_string());
        registry.add_plugin(manifest_with_renderers("plugin-1", renderers1)).unwrap();

        let mut renderers2 = HashMap::new();
        renderers2.insert("tool_b".to_string(), "renderer_b".to_string());
        registry.add_plugin(manifest_with_renderers("plugin-2", renderers2)).unwrap();

        assert_eq!(resolve_content_type(&registry, "tool_a"), "renderer_a");
        assert_eq!(resolve_content_type(&registry, "tool_b"), "renderer_b");
        assert_eq!(resolve_content_type(&registry, "tool_c"), "tool_c");
    }

    #[test]
    fn test_push_response_from_review_decision() {
        let mut op_decisions = HashMap::new();
        op_decisions.insert("op1".to_string(), "approved".to_string());
        let mut comments = HashMap::new();
        comments.insert("row1".to_string(), "looks good".to_string());
        let mut modifications = HashMap::new();
        modifications.insert("field1".to_string(), "new_value".to_string());

        let decision = ReviewDecision {
            session_id: "test-session".to_string(),
            status: "decision_received".to_string(),
            decision: Some("approved".to_string()),
            operation_decisions: Some(op_decisions.clone()),
            comments: Some(comments.clone()),
            modifications: Some(modifications.clone()),
            additions: Some(serde_json::json!({"extra": "data"})),
        };

        let response: PushResponse = decision.into();

        assert_eq!(response.session_id, "test-session");
        assert_eq!(response.status, "decision_received");
        assert_eq!(response.decision, Some("approved".to_string()));
        assert_eq!(response.operation_decisions, Some(op_decisions));
        assert_eq!(response.comments, Some(comments));
        assert_eq!(response.modifications, Some(modifications));
        assert_eq!(response.additions, Some(serde_json::json!({"extra": "data"})));
    }

    #[test]
    fn test_push_response_from_review_decision_minimal() {
        let decision = ReviewDecision {
            session_id: "s1".to_string(),
            status: "error".to_string(),
            decision: Some("not_found".to_string()),
            operation_decisions: None,
            comments: None,
            modifications: None,
            additions: None,
        };

        let response = PushResponse::from(decision);

        assert_eq!(response.session_id, "s1");
        assert_eq!(response.status, "error");
        assert_eq!(response.decision, Some("not_found".to_string()));
        assert!(response.operation_decisions.is_none());
        assert!(response.comments.is_none());
        assert!(response.modifications.is_none());
        assert!(response.additions.is_none());
    }
}
