use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use crate::http_server::AsyncAppState;

const ONBOARDING_PROMPT: &str = r#"# MCPViews Plugin Onboarding

You are helping the user discover and install MCPViews plugins. Follow these steps:

## Step 1: Show Available Plugins

Call `list_registry` to see all available plugins. Present them to the user in a clear format showing:
- Plugin name and description
- Whether it's already installed
- Whether auth is needed
- Whether an update is available

## Step 2: Install Plugins

Ask the user which plugins they'd like to install. For each one:
1. Call `mcpviews_install_plugin` with the `download_url` from the registry entry
2. Report success or failure

## Step 3: Authenticate Plugins

For plugins that require authentication:
1. Call `start_plugin_auth` with the plugin name
2. For OAuth plugins, this will open the user's browser — wait for them to complete the flow
3. For Bearer/ApiKey plugins, tell the user which environment variable to set

## Step 4: Verify

Call `init_session` to verify all plugins are loaded and authenticated.

## Troubleshooting Tips

- If a plugin's tools don't appear after install, the MCP connection may need to be refreshed. Suggest the user reconnect MCP (e.g., `/mcp` in Claude Code).
- For OAuth auth failures, suggest retrying `start_plugin_auth` — the browser flow may have timed out.
- For Bearer/ApiKey auth, remind the user to restart their agent after setting environment variables.
- If `list_registry` returns empty, the registry may be unreachable — check network connectivity.
"#;

fn builtin_prompt_definitions() -> Vec<(&'static str, &'static str, Vec<Value>, &'static str)> {
    vec![
        (
            "onboarding",
            "Guided setup to discover, install, and authenticate MCPViews plugins.",
            vec![],
            ONBOARDING_PROMPT,
        ),
    ]
}

/// Return all prompts available (built-in + plugin prompts) in MCP format.
pub async fn list_prompts(state: &Arc<TokioMutex<AsyncAppState>>) -> Vec<Value> {
    let mut prompts: Vec<Value> = Vec::new();

    // Built-in prompts
    for (name, description, arguments, _content) in builtin_prompt_definitions() {
        prompts.push(serde_json::json!({
            "name": name,
            "description": description,
            "arguments": arguments,
        }));
    }

    // Plugin prompts (namespaced as {plugin}/{prompt})
    {
        let state_guard = state.lock().await;
        let registry = state_guard.inner.plugin_registry.lock().unwrap();
        for manifest in &registry.manifests {
            for prompt_def in &manifest.prompt_definitions {
                let namespaced = format!("{}/{}", manifest.name, prompt_def.name);
                prompts.push(serde_json::json!({
                    "name": namespaced,
                    "description": prompt_def.description,
                    "arguments": prompt_def.arguments.iter().map(|a| serde_json::json!({
                        "name": a.name,
                        "description": a.description,
                        "required": a.required,
                    })).collect::<Vec<Value>>(),
                }));
            }
        }
    }

    prompts
}

/// Look up a built-in prompt by name. Returns the content if found.
fn resolve_builtin_prompt(name: &str) -> Option<&'static str> {
    builtin_prompt_definitions()
        .into_iter()
        .find(|(n, _, _, _)| *n == name)
        .map(|(_, _, _, content)| content)
}

/// Resolve a prompt by name and return MCP-formatted messages.
pub async fn get_prompt(
    name: &str,
    arguments: Option<Value>,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    // Check built-in prompts first
    if let Some(content) = resolve_builtin_prompt(name) {
        return Ok(serde_json::json!({
            "messages": [{
                "role": "user",
                "content": {
                    "type": "text",
                    "text": content
                }
            }]
        }));
    }

    // Check plugin prompts ({plugin}/{prompt} format)
    if let Some((plugin_name, prompt_name)) = name.split_once('/') {
        let mut args = serde_json::json!({
            "plugin": plugin_name,
            "prompt": prompt_name,
        });
        if let Some(template_args) = arguments {
            args.as_object_mut().unwrap().insert("arguments".to_string(), template_args);
        }
        let result = call_get_plugin_prompt(args, state).await?;
        // Transform plugin prompt result into MCP prompt format
        let text = result
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        return Ok(serde_json::json!({
            "messages": [{
                "role": "user",
                "content": {
                    "type": "text",
                    "text": text
                }
            }]
        }));
    }

    Err(format!("Unknown prompt: {}", name))
}

/// Fetch a prompt from a plugin by reading its source file and applying template arguments.
pub(crate) async fn call_get_plugin_prompt(
    arguments: Value,
    state: &Arc<TokioMutex<AsyncAppState>>,
) -> Result<Value, String> {
    let plugin_name = arguments
        .get("plugin")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: plugin")?;

    let prompt_name = arguments
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: prompt")?;

    let template_args: std::collections::HashMap<String, String> = arguments
        .get("arguments")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let (source_path, plugins_dir) = {
        let state_guard = state.lock().await;
        let registry = state_guard.inner.plugin_registry.lock().unwrap();

        let (_, manifest) = registry
            .find_plugin_by_name(plugin_name)
            .ok_or_else(|| format!("Plugin '{}' not found", plugin_name))?;

        let prompt_def = manifest
            .prompt_definitions
            .iter()
            .find(|p| p.name == prompt_name)
            .ok_or_else(|| format!("Prompt '{}' not found in plugin '{}'", prompt_name, plugin_name))?;

        (prompt_def.source.clone(), state_guard.inner.plugins_dir().to_path_buf())
    };

    let path = plugins_dir.join(plugin_name).join(&source_path);
    let mut content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read prompt '{}' from plugin '{}': {}", source_path, plugin_name, e))?;

    // Template arguments: replace {{arg_name}} with provided values
    for (key, value) in &template_args {
        let placeholder = format!("{{{{{}}}}}", key);
        content = content.replace(&placeholder, value);
    }

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": content
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_prompt_definitions_has_onboarding() {
        let defs = builtin_prompt_definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].0, "onboarding");
        assert!(!defs[0].3.is_empty()); // content is non-empty
    }

    #[test]
    fn test_resolve_builtin_prompt_found() {
        let content = resolve_builtin_prompt("onboarding");
        assert!(content.is_some());
        assert!(content.unwrap().contains("MCPViews Plugin Onboarding"));
    }

    #[test]
    fn test_resolve_builtin_prompt_not_found() {
        let content = resolve_builtin_prompt("nonexistent");
        assert!(content.is_none());
    }

    #[test]
    fn test_builtin_prompt_definitions_structure() {
        let defs = builtin_prompt_definitions();
        for (name, desc, _args, content) in &defs {
            assert!(!name.is_empty(), "name should not be empty");
            assert!(!desc.is_empty(), "description should not be empty");
            assert!(!content.is_empty(), "content should not be empty");
        }
    }
}
