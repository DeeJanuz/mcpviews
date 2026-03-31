# MCPViews — API Reference

## HTTP Endpoints

### `GET /health`

Returns server status.

**Response** `200 OK`
```json
{
  "version": "0.1.0",
  "port": 4200,
  "uptime_seconds": 123,
  "started_at": "2026-03-25T18:00:00Z"
}
```

### `POST /api/push`

Push content to the viewer. Optionally block until user reviews.

**Request Body**
```json
{
  "toolName": "string (required)",
  "toolArgs": {},
  "result": {
    "data": "any (required)",
    "meta": {}
  },
  "reviewRequired": false,
  "timeout": 120,
  "sessionId": "string (optional, auto-generated if absent)"
}
```

**Content Type Resolution**

Content type (renderer name) is resolved by searching all loaded plugin manifest renderer maps for a matching `toolName` key. If a plugin's `renderers` map contains an entry for the given tool name, that mapped renderer name is used as the `contentType`. If no plugin provides a mapping, the raw `toolName` is used as-is. This resolution is performed by `resolve_content_type()` in `http_server.rs`, matching the same logic used by `mcp_tools.rs` for MCP tool calls.

| `toolName` | Content Type | Renderer |
|------------|-------------|----------|
| `rich_content`, `push_to_companion` | `rich_content` | Markdown + mermaid fallback |
| `structured_data` | `structured_data` | Tabular data with hierarchical rows, change tracking, and review mode |
| _(plugin-mapped tool)_ | Renderer name from plugin manifest `renderers` map | Plugin-provided renderer |
| _(anything else)_ | Same as `toolName` | Falls back to `rich_content` if no matching renderer JS found |

**Response (non-review)** `201 Created`
```json
{
  "sessionId": "uuid",
  "status": "stored"
}
```

**Response (review — accepted)** `200 OK`
```json
{
  "sessionId": "uuid",
  "status": "decision_received",
  "decision": "accept"
}
```

**Response (review — rejected)** `200 OK`
```json
{
  "sessionId": "uuid",
  "status": "decision_received",
  "decision": "reject"
}
```

**Response (review — partial)** `200 OK`
```json
{
  "sessionId": "uuid",
  "status": "decision_received",
  "decision": "partial",
  "operationDecisions": {
    "op-1": "accepted",
    "op-2": "rejected"
  },
  "comments": { "op-2": "Needs rewording" },
  "modifications": { "op-1": "Edited text" }
}
```

**Response (review — timeout)** `408 Request Timeout`
```json
{
  "sessionId": "uuid",
  "status": "decision_received",
  "decision": "dismissed"
}
```

### `POST /api/reload-plugins`

Reload all plugins from disk and broadcast `notifications/tools/list_changed` to all active MCP SSE sessions.

**Response** `200 OK` (empty body)

### `GET /mcp`

Open an SSE stream for MCP Streamable HTTP server-to-client notifications.

**Required Headers:**
- `Accept: text/event-stream`

**Response** `200 OK` (SSE stream)
- Response header `mcp-session-id` contains the session ID
- Stream sends JSON-RPC notifications as SSE `data:` events
- Keepalive pings are sent automatically

**Error** `406 Not Acceptable` if `Accept` header missing or incorrect.

### `POST /mcp`

Send a JSON-RPC request to the MCP handler.

**Optional Headers:**
- `mcp-session-id` — bind request to an existing SSE session

**Request Body:** JSON-RPC 2.0 request

**Response** JSON-RPC 2.0 response with appropriate status code.

**Error** `404 Not Found` if `mcp-session-id` is provided but session does not exist.

### `DELETE /mcp`

Tear down an MCP SSE session.

**Required Headers:**
- `mcp-session-id` — the session to remove

**Response** `200 OK` if session was removed.

**Error** `400 Bad Request` if header missing, `404 Not Found` if session does not exist.

### Mock OAuth Endpoints

