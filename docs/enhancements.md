# Technical Debt & Enhancement Log

**Last Updated:** 2026-03-26
**Total Active Issues:** 3
**Resolved This Month:** 20

---

## Active Issues

### Critical

_None_

### High

_None_

### Medium

#### M-010: AppState carries test-only `plugins_dir_override` field in production struct
- **File(s):** `src-tauri/src/state.rs`
- **Principle:** SRP / Clean Architecture
- **Description:** The `plugins_dir_override: Option<PathBuf>` field and the conditional branch in `reload_plugins()` exist solely to support test injection. This leaks test infrastructure into the production struct. A cleaner approach would store the `PluginStore` itself as a field on `AppState` and always use it for reloads, eliminating the conditional entirely.
- **Suggested Fix:** Replace `plugins_dir_override: Option<PathBuf>` with `plugin_store: PluginStore` as a permanent field. `reload_plugins()` would always use `self.plugin_store` instead of branching. `new()` initializes with the default store; `new_with_store()` accepts a custom one.
- **Detected:** 2026-03-26 (commit a0ed7b5)

### Low

#### L-005: Hardcoded URL in mcp_mux_entry_for Claude Desktop case diverges from $MCP_MUX_URL variable
- **File(s):** `src-tauri/scripts/setup-integrations.sh`
- **Principle:** DRY / Single Source of Truth
- **Description:** The Claude Desktop case in `mcp_mux_entry_for()` (line 137) hardcodes `http://localhost:4200/mcp` inside a single-quoted heredoc, while all other cases reference `$MCP_MUX_URL`. If the variable is changed at the top of the script, the Claude Desktop entry will silently use the stale URL.
- **Suggested Fix:** Use an unquoted heredoc or `echo` with variable interpolation so the Claude Desktop case also references `$MCP_MUX_URL`.
- **Detected:** 2026-03-26 (commit 84e0e57)

#### L-004: Duplicated test helpers across commands.rs and state.rs
- **File(s):** `src-tauri/src/commands.rs`, `src-tauri/src/state.rs`
- **Principle:** DRY
- **Description:** `test_manifest()` and `test_app_state()` are defined identically in both test modules. As more test modules are added, this duplication will grow.
- **Suggested Fix:** Create a `#[cfg(test)] pub mod test_helpers` in a shared location (e.g., `state.rs` or a dedicated `test_utils.rs`) and import from both test modules.
- **Detected:** 2026-03-26 (commit a0ed7b5)

---

## Resolved Issues

### Resolved 2026-03-26 (commit a0ed7b5)

- **H-006:** No tests for Tauri commands and AppState -- added `AppState::new_with_store()` constructor for testable construction with temp dirs and `PluginStore::dir()` accessor; 10 unit tests added in `commands.rs` and `state.rs` covering command business logic (`get_health`, `install_or_update_from_entry`, plugin install/uninstall logic, `list_plugins_with_updates`) and AppState operations (`new_with_store`, `notify_tools_changed`, `reload_plugins`)

### Resolved 2026-03-26 (commit 5a83547)

- **M-008:** call_setup_agent_rules has three responsibilities -- extracted `collect_rules`, `collect_plugin_auth_status`, and `persistence_instructions` as separate pure functions
- **M-009:** Duplicated OAuth refresh-and-log pattern -- extracted `try_refresh_oauth` helper in `plugin.rs`, used by both `lookup_plugin_tool` and `refresh_stale_plugins`
- **L-003:** find_plugin_for_tool returns a 5-element tuple -- replaced with `PluginToolResult` struct with named fields
- **H-007:** No tests for setup_agent_rules or build_instructions -- extracted `collect_rules`, `collect_plugin_auth_status`, and `persistence_instructions` as testable helpers; 13 unit tests added covering all three functions

### Resolved 2026-03-26 (commit 6c7538b)

- **H-005:** Duplicated install/update orchestration in commands.rs -- extracted `install_or_update_from_entry` helper used by both `install_plugin_from_registry` and `update_plugin`
- **M-003:** PluginStore instantiated as concrete dependency in PluginRegistry methods -- `PluginStore` now injected as a field via `load_plugins_with_store(store)` constructor
- **M-006:** detect_content_type is effectively dead code -- replaced with `const CONTENT_TYPE: &str = "rich_content"`
- **M-007:** reload_plugins_handler mixes HTTP and plugin lifecycle concerns -- extracted `AppState::reload_plugins()` method, handler now delegates
- **L-002:** Settings stored/loaded as raw serde_json::Value -- replaced with typed `Settings` struct in `shared/src/settings.rs`
- **H-006 (partial):** No tests for McpSessionManager -- 14 unit tests added covering creation, broadcast, subscribe, removal, and retain_active

### Resolved 2026-03-25 (commit 102813b)

- **M-004:** Token reading logic duplicated across PluginAuth match arms and auth module -- extracted to `shared/src/token_store.rs` with `load_stored_token`, `store_token`, `has_stored_token`
- **M-005:** PluginAuth accumulating multiple responsibilities -- filesystem I/O extracted to `token_store` module, `PluginAuth` now delegates instead of doing inline JSON parsing

### Resolved 2026-03-25 (commit e4ca382)

- **H-001:** CLI duplicates registry fetch logic from Tauri backend -- extracted to `shared/src/registry.rs`
- **H-002:** CLI duplicates plugin add/remove filesystem logic -- extracted to `shared/src/plugin_store.rs`
- **H-003:** PluginRegistry God class -- split into `PluginRegistry` (coordination) + `ToolCache` (caching) + `PluginStore` (disk I/O)
- **H-004:** No tests for any new functionality -- 32 tests added across workspace
- **M-001:** Auth type matching uses string literals -- centralized in `PluginAuth::display_name()` + `Display` impl
- **M-002:** OAuth token expiry not checked on load -- expiry checks added in both `load_token()` and `resolve_header()`
- **L-001:** Settings saved to localStorage instead of config file -- frontend now uses Tauri IPC to persist to `config.json`

---

## Review History

| Commit | Date | Score | Rating |
|--------|------|-------|--------|
| 84e0e57 | 2026-03-26 | 78/100 | Good |
| abd466b | 2026-03-26 | 90/100 | Excellent |
| a0ed7b5 | 2026-03-26 | 82/100 | Good |
| 5a83547 | 2026-03-26 | 88/100 | Good |
| ebb9643 | 2026-03-26 | 68/100 | Acceptable |
| 6c7538b | 2026-03-26 | 85/100 | Good |
| 0fb86a3 | 2026-03-26 | 52/100 | Acceptable |
| 102813b | 2026-03-25 | 88/100 | Good |
| 6ebae60 | 2026-03-25 | 58/100 | Acceptable |
| e4ca382 | 2026-03-25 | 82/100 | Good |
| ba492ce | 2026-03-25 | 42/100 | Needs Improvement |
