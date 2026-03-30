# MCPViews — Architecture

## Overview

MCPViews is a Tauri v2 desktop app that provides a rich display surface for AI agents. It replaces the companion Node.js server (`companion/`) with a native app featuring a Rust backend, system tray integration, and auto-start.

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
| `main.rs` | Tauri entry point, plugin setup (shell, autostart), system tray, window event handling (hide-to-tray on close). Pre-binds the TCP listener on port 4200 synchronously in `setup()` before spawning the HTTP server thread, so the OS kernel accepts connections immediately and eliminates MCP startup race conditions (e.g., Claude Code probing the OAuth discovery endpoint before the server is ready). Main window is created programmatically (not declaratively via `tauri.conf.json`) so that `on_web_resource_request` hooks can inject a dynamic CSP header; the window defaults to `Theme::Light` at creation and is synced to the CSS theme at runtime via the `set_native_theme` command. `build_csp(extra_origins)` appends plugin MCP origins to the `connect-src` directive so plugin renderers can `fetch()` their plugin's API without CSP violations. `csp_request_hook(state)` returns a reusable closure for injecting dynamic CSP into any webview window (used by both main and plugin-manager windows). Listens for `reload_renderers` event to reload the main webview when new plugins are installed, picking up new CSP origins. The `plugin://` protocol handler includes `Cache-Control: no-store` to prevent stale renderer JS from being cached by the webview. Includes 3 unit tests for `build_csp` covering empty origins, multiple origins, and directive preservation |
| `http_server.rs` | axum HTTP server on `:4200`. Routes: `GET /health`, `POST /api/push`, `POST /api/reload-plugins`, MCP Streamable HTTP on `/mcp` (GET for SSE, POST for JSON-RPC, DELETE for session teardown), and mock OAuth endpoints (`GET /.well-known/oauth-protected-resource`, `GET /.well-known/oauth-authorization-server`, `POST /oauth/register`, `GET /oauth/authorize`, `POST /oauth/token`) that implement a complete mock OAuth flow so Claude Code's HTTP transport auth handshake completes instantly without real authentication. Runs on a dedicated thread with its own tokio runtime to avoid blocking the GTK event loop. Accepts a pre-bound `std::net::TcpListener` (converted to `tokio::net::TcpListener` via `from_std`) rather than binding the port itself, ensuring the port is ready before the async runtime starts. `resolve_content_type(registry, tool_name)` maps tool names to renderer names through plugin manifest renderer maps, falling back to the raw tool name if no mapping exists. Includes 11 unit tests covering content type resolution (single-plugin, multi-plugin, fallback, no-match) and mock OAuth endpoints (protected resource, authorization server metadata, register, authorize redirect, authorize missing params, token response) |
| `session.rs` | `SessionStore` — in-memory `HashMap<String, PreviewSession>` with 30-minute TTL and 60s GC interval. `PreviewSession` includes optional `timeout_secs` field for review countdown display |
| `review.rs` | `ReviewState` — pending review management via `tokio::oneshot` channels. `add_pending()` returns a receiver; `resolve()` or `dismiss()` sends the decision |
| `commands.rs` | Tauri IPC commands: `get_sessions`, `submit_decision`, `dismiss_session`, `get_health`, `save_file` (native save dialog for file export), `get_renderer_registry` (returns invocable renderer metadata for frontend invocation registry), `set_native_theme` (syncs the native WKWebView/window theme with the CSS theme to prevent OS dark mode from overriding CSS text colors), plus plugin management commands (`list_plugins`, `install_plugin`, `uninstall_plugin`, `install_plugin_from_file`, `install_plugin_from_registry`, `install_plugin_from_zip`, `fetch_registry`, `start_plugin_auth`, `get_plugin_auth_header` (resolves stored/env/refreshed auth token for a plugin), `store_plugin_token`, `update_plugin`, `get_plugin_renderers`) and settings/registry commands (`get_settings`, `save_settings`, `get_registry_sources`, `add_registry_source`, `remove_registry_source`, `toggle_registry_source`) |
| `state.rs` | `AppState` — shared state containing `Mutex<SessionStore>`, `Mutex<ReviewState>`, `Mutex<PluginRegistry>`, `Mutex<McpSessionManager>`, `Mutex<Vec<RegistryEntry>>` (cached registry), and `reqwest::Client`. Provides `reload_plugins()` for full disk reload, `notify_tools_changed()` for broadcasting `notifications/tools/list_changed` to all SSE sessions, `plugins_dir()` accessor delegating to `PluginStore::dir()`, `install_plugin_from_manifest(manifest)` for upsert-style plugin registration (removes existing plugin with same name, then adds), and `plugin_csp_origins()` which returns deduplicated origins (scheme + authority) from all installed plugin MCP URLs for dynamic CSP injection. `new_with_store(store)` constructor accepts a custom `PluginStore` for testable construction with temp directories (avoids touching the real filesystem). All plugin install/uninstall/update commands now call `notify_tools_changed()` so connected MCP clients are notified immediately |
| `mcp_session.rs` | `McpSessionManager` — manages MCP Streamable HTTP SSE sessions with `tokio::broadcast` channels. Supports session creation, teardown, broadcast to all sessions, and GC of sessions with no active receivers |
| `installer.rs` | Agent integration installer — first-run detection, bundled script resolution, and terminal spawning for setup scripts (Linux/macOS/Windows) |
| `renderer_scanner.rs` | Scans installed plugin directories for custom renderer JS files in `{plugin_dir}/renderers/*.js`, returning `RendererInfo` structs with `plugin://` protocol URLs. Appends `?v={mtime}` cache-busting query parameter (file modification timestamp in Unix seconds) to each renderer URL so reinstalling a plugin reliably loads new JS without requiring an uninstall/reinstall cycle |
| `registry.rs` | Re-exports `get_configured_registry_url` and `fetch_registry` from the shared crate |
| `plugin.rs` | `PluginRegistry` — plugin lifecycle management (load, add, remove, refresh). `PluginToolResult` struct for named-field plugin tool lookup results. `try_refresh_oauth(oauth_info, client) -> Option<String>` helper for deduplicated OAuth token refresh with logging, used by both `lookup_plugin_tool` and `refresh_stale_plugins`. `OAuthRefreshInfo` struct for refresh parameters |
| `tool_cache.rs` | `ToolCache` — per-plugin tool caching with 5-minute TTL, prefixed tool name indexing, and stale-detection logic (extracted from PluginRegistry). `plugin_tools(idx)` returns cached tool definitions for a plugin by index, encapsulating direct entry access |
| `mcp_tools.rs` | MCP tool definitions and dispatch. Built-in tools: `push_content`, `push_review`, `push_check`, `init_session`, `mcpviews_setup`. Plugin tool proxy with automatic OAuth token refresh on expired tokens. Pushes only happen via explicit `push_content`/`push_review` calls from the coordinator agent (auto-push of plugin results was removed to prevent sub-agent research calls from flooding the UI). `call_push_impl` auto-parses stringified JSON data payloads — if an agent passes `data` as a JSON string instead of an object, the server deserializes it automatically, preventing rendering failures from malformed payloads. `push_content` enforces read-only mode by stripping `change` fields from all row cells and columns before forwarding to the renderer, ensuring change markers never appear in non-review pushes. Renderer definitions (built-in + plugin) are collected and used to dynamically populate tool descriptions and MCP `initialize` instructions. Tool descriptions for `push_content` and `push_review` now include per-renderer `data_hint` values dynamically assembled from all available renderer definitions, so agents see payload schemas inline without calling `init_session`. `RICH_CONTENT_RULE` includes detailed formatting guidance for the `data` parameter (must be JSON object not string), mermaid diagram fencing (triple-backtick with `mermaid` language identifier), `<br/>` line breaks in node labels, and JSON string escaping rules. `synthesize_renderer_defs(manifest, cached_tools, known_names)` is a pure function that builds `RendererDef` entries from a manifest's `renderers` map, using cached tool definitions for descriptions; `available_renderers()` delegates to it as a thin aggregation layer. `gather_session_data(state)` collects renderer rules and plugin auth status. `call_init_session` returns rules, plugin status, and persistence instructions for per-session initialization. `call_mcpviews_setup` returns the same plus `setup_instructions(agent_type)` for one-time platform configuration. Both delegate to extracted pure functions: `collect_rules(builtin_renderers, manifests)`, `collect_plugin_auth_status(manifests)`, `persistence_instructions(agent_type)`, and `setup_instructions(agent_type)`. `collect_rules` emits a cross-cutting `renderer_selection` system rule (always first) that guides agents on choosing between `rich_content`, `structured_data`, and plugin renderers based on data shape, followed by per-renderer rules that always include `description`, `scope`, `data_hint`, and `tools` regardless of whether the renderer has an explicit rule or was synthesized. Includes 37 unit tests for all extracted helpers |
| `auth.rs` | Plugin authentication — OAuth browser-redirect flow with ephemeral localhost callback server. Token storage and loading delegate to `shared::token_store`. Includes `refresh_oauth_token()` for automatic refresh_token grant when access tokens expire |
| `scripts/` | Bundled installer scripts for agent integration setup: `setup-integrations.sh` (Linux/macOS) and `setup-integrations.ps1` (Windows). Generates platform-specific MCP configs: Claude Desktop receives an `mcp-remote` bridge entry (`npx mcp-remote`) because it only supports stdio transport; Claude Code CLI is configured via `claude mcp add --transport http --scope user` (native CLI command writing to `~/.claude.json`) instead of direct JSON merge, with detection via `claude mcp list`; all other JSON-based platforms receive a direct HTTP URL entry. Both scripts include an `already_configured` guard that skips platforms already configured, using `claude mcp list` for Claude Code and JSON key inspection (via Python or grep fallback) for other platforms |