MCPViews implements a complete mock OAuth flow so that Claude Code's HTTP transport auth handshake completes instantly without real authentication. These endpoints satisfy the RFC 9728 / RFC 8414 discovery probes and the subsequent registration, authorization, and token exchange.

#### `GET /.well-known/oauth-protected-resource`

RFC 9728 protected resource metadata.

**Response** `200 OK`
```json
{
  "resource": "http://localhost:4200",
  "authorization_servers": ["http://localhost:4200"]
}
```

#### `GET /.well-known/oauth-authorization-server`

RFC 8414 authorization server metadata.

**Response** `200 OK`
```json
{
  "issuer": "http://localhost:4200",
  "authorization_endpoint": "http://localhost:4200/oauth/authorize",
  "token_endpoint": "http://localhost:4200/oauth/token",
  "registration_endpoint": "http://localhost:4200/oauth/register",
  "response_types_supported": ["code"],
  "grant_types_supported": ["authorization_code", "refresh_token"],
  "code_challenge_methods_supported": ["S256"],
  "token_endpoint_auth_methods_supported": ["none"]
}
```

#### `POST /oauth/register`

Dynamic client registration (mock). Echoes back provided `redirect_uris` with a fixed `client_id`.

**Request Body** (JSON, extra fields ignored)
```json
{
  "redirect_uris": ["http://localhost:9999/callback"]
}
```

**Response** `200 OK`
```json
{
  "client_id": "mcpviews-mock-client",
  "client_name": "MCPViews Mock Client",
  "redirect_uris": ["http://localhost:9999/callback"],
  "grant_types": ["authorization_code", "refresh_token"],
  "response_types": ["code"],
  "token_endpoint_auth_method": "none"
}
```

#### `GET /oauth/authorize`

Immediately redirects with a mock authorization code.

**Query Parameters:**
| Param | Required | Description |
|-------|----------|-------------|
| `redirect_uri` | Yes | Client callback URL |
| `state` | No | Opaque state value passed through |

**Response** `302 Found` with `Location: {redirect_uri}?code=mcpviews-mock-code&state={state}`

**Error** `400 Bad Request` if `redirect_uri` is missing.

#### `POST /oauth/token`

Returns a mock access token.

**Response** `200 OK`
```json
{
  "access_token": "mcpviews-mock-token",
  "token_type": "bearer",
  "expires_in": 86400,
  "scope": "mcp"
}
```

### `OPTIONS /api/push`

CORS preflight. Returns `200` with:
- `Access-Control-Allow-Origin: *`
- `Access-Control-Allow-Methods: GET, POST, DELETE, OPTIONS`
- `Access-Control-Allow-Headers: *`
- `Access-Control-Expose-Headers: mcp-session-id`

## Tauri IPC Commands

These are called from the WebView via `window.__TAURI__.core.invoke()`.

### `get_sessions`

Returns all active sessions.

```javascript
const sessions = await invoke('get_sessions');
// Returns: PreviewSession[]
```

### `submit_decision`

Submit a review decision for a session.

```javascript
await invoke('submit_decision', {
  sessionId: 'uuid',
  decision: 'accept',           // 'accept' | 'reject' | 'partial'
  operationDecisions: null,     // Optional: { 'op-id': 'accepted' | 'rejected' }
  comments: null,               // Optional: { 'op-id': 'comment text' }
  modifications: null,          // Optional: { 'op-id': 'modified value' }
  additions: null               // Optional: JSON value
});
```

### `dismiss_session`

Remove a session without making a decision.

```javascript
await invoke('dismiss_session', { sessionId: 'uuid' });
```

### `get_health`

Returns app health info.

```javascript
const health = await invoke('get_health');
// Returns: { version: "0.1.0", status: "ok" }
```

### `list_plugins`

Returns all installed plugins.

```javascript
const plugins = await invoke('list_plugins');
// Returns: PluginInfo[]
// PluginInfo: { name, version, has_mcp, auth_type, auth_configured, tool_count }
```

