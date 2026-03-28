use mcpviews_shared::RendererDef;
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
        "init_session" => call_init_session(arguments, state).await,
        "mcpviews_setup" => call_mcpviews_setup(arguments, state).await,
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

/// Collect renderer and tool rules from all renderers and plugin manifests.
pub(crate) fn collect_rules(
    all_renderers: &[RendererDef],
    manifests: &[mcpviews_shared::PluginManifest],
) -> Vec<Value> {
    let mut rules: Vec<Value> = Vec::new();

    // Renderer rules — covers built-in, explicit, AND synthesized renderers.
    // Always include description, data_hint, scope, and tools so agents know
    // the payload schema regardless of how the renderer was defined.
    for renderer in all_renderers {
        if let Some(rule) = &renderer.rule {
            // Renderer has an explicit rule
            let source = if renderer.scope == "universal" { "built-in" } else { "plugin" };
            rules.push(serde_json::json!({
                "name": format!("{}_usage", renderer.name),
                "category": "renderer",
                "source": source,
                "renderer": renderer.name,
                "description": renderer.description,
                "scope": renderer.scope,
                "data_hint": renderer.data_hint,
                "tools": renderer.tools,
                "rule": rule,
            }));
        } else if renderer.scope == "tool" && !renderer.tools.is_empty() {
            // Synthesized tool-scoped renderer — generate a usage hint from description
            rules.push(serde_json::json!({
                "name": format!("{}_usage", renderer.name),
                "category": "renderer",
                "source": "plugin",
                "renderer": renderer.name,
                "description": renderer.description,
                "scope": renderer.scope,
                "data_hint": renderer.data_hint,
                "tools": renderer.tools,
            }));
        }
    }

    // Plugin tool rules
    for manifest in manifests {
        let plugin_name = &manifest.name;
        let tool_prefix = manifest
            .mcp
            .as_ref()
            .map(|m| m.tool_prefix.as_str())
            .unwrap_or("");

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
    manifests: &[mcpviews_shared::PluginManifest],
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
                    if let mcpviews_shared::PluginAuth::OAuth {
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
        "claude_code" => "Create a file at `.claude/rules/mcpviews.md` in the project root containing all the rules below. Each rule should be a separate section with its name as a heading and its rule text as the body. This file will be automatically loaded by Claude Code as project-level rules.".to_string(),

        "claude_desktop" => "For each rule below, create an individual memory using your memory creation capability. Use the title `mcpviews: <rule name>` and the rule text as the content. Creating individual memories (rather than one combined memory) ensures each rule is retrievable independently and survives context window limits.".to_string(),

        "codex" | "opencode" | "antigravity" => "Append all rules below to `AGENTS.md` in the project root. Each rule should be a separate section with its name as a heading and its rule text as the body.".to_string(),

        "cursor" => "Create a file at `.cursor/rules/mcpviews.mdc` in the project root containing all the rules below. Each rule should be a separate section with its name as a heading and its rule text as the body. This file is read by Cursor as project-level rules.".to_string(),

        "windsurf" => "Append all rules below to `.windsurfrules` in the project root. Each rule should be a separate section with its name as a heading and its rule text as the body.".to_string(),

        _ => "These rules should be persisted so they guide your behavior in future conversations. \
              Ask the user: \"I have received behavioral rules from MCPViews for its renderers and tools. \
              How would you like me to persist them? I can write them to a rules file in this project, \
              create memories, or use another method your platform supports.\" \
              Then follow the user's direction to commit the rules.".to_string(),
    }
}

/// Extract lightweight tool summaries (name + description) from full tool definitions.
fn extract_tool_summaries(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|t| {
            let name = t.get("name")?.as_str()?;
            let description = t
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            Some(serde_json::json!({
                "name": name,
                "description": description,
            }))
        })
        .collect()
}

/// Gather rules, plugin auth status, and available tool summaries from the current state.
async fn gather_session_data(state: &Arc<TokioMutex<AsyncAppState>>) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let all_tools = list_tools(state).await;
    let available_tools = extract_tool_summaries(&all_tools);

    let state_guard = state.lock().await;
    let all_renderers = available_renderers(&state_guard.inner);
    let registry = state_guard.inner.plugin_registry.lock().unwrap();
    let rules = collect_rules(&all_renderers, &registry.manifests);
    let plugin_status = collect_plugin_auth_status(&registry.manifests);
    (rules, plugin_status, available_tools)
}

