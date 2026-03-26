# MCP Mux

A standalone Tauri v2 desktop app that serves as a rich display for AI agents. Replaces the companion Node.js server with a native app featuring system tray, auto-start, and a built-in HTTP push API.

## Architecture

- **Rust backend** (axum): HTTP server on `:4200` for push API + review workflow
- **WebView frontend**: Vanilla JS renderers for core content types (rich content, document preview, citations); domain-specific renderers delivered via plugins
- **Node.js sidecar**: SSE bridge for remote server connections
- **System tray**: Hide-to-tray, click to show, auto-start on login

## Installation

**macOS / Windows**: Download the latest release from [Releases](https://github.com/DeeJanuz/mcp-mux/releases).

**Linux** (build from source):

```bash
# Prerequisites: Rust, Node.js 20+, and system libraries
# Debian/Ubuntu:
sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf

# Fedora:
sudo dnf install webkit2gtk4.1-devel libappindicator-gtk3-devel librsvg2-devel

# Arch:
sudo pacman -S webkit2gtk-4.1 libappindicator-gtk3 librsvg

# Build
git clone https://github.com/DeeJanuz/mcp-mux.git
cd mcp-mux
npm install
npm run build

# The binary is at src-tauri/target/release/mcp-mux
# Or install the Tauri CLI for a bundled .deb/.AppImage:
cargo install tauri-cli
cargo tauri build
```

## Development

```bash
# Install dependencies
npm install

# Dev mode (hot reload frontend + Rust backend)
npm run dev

# Build frontend only
npm run build:frontend

# Build Rust backend only (from src-tauri/)
cargo build

# Build full Tauri app (frontend + backend + installer)
npm run build
```

## Testing the Push API

```bash
# Health check
curl http://localhost:4200/health

# Push rich content
curl -X POST http://localhost:4200/api/push \
  -H 'Content-Type: application/json' \
  -d '{"toolName":"rich_content","result":{"data":{"title":"Test","body":"## Hello\n\nThis is a test."}}}'

# Push with review (blocks until user decides)
curl -X POST http://localhost:4200/api/push \
  -H 'Content-Type: application/json' \
  -d '{"toolName":"write_document","result":{"data":{"operations":[{"type":"replace","target":"Introduction","replacement":"New intro text"}]}},"reviewRequired":true}'
```

## SSE Sidecar

Connects to a remote app's companion stream and forwards events to the local HTTP server.

```bash
# Build
cd sidecar && bash build.sh

# Run
node sidecar/dist/sse-bridge.mjs --app-host https://app.example.com --key lf_companion_xxx
```

## Project Structure

```
mcp-mux/
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── main.rs         # Tauri entry, tray, plugin setup
│   │   ├── http_server.rs  # axum HTTP server (:4200)
│   │   ├── session.rs      # In-memory session store
│   │   ├── review.rs       # Pending review channels (oneshot)
│   │   ├── commands.rs     # Tauri IPC commands
│   │   └── state.rs        # Shared app state
│   ├── Cargo.toml
│   └── tauri.conf.json
├── src/                    # Frontend (Vite entry)
│   └── index.html          # HTML shell
├── public/                 # Static assets (copied to dist)
│   ├── main.js             # App bootstrap (Tauri IPC)
│   ├── styles.css          # All styles
│   └── renderers/          # Built-in content renderers
├── sidecar/                # Node.js SSE bridge
│   ├── sse-bridge.ts
│   └── build.sh
├── registry/               # Plugin registry
│   └── registry.json       # Default registry with available plugins
├── cli/                    # CLI plugin manager
│   └── src/main.rs
├── shared/                 # Shared types (manifest, auth, registry)
│   └── src/lib.rs
├── package.json
└── vite.config.ts
```

## Plugin System

MCP Mux supports plugins that extend the app with tools from third-party MCP servers. Each plugin is a JSON manifest that declares renderer mappings, MCP server configuration, and authentication. Plugins are stored as individual JSON files in `~/.mcp-mux/plugins/`.

For full documentation, see [docs/plugins.md](docs/plugins.md). For a step-by-step guide to creating your own plugin, see [docs/plugin-development.md](docs/plugin-development.md).

## Installing Plugins

### Via GUI

Open the system tray menu and select **Manage Plugins**. From there you can:

- Browse the plugin registry to discover and install available plugins
- Add a custom plugin from a local manifest file
- View installed plugins and remove them

### Via CLI

```bash
# Search the registry
mcp-mux-cli plugin search

# Install a plugin from the registry
mcp-mux-cli plugin add ludflow

# List installed plugins
mcp-mux-cli plugin list

# Install from a local manifest file
mcp-mux-cli plugin add-custom ./my-plugin.json

# Remove a plugin
mcp-mux-cli plugin remove ludflow
```

For full CLI documentation, see [docs/cli.md](docs/cli.md).

## Plugin Manifest Format

A plugin manifest is a JSON file with renderer mappings and MCP configuration:

```json
{
  "name": "my-plugin",
  "version": "1.0.0",
  "renderers": {
    "tool_name": "renderer_name"
  },
  "mcp": {
    "url": "http://localhost:8080/mcp",
    "auth": {
      "type": "bearer",
      "token_env": "MY_API_KEY"
    },
    "tool_prefix": "myplugin_"
  }
}
```

| Field | Description |
|-------|-------------|
| `name` | Unique plugin identifier |
| `version` | Semantic version |
| `renderers` | Maps MCP tool names to frontend renderers |
| `mcp.url` | MCP server endpoint |
| `mcp.auth` | Authentication config (`bearer`, `api_key`, or `oauth`) |
| `mcp.tool_prefix` | Prefix for tool names to avoid collisions |

Three auth types are supported: **bearer token** (env var), **API key** (custom header + env var), and **OAuth** (browser redirect flow). See [docs/plugins.md](docs/plugins.md) for the full schema reference.

## CLI Reference

| Command | Description |
|---------|-------------|
| `mcp-mux-cli plugin list` | List installed plugins |
| `mcp-mux-cli plugin add <name>` | Install a plugin from the registry |
| `mcp-mux-cli plugin remove <name>` | Remove an installed plugin |
| `mcp-mux-cli plugin add-custom <path>` | Install from a local manifest file |
| `mcp-mux-cli plugin search [query]` | Search the plugin registry |

See [docs/cli.md](docs/cli.md) for full usage examples and configuration.