### `install_plugin`

Install a plugin from a JSON manifest string.

```javascript
await invoke('install_plugin', { manifestJson: '{"name":"...","version":"...","renderers":{}}' });
```

### `uninstall_plugin`

Remove an installed plugin by name.

```javascript
await invoke('uninstall_plugin', { name: 'plugin-name' });
```

### `install_plugin_from_file`

Install a plugin from a local manifest file path.

```javascript
await invoke('install_plugin_from_file', { path: '/path/to/manifest.json' });
```

### `fetch_registry`

Fetch available plugins from the remote registry. Uses 1-hour cache.

```javascript
const entries = await invoke('fetch_registry', { registryUrl: null });
// Returns: RegistryEntry[]
// RegistryEntry: { name, version, description, author, homepage, manifest, tags }
```

### `start_plugin_auth`

Initiate the OAuth browser-redirect flow for an OAuth plugin.

```javascript
const token = await invoke('start_plugin_auth', { pluginName: 'my-plugin' });
// Returns: token string on success
```

### `get_plugin_auth_header`

Retrieve the resolved authentication header for a plugin. Returns the full header value (e.g., `Bearer <token>` or a custom header value). Checks stored tokens first, then environment variable fallbacks, and attempts an OAuth token refresh if the stored token has expired.

```javascript
const header = await invoke('get_plugin_auth_header', { pluginName: 'my-plugin' });
// Returns: "Bearer sk-abc123" (or custom header value)
// Throws: if plugin not found, has no auth config, or no token is available
```

### `store_plugin_token`

Store a Bearer token or API key for a plugin. Saves to `~/.mcpviews/auth/<pluginName>.json`.

```javascript
await invoke('store_plugin_token', { pluginName: 'my-plugin', token: 'sk-abc123' });
```

### `install_plugin_from_registry`

Install a plugin from a registry entry. If the entry has a `download_url`, downloads and extracts the ZIP package. Otherwise falls back to manifest-only install.

```javascript
await invoke('install_plugin_from_registry', { entryJson: '{"name":"...","version":"...","manifest":{...},"download_url":"..."}' });
```

### `install_plugin_from_zip`

Install a plugin from a local ZIP file. The ZIP must contain a `manifest.json` at the root (or under a single top-level directory).

```javascript
await invoke('install_plugin_from_zip', { path: '/path/to/plugin.zip' });
```

### `update_plugin`

Update an installed plugin to the latest version from the cached registry. Downloads the ZIP package if available.

```javascript
await invoke('update_plugin', { name: 'plugin-name' });
```

### `reinstall_plugin`

Reinstall a plugin from the registry. If the plugin exists in the cached registry, it is re-downloaded and installed (replacing the current version). For non-registry (local-only) plugins, the command verifies the plugin exists but does not re-download.

```javascript
await invoke('reinstall_plugin', { name: 'plugin-name' });
```

### `clear_plugin_auth`

Remove the stored authentication token for a plugin. Deletes the token file at `~/.mcpviews/auth/<name>.json`. Returns success even if no token file exists.

```javascript
await invoke('clear_plugin_auth', { name: 'plugin-name' });
```

### `get_plugin_renderers`

Scan installed plugin directories for custom renderer JS files.

```javascript
const renderers = await invoke('get_plugin_renderers');
// Returns: RendererInfo[]
// RendererInfo: { plugin_name, file_name, url, mcp_url }
// url format: plugin://localhost/{plugin_name}/renderers/{file_name}?v={mtime}
// mtime is the file's last-modified Unix timestamp for cache busting
// mcp_url: the plugin's MCP URL from manifest.json (mcp.url field), or null
//          Used by the frontend to populate window.__mcpviews_plugins
```

### `get_renderer_registry`

Returns all invocable renderer definitions (those with `invoke_schema` set). Used by the frontend invocation registry to populate the cross-renderer linking system.

