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
| `http_server.rs` | axum HTTP server on `:4200`. Routes: `GET /health`, `POST /api/push`. Runs on a dedicated thread with its own tokio runtime to avoid blocking the GTK event loop |
| `session.rs` | `SessionStore` — in-memory `HashMap<String, PreviewSession>` with 30-minute TTL and 60s GC interval |
| `review.rs` | `ReviewState` — pending review management via `tokio::oneshot` channels. `add_pending()` returns a receiver; `resolve()` or `dismiss()` sends the decision |
| `commands.rs` | Tauri IPC commands: `get_sessions`, `submit_decision`, `dismiss_session`, `get_health`, plus 6 plugin management commands (`list_plugins`, `install_plugin`, `uninstall_plugin`, `install_plugin_from_file`, `fetch_registry`, `start_plugin_auth`) |
| `state.rs` | `AppState` — shared state containing `Mutex<SessionStore>`, `Mutex<ReviewState>`, `Mutex<PluginRegistry>`, and `reqwest::Client` |
| `registry.rs` | Remote plugin registry client — fetches GitHub-hosted JSON index with 1-hour cache |
| `auth.rs` | Plugin authentication — OAuth browser-redirect flow with ephemeral localhost callback server, plus Bearer and API key resolution |

### Frontend (`src/` + `public/`)

The WebView loads `index.html` which includes:
- CDN scripts: `marked.js` (markdown), `mermaid` (diagrams)
- `styles.css` — all styling (ported from companion)
- `main.js` — app bootstrap, Tauri IPC event listener, session/queue management
- `renderers/*.js` — 14 content-type renderers (ported unchanged from companion)
- `plugin-manager.js` — Plugin Manager window logic (registry browser, installed list, settings)

**Key change from companion**: WebSocket replaced with Tauri IPC:
- Receive: `window.__TAURI__.event.listen('push_preview', callback)`
- Send: `window.__TAURI__.core.invoke('submit_decision', payload)`

### Shared Types (`shared/`)

`mcp-mux-shared` crate consumed by both the Tauri backend and CLI. Defines:
- `PluginManifest`, `PluginMcpConfig` — plugin definition and MCP connection config
- `PluginAuth` — tagged enum: `Bearer { token_env }`, `ApiKey { header_name, key_env }`, `OAuth { client_id, auth_url, token_url, scopes }`
- `RegistryEntry`, `RemoteRegistry` — remote registry schema
- `PluginInfo` — lightweight plugin summary for IPC
- Path helpers: `plugins_dir()`, `config_path()`, `auth_dir()`, `cache_dir()` — all under `~/.mcp-mux/`

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

## API Compatibility

The HTTP push API on `:4200` is fully compatible with the existing MCP server push logic:
- Same `POST /api/push` request shape (`PushRequest`)
- Same response shape (`PushResponse`)
- Same review timeout behavior (408 on timeout)
- Same CORS headers
- `GET /health` returns version and uptime

No changes needed on the MCP server side — it just POSTs to localhost:4200.
