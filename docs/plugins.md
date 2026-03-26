# MCP Mux -- Plugin System

## Overview

MCP Mux supports a plugin system that allows third-party MCP servers to register their tools, renderers, and authentication configuration with the desktop app. Plugins are JSON manifest files stored in `~/.mcp-mux/plugins/`. When loaded, MCP Mux discovers the plugin's tools via MCP, maps tool outputs to the appropriate frontend renderers, and handles authentication automatically.

Plugins can be installed from the built-in registry (a remote JSON file listing available plugins) or added manually from a local manifest file.

## Manifest Schema

A plugin manifest is a JSON file with the following structure:

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

### PluginManifest

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Unique identifier for the plugin. Used as the filename in `~/.mcp-mux/plugins/<name>.json`. |
| `version` | string | Yes | Semantic version of the plugin. |
| `renderers` | object | No | Map of MCP tool names to frontend renderer names. When a tool result arrives, MCP Mux uses this mapping to select the correct renderer. If a tool is not listed, the default `rich_content` renderer is used. |
| `mcp` | object | No | MCP server connection configuration. If omitted, the plugin provides renderers only (no remote tools). |

### PluginMcpConfig

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `url` | string | Yes | The MCP server endpoint URL. |
| `auth` | object | No | Authentication configuration. If omitted, no authentication is sent. |
| `tool_prefix` | string | Yes | Prefix added to tool names from this plugin to avoid collisions (e.g., `ludflow_search_codebase`). |

### PluginAuth

Authentication is configured via a tagged union on the `type` field. Three types are supported:

#### Bearer Token

