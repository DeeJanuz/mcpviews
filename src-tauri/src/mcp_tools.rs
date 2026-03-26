use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use crate::http_server::{execute_push, AsyncAppState, ExecutePushResult};
use crate::plugin::PluginRegistry;

/// Return all tool definitions (built-in + plugin tools)
pub async fn list_tools(state: &Arc<TokioMutex<AsyncAppState>>) -> Vec<Value> {
    let mut tools = builtin_tool_definitions();

    // Check for stale plugins and collect info needed for refresh
    let (plugins_to_refresh, client) = {
        let state_guard = state.lock().await;
        let registry = state_guard.inner.plugin_registry.lock().unwrap();
        let client = state_guard.inner.http_client.clone();
        let stale = registry.stale_plugin_indices();
        (stale, client)
    };

    if !plugins_to_refresh.is_empty() {
        // Mark plugins as refresh-pending
        {
            let state_guard = state.lock().await;
            let mut registry = state_guard.inner.plugin_registry.lock().unwrap();
            for idx in &plugins_to_refresh {
                registry.mark_refresh_pending(*idx);
            }
        }

        // Do the actual refresh (async HTTP calls)
        PluginRegistry::refresh_stale_plugins(state, &client).await;
    }

    // Collect plugin tools
    {
        let state_guard = state.lock().await;
        let registry = state_guard.inner.plugin_registry.lock().unwrap();
        tools.extend(registry.all_tools());
    }

    tools
}

/// Dispatch a tool call (built-in first, then plugins)
pub async fn call_tool(
    name: &str,
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    // Check built-in tools first
    match name {
        "push_content" => call_push_content(arguments, state).await,
        "push_review" => call_push_review(arguments, state).await,
        "push_check" => call_push_check(arguments, state).await,
        _ => {
            // Check plugin tools — scope MutexGuard to block before any .await
            let state_guard = state.lock().await;
            let (plugin_info, client) = {
                let registry = state_guard.inner.plugin_registry.lock().unwrap();
                let info = registry.find_plugin_for_tool(name);
                let c = state_guard.inner.http_client.clone();
                (info, c)
            };
            drop(state_guard);

            match plugin_info {
                Some((mcp_url, auth_header, unprefixed_name, renderer_map)) => {
                    let result =
                        proxy_plugin_tool_call(&client, &mcp_url, auth_header.as_deref(), &unprefixed_name, &arguments)
                            .await?;

                    // Auto-push to viewer as a side effect
                    auto_push_plugin_result(
                        state,
                        &unprefixed_name,
                        &arguments,
                        &result,
                        &renderer_map,
                    )
                    .await;

                    Ok(result)
                }
                None => Err(format!("Unknown tool: {}", name)),
            }
        }
    }
}

// ─── Built-in tool implementations ───

async fn call_push_content(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    call_push_impl(arguments, state, false).await
}

async fn call_push_review(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    call_push_impl(arguments, state, true).await
}

async fn call_push_impl(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
    review_required: bool,
) -> Result<Value, String> {
    let tool_name = arguments
        .get("tool_name")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: tool_name")?
        .to_string();
    let data = arguments
        .get("data")
        .ok_or("Missing required parameter: data")?
        .clone();
    let meta = arguments.get("meta").cloned();
    let timeout = if review_required {
        arguments
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(120)
    } else {
        120
    };

    let result = execute_push(
        state,
        tool_name,
        None, // tool_args
        data,
        meta,
        review_required,
        timeout,
        None, // session_id
    )
    .await;

    match result {
        ExecutePushResult::Stored { session_id } => Ok(serde_json::json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string(&serde_json::json!({
                    "session_id": session_id,
                    "status": "stored"
                })).unwrap()
            }]
        })),
        ExecutePushResult::Decision(resp) => Ok(serde_json::json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string(&resp).unwrap()
            }]
        })),
    }
}

async fn call_push_check(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    let session_id = arguments
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: session_id")?
        .to_string();

    let state_guard = state.lock().await;
    let sessions = state_guard.inner.sessions.lock().unwrap();

    let result = match sessions.get(&session_id) {
        Some(session) => {
            let has_decision = session.decided_at.is_some();
            serde_json::json!({
                "session_id": session_id,
                "status": if has_decision { "decided" } else { "pending" },
                "review_required": session.review_required,
                "has_decision": has_decision,
                "decision": session.decision,
            })
        }
        None => {
            serde_json::json!({
                "session_id": session_id,
                "status": "not_found",
                "review_required": false,
                "has_decision": false,
            })
        }
    };

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string(&result).unwrap()
        }]
    }))
}

