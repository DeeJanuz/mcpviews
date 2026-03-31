use mcpviews_shared::RendererDef;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use tauri::Emitter;

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
        "mcpviews_install_plugin" => call_install_plugin(arguments, state).await,
        "get_plugin_docs" => call_get_plugin_docs(arguments, state).await,
        "get_plugin_prompt" => crate::mcp_prompts::call_get_plugin_prompt(arguments, state).await,
        "update_plugins" => call_update_plugins(arguments, state).await,
        "list_registry" => crate::mcp_registry_tools::call_list_registry(arguments, state).await,
        "start_plugin_auth" => crate::mcp_registry_tools::call_start_plugin_auth(arguments, state).await,
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

/// Ensure the registry cache is populated. If empty, fetch from all sources
/// and resolve remote manifests. Errors are logged but swallowed (best-effort).
pub(crate) async fn ensure_registry_fresh(state: &Arc<TokioMutex<AsyncAppState>>) {
    let is_empty = {
        let state_guard = state.lock().await;
        let empty = state_guard.inner.latest_registry.lock().unwrap().is_empty();
        empty
    };

    if !is_empty {
        return;
    }

    let client = {
        let state_guard = state.lock().await;
        state_guard.inner.http_client.clone()
    };

    let sources = mcpviews_shared::registry::get_registry_sources();
    // fetch_all_registries already calls resolve_manifest_urls internally
    match mcpviews_shared::registry::fetch_all_registries(&client, &sources).await {
        Ok(entries) => {
            let state_guard = state.lock().await;
            let mut cached = state_guard.inner.latest_registry.lock().unwrap();
            *cached = entries;
        }
        Err(e) => {
            eprintln!("[mcpviews] ensure_registry_fresh failed: {}", e);
        }
    }
}

// ─── Built-in tool implementations ───

/// Remove `change` fields from structured_data payloads so the read-only view
/// never displays diff markers even if the caller accidentally includes them.
fn strip_change_fields(data: &mut Value) {
    if let Some(tables) = data.get_mut("tables").and_then(|t| t.as_array_mut()) {
        for table in tables {
            // Strip column-level change
            if let Some(columns) = table.get_mut("columns").and_then(|c| c.as_array_mut()) {
                for col in columns {
                    if let Some(obj) = col.as_object_mut() {
                        obj.insert("change".into(), Value::Null);
                    }
                }
            }
            // Strip cell-level change (recursive for nested rows)
            if let Some(rows) = table.get_mut("rows").and_then(|r| r.as_array_mut()) {
                strip_row_changes(rows);
            }
        }
    }
}

fn strip_row_changes(rows: &mut Vec<Value>) {
    for row in rows {
        if let Some(cells) = row.get_mut("cells").and_then(|c| c.as_object_mut()) {
            for (_key, cell) in cells.iter_mut() {
                if let Some(obj) = cell.as_object_mut() {
                    obj.insert("change".into(), Value::Null);
                }
            }
        }
        // Recurse into children
        if let Some(children) = row.get_mut("children").and_then(|c| c.as_array_mut()) {
            strip_row_changes(children);
        }
    }
}

async fn call_push_content(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    let mut arguments = arguments;
    // Strip change markers from structured_data — read-only view should never show diffs
    if let Some(data) = arguments.get_mut("data") {
        strip_change_fields(data);
    }
    call_push_impl(arguments, state, false).await
}

async fn call_push_review(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    call_push_impl(arguments, state, true).await
}