### Frontend (`src/` + `public/`)

The WebView loads `index.html` which includes a `<meta name="color-scheme">` tag and an inline `syncNativeTheme()` function that calls the `set_native_theme` Tauri command on initial load and on every theme toggle, ensuring the native WKWebView rendering context matches the CSS theme:
- CDN scripts: `marked.js` (markdown), `mermaid` (diagrams)
- `styles.css` — glassmorphism design system with 140+ CSS custom properties, light/dark theme support via `[data-theme]` attribute and `color-scheme` property, frosted glass panels, geometric dot-grid background, and refined typography hierarchy. Theme toggle with `prefers-color-scheme` system preference detection. All renderer styling uses CSS classes (no inline JS styles); renderer JS files reference CSS variables exclusively (no hardcoded hex colors). Includes component-specific variables for data governance grids, badge colors, and diff card states. Dark mode overrides for mermaid SVG elements (text fills, node/edge strokes, actor boxes, sequence diagram elements) applied via `[data-theme="dark"] .mermaid-rendered` and `.mermaid-modal-body` selectors
- `main.js` — app bootstrap, Tauri IPC event listener, multi-session tab bar with cached content containers, review countdown timers, theme toggle logic with system preference detection and `localStorage` persistence. Error states use CSS variable references (`--color-error`, `--text-secondary`) instead of hardcoded colors. Includes global `mcpview://` invocation click handler (delegated via `data-invoke-renderer` attributes), Escape key handler for closing topmost drawer, invocation registry population on startup and plugin reload, and drawer cleanup on session removal
- `renderers/*.js` — built-in content renderers: `rich-content`, `document-preview`, `citation-panel`, `mermaid-renderer`, `structured-data`, `drawer-stack`, `invocation-registry`, plus `shared.js` utilities. All renderers use CSS classes from `styles.css` instead of inline styles. `rich-content` includes a markdown source toggle button that switches between rendered HTML and raw markdown text for easy copying. `mermaid-renderer` stores base64-encoded source on each rendered diagram and exposes `utils.reRenderMermaid()` to re-render all diagrams on theme change, called from `index.html` theme toggle logic. `structured-data` delegates pure data logic (sort, filter, flatten, cell value/change extraction, decision payload building, bulk decisions, table state creation) to `structured-data-utils.js` — 9 exported functions with 31 unit tests. The renderer supports hierarchical rows (expand/collapse), change tracking (add/delete/update per cell), sort/filter, color legend, per-table CSV export via native save dialog (`save_file` Tauri command with blob download fallback), and review mode with per-row/column accept/reject toggles, inline cell editing, and Accept All/Reject All buttons. In read-only mode (`push_content`), the server strips change fields and the renderer ignores them, ensuring no change markers appear. `shared.js` exports `CITATION_COLORS` with per-type CSS variable backgrounds, `HTTP_METHOD_COLORS` using semantic CSS variables, `createStatusBadge`/`createScopeBadge` helpers, and a custom marked.js link renderer that converts `mcpview://` URIs to invocation buttons with `data-invoke-renderer` and `data-invoke-params` attributes, supporting query string parameters. `drawer-stack.js` manages stacking slide-out drawer panels for cross-renderer invocation — each invocation opens the target renderer in a new panel layered on top (z-index stacking with overlay backdrop), with close-on-overlay-click, close button, and animated slide transitions. `invocation-registry.js` maintains a frontend cache of invocable renderer metadata (fetched from `get_renderer_registry` Tauri command), supports `autoDetectLinks(container)` to scan for `<a>` tags matching registered `url_patterns` and convert them to invocation buttons, and `populateRendererRegistry()` to refresh from the backend. Domain-specific renderers (code analysis, data governance, etc.) are delivered via the plugin system
- `plugin-manager.js` — Plugin Manager window logic (registry browser, installed list, settings)

