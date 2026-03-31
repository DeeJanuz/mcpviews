# MCPViews -- Plugin System

## Overview

MCPViews supports a plugin system that allows third-party MCP servers to register their tools, renderers, and authentication configuration with the desktop app. Plugins are JSON manifest files stored in `~/.mcpviews/plugins/`. When loaded, MCPViews discovers the plugin's tools via MCP, maps tool outputs to the appropriate frontend renderers, and handles authentication automatically.

Plugins can be installed from the built-in registry (a remote JSON file listing available plugins) or added manually from a local manifest file. If all remote registry sources are unreachable, MCPViews falls back to a bundled registry (`shared/src/bundled_registry.json`) so that core plugins remain discoverable offline.

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
| `name` | string | Yes | Unique identifier for the plugin. Used as the filename in `~/.mcpviews/plugins/<name>.json`. |
| `version` | string | Yes | Semantic version of the plugin. |
| `renderers` | object | No | Map of MCP tool names to frontend renderer names. When a tool result arrives, MCPViews uses this mapping to select the correct renderer. If a tool is not listed, the default `rich_content` renderer is used. |
| `renderer_definitions` | RendererDef[] | **Recommended** | Structured renderer definitions with payload schemas for agent discovery. Each entry defines a renderer's name, description, scope, associated tools, data schema hint, and optional behavioral rule. Without these, agents can discover renderer names (via auto-discovery from the `renderers` map) but won't know how to construct payloads. See [Agent Discovery](#agent-discovery) below. |
| `tool_rules` | object | No | Map of tool names to behavioral rule strings. These rules are returned by the `get_plugin_docs` and `mcpviews_setup` MCP tools so agents can persist them for guided tool usage. Tool names are automatically prefixed with the plugin's `tool_prefix`. |
| `no_auto_push` | string[] | No | **Deprecated.** Previously controlled which tools skipped auto-push. Auto-push has been removed entirely -- pushes now only happen via explicit `push_content`/`push_review` calls. Field is still accepted for backward compatibility but has no effect. |
| `registry_index` | object | No | Pre-authored compact index for the `init_session` plugin registry. Contains `summary` (string), `tags` (string[]), `tool_groups` (ToolGroupEntry[]), and `renderer_names` (string[]). If omitted, MCPViews auto-derives the index from the `renderers` map and tool cache. |
| `mcp` | object | No | MCP server connection configuration. If omitted, the plugin provides renderers only (no remote tools). |
| `prompt_definitions` | PromptDef[] | No | Plugin prompt definitions for guided workflows. Each entry defines a prompt that can be discovered via the MCP `prompts/list` protocol and fetched via `get_plugin_prompt` or `prompts/get`. Prompts are markdown files bundled with the plugin that support `{{arg}}` template substitution. |
| `download_url` | string | No | URL to a ZIP package for this plugin version. Used by `manifest_url`-based registry entries and the `update_plugins` tool. |

### RendererDef

Structured renderer definition used for agent rule bootstrapping via the `get_plugin_docs` and `mcpviews_setup` MCP tools.

```json
{
  "name": "custom_view",
  "description": "Custom visualization for analysis results",
  "scope": "universal",
  "tools": [],
  "data_hint": "{ \"title\": \"string\", \"body\": \"markdown\" }",
  "rule": "When displaying analysis results, use push_content with tool_name 'custom_view'."
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Renderer key used in `tool_name` when calling `push_content`. |
| `description` | string | Yes | Human-readable description for agents. |
| `scope` | string | No | `"universal"` (any agent can use it) or `"tool"` (tied to specific MCP tools). Defaults to `"tool"`. |
| `tools` | string[] | No | For tool-scoped renderers: which tool names trigger this renderer. |
| `data_hint` | string | No | Data schema hint for agents (e.g., `"{ title: string, body: markdown }"`). |
| `rule` | string | No | Behavioral rule text returned by `get_plugin_docs`/`mcpviews_setup` for agent persistence. |

### PromptDef

Plugin prompt definition for guided workflows discoverable via the MCP prompts protocol.

```json
{
  "name": "onboarding",
  "description": "Guided setup workflow for this plugin",
  "source": "prompts/onboarding.md",
  "arguments": [
    { "name": "project_id", "description": "Target project ID", "required": true }
  ]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Prompt name. Exposed via `prompts/list` as `{plugin}/{name}`. |
| `description` | string | Yes | Human-readable description shown in prompt listings. |
| `source` | string | Yes | Relative path to the prompt markdown file within the plugin directory. |
| `arguments` | PromptArg[] | No | Template arguments. Each argument's `{{name}}` placeholder in the source file is replaced with the provided value. |

Each `PromptArg` has:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Argument name (used as `{{name}}` placeholder). |
| `description` | string | Yes | Human-readable description. |
| `required` | boolean | No | Whether the argument is required. Defaults to false. |

### Agent Discovery

MCPViews uses a two-tier lazy-loading approach for plugin documentation:

1. **`init_session`** returns only built-in (universal) rules and a compact `plugin_registry` index listing each plugin's name, summary, tags, tool groups, and renderer names. This keeps session-start token usage minimal.

2. **`get_plugin_docs`** fetches detailed rules for a specific plugin on-demand, with optional filters by tool group, tool name, or renderer name. Agents call this when they need to use a plugin's tools or renderers.

3. **`prompts/list`** and **`prompts/get`** expose plugin prompts via the standard MCP prompts protocol. Plugin prompts are namespaced as `{plugin}/{prompt}` and can also be fetched via the `get_plugin_prompt` tool.

The `plugin_registry` index is either read from the manifest's `registry_index` field (if provided) or auto-derived from the `renderers` map and tool cache. Auto-derivation groups tools by their mapped renderer, title-cases the group names, and uses truncated tool descriptions as hints.

MCPViews automatically discovers plugin renderers by reading the `renderers` map and enriching entries with tool metadata from the MCP tool cache. This gives agents the renderer names and tool associations without any extra plugin configuration.

However, **auto-discovery cannot infer payload shapes**. The tool cache contains tool *input* schemas (what you send to the tool), not renderer *data* schemas (what the renderer expects to display). Without `renderer_definitions` entries that include `data_hint`, agents will know a renderer exists but won't know how to format the `data` object when calling `push_content`.

**Best practice:** Provide a `renderer_definitions` entry with a `data_hint` for every renderer in your `renderers` map.

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
| `type` | `"OAuth"` or `"oauth"` | Yes | Selects OAuth browser redirect flow. Both casings are accepted. |
| `client_id` | string | No | OAuth client ID. Optional if the provider does not require one. |
| `auth_url` | string | Yes | Authorization endpoint URL. The user's browser is opened to this URL. |
| `token_url` | string | Yes | Token exchange endpoint URL. |
| `scopes` | string[] | No | OAuth scopes to request. Defaults to empty. |

## ZIP Plugin Packages

Plugins can be distributed as ZIP archives containing a `manifest.json` and optional assets (renderers, icons, etc.). The ZIP format supports:

- **GitHub release pattern**: If all files share a single top-level directory, it is automatically stripped during extraction
- **Zip-slip protection**: Paths containing `..` are rejected
- **Manifest validation**: The ZIP must contain a valid `manifest.json`
- **Max download size**: 50MB for remote downloads

Plugins are extracted to `~/.mcpviews/plugins/{plugin-name}/` as a directory (rather than a single JSON file). The `manifest.json` inside the directory is used for plugin configuration.

### Custom Renderers

Plugin ZIP packages can include custom renderer JS files in a `renderers/` subdirectory. These are served via the `plugin://` URI scheme and discovered by the renderer scanner.

```
my-plugin.zip
  manifest.json
  renderers/
    my-custom-view.js
```

Renderer files are accessible at `plugin://localhost/{plugin-name}/renderers/{file-name}.js`.

## Registry Format

The plugin registry is a JSON file hosted at a remote URL. MCPViews ships with a default registry URL but this can be overridden (see Custom Registries and Multiple Registry Sources below).

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
| `manifest` | PluginManifest | Yes | The full plugin manifest that gets installed to `~/.mcpviews/plugins/`. |
| `download_url` | string | No | URL to a ZIP package. If present, the plugin is downloaded and extracted instead of using the manifest alone. |
| `manifest_url` | string | No | URL to the provider's remote manifest.json. When present, MCPViews fetches this to get current version and download URL, instead of relying on the inline `manifest`. Enables providers to manage versions from their own repo. |

## Custom Registries

By default, MCPViews fetches the plugin registry from:

```
https://raw.githubusercontent.com/DeeJanuz/mcpviews/master/registry/registry.json
```

To use a different registry, create or edit `~/.mcpviews/config.json`:

```json
{
  "registry_url": "https://your-server.com/registry.json"
}
```

Both the CLI and the desktop app read `registry_url` from this config file. If the key is absent or the file does not exist, the default URL is used.

## Multiple Registry Sources

MCPViews supports multiple registry sources. Sources are stored in `~/.mcpviews/config.json` under the `registry_sources` key:

```json
{
  "registry_sources": [
    { "name": "Default", "url": "https://raw.githubusercontent.com/.../registry.json", "enabled": true },
    { "name": "Internal", "url": "https://corp.example.com/registry.json", "enabled": true }
  ]
}
```

When multiple sources are configured, MCPViews fetches from all enabled sources and merges the results. If two sources provide a plugin with the same name, the last source wins. Each source has its own disk cache with a 1-hour TTL. If all remote sources fail, the bundled registry is used as a final fallback.

The legacy single `registry_url` key is automatically migrated: if `registry_sources` is absent but `registry_url` is present, it is treated as a single default source. When sources are saved via the API, the `registry_url` key is removed.

Sources can be managed via the IPC commands: `get_registry_sources`, `add_registry_source`, `remove_registry_source`, `toggle_registry_source`.

## Developing a Plugin

To create an MCP server that works with MCPViews:

### 1. Build an MCP-compatible server

Your server must implement the [Model Context Protocol](https://spec.modelcontextprotocol.io/) and expose an HTTP endpoint. MCPViews connects to the server's URL and discovers available tools via the MCP `tools/list` method.

### 2. Choose renderers for your tools

MCPViews includes built-in renderers for general-purpose content:

| Renderer | Description |
|----------|-------------|
| `rich_content` | Markdown + mermaid fallback (default) |
| `document_preview` | Rendered markdown document |
| `document_diff` | Two-column diff with accept/reject |
| `citation_panel` | Citation list (used as sub-component) |

Domain-specific renderers (code analysis, data governance, knowledge management) are delivered via plugins. For example, the [Ludflow plugin](https://github.com/DeeJanuz/ludflow-mcpviews) provides renderers for `search_results`, `code_units`, `data_schema`, `column_context`, `module_overview`, `analysis_stats`, `knowledge_dex`, `data_draft_diff`, `dependencies`, and `file_content`.

Map each of your tools to the renderer that best fits its output in the `renderers` field.

### 3. Configure authentication

Choose the auth type that matches your server:

- **Bearer token** -- simplest option. After install, a modal prompts the user to enter their token. The token is stored in `~/.mcpviews/auth/<plugin-name>.json`. Falls back to reading from the environment variable if no stored token exists.
- **API key header** -- for services that use a custom header name. Same storage and fallback behavior as bearer tokens.
- **OAuth** -- for services requiring browser-based login. MCPViews handles the redirect flow. Tokens are stored in `~/.mcpviews/auth/` and checked for expiry on each use. **Automatic token refresh**: when an OAuth token expires and a `refresh_token` is available, MCPViews automatically attempts a `refresh_token` grant before making plugin API calls. If refresh succeeds, the new token is stored to disk and the call proceeds transparently. If refresh fails, auth status and re-authentication URLs are surfaced through both MCP `initialize` instructions and the `init_session` tool response, so agents can direct users to re-authenticate.

Auth resolution is centralized in the `PluginAuth::resolve_header()` method in the shared crate, which delegates all token file I/O to the `token_store` module (`shared/src/token_store.rs`). For Bearer and API Key auth, the resolution order is:

1. **Stored token** -- `token_store::load_stored_token()` reads `~/.mcpviews/auth/<plugin-name>.json`, deserializes it as a `StoredToken`, and checks expiry. Expired tokens return `None`.
2. **Environment variable** -- fall back to the configured `token_env` / `key_env` variable

For OAuth, `token_store::load_stored_token()` handles the full cycle: load, deserialize, expiry check. Token storage after OAuth flows uses `token_store::store_token()`. Token removal (for re-auth or uninstall cleanup) uses `token_store::remove_token()`, which deletes the token file and succeeds silently if no file exists.

This means users no longer need to set environment variables manually. After installing a plugin that requires Bearer or API Key auth, MCPViews immediately prompts for the token via a modal dialog. The token can also be configured later via the Plugin Manager -- "Configure Auth" for first-time setup or "Re-auth" for plugins with existing tokens.

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
mcpviews-cli plugin add-custom ./my-analysis-tool.json
```

Then call one of your tools via the push API to verify the renderer displays correctly.

### 6. Publish to the registry

To list your plugin in the official registry, submit a pull request adding a `RegistryEntry` to the registry's `registry.json` file. Include a description, tags, and your manifest.

## Plugin Lifecycle

### Discovery

When MCPViews starts, a `PluginStore` instance (from the shared crate) is injected into `PluginRegistry` at construction time. The registry scans `~/.mcpviews/plugins/` for JSON manifest files, loads each valid manifest, and registers its MCP configuration. The same `PluginStore` is used by both the CLI and Tauri app for all plugin CRUD operations. The `PluginRegistry::load_plugins_with_store()` constructor accepts a custom `PluginStore` for testing.

### Tool Caching

After connecting to a plugin's MCP server, MCPViews calls `tools/list` to discover available tools. The tool list is cached locally in `~/.mcpviews/cache/` to avoid repeated discovery calls on startup.

### Cache TTL

Two caches operate with different TTLs:
- **Registry cache**: 1-hour TTL (`3600` seconds). After expiry, the next registry fetch requests fresh data from the remote URL. The cached version is used as a fallback if the remote is unreachable. This cache is shared between CLI and Tauri via the `registry` module in the shared crate.
- **Tool cache**: 5-minute TTL (`300` seconds). Per-plugin tool lists fetched via MCP `tools/list` are cached in memory by the `ToolCache` struct. Stale entries are refreshed automatically on the next poll cycle.

### Runtime Add/Remove

Plugins can be added or removed at runtime via the CLI, GUI, or the `mcpviews_install_plugin` MCP tool (allowing agents to install plugins programmatically). When a plugin is added:

1. The manifest is written to `~/.mcpviews/plugins/<name>.json` (or extracted to `~/.mcpviews/plugins/<name>/` for ZIP packages)
2. The desktop app detects the change and loads the new plugin
3. Tools from the plugin become available immediately

When a plugin is removed:

1. The manifest file is deleted from `~/.mcpviews/plugins/`
2. Any stored auth token at `~/.mcpviews/auth/<name>.json` is deleted
3. The desktop app unloads the plugin
4. Cached tool data for the plugin is cleared

### Plugin Updates

The `list_plugins` command now compares installed plugin versions against the cached registry and returns an `update_available` field with the new version string when an update exists. The `update_plugin` command downloads and installs the latest version from the registry, replacing the existing plugin.

### Plugin Reinstall

Installed plugins can be reinstalled from the registry via the "Reinstall" button in the Plugin Manager or the `reinstall_plugin` IPC command. For registry plugins, this re-downloads and installs the current registry version, replacing the local copy. For local-only plugins (not in any registry), the command verifies the plugin exists but does not re-download.

### Re-authentication

Plugins with authentication can be re-authenticated via the "Re-auth" button in the Plugin Manager. This first clears the existing stored token (via `clear_plugin_auth`) and then triggers the standard auth configuration flow (token prompt for Bearer/API Key, browser redirect for OAuth). The button label shows "Configure Auth" for plugins that have not yet been authenticated and "Re-auth" for plugins with existing auth tokens.

### Hot Reload

`POST /api/reload-plugins` calls `AppState::reload_plugins()`, which reloads all plugins from disk and broadcasts a `notifications/tools/list_changed` JSON-RPC notification to all active MCP SSE sessions. This method is also used internally whenever plugin state needs refreshing. Connected MCP clients can listen for this notification to refresh their tool lists without reconnecting.
