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
                    }
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