async fn call_init_session(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    let agent_type = arguments
        .get("agent_type")
        .and_then(|v| v.as_str())
        .unwrap_or("generic");

    let (rules, plugin_status, available_tools) = gather_session_data(state).await;

    let response = serde_json::json!({
        "rules": rules,
        "plugin_status": plugin_status,
        "persistence_instructions": persistence_instructions(agent_type),
        "available_tools": available_tools,
    });

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string(&response).unwrap()
        }]
    }))
}

/// Return platform-specific instructions for configuring automatic session initialization.
pub(crate) fn setup_instructions(agent_type: &str) -> String {
    match agent_type {
        "claude_code" => "Add a rule in `.claude/rules/mcpviews-init.md` containing: 'IMPORTANT: At the start of every conversation, chat session, or interaction, call the `init_session` MCP tool to initialize MCPViews and get current renderer definitions, behavioral rules, plugin auth status, and the list of available tools. This must happen every time — not just once.'".to_string(),

        "claude_desktop" => "Create a memory: 'IMPORTANT: At the start of every conversation, chat session, or interaction, call the `init_session` MCP tool to initialize MCPViews and get current renderer definitions, rules, and the list of available tools. This must happen every time — not just once.'".to_string(),

        "cursor" => "Add to `.cursor/rules/mcpviews-init.mdc`: 'IMPORTANT: At the start of every conversation, chat session, or interaction, call the `init_session` MCP tool to initialize MCPViews and get current renderer definitions, rules, and the list of available tools. This must happen every time — not just once.'".to_string(),

        "codex" | "opencode" | "antigravity" => "Add to `AGENTS.md`: 'IMPORTANT: At the start of every conversation, chat session, or interaction, call the `init_session` MCP tool to initialize MCPViews and get current renderer definitions, rules, and the list of available tools. This must happen every time — not just once.'".to_string(),

        "windsurf" => "Add to `.windsurfrules`: 'IMPORTANT: At the start of every conversation, chat session, or interaction, call the `init_session` MCP tool to initialize MCPViews and get current renderer definitions, rules, and the list of available tools. This must happen every time — not just once.'".to_string(),

        _ => "Ask the user how to persist a rule that calls `init_session` at the start of every conversation, chat session, or interaction — not just once, but every time.".to_string(),
    }
}

