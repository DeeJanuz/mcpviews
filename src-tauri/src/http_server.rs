use axum::{
    extract::{Extension, Json},
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
    Decision(PushResponse),
}

/// Core push logic shared by HTTP `/api/push` and MCP `push_content`/`push_review` tools
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
        let rx = {
            let mut reviews = state_guard.inner.reviews.lock().unwrap();
            reviews.add_pending(session_id.clone())
        };

        // Set up resettable deadline
        let deadline = Arc::new(TokioMutex::new(
            tokio::time::Instant::now() + Duration::from_secs(timeout_secs),
        ));
        {
            let mut deadlines = state_guard.inner.review_deadlines.lock().unwrap();
            deadlines.insert(session_id.clone(), (deadline.clone(), timeout_secs));
        }
        drop(state_guard);

        // Resettable timeout loop
        let mut rx = rx;
        let result = loop {
            let current_deadline = *deadline.lock().await;
            tokio::select! {
                decision = &mut rx => {
                    break decision.ok();
                }
                _ = tokio::time::sleep_until(current_deadline) => {
                    let now = tokio::time::Instant::now();
                    let dl = *deadline.lock().await;
                    if dl > now {
                        continue; // deadline was bumped by heartbeat
                    }
                    break None; // truly expired
                }
            }
        };

        // Clean up deadline entry
        {
            let state_guard = state.lock().await;
            let mut deadlines = state_guard.inner.review_deadlines.lock().unwrap();
            deadlines.remove(&session_id);
        }

        match result {
            Some(decision) => ExecutePushResult::Decision(PushResponse {
                session_id: decision.session_id,
                status: decision.status,
                decision: decision.decision,
                operation_decisions: decision.operation_decisions,
                comments: decision.comments,
                modifications: decision.modifications,
                additions: decision.additions,
            }),
            None => {
                // Timeout or channel dropped
                let state_guard = state.lock().await;
                let mut reviews = state_guard.inner.reviews.lock().unwrap();
                reviews.dismiss(&session_id);
                ExecutePushResult::Decision(PushResponse {
                    session_id,
                    status: "decision_received".to_string(),
                    decision: Some("dismissed".to_string()),
                    operation_decisions: None,
                    comments: None,
                    modifications: None,
                    additions: None,
                })
            }
        }
    } else {
        drop(state_guard);
        ExecutePushResult::Stored { session_id }
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
            StatusCode::OK
        }
        None => StatusCode::NOT_FOUND,
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
    // If session header present, verify it exists
    if let Some(session_id) = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
    {
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
    let (status, value) = mcp::mcp_handler(state, body).await;
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

pub async fn start_http_server(app_state: Arc<AppState>, app_handle: AppHandle) {
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
        // Return JSON 404 for OAuth discovery — MCPViews doesn't use OAuth auth.
        // Claude Code's HTTP transport client probes this endpoint and expects
        // a parseable JSON response; axum's default plain-text 404 causes a parse error.
        .route(
            "/.well-known/oauth-authorization-server",
            get(|| async {
                (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({"error": "not_found"})))
            }),
        )
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

    match tokio::net::TcpListener::bind("0.0.0.0:4200").await {
        Ok(listener) => {
            eprintln!("[mcpviews] HTTP server listening on :4200");
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("[mcpviews] HTTP server error: {}", e);
            }
        }
        Err(e) => {
            eprintln!("[mcpviews] Failed to bind to port 4200: {}", e);
        }
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
}
