# MCP Mux — Architecture

## Overview

MCP Mux is a Tauri v2 desktop app that provides a rich display surface for AI agents. It replaces the companion Node.js server (`companion/`) with a native app featuring a Rust backend, system tray integration, and auto-start.

## Data Flow

```
MCP Agent → POST localhost:4200/api/push
                    │
            ┌───────▼────────┐
            │  Rust axum      │  (http_server.rs)
            │  HTTP server    │
            └───────┬────────┘
                    │
            ┌───────▼────────┐
            │  SessionStore   │  (session.rs)
            │  + ReviewState  │  (review.rs)
            └───────┬────────┘
                    │
            tauri::emit("push_preview", session)
                    │
            ┌───────▼────────┐
            │  WebView        │  (main.js + renderers/)
            │  renders content│
            └───────┬────────┘
                    │ (user decides)
            tauri::invoke("submit_decision", {sessionId, decision})
                    │
            ┌───────▼────────┐
            │  Rust resolves  │  (review.rs oneshot channel)
            │  pending review │
            └───────┬────────┘
                    │
            HTTP response → MCP Agent
```

## Components

### Rust Backend (`src-tauri/src/`)

| File | Responsibility |
|------|---------------|
| `main.rs` | Tauri entry point, plugin setup (shell, autostart), system tray, window event handling (hide-to-tray on close) |
| `http_server.rs` | axum HTTP server on `:4200`. Routes: `GET /health`, `POST /api/push`, `POST /api/reload-plugins`, and MCP Streamable HTTP on `/mcp` (GET for SSE, POST for JSON-RPC, DELETE for session teardown). Runs on a dedicated thread with its own tokio runtime to avoid blocking the GTK event loop. `resolve_content_type(registry, tool_name)` maps tool names to renderer names through plugin manifest renderer maps, falling back to the raw tool name if no mapping exists. Includes 4 unit tests covering single-plugin, multi-plugin, fallback, and no-match scenarios |
| `session.rs` | `SessionStore` — in-memory `HashMap<String, PreviewSession>` with 30-minute TTL and 60s GC interval |
| `review.rs` | `ReviewState` — pending review management via `tokio::oneshot` channels. `add_pending()` returns a receiver; `resolve()` or `dismiss()` sends the decision |
| `commands.rs` | Tauri IPC commands: `get_sessions`, `submit_decision`, `dismiss_session`, `get_health`, plus plugin management commands (`list_plugins`, `install_plugin`, `uninstall_plugin`, `install_plugin_from_file`, `install_plugin_from_registry`, `install_plugin_from_zip`, `fetch_registry`, `start_plugin_auth`, `store_plugin_token`, `update_plugin`, `get_plugin_renderers`) and settings/registry commands (`get_settings`, `save_settings`, `get_registry_sources`, `add_registry_source`, `remove_registry_source`, `toggle_registry_source`) |
| `state.rs` | `AppState` — shared state containing `Mutex<SessionStore>`, `Mutex<ReviewState>`, `Mutex<PluginRegistry>`, `Mutex<McpSessionManager>`, `Mutex<Vec<RegistryEntry>>` (cached registry), and `reqwest::Client`. Provides `reload_plugins()` for full disk reload and `notify_tools_changed()` for broadcasting `notifications/tools/list_changed` to all SSE sessions. `new_with_store(store)` constructor accepts a custom `PluginStore` for testable construction with temp directories (avoids touching the real filesystem). All plugin install/uninstall/update commands now call `notify_tools_changed()` so connected MCP clients are notified immediately |
| `mcp_session.rs` | `McpSessionManager` — manages MCP Streamable HTTP SSE sessions with `tokio::broadcast` channels. Supports session creation, teardown, broadcast to all sessions, and GC of sessions with no active receivers |
| `installer.rs` | Agent integration installer — first-run detection, bundled script resolution, and terminal spawning for setup scripts (Linux/macOS/Windows) |
| `renderer_scanner.rs` | Scans installed plugin directories for custom renderer JS files in `{plugin_dir}/renderers/*.js`, returning `RendererInfo` structs with `plugin://` protocol URLs |
| `registry.rs` | Re-exports `get_configured_registry_url` and `fetch_registry` from the shared crate |
| `plugin.rs` | `PluginRegistry` — plugin lifecycle management (load, add, remove, refresh). `PluginToolResult` struct for named-field plugin tool lookup results. `try_refresh_oauth(oauth_info, client) -> Option<String>` helper for deduplicated OAuth token refresh with logging, used by both `lookup_plugin_tool` and `refresh_stale_plugins`. `OAuthRefreshInfo` struct for refresh parameters |
| `tool_cache.rs` | `ToolCache` — per-plugin tool caching with 5-minute TTL, prefixed tool name indexing, and stale-detection logic (extracted from PluginRegistry) |
| `mcp_tools.rs` | MCP tool definitions and dispatch. Built-in tools: `push_content`, `push_review`, `push_check`, `setup_agent_rules`. Plugin tool proxy with automatic OAuth token refresh on expired tokens. Renderer definitions (built-in + plugin) are collected and used to dynamically populate tool descriptions and MCP `initialize` instructions. `call_setup_agent_rules` delegates to extracted pure functions: `collect_rules(builtin_renderers, manifests)`, `collect_plugin_auth_status(manifests)`, and `persistence_instructions(agent_type)`. Includes 13 unit tests for all extracted helpers |
| `auth.rs` | Plugin authentication — OAuth browser-redirect flow with ephemeral localhost callback server. Token storage and loading delegate to `shared::token_store`. Includes `refresh_oauth_token()` for automatic refresh_token grant when access tokens expire |
| `scripts/` | Bundled installer scripts for agent integration setup: `setup-integrations.sh` (Linux/macOS) and `setup-integrations.ps1` (Windows) |