```javascript
const renderers = await invoke('get_renderer_registry');
// Returns: RendererRegistryEntry[]
// RendererRegistryEntry: { name, description, display_mode, invoke_schema, url_patterns, plugin }
```

Each entry includes the renderer's preferred `display_mode` ("drawer", "modal", or "replace"), the `invoke_schema` (JSON schema hint for invocation params), `url_patterns` (glob patterns for auto-detecting URLs), and the `plugin` name that provides it. Only renderers with `invoke_schema` set are included.

### `get_registry_sources`

Get all configured registry sources.

```javascript
const sources = await invoke('get_registry_sources');
// Returns: RegistrySource[]
// RegistrySource: { name, url, enabled }
```

### `add_registry_source`

Add a new registry source. Errors if a source with the same URL already exists.

```javascript
await invoke('add_registry_source', { name: 'My Registry', url: 'https://example.com/registry.json' });
```

### `remove_registry_source`

Remove a registry source by URL.

```javascript
await invoke('remove_registry_source', { url: 'https://example.com/registry.json' });
```

### `toggle_registry_source`

Toggle a registry source's enabled state.

```javascript
await invoke('toggle_registry_source', { url: 'https://example.com/registry.json' });
```

### `get_settings`

Read the application settings from `~/.mcpviews/config.json`. Returns default (empty) settings if no config file exists or the file cannot be parsed.

```javascript
const settings = await invoke('get_settings');
// Returns: Settings
// Settings: { registry_url?: string, registry_sources?: RegistrySource[] }
```

### `save_settings`

Write application settings to `~/.mcpviews/config.json`. Accepts a typed `Settings` object. Creates the config directory and file if they do not exist. Empty/null fields are omitted from the saved JSON.

```javascript
await invoke('save_settings', {
  settings: {
    registry_sources: [
      { name: 'Default', url: 'https://example.com/registry.json', enabled: true }
    ]
  }
});
```

## MCP Tools

These tools are exposed via the MCP Streamable HTTP transport (`POST /mcp` with `tools/call`).

### `push_content`

Display content in the MCPViews window. Supports multiple content types.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `tool_name` | string | Yes | Content type identifier for renderer selection. Available renderers are listed dynamically based on installed plugins. Use `rich_content` for generic markdown display. |
| `data` | object | Yes | Content data to display. |

### `push_review`

Display content and block until the user accepts or rejects. Returns the user's decision.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `tool_name` | string | Yes | Content type identifier for renderer selection. |
| `data` | object | Yes | Content data to display. |
| `timeout` | number | No | Timeout in seconds (default: 120). |

#### structured_data renderer

**Read-only display (push_content):**

```json
{
  "tool_name": "structured_data",
  "data": {
    "title": "Optional Title",
    "tables": [{
      "id": "t1",
      "name": "Table Name",
      "columns": [
        { "id": "c1", "name": "Column Name", "change": null }
      ],
      "rows": [{
        "id": "r1",
        "cells": { "c1": { "value": "cell value", "change": null } },
        "children": []
      }]
    }]
  }
}
```

All `change` fields must be `null` for push_content. The server strips non-null change values automatically.

**Change review (push_review):**

```json
{
  "tool_name": "structured_data",
  "data": {
    "title": "Review Title",
    "tables": [{
      "id": "t1",
      "name": "Table Name",
      "columns": [
        { "id": "c1", "name": "Existing Col", "change": null },
        { "id": "c2", "name": "New Col", "change": "add" }
      ],
      "rows": [{
        "id": "r1",
        "cells": {
          "c1": { "value": "updated value", "change": "update" },
          "c2": { "value": "new value", "change": "add" }
        },
        "children": []
      }]
    }]
  },
  "timeout": 300
}
```

Change values: `"add"` (green highlight), `"delete"` (red strikethrough), `"update"` (yellow highlight), `null` (unchanged).

**push_review response (structured_data):**