/// Normalize a data parameter: if it's a JSON string, parse it into an object.
/// Falls back to the original value if parsing fails.
fn normalize_data_param(raw: &Value) -> Value {
    if let Some(s) = raw.as_str() {
        serde_json::from_str(s).unwrap_or_else(|_| raw.clone())
    } else {
        raw.clone()
    }
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
    let data = {
        let raw = arguments
            .get("data")
            .ok_or("Missing required parameter: data")?;
        normalize_data_param(raw)
    };
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

    // Cross-cutting renderer selection rule
    rules.push(serde_json::json!({
        "name": "renderer_selection",
        "category": "system",
        "source": "built-in",
        "rule": "When displaying content in MCPViews, choose the renderer based on data shape:\n\n- **rich_content**: Prose, explanations, diagrams (mermaid), code blocks, simple markdown tables (<10 rows). Default choice.\n- **structured_data**: Tabular data with sort/filter/expand needs, hierarchical rows, or proposed changes requiring accept/reject review. Use push_review for change approval workflows.\n- **Plugin renderers**: If a plugin provides a domain-specific renderer (e.g., search_results), prefer it over generic renderers for that plugin's tool output.\n\nWhen uncertain, default to rich_content. Only use structured_data when the data is genuinely tabular with columns and rows."
    }));

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
                format!("{}{}", tool_prefix, tool_name)
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

/// Collect only built-in (universal) rules — renderer_selection + universal renderer rules.
pub(crate) fn collect_builtin_rules(all_renderers: &[RendererDef]) -> Vec<Value> {
    let mut rules: Vec<Value> = Vec::new();

    // Cross-cutting renderer selection rule
    rules.push(serde_json::json!({
        "name": "renderer_selection",
        "category": "system",
        "source": "built-in",
        "rule": "When displaying content in MCPViews, choose the renderer based on data shape:\n\n- **rich_content**: Prose, explanations, diagrams (mermaid), code blocks, simple markdown tables (<10 rows). Default choice.\n- **structured_data**: Tabular data with sort/filter/expand needs, hierarchical rows, or proposed changes requiring accept/reject review. Use push_review for change approval workflows.\n- **Plugin renderers**: If a plugin provides a domain-specific renderer (e.g., search_results), prefer it over generic renderers for that plugin's tool output.\n\nWhen uncertain, default to rich_content. Only use structured_data when the data is genuinely tabular with columns and rows."
    }));

    // Only built-in (universal scope) renderers with rules
    for renderer in all_renderers {
        if renderer.scope == "universal" {
            if let Some(rule) = &renderer.rule {
                rules.push(serde_json::json!({
                    "name": format!("{}_usage", renderer.name),
                    "category": "renderer",
                    "source": "built-in",
                    "renderer": renderer.name,
                    "description": renderer.description,
                    "scope": renderer.scope,
                    "data_hint": renderer.data_hint,
                    "tools": renderer.tools,
                    "rule": rule,
                }));
            }
        }
    }

    rules
}

/// Collect rules for a single plugin, optionally filtered by tool names and/or renderer names.
pub(crate) fn collect_plugin_rules(
    all_renderers: &[RendererDef],
    manifest: &mcpviews_shared::PluginManifest,
    tool_filter: Option<&[String]>,
    renderer_filter: Option<&[String]>,
) -> Vec<Value> {
    let mut rules: Vec<Value> = Vec::new();

    let tool_prefix = manifest
        .mcp
        .as_ref()
        .map(|m| m.tool_prefix.as_str())
        .unwrap_or("");

    // Determine which renderers are associated with filtered tools
    let mut relevant_renderers: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Some(tools) = tool_filter {
        for tool_name in tools {
            if let Some(renderer_name) = manifest.renderers.get(tool_name) {
                relevant_renderers.insert(renderer_name.clone());
            }
        }
    }
    if let Some(renderers) = renderer_filter {
        for r in renderers {
            relevant_renderers.insert(r.clone());
        }
    }

    let has_filter = tool_filter.is_some() || renderer_filter.is_some();

    // Renderer rules — only non-universal (plugin) renderers
    for renderer in all_renderers {
        if renderer.scope == "universal" {
            continue;
        }

        // If filters are active, only include matching renderers
        if has_filter && !relevant_renderers.contains(&renderer.name) {
            continue;
        }

        if let Some(rule) = &renderer.rule {
            rules.push(serde_json::json!({
                "name": format!("{}_usage", renderer.name),
                "category": "renderer",
                "source": "plugin",
                "renderer": renderer.name,
                "description": renderer.description,
                "scope": renderer.scope,
                "data_hint": renderer.data_hint,
                "tools": renderer.tools,
                "rule": rule,
            }));
        } else if renderer.scope == "tool" && !renderer.tools.is_empty() {
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
    for (tool_name, rule) in &manifest.tool_rules {
        // If tools filter is active, only include matching tools
        if let Some(tools) = tool_filter {
            if !tools.iter().any(|t| t == tool_name) {
                continue;
            }
        }

        let prefixed_name = if tool_prefix.is_empty() {
            tool_name.clone()
        } else {
            format!("{}{}", tool_prefix, tool_name)
        };
        rules.push(serde_json::json!({
            "name": format!("{}_usage", prefixed_name),
            "category": "tool",
            "source": &manifest.name,
            "tool": prefixed_name,
            "rule": rule,
        }));
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

fn auto_derive_registry_index(
    manifest: &mcpviews_shared::PluginManifest,
    cached_tools: Option<&[serde_json::Value]>,
) -> mcpviews_shared::PluginRegistryIndex {
    let prefix = manifest
        .mcp
        .as_ref()
        .map(|m| m.tool_prefix.as_str())
        .unwrap_or("");

    // Group tools by renderer name
    let mut renderer_tools: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    let mut ungrouped_tools: Vec<&str> = Vec::new();

    // Track which tools are mapped to renderers
    let mapped_tools: std::collections::HashSet<&str> = manifest.renderers.keys().map(|s| s.as_str()).collect();

    for (tool_name, renderer_name) in &manifest.renderers {
        renderer_tools
            .entry(renderer_name.as_str())
            .or_default()
            .push(tool_name.as_str());
    }

    // Find unmapped tools from cache
    if let Some(tools) = cached_tools {
        for tool in tools {
            if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                let unprefixed = if !prefix.is_empty() {
                    name.strip_prefix(prefix).unwrap_or(name)
                } else {
                    name
                };
                if !mapped_tools.contains(unprefixed) {
                    ungrouped_tools.push(unprefixed);
                }
            }
        }
    }

    let mut tool_groups: Vec<mcpviews_shared::ToolGroupEntry> = Vec::new();

    for (renderer_name, tool_names) in &renderer_tools {
        // Get a hint from the first tool's description
        let hint = if let Some(tools) = cached_tools {
            let prefixed = format!("{}{}", prefix, tool_names[0]);
            tools.iter()
                .find(|t| t.get("name").and_then(|n| n.as_str()) == Some(&prefixed))
                .and_then(|t| t.get("description").and_then(|d| d.as_str()))
                .map(|d| {
                    let truncated: String = d.chars().take(80).collect();
                    if d.len() > 80 { format!("{}...", truncated) } else { truncated }
                })
                .unwrap_or_else(|| format!("Tools for {}", renderer_name))
        } else {
            format!("Tools for {}", renderer_name)
        };

        // Title-case the renderer name
        let name = renderer_name
            .split('_')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().to_string() + c.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        tool_groups.push(mcpviews_shared::ToolGroupEntry {
            name,
            hint,
            tools: tool_names.iter().map(|s| s.to_string()).collect(),
        });
    }

    // Add ungrouped tools if any
    if !ungrouped_tools.is_empty() {
        tool_groups.push(mcpviews_shared::ToolGroupEntry {
            name: "Other".to_string(),
            hint: "Additional tools".to_string(),
            tools: ungrouped_tools.iter().map(|s| s.to_string()).collect(),
        });
    }

    let renderer_names: Vec<String> = renderer_tools.keys().map(|s| s.to_string()).collect();
    let tags: Vec<String> = renderer_names.iter().map(|r| r.replace('_', "-")).collect();

    mcpviews_shared::PluginRegistryIndex {
        summary: format!("{} plugin", manifest.name),
        tags,
        tool_groups,
        renderer_names,
    }
}

fn build_plugin_registry(
    manifests: &[mcpviews_shared::PluginManifest],
    tool_cache: &crate::tool_cache::ToolCache,
) -> Vec<Value> {
    manifests.iter().enumerate().map(|(idx, manifest)| {
        let index = match &manifest.registry_index {
            Some(ri) => ri.clone(),
            None => {
                let cached_tools = tool_cache.plugin_tools(idx);
                auto_derive_registry_index(manifest, cached_tools)
            }
        };

        serde_json::json!({
            "name": manifest.name,
            "summary": index.summary,
            "tags": index.tags,
            "tool_groups": index.tool_groups.iter().map(|g| serde_json::json!({
                "name": g.name,
                "hint": g.hint,
                "tools": g.tools,
            })).collect::<Vec<Value>>(),
            "renderers": index.renderer_names,
            "prompts": manifest.prompt_definitions.iter().map(|p| serde_json::json!({
                "name": p.name,
                "description": p.description,
                "arguments": p.arguments,
            })).collect::<Vec<Value>>(),
        })
    }).collect()
}

/// Collect plugin updates by comparing installed versions against registry versions.
fn collect_plugin_updates(
    manifests: &[mcpviews_shared::PluginManifest],
    registry_entries: &[mcpviews_shared::RegistryEntry],
) -> Vec<Value> {
    manifests
        .iter()
        .filter_map(|manifest| {
            let entry = registry_entries.iter().find(|e| e.name == manifest.name)?;
            let new_ver = mcpviews_shared::newer_version(&manifest.version, &entry.version)?;
            Some(serde_json::json!({
                "name": manifest.name,
                "installed_version": manifest.version,
                "available_version": new_ver,
            }))
        })
        .collect()
}

async fn gather_slim_session_data(state: &Arc<TokioMutex<AsyncAppState>>) -> (Vec<Value>, Vec<Value>, Vec<Value>, Vec<Value>) {
    ensure_registry_fresh(state).await;

    let state_guard = state.lock().await;
    let all_renderers = available_renderers(&state_guard.inner);
    let registry = state_guard.inner.plugin_registry.lock().unwrap();
    let cached_registry = state_guard.inner.latest_registry.lock().unwrap();
    let rules = collect_builtin_rules(&all_renderers);
    let plugin_status = collect_plugin_auth_status(&registry.manifests);
    let plugin_registry = build_plugin_registry(&registry.manifests, &registry.tool_cache);
    let plugin_updates = collect_plugin_updates(&registry.manifests, &cached_registry);
    (rules, plugin_status, plugin_registry, plugin_updates)
}

async fn call_init_session(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    let agent_type = arguments
        .get("agent_type")
        .and_then(|v| v.as_str())
        .unwrap_or("generic");

    let (rules, plugin_status, plugin_registry, plugin_updates) = gather_slim_session_data(state).await;

    let response = serde_json::json!({
        "rules": rules,
        "plugin_status": plugin_status,
        "persistence_instructions": persistence_instructions(agent_type),
        "plugin_registry": plugin_registry,
        "plugin_updates": plugin_updates,
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

async fn call_get_plugin_docs(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    let plugin_name = arguments
        .get("plugin")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: plugin")?;

    let groups_filter: Option<Vec<String>> = arguments
        .get("groups")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());

    let tools_filter: Option<Vec<String>> = arguments
        .get("tools")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());

    let renderers_filter: Option<Vec<String>> = arguments
        .get("renderers")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());

    let state_guard = state.lock().await;
    let all_renderers = available_renderers(&state_guard.inner);
    let registry = state_guard.inner.plugin_registry.lock().unwrap();

    let (_, manifest) = registry
        .find_plugin_by_name(plugin_name)
        .ok_or_else(|| format!("Plugin '{}' not found", plugin_name))?;

    // Expand groups filter to tool names
    let mut expanded_tools: Vec<String> = Vec::new();
    if let Some(groups) = &groups_filter {
        if let Some(ri) = &manifest.registry_index {
            for group in &ri.tool_groups {
                if groups.iter().any(|g| g.eq_ignore_ascii_case(&group.name)) {
                    expanded_tools.extend(group.tools.clone());
                }
            }
        }
        // Also try auto-derived index for manifests without registry_index
        if manifest.registry_index.is_none() {
            let cached_tools = registry.tool_cache.plugin_tools(
                registry.manifests.iter().position(|m| m.name == plugin_name).unwrap_or(0)
            );
            let derived = auto_derive_registry_index(manifest, cached_tools);
            for group in &derived.tool_groups {
                if groups.iter().any(|g| g.eq_ignore_ascii_case(&group.name)) {
                    expanded_tools.extend(group.tools.clone());
                }
            }
        }
    }

    // Merge expanded group tools with explicit tool filter
    let final_tool_filter = if expanded_tools.is_empty() {
        tools_filter.as_deref()
    } else {
        if let Some(extra) = &tools_filter {
            expanded_tools.extend(extra.clone());
        }
        Some(expanded_tools.as_slice())
    };

    let rules = collect_plugin_rules(
        &all_renderers,
        manifest,
        final_tool_filter,
        renderers_filter.as_deref(),
    );

    let response = serde_json::json!({
        "plugin": plugin_name,
        "rules": rules,
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

const RICH_CONTENT_RULE: &str = r#"CALLER RESTRICTION: ONLY the main/coordinator agent may call push_content, push_review, and push_check. Sub-agents must NEVER call these — return results to the coordinator.

When to push: detailed explanations, plans, architecture/data-flow diagrams, API designs, database schemas. Keep chat concise; rich detail goes to push_content.

## `data` parameter

`data` MUST be a JSON **object**, not a stringified JSON string.
Correct: `"data": { "title": "...", "body": "..." }`
Wrong:   `"data": "{\"title\": \"...\"}"`

## Formatting the `body` field

Body is markdown (CommonMark). Supported: headings, bold/italic, lists, blockquotes, fenced code blocks, markdown tables (<10 rows; use structured_data for more), horizontal rules.

### Mermaid diagrams

MUST be wrapped in a fenced code block with language identifier `mermaid`. Bare `mermaid` without triple-backtick fences renders as plain text — this is the most common mistake.

In the JSON string value for body, a mermaid block looks like:
`"```mermaid\\nflowchart TD\\n  A[Start] --> B[End]\\n```"`

**Line breaks in node labels**: use `<br/>` tags. Never use `\\n` or literal newlines inside node text.
Correct: `A[Line one<br/>Line two]`
Wrong:   `A[Line one\nLine two]`

**Special characters in node text**: wrap node labels in quotes if they contain parentheses, brackets, or other Mermaid syntax characters.

### JSON string escaping

The body value is a JSON string. Use `\n` for newlines, `\"` for quotes, `\\` for backslashes. Backticks need no escaping."#;

const STRUCTURED_DATA_RULE: &str = r#"Use structured_data when presenting tabular or schema data that benefits from sort, filter, expand/collapse, or review workflows. Prefer it over rich_content markdown tables when:
- Data has hierarchical/nested rows (parent-child relationships)
- Users need to sort or filter interactively
- Data represents proposed changes that need accept/reject review
- Tables have many rows (>10) where scrolling + filtering helps

Use rich_content with markdown tables for simple, small, static tables.

## push_content (read-only display)

Display-only mode. Change markers are automatically stripped by the server and ignored by the renderer. Set all `change` fields to null.

Example:
```json
{
  "tool_name": "structured_data",
  "data": {
    "title": "Server Inventory",
    "tables": [{
      "id": "t1",
      "name": "Production Servers",
      "columns": [
        { "id": "name", "name": "Name", "change": null },
        { "id": "type", "name": "Type", "change": null },
        { "id": "status", "name": "Status", "change": null }
      ],
      "rows": [
        {
          "id": "r1",
          "cells": {
            "name": { "value": "api-01", "change": null },
            "type": { "value": "m5.xlarge", "change": null },
            "status": { "value": "Running", "change": null }
          },
          "children": []
        }
      ]
    }]
  }
}
```

## push_review (change review mode)

Shows proposed changes with color-coded diffs. Users can accept/reject individual rows and columns, edit cell values, then submit. Use `change` fields to mark what was added, deleted, or updated.

Change values: "add" (green), "delete" (red strikethrough), "update" (yellow), null (unchanged).

Example:
```json
{
  "tool_name": "structured_data",
  "data": {
    "title": "Schema Migration Review",
    "tables": [{
      "id": "t1",
      "name": "users",
      "columns": [
        { "id": "col", "name": "Column", "change": null },
        { "id": "type", "name": "Type", "change": null },
        { "id": "new_col", "name": "MFA Provider", "change": "add" }
      ],
      "rows": [
        {
          "id": "r1",
          "cells": {
            "col": { "value": "display_name", "change": "add" },
            "type": { "value": "varchar(100)", "change": "add" },
            "new_col": { "value": null, "change": null }
          },
          "children": []
        }
      ]
    }]
  },
  "timeout": 300
}
```

push_review response contains user decisions:
```json
{
  "sessionId": "uuid",
  "status": "decision_received",
  "decision": "partial",
  "operationDecisions": { "r1": "accept", "col:new_col": "reject" },
  "modifications": { "r1.type": "{\"value\":\"text\",\"user_edited\":true}" },
  "additions": { "user_edits": { "r1.type": "text" } }
}
```

## Data shape reference

- `tables[]`: Array of table objects, each with `id`, `name`, `columns[]`, `rows[]`
- `columns[]`: `{ id, name, change }` — change is null for read-only, "add"/"delete" for review
- `rows[]`: `{ id, cells, children }` — cells is `{ [colId]: { value, change } }`, children enables arbitrary nesting
- Nested rows auto-expand to depth 2; deeper rows start collapsed"#;

fn builtin_renderer_definitions() -> Vec<RendererDef> {
    vec![
        RendererDef {
            name: "rich_content".into(),
            description: "Universal markdown display with mermaid diagrams, tables, code blocks, and citations. Use for any rich text content.".into(),
            scope: "universal".into(),
            tools: vec![],
            data_hint: Some("{ \"title\": \"Optional heading\", \"body\": \"Markdown with ```mermaid blocks\" } — data must be a JSON object, not a string".into()),
            rule: Some(RICH_CONTENT_RULE.into()),
            display_mode: None,
            invoke_schema: None,
            url_patterns: vec![],
        },
        RendererDef {
            name: "structured_data".into(),
            description: "Tabular data with hierarchical rows, change tracking, sort/filter, and review mode with per-row/column accept/reject and cell editing.".into(),
            scope: "universal".into(),
            tools: vec![],
            data_hint: Some(r#"{ "title": "Optional", "tables": [{ "id": "t1", "name": "Name", "columns": [{ "id": "c1", "name": "Col", "change": null|"add"|"delete" }], "rows": [{ "id": "r1", "cells": { "c1": { "value": "v", "change": null|"add"|"delete"|"update" } }, "children": [] }] }] }"#.into()),
            display_mode: None,
            invoke_schema: None,
            url_patterns: vec![],
            rule: Some(STRUCTURED_DATA_RULE.into()),
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
            display_mode: None,
            invoke_schema: None,
            url_patterns: vec![],
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

async fn call_install_plugin(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    let manifest_json = arguments
        .get("manifest_json")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: manifest_json")?;

    let download_url = arguments
        .get("download_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let manifest = if let Some(url) = &download_url {
        let (client, plugins_dir) = {
            let state_guard = state.lock().await;
            (
                state_guard.inner.http_client.clone(),
                state_guard.inner.plugins_dir().to_path_buf(),
            )
        };
        mcpviews_shared::package::download_and_install_plugin(&client, url, &plugins_dir).await?
    } else {
        serde_json::from_str::<mcpviews_shared::PluginManifest>(manifest_json)
            .map_err(|e| format!("Invalid manifest JSON: {}", e))?
    };

    let plugin_name = {
        let state_guard = state.lock().await;
        state_guard.inner.install_plugin_from_manifest(manifest, download_url.is_some())?
    };

    // Notify MCP clients and GUI
    {
        let state_guard = state.lock().await;
        state_guard.inner.notify_tools_changed();
        let _ = state_guard.app_handle.emit("reload_renderers", ());
    }

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": format!("Plugin '{}' installed successfully.", plugin_name)
        }]
    }))
}

async fn call_update_plugins(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    let plugin_name = arguments
        .get("plugin_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Ensure registry is fresh
    ensure_registry_fresh(state).await;

    // Identify plugins needing updates
    let updates_needed: Vec<(String, String, mcpviews_shared::RegistryEntry)> = {
        let state_guard = state.lock().await;
        let registry = state_guard.inner.plugin_registry.lock().unwrap();
        let cached = state_guard.inner.latest_registry.lock().unwrap();

        let plugins_with_updates = registry.list_plugins_with_updates(&cached);
        plugins_with_updates
            .iter()
            .filter(|p| p.update_available.is_some())
            .filter(|p| {
                if let Some(ref name) = plugin_name {
                    p.name == *name
                } else {
                    true
                }
            })
            .filter_map(|p| {
                let entry = cached.iter().find(|e| e.name == p.name)?.clone();
                Some((p.name.clone(), p.version.clone(), entry))
            })
            .collect()
    };

    if updates_needed.is_empty() {
        return Ok(serde_json::json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string(&serde_json::json!({
                    "updated": []
                })).unwrap()
            }]
        }));
    }

    let mut results: Vec<Value> = Vec::new();

    for (name, from_version, entry) in &updates_needed {
        let install_result = {
            let state_guard = state.lock().await;
            state_guard.inner.install_or_update_from_entry(entry).await
        };

        match install_result {
            Ok(()) => {
                results.push(serde_json::json!({
                    "plugin": name,
                    "from": from_version,
                    "to": entry.version,
                    "status": "success",
                }));
            }
            Err(e) => {
                results.push(serde_json::json!({
                    "plugin": name,
                    "from": from_version,
                    "to": entry.version,
                    "status": "error",
                    "error": e,
                }));
            }
        }
    }

    // Notify MCP clients and GUI
    {
        let state_guard = state.lock().await;
        state_guard.inner.notify_tools_changed();
        let _ = state_guard.app_handle.emit("reload_renderers", ());
    }

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string(&serde_json::json!({
                "updated": results
            })).unwrap()
        }]
    }))
}


