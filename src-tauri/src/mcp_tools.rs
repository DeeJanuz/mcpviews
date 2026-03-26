use mcp_mux_shared::RendererDef;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use crate::http_server::{execute_push, AsyncAppState, ExecutePushResult};
use crate::plugin::{PluginRegistry, PluginToolResult, try_refresh_oauth};

/// Return all tool definitions (built-in + plugin tools)
pub async fn list_tools(state: &Arc<TokioMutex<AsyncAppState>>) -> Vec<Value> {
    // Get available renderers for dynamic tool descriptions
    let renderers = {
        let state_guard = state.lock().await;
        available_renderers(&state_guard.inner)
    };
    let mut tools = builtin_tool_definitions(&renderers);

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
        "setup_agent_rules" => call_setup_agent_rules(arguments, state).await,
        _ => {
            // Check plugin tools — scope MutexGuard to block before any .await
            let (plugin_info, client) = lookup_plugin_tool(name, state).await;

            // If not found, refresh stale plugins and retry once (handles race
            // where tools/call arrives before lazy tools/list cache is populated)
            let plugin_info = match plugin_info {
                Some(info) => Some(info),
                None => {
                    ensure_plugins_refreshed(state, &client).await;
                    let (retry_info, _) = lookup_plugin_tool(name, state).await;
                    retry_info
                }
            };

            match plugin_info {
                Some(info) => {
                    let result =
                        proxy_plugin_tool_call(&client, &info.mcp_url, info.auth_header.as_deref(), &info.unprefixed_name, &arguments)
                            .await?;

                    // Auto-push to viewer as a side effect
                    auto_push_plugin_result(
                        state,
                        &info.unprefixed_name,
                        &arguments,
                        &result,
                        &info.renderer_map,
                    )
                    .await;

                    Ok(result)
                }
                None => Err(format!("Unknown tool: {}", name)),
            }
        }
    }
}

