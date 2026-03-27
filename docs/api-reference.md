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

### `get_plugin_renderers`

Scan installed plugin directories for custom renderer JS files.

```javascript
const renderers = await invoke('get_plugin_renderers');
// Returns: RendererInfo[]
// RendererInfo: { plugin_name, file_name, url }
// url format: plugin://localhost/{plugin_name}/renderers/{file_name}
```

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

### `push_check`

Check the status or result of a previously pushed review session.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `session_id` | string | Yes | The session ID to check. |

### `setup_agent_rules`

Bootstrap behavioral rules for all mcpviews renderers and plugin tools. Call once per project to receive rules that should be persisted in the agent's native memory/rule system. Returns renderer rules, plugin tool rules, plugin auth status, and agent-type-specific persistence instructions.

**Parameters:**
| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `agent_type` | string | No | The agent platform calling this tool. Supported: `claude_code`, `claude_desktop`, `codex`, `cursor`, `windsurf`, `opencode`, `antigravity`. Tailors the persistence instructions in the response. If omitted or unrecognized, returns instructions that ask the user how to persist rules. |

**Response:**
```json
{
  "rules": [
    {
      "name": "rich_content_usage",
      "category": "renderer",
      "source": "built-in",
      "renderer": "rich_content",
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
  "persistence_instructions": "Persist each rule as a memory file..."
}
```

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
  "createdAt": 1711388400000
}
```
