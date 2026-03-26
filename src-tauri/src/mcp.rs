use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use crate::http_server::AsyncAppState;
use crate::mcp_tools;

const SUPPORTED_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(rename = "jsonrpc")]
    pub _jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        }
    }
}

async fn build_instructions(state: &Arc<TokioMutex<AsyncAppState>>) -> String {
    let state_guard = state.lock().await;
    let renderers = mcp_tools::available_renderers(&state_guard.inner);

    let universal: Vec<_> = renderers.iter()
        .filter(|r| r.scope == "universal")
        .collect();

    let mut instructions = String::from(
        "MCP Mux provides a rich display window for presenting content to the user.\n\n"
    );

    if !universal.is_empty() {
        instructions.push_str("## Available Display Renderers\n\n");
        instructions.push_str("Use the `push_content` tool with `tool_name` set to a renderer name:\n\n");
        for r in &universal {
            instructions.push_str(&format!("- **{}**: {}", r.name, r.description));
            if let Some(hint) = &r.data_hint {
                instructions.push_str(&format!(" Data: `{}`", hint));
            }
            instructions.push_str("\n");
        }
    }

    instructions.push_str("\n## Complex Explanations → Companion Window\n\n");
    instructions.push_str(
        "ALWAYS push complex explanations to the companion window instead of explaining \n\
         them inline in chat. This includes: implementation plans, architectural decisions,\n\
         data architecture, data flows, networking topology, system diagrams, API designs,\n\
         database schemas, and any multi-step technical explanation.\n\n\
         Your chat response should be a concise agent-oriented summary — context for what\n\
         was pushed, next steps, and decisions needed. The human-readable explanation with\n\
         mermaid diagrams, tables, code examples, and formatted markdown goes to `push_content`.\n\
         Think of chat as the agent log and the companion window as the user-facing document.\n\n\
         ```\n\
         push_content({\n\
           tool_name: \"rich_content\",\n\
           data: {\n\
             title: \"Implementation Plan: <feature name>\",\n\
             body: \"## Context\\n...\\n## Architecture\\n```mermaid\\n...\\n```\\n## Changes\\n| File | Change |\\n...\"\n\
           }\n\
         })\n\
         ```\n\n\
         For plans requiring explicit approval, use `push_review` instead — this blocks\n\
         until the user accepts/rejects in the companion window.\n"
    );

    instructions.push_str("\n## Agent Rule Bootstrap\n\n");
    instructions.push_str(
        "Call `setup_agent_rules` once per project to receive behavioral rules for all\n\
         available renderers and tools. Pass your `agent_type` (e.g. 'claude_code',\n\
         'claude_desktop', 'cursor', 'codex') so the response includes platform-specific\n\
         instructions for persisting the rules. Then follow those instructions to commit\n\
         the rules into your memory/rule system so they guide your behavior across\n\
         conversations.\n"
    );

    // Check for plugins needing authentication
    let registry = state_guard.inner.plugin_registry.lock().unwrap();
    let mut auth_warnings: Vec<String> = Vec::new();
    for manifest in &registry.manifests {
        if let Some(mcp) = &manifest.mcp {
            if let Some(auth) = &mcp.auth {
                if !auth.is_configured(&manifest.name) {
                    auth_warnings.push(format!(
                        "- Plugin '{}' requires authentication ({})",
                        manifest.name,
                        auth.display_name()
                    ));
                }
            }
        }
    }
    drop(registry);

    if !auth_warnings.is_empty() {
        instructions.push_str("\n## Authentication Required\n\n");
        instructions.push_str(
            "The following plugins need authentication before their tools are available:\n\n",
        );
        for warning in &auth_warnings {
            instructions.push_str(warning);
            instructions.push_str("\n");
        }
        instructions.push_str(
            "\nCall `setup_agent_rules` to get auth URLs and status details.\n",
        );
    }

    instructions
}

/// Handle a single JSON-RPC request
async fn handle_single_request(
    req: JsonRpcRequest,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Option<JsonRpcResponse> {
    let id = req.id.clone();

    match req.method.as_str() {
        "initialize" => {
            let requested_version = req
                .params
                .as_ref()
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or(SUPPORTED_VERSIONS[0]);

            let negotiated = if SUPPORTED_VERSIONS.contains(&requested_version) {
                requested_version
            } else {
                SUPPORTED_VERSIONS[0]
            };

            let instructions = build_instructions(state).await;

            Some(JsonRpcResponse::success(
                id,
                serde_json::json!({
                    "protocolVersion": negotiated,
                    "capabilities": {
                        "tools": { "listChanged": true }
                    },
                    "serverInfo": {
                        "name": "mcp-mux",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "instructions": instructions
                }),
            ))
        }

        // Notification — no response
        "notifications/initialized" => None,

        "tools/list" => {
            let tools = mcp_tools::list_tools(state).await;
            Some(JsonRpcResponse::success(
                id,
                serde_json::json!({ "tools": tools }),
            ))
        }

        "tools/call" => {
            let params = req.params.unwrap_or(Value::Object(Default::default()));
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));

            match mcp_tools::call_tool(&name, arguments, state).await {
                Ok(result) => Some(JsonRpcResponse::success(id, result)),
                Err(err_msg) => Some(JsonRpcResponse::success(
                    id,
                    serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": err_msg
                        }],
                        "isError": true
                    }),
                )),
            }
        }

        _ => Some(JsonRpcResponse::error(
            id,
            -32601,
            format!("Method not found: {}", req.method),
        )),
    }
}

pub async fn mcp_handler(
    state: Arc<TokioMutex<AsyncAppState>>,
    body: String,
) -> (StatusCode, serde_json::Value) {
    // Try parsing as a single request or a batch
    let body_value: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::OK,
                serde_json::to_value(JsonRpcResponse::error(
                    None,
                    -32700,
                    "Parse error".to_string(),
                ))
                .unwrap(),
            );
        }
    };

    if body_value.is_array() {
        // Batch request
        let requests: Vec<JsonRpcRequest> = match serde_json::from_value(body_value) {
            Ok(r) => r,
            Err(_) => {
                return (
                    StatusCode::OK,
                    serde_json::to_value(JsonRpcResponse::error(
                        None,
                        -32700,
                        "Parse error: invalid batch".to_string(),
                    ))
                    .unwrap(),
                );
            }
        };

        let mut responses = Vec::new();
        for req in requests {
            if let Some(resp) = handle_single_request(req, &state).await {
                responses.push(resp);
            }
        }

        if responses.is_empty() {
            // All were notifications — return empty OK (no JSON-RPC response for notifications)
            return (StatusCode::OK, Value::Null);
        }

        (StatusCode::OK, serde_json::to_value(responses).unwrap())
    } else {
        // Single request
        let request: JsonRpcRequest = match serde_json::from_value(body_value) {
            Ok(r) => r,
            Err(_) => {
                return (
                    StatusCode::OK,
                    serde_json::to_value(JsonRpcResponse::error(
                        None,
                        -32700,
                        "Parse error: invalid request".to_string(),
                    ))
                    .unwrap(),
                );
            }
        };

        match handle_single_request(request, &state).await {
            Some(resp) => (StatusCode::OK, serde_json::to_value(resp).unwrap()),
            // Notification — return empty OK
            None => (StatusCode::OK, Value::Null),
        }
    }
}