**Key change from companion**: WebSocket replaced with Tauri IPC:
- Receive: `window.__TAURI__.event.listen('push_preview', callback)`
- Send: `window.__TAURI__.core.invoke('submit_decision', payload)`

### Shared Types (`shared/`)

`mcpviews-shared` crate consumed by both the Tauri backend and CLI. Contains:
- `RendererDef` — structured renderer definition with `name`, `description`, `scope` (universal/tool), `tools`, `data_hint`, `rule`, `display_mode` (preferred display when invoked: "drawer", "modal", or "replace"), `invoke_schema` (JSON schema hint for invocation params), and `url_patterns` (glob patterns for auto-detecting URLs to convert to invocation links). Used by `init_session`/`mcpviews_setup` to bootstrap agent behavioral rules, by `initialize` to build dynamic MCP instructions, and by the frontend invocation registry for cross-renderer linking
- `PluginManifest`, `PluginMcpConfig` — plugin definition and MCP connection config. Manifests now support `renderer_definitions: Vec<RendererDef>` for structured renderer metadata and `tool_rules: HashMap<String, String>` for per-tool behavioral rules. `no_auto_push: Vec<String>` is retained for backward compatibility but has no effect since auto-push was removed
- `PluginAuth` — tagged enum: `Bearer { token_env }`, `ApiKey { header_name, key_env }`, `OAuth { client_id?, auth_url, token_url, scopes }`. Implements `Display`, `display_name()`, `is_configured()`, and `resolve_header()` for centralized auth resolution. All variants delegate token I/O to the `token_store` module, falling back to environment variables for Bearer and ApiKey. OAuth `client_id` is now optional
- `RegistryEntry`, `RemoteRegistry` — remote registry schema. `RegistryEntry` now includes optional `download_url` for ZIP package distribution
- `RegistrySource` — `{ name, url, enabled }` struct for multi-source registry configuration
- `PluginInfo` — lightweight plugin summary for IPC, now includes `update_available: Option<String>` for version comparison
- Path helpers: `plugins_dir()`, `config_path()`, `auth_dir()`, `cache_dir()` — all under `~/.mcpviews/`
- `plugin_store::PluginStore` — filesystem-based plugin CRUD (list, load, save, remove, exists). Used by both CLI and Tauri app, eliminating duplicated disk I/O logic. Injected into `PluginRegistry` at construction time for testability. `dir()` accessor exposes the plugin directory path (used by `AppState::reload_plugins()` to reconstruct a fresh store)
- `settings::Settings` — typed representation of `~/.mcpviews/config.json` with `load()` and `save()` methods. Fields: optional legacy `registry_url`, and `registry_sources` vec. Replaces raw `serde_json::Value` handling in settings commands
- `token_store` module — `StoredToken` struct with `load_stored_token()`, `load_stored_token_unvalidated()`, `store_token()`, `has_stored_token()`, and expiry checking. `load_stored_token_unvalidated()` returns expired tokens without filtering (used by OAuth refresh to retrieve the refresh_token from an expired entry). Centralizes all token file I/O (read, write, existence check, expiry detection) previously duplicated across `PluginAuth` match arms and `auth.rs`
- `registry` module — `get_configured_registry_url()`, `fetch_registry()`, `get_registry_sources()`, `save_registry_sources()`, and `fetch_all_registries()` with per-source 1-hour disk caching. Supports multi-source registry configuration with fallback to legacy single `registry_url`. When all remote sources fail, falls back to a bundled registry (`bundled_registry.json`) compiled into the binary via `include_str!`. Shared by both CLI and Tauri app
- `package` module — ZIP plugin package handling: `extract_plugin_zip()` (with zip-slip protection, GitHub-style prefix stripping, manifest validation), `download_and_install_plugin()` (download + extract + install), and `install_from_local_zip()`. Max download size: 50MB