// ─── Tool definitions ───

fn build_data_description(renderers: &[RendererDef], prefix: &str) -> String {
    let hints = renderers.iter()
        .filter(|r| r.scope == "universal")
        .filter_map(|r| r.data_hint.as_ref().map(|h| format!("For {}: {}", r.name, h)))
        .collect::<Vec<_>>()
        .join(". ");
    format!("{} {} For plugin renderer data shapes, call get_plugin_docs.", prefix, hints)
}

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
                        "description": build_data_description(renderers, "Content payload — shape depends on tool_name.")
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
                        "description": build_data_description(renderers, "Content payload for review display — shape depends on tool_name.")
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
        serde_json::json!({
            "name": "mcpviews_install_plugin",
            "description": "Install a plugin into MCPViews. Provide a plugin manifest as JSON, and optionally a download URL for a .zip package containing renderer assets.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "manifest_json": {
                        "type": "string",
                        "description": "JSON string of a PluginManifest object defining the plugin's name, version, renderers, MCP config, and tool rules."
                    },
                    "download_url": {
                        "type": "string",
                        "description": "Optional URL to a .zip package to download and install. If provided, the manifest is extracted from the package and the manifest_json parameter is not used."
                    }
                },
                "required": ["manifest_json"]
            }
        }),
        serde_json::json!({
            "name": "get_plugin_docs",
            "description": "Fetch detailed usage docs for a plugin's tools and renderers. Call after init_session identifies which plugin you need.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "plugin": {
                        "type": "string",
                        "description": "Plugin name (e.g., 'ludflow', 'decidr')"
                    },
                    "groups": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional: specific tool group names to fetch (e.g., ['Search', 'Code Analysis'])"
                    },
                    "tools": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional: specific tool names to fetch (unprefixed, e.g., ['search_codebase'])"
                    },
                    "renderers": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional: specific renderer names to fetch (e.g., ['code_units', 'search_results'])"
                    }
                },
                "required": ["plugin"]
            }
        }),
        serde_json::json!({
            "name": "update_plugins",
            "description": "Update installed plugins to their latest versions from the registry. Uses remote manifest resolution to discover available updates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "plugin_name": {
                        "type": "string",
                        "description": "Specific plugin to update. If omitted, updates all plugins with available updates."
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "get_plugin_prompt",
            "description": "Fetch a prompt from a plugin. Returns the prompt content that should be used as system instructions for a guided workflow.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "plugin": { "type": "string", "description": "Plugin name" },
                    "prompt": { "type": "string", "description": "Prompt name" },
                    "arguments": {
                        "type": "object",
                        "description": "Optional arguments to template into the prompt",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["plugin", "prompt"]
            }
        }),
        serde_json::json!({
            "name": "list_registry",
            "description": "List all available plugins from the MCPViews registry, including install status, auth status, and available updates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tag": { "type": "string", "description": "Optional: filter plugins by tag" }
                }
            }
        }),
        serde_json::json!({
            "name": "start_plugin_auth",
            "description": "Start authentication for an installed plugin. Opens browser for OAuth, or checks env var for Bearer/ApiKey.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "plugin_name": { "type": "string", "description": "Name of the plugin to authenticate" }
                },
                "required": ["plugin_name"]
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
            registry_index: None,
            download_url: None,
            prompt_definitions: vec![],
        }
    }

    // ─── collect_rules tests ───

    #[test]
    fn test_collect_rules_includes_renderer_selection() {
        let rules = collect_rules(&[], &[]);
        assert_eq!(rules.len(), 1);
        let sel = rules.iter().find(|r| r["name"] == "renderer_selection").expect("renderer_selection rule should exist");
        assert_eq!(sel["category"], "system");
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
            display_mode: None,
            invoke_schema: None,
            url_patterns: vec![],
        }];
        let rules = collect_rules(&renderers, &[]);
        assert_eq!(rules.len(), 2);
        let sel = rules.iter().find(|r| r["name"] == "renderer_selection").expect("renderer_selection rule should exist");
        assert_eq!(sel["category"], "system");

        let rc = rules.iter().find(|r| r["name"] == "rich_content_usage").expect("rich_content_usage rule should exist");
        assert_eq!(rc["category"], "renderer");
        assert_eq!(rc["source"], "built-in");
        assert_eq!(rc["renderer"], "rich_content");
        assert_eq!(rc["rule"], "Always use rich_content for plans.");
        assert_eq!(rc["description"], "Universal markdown display");
        assert_eq!(rc["scope"], "universal");
        assert_eq!(rc["data_hint"], r#"{ "title": "heading", "body": "markdown" }"#);
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
            display_mode: None,
            invoke_schema: None,
            url_patterns: vec![],
        }];
        let rules = collect_rules(&renderers, &[]);
        // Only the renderer_selection rule, no renderer-specific rule
        assert_eq!(rules.len(), 1);
        let sel = rules.iter().find(|r| r["name"] == "renderer_selection").expect("renderer_selection rule should exist");
        assert_eq!(sel["category"], "system");
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
            display_mode: None,
            invoke_schema: None,
            url_patterns: vec![],
        }];
        let rules = collect_rules(&renderers, &[]);
        assert_eq!(rules.len(), 2);
        let cv = rules.iter().find(|r| r["renderer"] == "custom_view").expect("custom_view rule should exist");
        assert_eq!(cv["source"], "plugin");
        assert_eq!(cv["description"], "Custom");
        assert_eq!(cv["scope"], "tool");
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
            display_mode: None,
            invoke_schema: None,
            url_patterns: vec![],
        }];
        let rules = collect_rules(&renderers, &[]);
        assert_eq!(rules.len(), 2);
        let sr = rules.iter().find(|r| r["renderer"] == "search_results").expect("search_results rule should exist");
        assert_eq!(sr["category"], "renderer");
        assert_eq!(sr["source"], "plugin");
        assert_eq!(sr["tools"][0], "search_codebase");
        assert_eq!(sr["scope"], "tool");
        assert_eq!(sr["description"], "Renders search output");
        assert_eq!(sr["data_hint"], "Pass search results");
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
        assert_eq!(rules.len(), 2);
        let tr = rules.iter().find(|r| r["name"] == "sp__search_usage").expect("sp__search_usage rule should exist");
        assert_eq!(tr["category"], "tool");
        assert_eq!(tr["tool"], "sp__search");
        assert_eq!(tr["source"], "search-plugin");
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
        assert_eq!(rules.len(), 2);
        let tr = rules.iter().find(|r| r["tool"] == "do_thing").expect("do_thing rule should exist");
        assert_eq!(tr["name"], "do_thing_usage");
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
            registry_index: None,
            download_url: None,
            prompt_definitions: vec![],
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

    // ─── install_plugin_from_manifest tests ───

    #[test]
    fn test_install_plugin_manifest_only() {
        let (state, _dir) = crate::test_utils::test_app_state();
        let manifest = crate::test_utils::test_manifest("test-install");

        let result = state.install_plugin_from_manifest(manifest, false);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test-install");

        let registry = state.plugin_registry.lock().unwrap();
        assert_eq!(registry.manifests.len(), 1);
        assert_eq!(registry.manifests[0].name, "test-install");
    }

    #[test]
    fn test_install_plugin_invalid_manifest_json() {
        // Verify that serde_json rejects invalid JSON before it reaches install_plugin_from_manifest
        let bad_json = "{ not valid json }";
        let result = serde_json::from_str::<mcpviews_shared::PluginManifest>(bad_json);
        assert!(result.is_err());
    }

    #[test]
    fn test_install_plugin_upsert_replaces_existing() {
        let (state, _dir) = crate::test_utils::test_app_state();
        let manifest_v1 = crate::test_utils::test_manifest("upsert-plugin");

        state.install_plugin_from_manifest(manifest_v1, false).unwrap();
        {
            let registry = state.plugin_registry.lock().unwrap();
            assert_eq!(registry.manifests.len(), 1);
        }

        let mut manifest_v2 = crate::test_utils::test_manifest("upsert-plugin");
        manifest_v2.version = "2.0.0".to_string();
        state.install_plugin_from_manifest(manifest_v2, false).unwrap();

        let registry = state.plugin_registry.lock().unwrap();
        assert_eq!(registry.manifests.len(), 1);
        assert_eq!(registry.manifests[0].name, "upsert-plugin");
        assert_eq!(registry.manifests[0].version, "2.0.0");
    }

    #[test]
    fn test_install_plugin_missing_manifest_json_param() {
        // Simulates the extraction logic in call_install_plugin: missing manifest_json → error
        let arguments = serde_json::json!({});
        let result = arguments
            .get("manifest_json")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: manifest_json");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Missing required parameter: manifest_json");
    }

    // ─── schema description tests ───

    #[test]
    fn test_install_plugin_schema_download_url_description() {
        let tools = builtin_tool_definitions(&[]);
        let install_tool = tools.iter()
            .find(|t| t["name"] == "mcpviews_install_plugin")
            .expect("mcpviews_install_plugin tool should exist");
        let desc = install_tool["inputSchema"]["properties"]["download_url"]["description"]
            .as_str()
            .unwrap();
        assert!(
            desc.contains("the manifest_json parameter is not used"),
            "Description should accurately reflect that manifest_json is not used when download_url is provided. Got: {}",
            desc,
        );
        assert!(
            !desc.contains("still required for validation"),
            "Description should not claim manifest_json is required for validation. Got: {}",
            desc,
        );
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

    // ─── collect_builtin_rules tests ───

    #[test]
    fn test_collect_builtin_rules_includes_renderer_selection() {
        let rules = collect_builtin_rules(&[]);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["name"], "renderer_selection");
    }

    #[test]
    fn test_collect_builtin_rules_includes_universal_renderers_only() {
        let renderers = vec![
            RendererDef {
                name: "rich_content".into(),
                description: "Universal markdown".into(),
                scope: "universal".into(),
                tools: vec![],
                data_hint: Some("{ title, body }".into()),
                rule: Some("Use for prose.".into()),
                display_mode: None,
                invoke_schema: None,
                url_patterns: vec![],
            },
            RendererDef {
                name: "search_results".into(),
                description: "Search output".into(),
                scope: "tool".into(),
                tools: vec!["search_codebase".into()],
                data_hint: Some("Pass search results".into()),
                rule: Some("Use for search output.".into()),
                display_mode: None,
                invoke_schema: None,
                url_patterns: vec![],
            },
        ];
        let rules = collect_builtin_rules(&renderers);
        // renderer_selection + rich_content_usage, but NOT search_results
        assert_eq!(rules.len(), 2);
        assert!(rules.iter().any(|r| r["name"] == "rich_content_usage"));
        assert!(!rules.iter().any(|r| r["name"] == "search_results_usage"));
    }

    // ─── collect_plugin_rules tests ───

    #[test]
    fn test_collect_plugin_rules_unfiltered() {
        let renderers = vec![RendererDef {
            name: "search_results".into(),
            description: "Search output".into(),
            scope: "tool".into(),
            tools: vec!["search_codebase".into()],
            data_hint: Some("Pass search results".into()),
            rule: None,
            display_mode: None,
            invoke_schema: None,
            url_patterns: vec![],
        }];
        let mut tool_rules = std::collections::HashMap::new();
        tool_rules.insert("search_codebase".to_string(), "Use search for queries.".to_string());
        let manifest = make_manifest(
            "test-plugin",
            vec![],
            tool_rules,
            Some(PluginMcpConfig {
                url: "http://localhost:8080".into(),
                auth: None,
                tool_prefix: "tp".into(),
            }),
        );
        let rules = collect_plugin_rules(&renderers, &manifest, None, None);
        // search_results renderer + search_codebase tool rule
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn test_collect_plugin_rules_filtered_by_renderer() {
        let renderers = vec![
            RendererDef {
                name: "search_results".into(),
                description: "Search".into(),
                scope: "tool".into(),
                tools: vec!["search_codebase".into()],
                data_hint: None,
                rule: None,
                display_mode: None,
                invoke_schema: None,
                url_patterns: vec![],
            },
            RendererDef {
                name: "code_units".into(),
                description: "Code".into(),
                scope: "tool".into(),
                tools: vec!["get_code_units".into()],
                data_hint: None,
                rule: None,
                display_mode: None,
                invoke_schema: None,
                url_patterns: vec![],
            },
        ];
        let manifest = make_manifest("test-plugin", vec![], std::collections::HashMap::new(), None);
        let renderer_filter = vec!["search_results".to_string()];
        let rules = collect_plugin_rules(&renderers, &manifest, None, Some(&renderer_filter));
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["renderer"], "search_results");
    }

    #[test]
    fn test_collect_plugin_rules_skips_universal() {
        let renderers = vec![RendererDef {
            name: "rich_content".into(),
            description: "Universal".into(),
            scope: "universal".into(),
            tools: vec![],
            data_hint: None,
            rule: Some("Use for prose.".into()),
            display_mode: None,
            invoke_schema: None,
            url_patterns: vec![],
        }];
        let manifest = make_manifest("test-plugin", vec![], std::collections::HashMap::new(), None);
        let rules = collect_plugin_rules(&renderers, &manifest, None, None);
        assert!(rules.is_empty());
    }

    // ─── auto_derive_registry_index tests ───

    #[test]
    fn test_auto_derive_registry_index_basic() {
        let mut renderers_map = std::collections::HashMap::new();
        renderers_map.insert("search_codebase".to_string(), "search_results".to_string());
        renderers_map.insert("get_code_units".to_string(), "code_units".to_string());
        let manifest = make_manifest_with_renderers("test-plugin", renderers_map, "tp__");
        let index = auto_derive_registry_index(&manifest, None);
        assert_eq!(index.summary, "test-plugin plugin");
        assert_eq!(index.tool_groups.len(), 2);
        assert!(index.renderer_names.contains(&"search_results".to_string()));
        assert!(index.renderer_names.contains(&"code_units".to_string()));
    }

    #[test]
    fn test_auto_derive_registry_index_with_cache() {
        let mut renderers_map = std::collections::HashMap::new();
        renderers_map.insert("search_codebase".to_string(), "search_results".to_string());
        let manifest = make_manifest_with_renderers("test-plugin", renderers_map, "tp__");
        let cached_tools = vec![serde_json::json!({
            "name": "tp__search_codebase",
            "description": "Search the codebase for matching code snippets"
        })];
        let index = auto_derive_registry_index(&manifest, Some(&cached_tools));
        let group = index.tool_groups.iter().find(|g| g.tools.contains(&"search_codebase".to_string())).unwrap();
        assert!(group.hint.contains("Search the codebase"));
    }

    // ─── build_data_description tests ───

    #[test]
    fn test_build_data_description_only_universal() {
        let renderers = vec![
            RendererDef {
                name: "rich_content".into(),
                description: "Universal".into(),
                scope: "universal".into(),
                tools: vec![],
                data_hint: Some("{ title, body }".into()),
                rule: None,
                display_mode: None,
                invoke_schema: None,
                url_patterns: vec![],
            },
            RendererDef {
                name: "search_results".into(),
                description: "Search".into(),
                scope: "tool".into(),
                tools: vec![],
                data_hint: Some("{ results: [...] }".into()),
                rule: None,
                display_mode: None,
                invoke_schema: None,
                url_patterns: vec![],
            },
        ];
        let desc = build_data_description(&renderers, "Payload.");
        assert!(desc.contains("rich_content"));
        assert!(!desc.contains("search_results"));
        assert!(desc.contains("get_plugin_docs"));
    }

    // ─── collect_plugin_updates tests ───

    #[test]
    fn test_collect_plugin_updates_no_updates() {
        let manifest = make_manifest("test-plugin", vec![], std::collections::HashMap::new(), None);
        let entry = mcpviews_shared::RegistryEntry {
            name: "test-plugin".to_string(),
            version: "1.0.0".to_string(),
            description: "Test".to_string(),
            author: None,
            homepage: None,
            manifest: manifest.clone(),
            tags: vec![],
            download_url: None,
            manifest_url: None,
        };
        let updates = collect_plugin_updates(&[manifest], &[entry]);
        assert!(updates.is_empty());
    }

    #[test]
    fn test_collect_plugin_updates_has_update() {
        let manifest = make_manifest("test-plugin", vec![], std::collections::HashMap::new(), None);
        let mut entry_manifest = manifest.clone();
        entry_manifest.version = "2.0.0".to_string();
        let entry = mcpviews_shared::RegistryEntry {
            name: "test-plugin".to_string(),
            version: "2.0.0".to_string(),
            description: "Test".to_string(),
            author: None,
            homepage: None,
            manifest: entry_manifest,
            tags: vec![],
            download_url: None,
            manifest_url: None,
        };
        let updates = collect_plugin_updates(&[manifest], &[entry]);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0]["name"], "test-plugin");
        assert_eq!(updates[0]["installed_version"], "1.0.0");
        assert_eq!(updates[0]["available_version"], "2.0.0");
    }

    #[test]
    fn test_collect_plugin_updates_older_registry_ignored() {
        let mut manifest = make_manifest("test-plugin", vec![], std::collections::HashMap::new(), None);
        manifest.version = "3.0.0".to_string();
        let entry = mcpviews_shared::RegistryEntry {
            name: "test-plugin".to_string(),
            version: "2.0.0".to_string(),
            description: "Test".to_string(),
            author: None,
            homepage: None,
            manifest: make_manifest("test-plugin", vec![], std::collections::HashMap::new(), None),
            tags: vec![],
            download_url: None,
            manifest_url: None,
        };
        let updates = collect_plugin_updates(&[manifest], &[entry]);
        assert!(updates.is_empty());
    }

    #[test]
    fn test_collect_plugin_updates_no_matching_entry() {
        let manifest = make_manifest("test-plugin", vec![], std::collections::HashMap::new(), None);
        let updates = collect_plugin_updates(&[manifest], &[]);
        assert!(updates.is_empty());
    }

    // ─── update_plugins tool definition test ───

    #[test]
    fn test_update_plugins_tool_defined() {
        let renderers = builtin_renderer_definitions();
        let tools = builtin_tool_definitions(&renderers);
        let update_tool = tools.iter().find(|t| t["name"] == "update_plugins");
        assert!(update_tool.is_some(), "update_plugins tool should be defined");
        let schema = &update_tool.unwrap()["inputSchema"];
        assert!(schema["properties"]["plugin_name"].is_object());
    }

    // ─── M-028: tool definition tests ───

    #[test]
    fn test_list_registry_tool_defined() {
        let tools = builtin_tool_definitions(&[]);
        let tool = tools.iter().find(|t| t["name"] == "list_registry");
        assert!(tool.is_some(), "list_registry tool should be defined");
    }

    #[test]
    fn test_start_plugin_auth_tool_defined() {
        let tools = builtin_tool_definitions(&[]);
        let tool = tools.iter().find(|t| t["name"] == "start_plugin_auth");
        assert!(tool.is_some(), "start_plugin_auth tool should be defined");
        let schema = &tool.unwrap()["inputSchema"];
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "plugin_name"));
    }

    #[test]
    fn test_get_plugin_prompt_tool_defined() {
        let tools = builtin_tool_definitions(&[]);
        let tool = tools.iter().find(|t| t["name"] == "get_plugin_prompt");
        assert!(tool.is_some(), "get_plugin_prompt tool should be defined");
    }

    #[test]
    fn test_normalize_data_param_object_passthrough() {
        let obj = serde_json::json!({"key": "value"});
        assert_eq!(normalize_data_param(&obj), obj);
    }

    #[test]
    fn test_normalize_data_param_valid_json_string() {
        let s = serde_json::json!("{\"key\": \"value\"}");
        let result = normalize_data_param(&s);
        assert_eq!(result, serde_json::json!({"key": "value"}));
    }

    #[test]
    fn test_normalize_data_param_invalid_json_string() {
        let s = serde_json::json!("not json at all");
        let result = normalize_data_param(&s);
        assert_eq!(result, serde_json::json!("not json at all"));
    }
}