```json
{
  "sessionId": "uuid",
  "status": "decision_received",
  "decision": "partial",
  "operationDecisions": {
    "r1": "accept",
    "col:c2": "reject"
  },
  "modifications": {
    "r1.c1": "{\"value\":\"user changed this\",\"user_edited\":true}"
  },
  "additions": {
    "user_edits": { "r1.c1": "user changed this" }
  }
}
```

- `operationDecisions`: Row IDs map to "accept"/"reject". Column decisions use `"col:<colId>"` prefix.
- `modifications`: Cell edits as `"<rowId>.<colId>"` keys with JSON-encoded value objects.
- `additions.user_edits`: Convenience map of user-edited cell values.

### `push_check`

Check the status or result of a previously pushed review session.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `session_id` | string | Yes | The session ID to check. |

### `init_session`

Initialize MCPViews for the current session. Returns current renderer definitions, behavioral rules, plugin auth status, and persistence instructions. Must be called at the start of every conversation, chat session, or interaction -- not just once.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `agent_type` | string | No | The agent platform calling this tool. Supported: `claude_code`, `claude_desktop`, `codex`, `cursor`, `windsurf`, `opencode`, `antigravity`. Tailors the persistence instructions in the response. If omitted or unrecognized, returns instructions that ask the user how to persist rules. |

**Response:**
```json
{
  "rules": [
    {
      "name": "renderer_selection",
      "category": "system",
      "source": "built-in",
      "rule": "When displaying content in MCPViews, choose the renderer based on data shape..."
    },
    {
      "name": "rich_content_usage",
      "category": "renderer",
      "source": "built-in",
      "renderer": "rich_content",
      "description": "Universal markdown display with mermaid diagrams",
      "scope": "universal",
      "data_hint": "{ \"title\": \"heading\", \"body\": \"markdown\" }",
      "tools": [],
      "rule": "When presenting implementation plans..."
    }
  ],
  "plugin_status": [
    {
      "plugin": "my-plugin",
      "auth_type": "OAuth",
      "auth_configured": false,
      "auth_url": "https://...",
      "message": "Plugin 'my-plugin' requires re-authentication..."
    }
  ],
  "persistence_instructions": "Persist each rule as a memory file...",
  "plugin_registry": [
    {
      "name": "my-plugin",
      "summary": "my-plugin plugin",
      "tags": ["search-results", "code-units"],
      "tool_groups": [
        {
          "name": "Search Results",
          "hint": "Search the codebase for matching code snippets...",
          "tools": ["search_codebase", "vector_search"]
        }
      ],
      "renderers": ["search_results", "code_units"]
    }
  ],
  "plugin_updates": [
    {
      "name": "my-plugin",
      "installed_version": "1.0.0",
      "available_version": "1.2.0"
    }
  ]
}
```

The `rules` array now contains only built-in (universal) rules -- the `renderer_selection` system rule and rules for universal-scope renderers. Plugin-specific rules are fetched on-demand via `get_plugin_docs`.

The `plugin_registry` array is a compact index of installed plugins, listing their tool groups, renderer names, and tags. Agents use this to identify which plugin to query for detailed docs, then call `get_plugin_docs` with the plugin name and optional filters.

The `plugin_updates` array lists plugins that have newer versions available in the registry. Each entry includes the plugin name, installed version, and available version. Call `update_plugins` to apply updates.

### `get_plugin_docs`

Fetch detailed usage docs for a plugin's tools and renderers. Call after `init_session` identifies which plugin you need. Returns plugin-specific renderer rules and tool rules, optionally filtered by group, tool, or renderer name.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `plugin` | string | Yes | Plugin name (e.g., `"ludflow"`, `"decidr"`). |
| `groups` | string[] | No | Specific tool group names to fetch (e.g., `["Search", "Code Analysis"]`). Group names come from `plugin_registry[].tool_groups[].name` in the `init_session` response. |
| `tools` | string[] | No | Specific tool names to fetch, unprefixed (e.g., `["search_codebase"]`). |
| `renderers` | string[] | No | Specific renderer names to fetch (e.g., `["code_units", "search_results"]`). |