async fn call_mcpviews_setup(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    let agent_type = arguments
        .get("agent_type")
        .and_then(|v| v.as_str())
        .unwrap_or("generic");

    let (rules, plugin_status, available_tools) = gather_session_data(state).await;

    let response = serde_json::json!({
        "rules": rules,
        "plugin_status": plugin_status,
        "persistence_instructions": persistence_instructions(agent_type),
        "setup_instructions": setup_instructions(agent_type),
        "available_tools": available_tools,
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

// ─── Renderer definitions ───

fn builtin_renderer_definitions() -> Vec<RendererDef> {
    vec![
        RendererDef {
            name: "rich_content".into(),
            description: "Universal markdown display with mermaid diagrams, tables, code blocks, and citations. Use for any rich text content.".into(),
            scope: "universal".into(),
            tools: vec![],
            data_hint: Some("{ \"title\": \"Optional heading\", \"body\": \"Markdown content\" }".into()),
            rule: Some("CALLER RESTRICTION: ONLY the main/coordinator agent may call push_content, push_review, and push_check. Sub-agents and background agents must NEVER call these tools — they return results to the coordinator, which decides what to push.\n\nWhen to push (main agent only):\n- Detailed explanations that benefit from structured formatting, diagrams, or tables\n- Plan summaries for human review\n- Architecture, data flows, system diagrams, API designs, database schemas\n- Implementation plans with structural decisions\n\nKeep your chat response concise (context, next steps, decisions needed). The detailed explanation with mermaid diagrams, tables, and formatted markdown goes to push_content.".into()),
        },
        RendererDef {
            name: "structured_data".into(),
            description: "Tabular data with hierarchical rows, change tracking, sort/filter, and review mode with per-row/column accept/reject and cell editing.".into(),
            scope: "universal".into(),
            tools: vec![],
            data_hint: Some(r#"{ "title": "Optional", "tables": [{ "id": "t1", "name": "Name", "columns": [{ "id": "c1", "name": "Col", "change": null|"add"|"delete" }], "rows": [{ "id": "r1", "cells": { "c1": { "value": "v", "change": null|"add"|"delete"|"update" } }, "children": [] }] }] }"#.into()),
            rule: Some("Use structured_data for tabular/schema data. Supports nested rows via children arrays (arbitrary depth). In review mode (push_review), users can accept/reject individual rows and new columns, edit cells, then submit. For simple flat tables without change tracking, prefer rich_content with markdown tables.".into()),
        },
    ]
}

/// Synthesize `RendererDef` entries from a manifest's `renderers` map for any
/// renderer names not already in `known_names`. Uses cached tool definitions
/// to derive descriptions when available.
fn synthesize_renderer_defs(
    manifest: &mcpviews_shared::PluginManifest,
    cached_tools: Option<&[serde_json::Value]>,
    known_names: &std::collections::HashSet<&str>,
) -> Vec<RendererDef> {
    // Group tools by renderer name, skipping already-known renderers
    let mut renderer_tools: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    for (tool_name, renderer_name) in &manifest.renderers {
        if !known_names.contains(renderer_name.as_str()) {
            renderer_tools
                .entry(renderer_name.as_str())
                .or_default()
                .push(tool_name.as_str());
        }
    }

    let prefix = manifest
        .mcp
        .as_ref()
        .map(|m| m.tool_prefix.as_str())
        .unwrap_or("");

    let mut result = Vec::new();
    for (renderer_name, tool_names) in renderer_tools {
        let mut tool_descriptions: Vec<String> = Vec::new();

        for tool_name in &tool_names {
            let prefixed = format!("{}{}", prefix, tool_name);
            if let Some(tools) = cached_tools {
                if let Some(tool_def) = tools
                    .iter()
                    .find(|t| t.get("name").and_then(|n| n.as_str()) == Some(&prefixed))
                {
                    if let Some(desc) = tool_def.get("description").and_then(|d| d.as_str()) {
                        tool_descriptions.push(format!("- {}: {}", tool_name, desc));
                    }
                }
            }
        }

        let description = if tool_descriptions.is_empty() {
            format!("Renderer for {} plugin", manifest.name)
        } else {
            format!(
                "Renders output from these tools:\n{}",
                tool_descriptions.join("\n")
            )
        };

        let data_hint = format!(
            "Pass the result from any of these tools: {}. The data shape matches the tool's response.",
            tool_names.join(", ")
        );

        result.push(RendererDef {
            name: renderer_name.to_string(),
            description,
            scope: "tool".to_string(),
            tools: tool_names.iter().map(|s| s.to_string()).collect(),
            data_hint: Some(data_hint),
            rule: None,
        });
    }

    result
}

pub fn available_renderers(state: &std::sync::Arc<crate::state::AppState>) -> Vec<RendererDef> {
    let mut renderers = builtin_renderer_definitions();
    let registry = state.plugin_registry.lock().unwrap();

    for (idx, manifest) in registry.manifests.iter().enumerate() {
        // 1. Add explicit renderer definitions (plugin-provided, rich metadata)
        renderers.extend(manifest.renderer_definitions.clone());

        // 2. Collect names already covered
        let known: std::collections::HashSet<&str> =
            renderers.iter().map(|r| r.name.as_str()).collect();

        // 3. Synthesize from renderers map for any not already covered
        let cached_tools = registry.tool_cache.plugin_tools(idx);
        renderers.extend(synthesize_renderer_defs(manifest, cached_tools, &known));
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
            "description": "Display content in the MCPViews window. Supports multiple content types.",
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
            "description": "Display content in the MCPViews window and block until the user submits a review decision (accept/reject/partial). Use for mutation operations that need user approval before proceeding.",
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
            "name": "init_session",
            "description": "Initialize MCPViews for this session. Returns current renderer definitions, behavioral rules, plugin auth status, and persistence instructions. Should be called at the start of every new agent session.",
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
        serde_json::json!({
            "name": "mcpviews_setup",
            "description": "One-time setup for MCPViews. Returns instructions for persisting a session-start rule that ensures init_session is called automatically in every new session. Also returns current rules and plugin status.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent_type": {
                        "type": "string",
                        "description": "The agent platform calling this tool. Supported: 'claude_code', 'claude_desktop', 'codex', 'cursor', 'windsurf', 'opencode', 'antigravity'. If omitted or unrecognized, returns generic instructions."
                    }
                }
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcpviews_shared::{PluginManifest, PluginMcpConfig, PluginAuth};

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
            no_auto_push: vec![],
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
            description: "Universal markdown display".into(),
            scope: "universal".into(),
            tools: vec![],
            data_hint: Some(r#"{ "title": "heading", "body": "markdown" }"#.into()),
            rule: Some("Always use rich_content for plans.".into()),
        }];
        let rules = collect_rules(&renderers, &[]);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["name"], "rich_content_usage");
        assert_eq!(rules[0]["category"], "renderer");
        assert_eq!(rules[0]["source"], "built-in");
        assert_eq!(rules[0]["renderer"], "rich_content");
        assert_eq!(rules[0]["rule"], "Always use rich_content for plans.");
        // New fields: description, scope, data_hint always present
        assert_eq!(rules[0]["description"], "Universal markdown display");
        assert_eq!(rules[0]["scope"], "universal");
        assert_eq!(rules[0]["data_hint"], r#"{ "title": "heading", "body": "markdown" }"#);
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
    fn test_collect_rules_renderer_with_rule() {
        let renderers = vec![RendererDef {
            name: "custom_view".into(),
            description: "Custom".into(),
            scope: "tool".into(),
            tools: vec![],
            data_hint: None,
            rule: Some("Use custom_view for X.".into()),
        }];
        let rules = collect_rules(&renderers, &[]);
        assert_eq!(rules.len(), 1);
        // tool-scoped renderer with rule → source is "plugin"
        assert_eq!(rules[0]["source"], "plugin");
        assert_eq!(rules[0]["renderer"], "custom_view");
        assert_eq!(rules[0]["description"], "Custom");
        assert_eq!(rules[0]["scope"], "tool");
    }

    #[test]
    fn test_collect_rules_synthesized_renderer_included() {
        let renderers = vec![RendererDef {
            name: "search_results".into(),
            description: "Renders search output".into(),
            scope: "tool".into(),
            tools: vec!["search_codebase".into()],
            data_hint: Some("Pass search results".into()),
            rule: None,
        }];
        let rules = collect_rules(&renderers, &[]);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["category"], "renderer");
        assert_eq!(rules[0]["source"], "plugin");
        assert_eq!(rules[0]["renderer"], "search_results");
        assert_eq!(rules[0]["tools"][0], "search_codebase");
        assert_eq!(rules[0]["scope"], "tool");
        assert_eq!(rules[0]["description"], "Renders search output");
        assert_eq!(rules[0]["data_hint"], "Pass search results");
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

    // ─── synthesize_renderer_defs tests ───

    fn make_manifest_with_renderers(
        name: &str,
        renderers: std::collections::HashMap<String, String>,
        prefix: &str,
    ) -> PluginManifest {
        PluginManifest {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            renderers,
            mcp: Some(PluginMcpConfig {
                url: "http://localhost:8080".into(),
                auth: None,
                tool_prefix: prefix.to_string(),
            }),
            renderer_definitions: vec![],
            tool_rules: std::collections::HashMap::new(),
            no_auto_push: vec![],
        }
    }

    #[test]
    fn test_synthesize_with_tool_cache_data() {
        let mut renderers_map = std::collections::HashMap::new();
        renderers_map.insert("search_codebase".to_string(), "search_results".to_string());
        let manifest = make_manifest_with_renderers("ludflow", renderers_map, "ludflow__");

        let cached_tools = vec![
            serde_json::json!({
                "name": "ludflow__search_codebase",
                "description": "Search the codebase for matching code"
            }),
        ];

        let known = std::collections::HashSet::new();
        let result = synthesize_renderer_defs(&manifest, Some(&cached_tools), &known);

        assert_eq!(result.len(), 1);
        let def = &result[0];
        assert_eq!(def.name, "search_results");
        assert!(def.description.contains("search_codebase"));
        assert!(def.description.contains("Search the codebase"));
        assert_eq!(def.tools, vec!["search_codebase"]);
        assert!(def.data_hint.is_some());
        assert_eq!(def.scope, "tool");
        assert!(def.rule.is_none());
    }

    #[test]
    fn test_synthesize_skips_known_renderers() {
        let mut renderers_map = std::collections::HashMap::new();
        renderers_map.insert("search_codebase".to_string(), "search_results".to_string());
        let manifest = make_manifest_with_renderers("ludflow", renderers_map, "ludflow__");

        let cached_tools = vec![
            serde_json::json!({
                "name": "ludflow__search_codebase",
                "description": "Search the codebase"
            }),
        ];

        let mut known = std::collections::HashSet::new();
        known.insert("search_results");
        let result = synthesize_renderer_defs(&manifest, Some(&cached_tools), &known);

        assert!(result.is_empty());
    }

    #[test]
    fn test_synthesize_without_cache_data() {
        let mut renderers_map = std::collections::HashMap::new();
        renderers_map.insert("search_codebase".to_string(), "search_results".to_string());
        let manifest = make_manifest_with_renderers("ludflow", renderers_map, "ludflow__");

        let known = std::collections::HashSet::new();
        let result = synthesize_renderer_defs(&manifest, None, &known);

        assert_eq!(result.len(), 1);
        let def = &result[0];
        assert_eq!(def.name, "search_results");
        assert!(def.description.contains("Renderer for ludflow plugin"));
        assert_eq!(def.tools, vec!["search_codebase"]);
    }

    // ─── setup_instructions tests ───

    #[test]
    fn test_setup_instructions_claude_code() {
        let instr = setup_instructions("claude_code");
        assert!(instr.contains("init_session"));
        assert!(instr.contains(".claude/rules"));
    }

    #[test]
    fn test_setup_instructions_claude_desktop() {
        let instr = setup_instructions("claude_desktop");
        assert!(instr.contains("init_session"));
        assert!(instr.contains("memory"));
    }

    #[test]
    fn test_setup_instructions_cursor() {
        let instr = setup_instructions("cursor");
        assert!(instr.contains("init_session"));
        assert!(instr.contains(".cursor/rules"));
    }

    #[test]
    fn test_setup_instructions_codex() {
        let instr = setup_instructions("codex");
        assert!(instr.contains("init_session"));
        assert!(instr.contains("AGENTS.md"));
    }

    #[test]
    fn test_setup_instructions_windsurf() {
        let instr = setup_instructions("windsurf");
        assert!(instr.contains("init_session"));
        assert!(instr.contains(".windsurfrules"));
    }

    #[test]
    fn test_setup_instructions_generic() {
        let instr = setup_instructions("generic");
        assert!(instr.contains("init_session"));
    }

    #[test]
    fn test_setup_instructions_unknown() {
        let instr = setup_instructions("some_unknown_agent");
        assert!(instr.contains("init_session"));
    }

    // ─── synthesize_renderer_defs tests ───

    // ─── extract_tool_summaries tests ───

    #[test]
    fn test_extract_tool_summaries_extracts_name_and_description() {
        let tools = vec![
            serde_json::json!({
                "name": "push_content",
                "description": "Display content in the MCPViews window.",
                "inputSchema": { "type": "object" }
            }),
            serde_json::json!({
                "name": "push_review",
                "description": "Display content and block until review.",
                "inputSchema": { "type": "object" }
            }),
        ];
        let summaries = extract_tool_summaries(&tools);
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0]["name"], "push_content");
        assert_eq!(summaries[0]["description"], "Display content in the MCPViews window.");
        // Should NOT include inputSchema
        assert!(summaries[0].get("inputSchema").is_none());
        assert_eq!(summaries[1]["name"], "push_review");
    }

    #[test]
    fn test_extract_tool_summaries_skips_entries_without_name() {
        let tools = vec![
            serde_json::json!({ "description": "no name field" }),
            serde_json::json!({ "name": "valid_tool", "description": "has name" }),
        ];
        let summaries = extract_tool_summaries(&tools);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0]["name"], "valid_tool");
    }

    #[test]
    fn test_extract_tool_summaries_handles_missing_description() {
        let tools = vec![
            serde_json::json!({ "name": "no_desc_tool" }),
        ];
        let summaries = extract_tool_summaries(&tools);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0]["name"], "no_desc_tool");
        assert_eq!(summaries[0]["description"], "");
    }

    #[test]
    fn test_synthesize_groups_multiple_tools_under_one_renderer() {
        let mut renderers_map = std::collections::HashMap::new();
        renderers_map.insert("search_codebase".to_string(), "search_results".to_string());
        renderers_map.insert("vector_search".to_string(), "search_results".to_string());
        let manifest = make_manifest_with_renderers("ludflow", renderers_map, "ludflow__");

        let cached_tools = vec![
            serde_json::json!({
                "name": "ludflow__search_codebase",
                "description": "Search the codebase"
            }),
            serde_json::json!({
                "name": "ludflow__vector_search",
                "description": "Vector search"
            }),
        ];

        let known = std::collections::HashSet::new();
        let result = synthesize_renderer_defs(&manifest, Some(&cached_tools), &known);

        assert_eq!(result.len(), 1);
        let def = &result[0];
        assert_eq!(def.name, "search_results");
        assert_eq!(def.tools.len(), 2);
        assert!(def.tools.contains(&"search_codebase".to_string()));
        assert!(def.tools.contains(&"vector_search".to_string()));
    }
}