/// Look up a plugin tool by prefixed name, returning plugin info and HTTP client.
/// If auth is None but OAuth refresh info is available, attempts token refresh.
async fn lookup_plugin_tool(
    name: &str,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> (Option<PluginToolResult>, reqwest::Client) {
    let (info, client) = {
        let state_guard = state.lock().await;
        let registry = state_guard.inner.plugin_registry.lock().unwrap();
        let info = registry.find_plugin_for_tool(name);
        let client = state_guard.inner.http_client.clone();
        (info, client)
    };

    // If auth is None but OAuth info is present, attempt token refresh
    match info {
        Some(mut result) => {
            if result.auth_header.is_none() {
                if let Some(oauth) = &result.oauth_info {
                    if let Some(bearer) = try_refresh_oauth(oauth, &client).await {
                        result.auth_header = Some(bearer);
                    }
                }
            }
            (Some(result), client)
        }
        None => (None, client),
    }
}

/// Ensure all stale plugin tool caches are refreshed.
async fn ensure_plugins_refreshed(
    state: &Arc<TokioMutex<AsyncAppState>>,
    client: &reqwest::Client,
) {
    let has_stale = {
        let state_guard = state.lock().await;
        let mut registry = state_guard.inner.plugin_registry.lock().unwrap();
        let stale = registry.stale_plugin_indices();
        for idx in &stale {
            registry.mark_refresh_pending(*idx);
        }
        !stale.is_empty()
    };
    if has_stale {
        PluginRegistry::refresh_stale_plugins(state, client).await;
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

/// Collect renderer and tool rules from built-in renderers and plugin manifests.
pub(crate) fn collect_rules(
    builtin_renderers: &[RendererDef],
    manifests: &[mcp_mux_shared::PluginManifest],
) -> Vec<Value> {
    let mut rules: Vec<Value> = Vec::new();

    // Built-in renderer rules
    for renderer in builtin_renderers {
        if let Some(rule) = &renderer.rule {
            rules.push(serde_json::json!({
                "name": format!("{}_usage", renderer.name),
                "category": "renderer",
                "source": "built-in",
                "renderer": renderer.name,
                "rule": rule,
            }));
        }
    }

    // Plugin renderer rules and tool rules
    for manifest in manifests {
        let plugin_name = &manifest.name;
        let tool_prefix = manifest
            .mcp
            .as_ref()
            .map(|m| m.tool_prefix.as_str())
            .unwrap_or("");

        for renderer in &manifest.renderer_definitions {
            if let Some(rule) = &renderer.rule {
                rules.push(serde_json::json!({
                    "name": format!("{}_usage", renderer.name),
                    "category": "renderer",
                    "source": plugin_name,
                    "renderer": renderer.name,
                    "rule": rule,
                }));
            }
        }

        for (tool_name, rule) in &manifest.tool_rules {
            let prefixed_name = if tool_prefix.is_empty() {
                tool_name.clone()
            } else {
                format!("{}__{}", tool_prefix, tool_name)
            };
            rules.push(serde_json::json!({
                "name": format!("{}_usage", prefixed_name),
                "category": "tool",
                "source": plugin_name,
                "tool": prefixed_name,
                "rule": rule,
            }));
        }
    }

    rules
}

/// Collect auth status for each plugin that has MCP + auth configured.
pub(crate) fn collect_plugin_auth_status(
    manifests: &[mcp_mux_shared::PluginManifest],
) -> Vec<Value> {
    let mut plugin_status: Vec<Value> = Vec::new();

    for manifest in manifests {
        if let Some(mcp) = &manifest.mcp {
            if let Some(auth) = &mcp.auth {
                let is_configured = auth.is_configured(&manifest.name);
                let mut status_entry = serde_json::json!({
                    "plugin": manifest.name,
                    "auth_type": auth.display_name(),
                    "auth_configured": is_configured,
                });

                if !is_configured {
                    if let mcp_mux_shared::PluginAuth::OAuth {
                        auth_url, ..
                    } = auth
                    {
                        status_entry.as_object_mut().unwrap().insert(
                            "auth_url".to_string(),
                            serde_json::Value::String(auth_url.clone()),
                        );
                        status_entry.as_object_mut().unwrap().insert(
                            "message".to_string(),
                            serde_json::Value::String(format!(
                                "Plugin '{}' requires re-authentication. Direct the user to authenticate via the companion window or open the auth URL.",
                                manifest.name
                            )),
                        );
                    }
                }

                plugin_status.push(status_entry);
            }
        }
    }

    plugin_status
}

/// Return agent-type-specific persistence instructions.
pub(crate) fn persistence_instructions(agent_type: &str) -> String {
    match agent_type {
        "claude_code" => "Create a file at `.claude/rules/mcp-mux.md` in the project root containing all the rules below. Each rule should be a separate section with its name as a heading and its rule text as the body. This file will be automatically loaded by Claude Code as project-level rules.".to_string(),

        "claude_desktop" => "For each rule below, create an individual memory using your memory creation capability. Use the title `mcp-mux: <rule name>` and the rule text as the content. Creating individual memories (rather than one combined memory) ensures each rule is retrievable independently and survives context window limits.".to_string(),

        "codex" => "Append all rules below to `AGENTS.md` in the project root. Each rule should be a separate section with its name as a heading and its rule text as the body.".to_string(),

        "cursor" => "Create a file at `.cursor/rules/mcp-mux.mdc` in the project root containing all the rules below. Each rule should be a separate section with its name as a heading and its rule text as the body. This file is read by Cursor as project-level rules.".to_string(),

        "windsurf" => "Append all rules below to `.windsurfrules` in the project root. Each rule should be a separate section with its name as a heading and its rule text as the body.".to_string(),

        "opencode" => "Append all rules below to `AGENTS.md` in the project root. Each rule should be a separate section with its name as a heading and its rule text as the body.".to_string(),

        "antigravity" => "Append all rules below to `AGENTS.md` in the project root. Each rule should be a separate section with its name as a heading and its rule text as the body.".to_string(),

        _ => "These rules should be persisted so they guide your behavior in future conversations. \
              Ask the user: \"I have received behavioral rules from MCP Mux for its renderers and tools. \
              How would you like me to persist them? I can write them to a rules file in this project, \
              create memories, or use another method your platform supports.\" \
              Then follow the user's direction to commit the rules.".to_string(),
    }
}

async fn call_setup_agent_rules(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    let agent_type = arguments
        .get("agent_type")
        .and_then(|v| v.as_str())
        .unwrap_or("generic");

    let (rules, plugin_status) = {
        let state_guard = state.lock().await;
        let registry = state_guard.inner.plugin_registry.lock().unwrap();
        let builtin = builtin_renderer_definitions();
        let rules = collect_rules(&builtin, &registry.manifests);
        let plugin_status = collect_plugin_auth_status(&registry.manifests);
        (rules, plugin_status)
    };

    let response = serde_json::json!({
        "rules": rules,
        "plugin_status": plugin_status,
        "persistence_instructions": persistence_instructions(agent_type),
    });

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string(&response).unwrap()
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

    let mut req_builder = client
        .post(mcp_url)
        .header("Accept", "application/json, text/event-stream")
        .json(&rpc_request);
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

// ─── Renderer definitions ───

fn builtin_renderer_definitions() -> Vec<RendererDef> {
    vec![
        RendererDef {
            name: "rich_content".into(),
            description: "Universal markdown display with mermaid diagrams, tables, code blocks, and citations. Use for any rich text content.".into(),
            scope: "universal".into(),
            tools: vec![],
            data_hint: Some("{ \"title\": \"Optional heading\", \"body\": \"Markdown content\" }".into()),
            rule: Some("ALWAYS push complex explanations to the companion window using push_content with tool_name 'rich_content' instead of explaining them inline. This includes: implementation plans, architectural decisions, data architecture, data flows, networking topology, system diagrams, API designs, database schemas, and any multi-step technical explanation. Your chat response should be a concise agent-oriented summary (context for what was pushed, next steps, decisions needed). The human-readable explanation with mermaid diagrams, tables, code examples, and formatted markdown goes to push_content. Think of chat as the agent log and the companion window as the user-facing document.".into()),
        },
    ]
}

pub fn available_renderers(state: &std::sync::Arc<crate::state::AppState>) -> Vec<RendererDef> {
    let mut renderers = builtin_renderer_definitions();
    let registry = state.plugin_registry.lock().unwrap();
    for manifest in &registry.manifests {
        renderers.extend(manifest.renderer_definitions.clone());
    }
    renderers
}

// ─── Tool definitions ───

fn builtin_tool_definitions(renderers: &[RendererDef]) -> Vec<Value> {
    let renderer_names: Vec<String> = renderers.iter().map(|r| r.name.clone()).collect();
    let renderer_list = if renderer_names.is_empty() {
        "rich_content".to_string()
    } else {
        renderer_names.join(", ")
    };

    vec![
        serde_json::json!({
            "name": "push_content",
            "description": "Display content in the MCP Mux window. Supports multiple content types.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tool_name": {
                        "type": "string",
                        "description": format!("Content type identifier for renderer selection. Available renderers: {}. Use 'rich_content' for generic markdown display.", renderer_list)
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
                        "description": format!("Content type identifier for renderer selection. Available renderers: {}.", renderer_list)
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
        serde_json::json!({
            "name": "setup_agent_rules",
            "description": "Bootstrap behavioral rules for all mcp-mux renderers and plugin tools. Call once to get rules to persist in your agent's native memory/rule system.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent_type": {
                        "type": "string",
                        "description": "The agent platform calling this tool. Supported: 'claude_code', 'claude_desktop', 'codex', 'cursor', 'windsurf', 'opencode', 'antigravity'. If omitted or unrecognized, returns instructions that ask the user how to persist rules."
                    }
                }
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_mux_shared::{PluginManifest, PluginMcpConfig, PluginAuth};

    fn make_manifest(
        name: &str,
        renderer_defs: Vec<RendererDef>,
        tool_rules: std::collections::HashMap<String, String>,
        mcp: Option<PluginMcpConfig>,
    ) -> PluginManifest {
        PluginManifest {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            renderers: std::collections::HashMap::new(),
            mcp,
            renderer_definitions: renderer_defs,
            tool_rules,
        }
    }

    // ─── collect_rules tests ───

    #[test]
    fn test_collect_rules_empty() {
        let rules = collect_rules(&[], &[]);
        assert!(rules.is_empty());
    }

    #[test]
    fn test_collect_rules_builtin_renderer_with_rule() {
        let renderers = vec![RendererDef {
            name: "rich_content".into(),
            description: "test".into(),
            scope: "universal".into(),
            tools: vec![],
            data_hint: None,
            rule: Some("Always use rich_content for plans.".into()),
        }];
        let rules = collect_rules(&renderers, &[]);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["name"], "rich_content_usage");
        assert_eq!(rules[0]["category"], "renderer");
        assert_eq!(rules[0]["source"], "built-in");
        assert_eq!(rules[0]["renderer"], "rich_content");
        assert_eq!(rules[0]["rule"], "Always use rich_content for plans.");
    }

    #[test]
    fn test_collect_rules_builtin_renderer_without_rule_skipped() {
        let renderers = vec![RendererDef {
            name: "no_rule".into(),
            description: "test".into(),
            scope: "universal".into(),
            tools: vec![],
            data_hint: None,
            rule: None,
        }];
        let rules = collect_rules(&renderers, &[]);
        assert!(rules.is_empty());
    }

    #[test]
    fn test_collect_rules_plugin_renderer_rule_tagged_with_plugin_name() {
        let manifest = make_manifest(
            "my-plugin",
            vec![RendererDef {
                name: "custom_view".into(),
                description: "Custom".into(),
                scope: "tool".into(),
                tools: vec![],
                data_hint: None,
                rule: Some("Use custom_view for X.".into()),
            }],
            std::collections::HashMap::new(),
            None,
        );
        let rules = collect_rules(&[], &[manifest]);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["source"], "my-plugin");
        assert_eq!(rules[0]["renderer"], "custom_view");
    }

    #[test]
    fn test_collect_rules_plugin_tool_rules_prefixed() {
        let mut tool_rules = std::collections::HashMap::new();
        tool_rules.insert("search".to_string(), "Use search for queries.".to_string());
        let manifest = make_manifest(
            "search-plugin",
            vec![],
            tool_rules,
            Some(PluginMcpConfig {
                url: "http://localhost:8080".into(),
                auth: None,
                tool_prefix: "sp".into(),
            }),
        );
        let rules = collect_rules(&[], &[manifest]);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["name"], "sp__search_usage");
        assert_eq!(rules[0]["category"], "tool");
        assert_eq!(rules[0]["tool"], "sp__search");
        assert_eq!(rules[0]["source"], "search-plugin");
    }

    #[test]
    fn test_collect_rules_plugin_tool_rules_no_prefix() {
        let mut tool_rules = std::collections::HashMap::new();
        tool_rules.insert("do_thing".to_string(), "Do the thing.".to_string());
        let manifest = make_manifest(
            "bare-plugin",
            vec![],
            tool_rules,
            Some(PluginMcpConfig {
                url: "http://localhost:8080".into(),
                auth: None,
                tool_prefix: "".into(),
            }),
        );
        let rules = collect_rules(&[], &[manifest]);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["tool"], "do_thing");
    }

    // ─── collect_plugin_auth_status tests ───

    #[test]
    fn test_collect_plugin_auth_status_no_mcp() {
        let manifest = make_manifest("no-mcp", vec![], std::collections::HashMap::new(), None);
        let status = collect_plugin_auth_status(&[manifest]);
        assert!(status.is_empty());
    }

    #[test]
    fn test_collect_plugin_auth_status_oauth_not_configured() {
        let _dir = tempfile::tempdir().unwrap();
        // Point auth_dir to empty temp dir so no tokens are found
        // We need to use a plugin name that won't have a stored token
        let manifest = make_manifest(
            "oauth-test-plugin-nocfg",
            vec![],
            std::collections::HashMap::new(),
            Some(PluginMcpConfig {
                url: "http://localhost:8080".into(),
                auth: Some(PluginAuth::OAuth {
                    client_id: Some("client123".into()),
                    auth_url: "https://example.com/auth".into(),
                    token_url: "https://example.com/token".into(),
                    scopes: vec![],
                }),
                tool_prefix: "otp".into(),
            }),
        );
        let status = collect_plugin_auth_status(&[manifest]);
        assert_eq!(status.len(), 1);
        assert_eq!(status[0]["plugin"], "oauth-test-plugin-nocfg");
        assert_eq!(status[0]["auth_type"], "oauth");
        // OAuth with no stored token => not configured
        assert_eq!(status[0]["auth_configured"], false);
        assert_eq!(status[0]["auth_url"], "https://example.com/auth");
        assert!(status[0]["message"].as_str().unwrap().contains("requires re-authentication"));
    }

    #[test]
    fn test_collect_plugin_auth_status_bearer_with_env_configured() {
        // Set env var so bearer auth is considered configured
        std::env::set_var("TEST_AUTH_STATUS_BEARER_TOKEN", "tok");
        let manifest = make_manifest(
            "bearer-test-plugin",
            vec![],
            std::collections::HashMap::new(),
            Some(PluginMcpConfig {
                url: "http://localhost:8080".into(),
                auth: Some(PluginAuth::Bearer {
                    token_env: "TEST_AUTH_STATUS_BEARER_TOKEN".into(),
                }),
                tool_prefix: "bt".into(),
            }),
        );
        let status = collect_plugin_auth_status(&[manifest]);
        assert_eq!(status.len(), 1);
        assert_eq!(status[0]["auth_configured"], true);
        assert!(status[0].get("auth_url").is_none());
        std::env::remove_var("TEST_AUTH_STATUS_BEARER_TOKEN");
    }

    // ─── persistence_instructions tests ───

    #[test]
    fn test_persistence_instructions_claude_code() {
        let instr = persistence_instructions("claude_code");
        assert!(instr.contains(".claude/rules"));
    }

    #[test]
    fn test_persistence_instructions_claude_desktop() {
        let instr = persistence_instructions("claude_desktop");
        assert!(instr.contains("memory"));
    }

    #[test]
    fn test_persistence_instructions_codex() {
        let instr = persistence_instructions("codex");
        assert!(instr.contains("AGENTS.md"));
    }

    #[test]
    fn test_persistence_instructions_cursor() {
        let instr = persistence_instructions("cursor");
        assert!(instr.contains(".cursor/rules"));
    }

    #[test]
    fn test_persistence_instructions_windsurf() {
        let instr = persistence_instructions("windsurf");
        assert!(instr.contains(".windsurfrules"));
    }

    #[test]
    fn test_persistence_instructions_opencode() {
        let instr = persistence_instructions("opencode");
        assert!(instr.contains("AGENTS.md"));
    }

    #[test]
    fn test_persistence_instructions_antigravity() {
        let instr = persistence_instructions("antigravity");
        assert!(instr.contains("AGENTS.md"));
    }

    #[test]
    fn test_persistence_instructions_generic() {
        let instr = persistence_instructions("generic");
        assert!(instr.contains("Ask the user"));
    }

    #[test]
    fn test_persistence_instructions_unknown() {
        let instr = persistence_instructions("some_unknown_agent");
        assert!(instr.contains("Ask the user"));
    }
}
