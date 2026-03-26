# MCP Mux — API Reference

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

**Content Type Detection**

| `toolName` | Content Type | Renderer |
|------------|-------------|----------|
| `search_codebase`, `vector_search` | `search_results` | Grouped results with type chips |
| `get_code_units` | `code_units` | Source code with complexity badges |
| `get_document` | `document_preview` | Rendered markdown document |
| `write_document`, `propose_actions` | `document_diff` | Two-column diff with accept/reject |
| `get_data_schema` | `data_schema` | Expandable table/column view |
| `manage_data_draft` | `data_draft_diff` | Grid-based draft review |
| `get_dependencies` | `dependencies` | Grouped imports by source file |
| `get_file_content` | `file_content` | Source with line numbers |
| `get_module_overview` | `module_overview` | File tree + exports + deps |
| `get_analysis_stats` | `analysis_stats` | Metric cards + repo list |
| `get_business_concepts`, `manage_knowledge_entries` | `knowledge_dex` | Table with bulk accept/reject |
| `get_column_context` | `column_context` | Breadcrumb nav + related entities |
| `rich_content`, `push_to_companion` | `rich_content` | Markdown + mermaid fallback |
| _(anything else)_ | `rich_content` | Markdown + mermaid fallback |

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

### `OPTIONS /api/push`

CORS preflight. Returns `200` with:
- `Access-Control-Allow-Origin: *`
- `Access-Control-Allow-Methods: GET, POST, OPTIONS`
- `Access-Control-Allow-Headers: *`

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
// PluginInfo: { name, version, has_mcp, auth_type, tool_count }
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

Initiate authentication for a plugin. Starts OAuth browser flow for OAuth plugins, or resolves env vars for Bearer/ApiKey plugins.

```javascript
const token = await invoke('start_plugin_auth', { pluginName: 'my-plugin' });
// Returns: token string on success
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