### Frontend (`src/` + `public/`)

The WebView loads `index.html` which includes:
- CDN scripts: `marked.js` (markdown), `mermaid` (diagrams)
- `styles.css` — all styling (ported from companion)
- `main.js` — app bootstrap, Tauri IPC event listener, session/queue management
- `renderers/*.js` — built-in content renderers: `rich-content`, `document-preview`, `citation-panel`, `mermaid-renderer`, plus `shared.js` utilities. Domain-specific renderers (code analysis, data governance, etc.) are delivered via the plugin system
- `plugin-manager.js` — Plugin Manager window logic (registry browser, installed list, settings)

**Key change from companion**: WebSocket replaced with Tauri IPC:
- Receive: `window.__TAURI__.event.listen('push_preview', callback)`
- Send: `window.__TAURI__.core.invoke('submit_decision', payload)`

### Shared Types (`shared/`)

`mcp-mux-shared` crate consumed by both the Tauri backend and CLI. Contains:
- `RendererDef` — structured renderer definition with `name`, `description`, `scope` (universal/tool), `tools`, `data_hint`, and `rule` fields. Used by `setup_agent_rules` to bootstrap agent behavioral rules and by `initialize` to build dynamic MCP instructions
- `PluginManifest`, `PluginMcpConfig` — plugin definition and MCP connection config. Manifests now support `renderer_definitions: Vec<RendererDef>` for structured renderer metadata and `tool_rules: HashMap<String, String>` for per-tool behavioral rules
- `PluginAuth` — tagged enum: `Bearer { token_env }`, `ApiKey { header_name, key_env }`, `OAuth { client_id?, auth_url, token_url, scopes }`. Implements `Display`, `display_name()`, `is_configured()`, and `resolve_header()` for centralized auth resolution. All variants delegate token I/O to the `token_store` module, falling back to environment variables for Bearer and ApiKey. OAuth `client_id` is now optional
- `RegistryEntry`, `RemoteRegistry` — remote registry schema. `RegistryEntry` now includes optional `download_url` for ZIP package distribution
- `RegistrySource` — `{ name, url, enabled }` struct for multi-source registry configuration
- `PluginInfo` — lightweight plugin summary for IPC, now includes `update_available: Option<String>` for version comparison
- Path helpers: `plugins_dir()`, `config_path()`, `auth_dir()`, `cache_dir()` — all under `~/.mcp-mux/`
- `plugin_store::PluginStore` — filesystem-based plugin CRUD (list, load, save, remove, exists). Used by both CLI and Tauri app, eliminating duplicated disk I/O logic. Injected into `PluginRegistry` at construction time for testability. `dir()` accessor exposes the plugin directory path (used by `AppState::reload_plugins()` to reconstruct a fresh store)
- `settings::Settings` — typed representation of `~/.mcp-mux/config.json` with `load()` and `save()` methods. Fields: optional legacy `registry_url`, and `registry_sources` vec. Replaces raw `serde_json::Value` handling in settings commands
- `token_store` module — `StoredToken` struct with `load_stored_token()`, `load_stored_token_unvalidated()`, `store_token()`, `has_stored_token()`, and expiry checking. `load_stored_token_unvalidated()` returns expired tokens without filtering (used by OAuth refresh to retrieve the refresh_token from an expired entry). Centralizes all token file I/O (read, write, existence check, expiry detection) previously duplicated across `PluginAuth` match arms and `auth.rs`
- `registry` module — `get_configured_registry_url()`, `fetch_registry()`, `get_registry_sources()`, `save_registry_sources()`, and `fetch_all_registries()` with per-source 1-hour disk caching. Supports multi-source registry configuration with fallback to legacy single `registry_url`. Shared by both CLI and Tauri app
- `package` module — ZIP plugin package handling: `extract_plugin_zip()` (with zip-slip protection, GitHub-style prefix stripping, manifest validation), `download_and_install_plugin()` (download + extract + install), and `install_from_local_zip()`. Max download size: 50MB

