# MCPViews CLI Reference

## Installation

### From crates.io

```bash
cargo install mcpviews-cli
```

### From source

```bash
git clone https://github.com/DeeJanuz/mcpviews.git
cd mcpviews/cli
cargo build --release
# Binary is at target/release/mcpviews-cli
```

## Commands

All commands are under the `plugin` subcommand:

```
mcpviews-cli plugin <action>
```

### `plugin list`

List all installed plugins.

```bash
mcpviews-cli plugin list
```

**Example output:**

```
Installed Plugins:
  ludflow  0.1.0  bearer auth  http://localhost:4200/mcp
```

Shows each plugin's name, version, authentication type, and MCP server URL. If no plugins are installed, prints "No plugins installed."

### `plugin add <name>`

Install a plugin from the registry by name.

```bash
mcpviews-cli plugin add ludflow
```

**Example output:**

```
Installed plugin 'ludflow' v0.1.0
```

This fetches the registry, finds the entry matching `<name>`, and writes its manifest to `~/.mcpviews/plugins/<name>.json`. If the plugin is not found, the CLI prints available plugins and exits with an error.

### `plugin remove <name>`

Remove an installed plugin.

```bash
mcpviews-cli plugin remove ludflow
```

**Example output:**

```
Removed plugin 'ludflow'.
```

Deletes the manifest file from `~/.mcpviews/plugins/`. If the plugin is not installed, the CLI exits with an error.

### `plugin add-custom <path>`

Install a plugin from a local JSON manifest file.

```bash
mcpviews-cli plugin add-custom ./my-plugin-manifest.json
```

**Example output:**

```
Installed custom plugin 'my-plugin' v1.0.0
```

Reads the manifest at `<path>`, validates it, and copies it to `~/.mcpviews/plugins/<name>.json` (where `<name>` is the `name` field from the manifest). This is useful for testing plugins during development or for private plugins not published to the registry.

### `plugin search [query]`

Search the plugin registry. If no query is given, lists all available plugins.

```bash
# List all plugins
mcpviews-cli plugin search

# Search by keyword
mcpviews-cli plugin search code-analysis
```

**Example output:**

```
Registry (1 plugin available):
  ludflow  0.1.0  Code analysis, documentation, and data governance powered by Ludflow
```

Search matches against plugin name, description, and tags. The match is case-insensitive.

## Configuration

### Registry URL

The CLI fetches the plugin registry from a default URL. To use a custom registry, create `~/.mcpviews/config.json`:

```json
{
  "registry_url": "https://your-server.com/registry.json"
}
```

The CLI reads this file on every registry operation. If the file does not exist or does not contain `registry_url`, the default URL is used.

### Data directories

| Path | Purpose |
|------|---------|
| `~/.mcpviews/plugins/` | Installed plugin manifests |
| `~/.mcpviews/config.json` | Configuration (registry URL) |
| `~/.mcpviews/cache/` | Cached registry data |
| `~/.mcpviews/auth/` | Stored authentication tokens |

## Interaction with Desktop App

The CLI and the desktop app share the same `~/.mcpviews/plugins/` directory and use the same `PluginStore` and `registry` modules from the `mcpviews-shared` crate. This means plugin CRUD operations and registry fetching (including the 1-hour disk cache) behave identically in both the CLI and desktop app.

Changes made by the CLI (adding or removing plugins) are picked up by the desktop app on the next plugin scan -- no restart is required. Both the CLI and desktop app now share the same registry cache on disk, so a fetch by either one benefits the other.