When `groups` is provided, the group names are expanded to their constituent tool names. When multiple filters are provided, their tool sets are merged. When no filters are provided, all plugin rules are returned.

**Response:**
```json
{
  "plugin": "my-plugin",
  "rules": [
    {
      "name": "search_results_usage",
      "category": "renderer",
      "source": "plugin",
      "renderer": "search_results",
      "description": "Search output",
      "scope": "tool",
      "data_hint": "{ results: [...] }",
      "tools": ["search_codebase"]
    },
    {
      "name": "tp__search_codebase_usage",
      "category": "tool",
      "source": "my-plugin",
      "tool": "tp__search_codebase",
      "rule": "Use search for queries."
    }
  ]
}
```

### `update_plugins`

Update installed plugins to their latest versions from the registry. Uses remote manifest resolution to discover available updates. If no plugin name is provided, updates all plugins with available updates.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `plugin_name` | string | No | Specific plugin to update. If omitted, updates all plugins with available updates. |

**Response:**
```json
{
  "updated": [
    {
      "plugin": "my-plugin",
      "from": "1.0.0",
      "to": "1.2.0",
      "status": "success"
    }
  ]
}
```

### `mcpviews_install_plugin`

Install a plugin into MCPViews programmatically. Accepts a plugin manifest as JSON and optionally a download URL for a ZIP package containing renderer assets. If a plugin with the same name already exists, it is replaced. After installation, connected MCP clients are notified via `notifications/tools/list_changed` and the GUI receives a `reload_renderers` event.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `manifest_json` | string | Yes | JSON string of a `PluginManifest` object defining the plugin's name, version, renderers, MCP config, and tool rules. |
| `download_url` | string | No | URL to a `.zip` package to download and install. If provided, the manifest is extracted from the package and the `manifest_json` parameter is not used. |

**Response:**
```json
{
  "content": [{
    "type": "text",
    "text": "Plugin 'my-plugin' installed successfully."
  }]
}
```

**Behavior:**
- **Manifest-only install** (no `download_url`): Parses `manifest_json` and registers the plugin in the in-memory registry.
- **ZIP install** (with `download_url`): Downloads the ZIP package, extracts it to `~/.mcpviews/plugins/{plugin-name}/`, and registers the extracted manifest.
- If a plugin with the same name is already installed, it is removed first and then re-added.
- After installation, a `notifications/tools/list_changed` notification is broadcast to all MCP SSE sessions and a `reload_renderers` event is emitted to the WebView.

### `list_registry`

List all available plugins from the MCPViews registry, including install status, auth status, and available updates. Useful for guided plugin discovery workflows.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `tag` | string | No | Optional filter: only return plugins matching this tag. |

**Response:**
```json
{
  "content": [{
    "type": "text",
    "text": "{ \"plugins\": [...], \"total\": 3 }"
  }]
}
```

Each plugin entry includes:
| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Plugin name. |
| `description` | string | Plugin description. |
| `version` | string | Registry version. |
| `author` | string | Author name. |
| `tags` | string[] | Plugin tags. |
| `download_url` | string | ZIP download URL. |
| `installed` | boolean | Whether the plugin is currently installed. |
| `installed_version` | string | Installed version (if installed). |
| `auth_type` | string | Auth type ("OAuth", "Bearer", "ApiKey") if installed. |
| `auth_configured` | boolean | Whether auth is configured (only true if installed). |
| `update_available` | string | Newer version string if an update exists. |

### `start_plugin_auth`

Start authentication for an installed plugin. For OAuth plugins, this opens the user's browser and waits for the redirect flow to complete. For Bearer/ApiKey plugins, this checks whether the required environment variable is set.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `plugin_name` | string | Yes | Name of the plugin to authenticate. |

**Response (success):**
```json
{
  "content": [{
    "type": "text",
    "text": "OAuth authentication for 'my-plugin' completed successfully."
  }]
}
```