// ─── Plugin proxy ───

async fn proxy_plugin_tool_call(
    client: &reqwest::Client,
    mcp_url: &str,
    auth_header: Option<&str>,
    tool_name: &str,
    arguments: &Value,
) -> Result<Value, String> {
    let rpc_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    });

    let mut req_builder = client.post(mcp_url).json(&rpc_request);
    if let Some(auth) = auth_header {
        req_builder = req_builder.header("Authorization", auth);
    }

    let response = req_builder
        .send()
        .await
        .map_err(|e| format!("Plugin request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Plugin returned HTTP {}",
            response.status().as_u16()
        ));
    }

    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse plugin response: {}", e))?;

    // Extract result from JSON-RPC response
    if let Some(error) = body.get("error") {
        return Err(format!(
            "Plugin error: {}",
            error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error")
        ));
    }

    body.get("result")
        .cloned()
        .ok_or_else(|| "Plugin response missing result".to_string())
}

/// Auto-push plugin tool results to the viewer as a side effect
async fn auto_push_plugin_result(
    state: &Arc<TokioMutex<AsyncAppState>>,
    tool_name: &str,
    _arguments: &Value,
    mcp_result: &Value,
    renderer_map: &std::collections::HashMap<String, String>,
) {
    // Extract text content from MCP result
    let data = if let Some(content) = mcp_result.get("content").and_then(|c| c.as_array()) {
        // Try to parse the first text content as JSON for structured display
        content
            .iter()
            .find_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    item.get("text")
                        .and_then(|t| t.as_str())
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| mcp_result.clone())
    } else {
        mcp_result.clone()
    };

    // Use renderer_map to map tool_name -> content_type for display, fallback to tool_name
    let display_tool = renderer_map
        .get(tool_name)
        .cloned()
        .unwrap_or_else(|| tool_name.to_string());

    let _ = execute_push(
        state,
        display_tool,
        None,
        data,
        None,
        false, // non-blocking
        120,
        None,
    )
    .await;
}

// ─── Tool definitions ───

fn builtin_tool_definitions() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": "push_content",
            "description": "Display content in the MCP Mux window. Supports multiple content types: search results, documents, diffs, code units, data schemas, dependencies, mermaid diagrams, and generic rich content (markdown + mermaid + tables).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tool_name": {
                        "type": "string",
                        "description": "Content type identifier for renderer selection. Known types: search_codebase, vector_search, get_code_units, get_document, write_document, propose_actions, get_data_schema, manage_data_draft, get_dependencies, get_file_content, get_module_overview, get_analysis_stats, get_business_concepts, manage_knowledge_entries, get_column_context, rich_content. Use 'rich_content' for generic markdown display."
                    },
                    "data": {
                        "type": "object",
                        "description": "Content payload. For rich_content: { \"title\": \"Optional heading\", \"body\": \"Markdown content with ```mermaid blocks, tables, etc.\" }. The 'body' field is required and supports full markdown + mermaid diagrams. For other tool_name types, pass the structured data matching that renderer's expected shape."
                    },
                    "meta": {
                        "type": "object",
                        "description": "Optional metadata (e.g., citation data, source info)."
                    }
                },
                "required": ["tool_name", "data"]
            }
        }),
        serde_json::json!({
            "name": "push_review",
            "description": "Display content in the MCP Mux window and block until the user submits a review decision (accept/reject/partial). Use for mutation operations that need user approval before proceeding.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tool_name": {
                        "type": "string",
                        "description": "Content type identifier for renderer selection."
                    },
                    "data": {
                        "type": "object",
                        "description": "Content payload for review display."
                    },
                    "meta": {
                        "type": "object",
                        "description": "Optional metadata."
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Review timeout in seconds. Default: 120. The timeout resets on user activity (heartbeat)."
                    }
                },
                "required": ["tool_name", "data"]
            }
        }),
        serde_json::json!({
            "name": "push_check",
            "description": "Check the status of a pending review session. Use as a fallback if push_review timed out, to see if the user has since submitted a decision.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "The session ID returned by push_review."
                    }
                },
                "required": ["session_id"]
            }
        }),
    ]
}