```json
{
  "type": "bearer",
  "token_env": "MY_API_KEY"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | `"bearer"` | Yes | Selects bearer token authentication. |
| `token_env` | string | Yes | Name of the environment variable containing the bearer token. The token is read at runtime and sent as `Authorization: Bearer <token>`. |

#### API Key

```json
{
  "type": "api_key",
  "header_name": "X-API-Key",
  "key_env": "MY_SERVICE_KEY"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | `"api_key"` | Yes | Selects API key authentication. |
| `header_name` | string | No | HTTP header name for the key. Defaults to `X-API-Key`. |
| `key_env` | string | No | Name of the environment variable containing the API key. |

#### OAuth

```json
{
  "type": "oauth",
  "client_id": "abc123",
  "auth_url": "https://provider.com/authorize",
  "token_url": "https://provider.com/token",
  "scopes": ["read", "write"]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | `"oauth"` | Yes | Selects OAuth browser redirect flow. |
| `client_id` | string | Yes | OAuth client ID. |
| `auth_url` | string | Yes | Authorization endpoint URL. The user's browser is opened to this URL. |
| `token_url` | string | Yes | Token exchange endpoint URL. |
| `scopes` | string[] | No | OAuth scopes to request. Defaults to empty. |

## Registry Format

The plugin registry is a JSON file hosted at a remote URL. MCP Mux ships with a default registry URL but this can be overridden (see Custom Registries below).

### RemoteRegistry

```json
{
  "version": "1",
  "plugins": [
    {
      "name": "my-plugin",
      "version": "1.0.0",
      "description": "Short description of what the plugin does",
      "author": "Author Name",
      "homepage": "https://example.com",
      "tags": ["tag1", "tag2"],
      "manifest": { ... }
    }
  ]
}
```

### RegistryEntry

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Plugin name (must match the manifest's `name` field). |
| `version` | string | Yes | Plugin version. |
| `description` | string | Yes | Human-readable description shown in search results and the GUI. |
| `author` | string | No | Author or organization name. |
| `homepage` | string | No | URL to the plugin's homepage or documentation. |
| `tags` | string[] | No | Tags for search filtering (e.g., `["code-analysis", "documentation"]`). |
| `manifest` | PluginManifest | Yes | The full plugin manifest that gets installed to `~/.mcp-mux/plugins/`. |

## Custom Registries

By default, MCP Mux fetches the plugin registry from:

```
https://raw.githubusercontent.com/anthropics/mcp-mux-registry/main/registry.json
```

To use a different registry, create or edit `~/.mcp-mux/config.json`:

```json
{
  "registry_url": "https://your-server.com/registry.json"
}
```

Both the CLI and the desktop app read `registry_url` from this config file. If the key is absent or the file does not exist, the default URL is used.

## Developing a Plugin

To create an MCP server that works with MCP Mux:

### 1. Build an MCP-compatible server

Your server must implement the [Model Context Protocol](https://spec.modelcontextprotocol.io/) and expose an HTTP endpoint. MCP Mux connects to the server's URL and discovers available tools via the MCP `tools/list` method.

### 2. Choose renderers for your tools

MCP Mux includes built-in renderers for common content types:

| Renderer | Description |
|----------|-------------|
| `search_results` | Grouped search results with type chips |
| `code_units` | Source code with complexity badges |
| `document_preview` | Rendered markdown document |
| `document_diff` | Two-column diff with accept/reject |
| `data_schema` | Expandable table/column view |
| `data_draft_diff` | Grid-based draft review |
| `dependencies` | Grouped imports by source file |
| `file_content` | Source with line numbers |
| `module_overview` | File tree + exports + dependencies |
| `analysis_stats` | Metric cards + repository list |
| `knowledge_dex` | Table with bulk accept/reject |
| `column_context` | Breadcrumb navigation + related entities |
| `rich_content` | Markdown + mermaid fallback (default) |

Map each of your tools to the renderer that best fits its output in the `renderers` field.

### 3. Configure authentication

Choose the auth type that matches your server:

- **Bearer token** -- simplest option. After install, a modal prompts the user to enter their token. The token is stored in `~/.mcp-mux/auth/<plugin-name>.json`. Falls back to reading from the environment variable if no stored token exists.
- **API key header** -- for services that use a custom header name. Same storage and fallback behavior as bearer tokens.
- **OAuth** -- for services requiring browser-based login. MCP Mux handles the redirect flow. Tokens are stored in `~/.mcp-mux/auth/` and checked for expiry on each use; expired tokens are rejected rather than silently sent.

Auth resolution is centralized in the `PluginAuth::resolve_header()` method in the shared crate, which delegates all token file I/O to the `token_store` module (`shared/src/token_store.rs`). For Bearer and API Key auth, the resolution order is:

1. **Stored token** -- `token_store::load_stored_token()` reads `~/.mcp-mux/auth/<plugin-name>.json`, deserializes it as a `StoredToken`, and checks expiry. Expired tokens return `None`.
2. **Environment variable** -- fall back to the configured `token_env` / `key_env` variable

For OAuth, `token_store::load_stored_token()` handles the full cycle: load, deserialize, expiry check. Token storage after OAuth flows uses `token_store::store_token()`.

This means users no longer need to set environment variables manually. After installing a plugin that requires Bearer or API Key auth, MCP Mux immediately prompts for the token via a modal dialog. The token can also be configured later via the "Configure Auth" button in the Plugin Manager.

### 4. Create the manifest file

Write a JSON file following the PluginManifest schema above. Example:

```json
{
  "name": "my-analysis-tool",
  "version": "0.1.0",
  "renderers": {
    "analyze_code": "code_units",
    "search_files": "search_results"
  },
  "mcp": {
    "url": "http://localhost:9000/mcp",
    "auth": {
      "type": "bearer",
      "token_env": "MY_TOOL_API_KEY"
    },
    "tool_prefix": "mytool_"
  }
}
```

### 5. Test locally

Install your manifest with the CLI:

```bash
mcp-mux-cli plugin add-custom ./my-analysis-tool.json
```

Then call one of your tools via the push API to verify the renderer displays correctly.

### 6. Publish to the registry

To list your plugin in the official registry, submit a pull request adding a `RegistryEntry` to the registry's `registry.json` file. Include a description, tags, and your manifest.

## Plugin Lifecycle

### Discovery

When MCP Mux starts, the `PluginStore` (from the shared crate) scans `~/.mcp-mux/plugins/` for JSON manifest files. Each valid manifest is loaded and its MCP configuration is registered. The same `PluginStore` is used by both the Tauri app and the CLI for all plugin CRUD operations.

### Tool Caching

After connecting to a plugin's MCP server, MCP Mux calls `tools/list` to discover available tools. The tool list is cached locally in `~/.mcp-mux/cache/` to avoid repeated discovery calls on startup.

### Cache TTL

Two caches operate with different TTLs:
- **Registry cache**: 1-hour TTL (`3600` seconds). After expiry, the next registry fetch requests fresh data from the remote URL. The cached version is used as a fallback if the remote is unreachable. This cache is shared between CLI and Tauri via the `registry` module in the shared crate.
- **Tool cache**: 5-minute TTL (`300` seconds). Per-plugin tool lists fetched via MCP `tools/list` are cached in memory by the `ToolCache` struct. Stale entries are refreshed automatically on the next poll cycle.

### Runtime Add/Remove

Plugins can be added or removed at runtime via the CLI or GUI. When a plugin is added:

1. The manifest is written to `~/.mcp-mux/plugins/<name>.json`
2. The desktop app detects the change and loads the new plugin
3. Tools from the plugin become available immediately

When a plugin is removed:

1. The manifest file is deleted from `~/.mcp-mux/plugins/`
2. The desktop app unloads the plugin
3. Cached tool data for the plugin is cleared