**Error:** Returns an error string if the environment variable is not set (Bearer/ApiKey) or the OAuth flow fails.

### `get_plugin_prompt`

Fetch a prompt from a plugin. Returns the prompt content with optional template argument substitution. The returned content should be used as system instructions for a guided workflow.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `plugin` | string | Yes | Plugin name. |
| `prompt` | string | Yes | Prompt name. |
| `arguments` | object | No | Optional key-value arguments to template into the prompt. Replaces `{{key}}` placeholders in the prompt source. |

**Response:**
```json
{
  "content": [{
    "type": "text",
    "text": "The rendered prompt content..."
  }]
}
```

### `mcpviews_setup`

One-time setup for MCPViews. Returns instructions for persisting a rule that ensures `init_session` is called automatically at the start of every conversation, chat session, or interaction. Also returns current rules and plugin status.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `agent_type` | string | No | The agent platform calling this tool. Supported: `claude_code`, `claude_desktop`, `codex`, `cursor`, `windsurf`, `opencode`, `antigravity`. Determines the platform-specific setup instructions. If omitted or unrecognized, returns generic instructions. |

**Response:**
```json
{
  "rules": [ ... ],
  "plugin_status": [ ... ],
  "persistence_instructions": "Persist each rule as a memory file...",
  "setup_instructions": "Add a rule in `.claude/rules/mcpviews-init.md` containing: ..."
}
```

## MCP Prompts

MCPViews implements the MCP prompts protocol (`prompts/list` and `prompts/get`), enabling native prompt discovery by Claude Code and other MCP clients. Prompts are advertised in the `initialize` response via the `capabilities.prompts.listChanged` field.

### `prompts/list`

Returns all available prompts (built-in + plugin prompts).

**Response:**
```json
{
  "prompts": [
    {
      "name": "onboarding",
      "description": "Guided setup to discover, install, and authenticate MCPViews plugins.",
      "arguments": []
    },
    {
      "name": "my-plugin/workflow",
      "description": "Plugin-provided prompt",
      "arguments": [
        { "name": "project_id", "description": "Target project", "required": true }
      ]
    }
  ]
}
```

Plugin prompts are namespaced as `{plugin}/{prompt}` (e.g., `my-plugin/workflow`).

### `prompts/get`

Resolve a prompt by name and return MCP-formatted messages.

**Request parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Prompt name. For plugin prompts, use `{plugin}/{prompt}` format. |
| `arguments` | object | No | Template arguments for plugin prompts (replaces `{{key}}` placeholders). |

**Response:**
```json
{
  "messages": [{
    "role": "user",
    "content": {
      "type": "text",
      "text": "The prompt content..."
    }
  }]
}
```

**Error:** JSON-RPC error `-32602` if the prompt name is not recognized.

### Built-in Prompts

| Name | Description |
|------|-------------|
| `onboarding` | Guided setup to discover, install, and authenticate MCPViews plugins. Walks through `list_registry`, `mcpviews_install_plugin`, `start_plugin_auth`, and `init_session`. |

## Tauri Events

### `push_preview` (Rust → WebView)

Emitted when a new push arrives.

```javascript
listen('push_preview', (event) => {
  const session = event.payload;
  // session: PreviewSession
});
```

**PreviewSession shape:**
```json
{
  "sessionId": "uuid",
  "toolName": "search_codebase",
  "toolArgs": {},
  "contentType": "search_results",
  "data": {},
  "meta": {},
  "reviewRequired": false,
  "timeoutSecs": null,
  "createdAt": 1711388400000
}
```

### `reload_renderers` (Rust → WebView)

Emitted when plugin renderers should be reloaded (e.g., after a plugin is installed or updated via the `mcpviews_install_plugin` MCP tool). The WebView re-runs `loadPluginRenderers()` to discover and load any new renderer scripts.

```javascript
listen('reload_renderers', () => {
  loadPluginRenderers();
});
```