### CLI (`cli/`)

Standalone binary (`mcp-mux-cli`) for headless plugin management. Commands: `list`, `add`, `remove`, `add-custom`, `search`. Shares `mcp-mux-shared` types with the Tauri app. See [docs/cli.md](cli.md).

### Plugin Registry (`registry/`)

GitHub-hosted `registry.json` containing available plugins with manifests, descriptions, tags, and author info. Fetched by both the Tauri app and CLI with a 1-hour cache.

### SSE Sidecar (`sidecar/`)

Standalone Node.js script that bridges a remote server's SSE stream to the local HTTP API:
1. Connects to `{appHost}/api/companion/stream` with Bearer auth
2. Parses SSE `data:` events
3. Forwards each event as `POST localhost:4200/api/push`
4. Exponential backoff reconnection (5s → 60s)
5. Keepalive timeout detection (45s)

## Key Design Decisions

### Dedicated HTTP Thread
The axum server runs on `std::thread::spawn` with its own `tokio::Runtime`, not `tauri::async_runtime::spawn`. This is necessary because Tauri's main thread runs the GTK event loop, and `tauri::async_runtime::spawn` tasks don't execute until after WebKit2GTK initializes (which can take 20+ seconds on some systems).

### Single-Session Model
Each push clears all existing sessions before creating a new one. This matches the companion's behavior and keeps the UI focused on the latest content.

### Review Workflow
For `reviewRequired: true` pushes, the HTTP handler:
1. Creates a `tokio::oneshot` channel
2. Stores the sender in `ReviewState`
3. Drops the async lock and `await`s the receiver (with timeout)
4. When the user clicks accept/reject in the WebView, `submit_decision` IPC command resolves the channel
5. The HTTP response is sent back to the MCP agent

### Window Management
- Close → hide to tray (not quit)
- Tray click → show + focus main window
- Push event → show + focus main window (automatic)
- Tray menu → "Show Window" / "Manage Plugins" / "Setup Agent Integrations" / "Quit"
- "Manage Plugins" opens a separate Plugin Manager window (`plugin-manager.html`, 800x600)
- Custom `plugin://` URI scheme serves plugin renderer assets from `~/.mcp-mux/plugins/{name}/` with path traversal protection and MIME type detection

## MCP Streamable HTTP Transport

The `/mcp` endpoint implements the MCP Streamable HTTP transport specification:

- **`GET /mcp`** — SSE stream for server-to-client notifications. Requires `Accept: text/event-stream` header. Returns a `mcp-session-id` response header for session tracking. Uses `tokio::broadcast` channels per session with keepalive.
- **`POST /mcp`** — JSON-RPC request/response for client-to-server calls. Optional `mcp-session-id` header to bind to an existing session (returns 404 if session not found). The `initialize` response now includes dynamic `instructions` text built from available renderer definitions and plugin auth status warnings.
- **`DELETE /mcp`** — Session teardown. Requires `mcp-session-id` header (returns 400 if missing, 404 if not found).
- **`POST /api/reload-plugins`** — Triggers plugin hot-reload and broadcasts `notifications/tools/list_changed` to all active SSE sessions.

Session management is handled by `McpSessionManager` which tracks sessions in a `HashMap`, supports broadcast to all sessions, and provides GC for sessions with no active receivers.

## API Compatibility

The HTTP push API on `:4200` is fully compatible with the existing MCP server push logic:
- Same `POST /api/push` request shape (`PushRequest`)
- Same response shape (`PushResponse`)
- Same review timeout behavior (408 on timeout)
- CORS headers now include `DELETE` method and expose `mcp-session-id` header
- `GET /health` returns version and uptime

No changes needed on the MCP server side — it just POSTs to localhost:4200.