### CLI (`cli/`)

Standalone binary (`mcpviews-cli`) for headless plugin management. Commands: `list`, `add`, `remove`, `add-custom`, `search`. Shares `mcpviews-shared` types with the Tauri app. See [docs/cli.md](cli.md).

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
The axum server runs on `std::thread::spawn` with its own `tokio::Runtime`, not `tauri::async_runtime::spawn`. This is necessary because Tauri's main thread runs the GTK event loop, and `tauri::async_runtime::spawn` tasks don't execute until after WebKit2GTK initializes (which can take 20+ seconds on some systems). The TCP listener is pre-bound synchronously in the Tauri `setup()` hook (on the main thread) and then passed to the HTTP server thread. This ensures the OS kernel is accepting connections on port 4200 before the async runtime even starts, eliminating race conditions where MCP clients (e.g., Claude Code) probe the server before it is ready.

### Multi-Session Tab Bar
Each push creates a new session tab in a Chrome-style tab bar. Clicking a tab switches to its cached content; closing a tab dismisses the session. Review tabs display a countdown timer that resets on user activity (click, scroll, keydown). Tab labels use `data.title` (falling back to `data.name`, then `toolArgs.title`, then `toolName`) for clarity.

### Review Workflow
For `reviewRequired: true` pushes, the HTTP handler:
1. Creates a `tokio::oneshot` channel
2. Stores the sender in `ReviewState`
3. Drops the async lock and `await`s the receiver (with timeout)
4. When the user clicks accept/reject in the WebView, `submit_decision` IPC command resolves the channel
5. The HTTP response is sent back to the MCP agent

### Window Management
- Main window created programmatically via `WebviewWindowBuilder` (not declaratively in `tauri.conf.json`) to support dynamic CSP injection via `on_web_resource_request` hooks
- Close → hide to tray (not quit)
- Tray click → show + focus main window
- Push event → show + focus main window (automatic)
- Tray menu → "Show Window" / "Manage Plugins" / "Setup Agent Integrations" / "Quit"
- "Manage Plugins" opens a separate Plugin Manager window (`plugin-manager.html`, 800x600), also with dynamic CSP hooks
- Custom `plugin://` URI scheme serves plugin renderer assets from `~/.mcpviews/plugins/{name}/` with path traversal protection, MIME type detection, and `Cache-Control: no-store` to prevent stale caching
- On plugin install, `reload_renderers` event triggers a webview reload so the main window picks up new CSP origins

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
