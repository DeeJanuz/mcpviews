# Technical Debt & Enhancement Log

**Last Updated:** 2026-04-03
**Total Active Issues:** 3
**Resolved This Month:** 59

---

## Active Issues

### Critical

_None_

### High

_None_

### Medium

- **M-028:** No async integration test coverage for `list_prompts`, or `get_prompt` -- `build_registry_entries` and `resolve_builtin_prompt` now have pure-function tests (a36294a), but remaining async functions (~5 code paths) still need integration tests. _(Commit 44e1f76, partially addressed 4d55dc6, a36294a)_
- **M-023:** No test coverage for `get_plugin_auth_header` command -- function has 3 code paths (stored token, OAuth refresh, no token error) with no tests. Requires integration test infrastructure. _(Commit 2565475)_

### Low

- **L-020:** No test coverage for `set_native_theme` command (`commands.rs`) -- function has 3 code paths (dark, light, unrecognized string) with no tests. Requires Tauri integration test infrastructure. _(Commit c5f6d1c)_

---

## Resolved Issues

### Resolved 2026-04-03

- **M-031:** `set_plugin_update_policy` and `get_plugin_update_policy` in `commands.rs` now accept `State<Arc<AppState>>` and use `state.plugin_store()` instead of constructing `PluginStore::new()` directly. Consistent with DIP pattern used by all other commands.
- **M-030:** Extracted `evaluate_update_preferences(plugin_updates, store) -> Value` as a pure function in `mcp_tools.rs`. `gather_slim_session_data` now delegates to it, improving SRP and testability.
- **M-029:** Added 7 unit tests for `evaluate_update_preferences` covering: no updates, default ask policy, always auto-update, skip matching version, skip different version re-ask, and mixed policies integration.

### Resolved 2026-03-31 (commit a36294a)

- **M-024 (complete):** Extracted `call_list_registry` and `call_start_plugin_auth` from `mcp_tools.rs` into new `mcp_registry_tools.rs` module (~160 lines moved). Extracted `build_registry_entries` as a pure synchronous function with 7 unit tests. `mcp_tools.rs` dispatch now delegates to `crate::mcp_registry_tools`.
- **M-027:** Added 5 unit tests for `newer_version` in `shared/src/lib.rs` covering all 4 code paths (newer available, same version, older available, invalid semver on either side).
- **M-028 (partial):** `build_registry_entries` (7 tests) and `resolve_builtin_prompt` (3 tests) now tested as pure functions. Remaining async integration tests tracked under active M-028.
- **L-019:** Extracted `normalize_data_param` helper from inline logic in `call_push_impl`, added 3 unit tests covering object passthrough, valid JSON string parsing, and invalid string fallback.

### Resolved 2026-03-31 (commit 4d55dc6)

- **M-024 (partial):** Extracted prompt system (`ONBOARDING_PROMPT`, `builtin_prompt_definitions`, `list_prompts`, `get_prompt`, `call_get_plugin_prompt`) from `mcp_tools.rs` into new `mcp_prompts.rs` module, reducing `mcp_tools.rs` by ~210 lines. `mcp.rs` now routes `prompts/list` and `prompts/get` to `crate::mcp_prompts`. Remaining: registry/auth tools still in `mcp_tools.rs`
- **M-022:** Duplicated auth-lookup block -- extracted `PluginRegistry::resolve_plugin_auth(&self, plugin_name) -> Result<PluginAuth, String>` in `plugin.rs`, replacing 3 identical 12-line blocks in `commands.rs` (`start_plugin_auth`, `get_plugin_auth_header`) and `mcp_tools.rs` (`call_start_plugin_auth`). Added 3 unit tests (not found, no auth config, success)
- **M-029:** `get_prompt` hard-coded match on `"onboarding"` -- `builtin_prompt_definitions()` now returns a 4-tuple `(name, description, arguments, content)` pairing content with definitions; `get_prompt` iterates the definitions list instead of matching on a string literal, so new built-in prompts only require adding a tuple entry
- **M-028 (partial):** Added 7 tests: 3 for `resolve_plugin_auth` in `plugin.rs`, 3 for tool definitions (`list_registry`, `start_plugin_auth`, `get_plugin_prompt`) in `mcp_tools.rs`, 1 for `builtin_prompt_definitions` in `mcp_prompts.rs`. Remaining async integration tests tracked under active M-028

### Resolved 2026-03-30 (commit 846d72e)

- **M-025:** `ensure_registry_fresh` double-resolved manifest URLs -- removed redundant `resolve_manifest_urls` call since `fetch_all_registries` already resolves internally
- **M-026:** Duplicated semver comparison logic across `commands.rs`, `mcp_tools.rs`, and `plugin.rs` -- extracted shared `newer_version()` helper into `shared/src/lib.rs`, all three call sites now use it

### Resolved 2026-03-29

- **M-021:** Duplicated `on_web_resource_request` CSP-injection closure in `main.rs` -- extracted `csp_request_hook(state)` helper function that returns the closure, used by both main and plugin-manager window builders _(Commit c88d26f → resolved)_

### Resolved 2026-03-29 (commit 8e9fc5f)

- **M-020:** `call_install_plugin` in `mcp_tools.rs` has no test coverage -- extracted `install_plugin_from_manifest()` on `AppState` for testability, added 5 unit tests covering manifest install, missing params, invalid JSON, upsert behavior, and schema description accuracy
- **L-018:** `call_install_plugin` calls `mcpviews_shared::plugins_dir()` global instead of using `plugin_store` from `AppState` -- replaced with `AppState::plugins_dir()` which delegates to `PluginStore::dir()`, consistent with prior M-010 refactoring

### Resolved 2026-03-28 (commit 4b0b747)

- **M-018:** No tests for drawer-stack, invocation-registry, or mcpview:// URI parsing -- added 26 vitest tests covering drawer-stack, invocation-registry, and mcpview:// URI parsing
- **M-019:** get_renderer_registry test duplicates filtering logic instead of calling the function -- extracted `collect_invocable_renderers()` so test calls real logic instead of duplicating it
- **L-017:** display_mode is stringly-typed Option<String> instead of an enum -- replaced with `DisplayMode` enum (Drawer/Modal/Replace) with serde rename attributes
- **L-014:** Large inline documentation strings in builtin_renderer_definitions() -- extracted `RICH_CONTENT_RULE` and `STRUCTURED_DATA_RULE` constants from inline strings
- **L-015:** Fragile positional index assertions in collect_rules tests -- replaced `rules[0]`/`rules[1]` positional indexing with `.iter().find()`
- **L-016:** Duplicated renderer hint iteration in builtin_tool_definitions -- extracted `build_data_description()` helper to DRY renderer hint iteration
- **L-011:** PluginStore reconstructed via with_dir instead of reused in AppState -- derived Clone on PluginStore, use `store.clone()` instead of reconstructing via `with_dir`

### Resolved 2026-03-28 (commit 9663b17)

- **M-015:** Duplicated dark mode CSS for mermaid-rendered and mermaid-modal-body -- consolidated using `:is(.mermaid-rendered, .mermaid-modal-body)` selectors, reducing ~100 lines of near-duplicate CSS to ~50 lines
- **M-016:** blocking_save_file called in async Tauri command -- replaced with async oneshot channel pattern and added proper error handling via `ok_or_else` instead of `unwrap()`
- **M-017:** No tests for CSV export, save_file command, or markdown toggle -- extracted `buildCsvString` to `structured-data-utils.js` and added 6 unit tests covering escaping, null handling, nested rows, and modifications

### Resolved 2026-03-28 (commit 4191125)

- **M-013:** structured-data.js is a 743-line monolith with 7+ responsibilities -- extracted 9 pure data functions into `structured-data-utils.js`, reducing the main renderer by ~240 lines and enabling isolated testing
- **M-014:** Duplicated decision toggle builders in structured-data.js -- unified `buildRowDecisionToggle` and `buildColumnDecisionToggle` into a single `buildDecisionToggle(key, state, rerenderFn, opts)` function; extracted `applyBulkDecision` to replace 4 duplicated iteration blocks
- **L-013:** No tests for structured-data renderer logic -- added 31 unit tests via vitest + happy-dom covering getCellValue, getCellChange, flattenRows, sortRows, filterRows, createTableState, setAllRowDecisions, buildDecisionPayload, and applyBulkDecision

### Resolved 2026-03-28 (commit 510f754)

- **L-012:** Duplicated session cleanup logic in main.js closeTab and onDecision -- extracted `removeSession(sessionId)` helper to deduplicate cleanup logic

### Resolved 2026-03-27 (commit 4da90fc)

- **M-012:** available_renderers() mixes aggregation with renderer synthesis logic -- extracted `synthesize_renderer_defs()` as a pure function with `ToolCache::plugin_tools()` encapsulating index access; 7 unit tests added covering cache hit/miss, known-renderer filtering, and multi-tool grouping

### Resolved 2026-03-26 (commit 2b0f6cb)

- **M-010:** AppState carries test-only `plugins_dir_override` field in production struct -- replaced `plugins_dir_override: Option<PathBuf>` with permanent `plugin_store: PluginStore` field on `AppState`; `reload_plugins()` now always uses `self.plugin_store` instead of branching
- **L-004:** Duplicated test helpers across commands.rs and state.rs -- extracted shared `test_utils.rs` module with `test_manifest()` and `test_app_state()` helpers, imported by both test modules
- **L-005:** Hardcoded URL in setup-integrations.sh diverges from $MCP_MUX_URL variable -- switched codex heredoc from single-quoted to unquoted so `$MCP_MUX_URL` is interpolated; also added Claude Desktop mcp-remote entry to PowerShell script
- **L-006:** Bundled registry fallback parse failure silently ignored -- replaced `if let Ok` with `expect()` since bundled JSON is compile-time data that must always parse
- **L-007:** Duplicated inline HTML empty-state markup in plugin-manager.js -- extracted `renderEmptyState(title, message)` helper, called from all three locations

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
| 6fe89c5 | 2026-04-03 | 82/100 | Good |
| 1924ce6 | 2026-04-03 | 68/100 | Acceptable |
| 6eaaebe | 2026-04-01 | 82/100 | Good |
| a36294a | 2026-03-31 | 92/100 | Excellent |
| 4d55dc6 | 2026-03-31 | 88/100 | Good |
| 44e1f76 | 2026-03-31 | 62/100 | Acceptable |
| 7ed9962 | 2026-03-31 | 80/100 | Good |
| 846d72e | 2026-03-30 | 88/100 | Good |
| 3b9f265 | 2026-03-30 | 78/100 | Good |
| ce2de40 | 2026-03-30 | 75/100 | Good |
| c5f6d1c | 2026-03-30 | 82/100 | Good |
| ec4a811 | 2026-03-30 | 78/100 | Good |
| 85c3c5d | 2026-03-30 | 80/100 | Good |
| c2070b7 | 2026-03-29 | 82/100 | Good |
| aa0c85d | 2026-03-29 | 78/100 | Good |
| 2565475 | 2026-03-29 | 72/100 | Good |
| c88d26f | 2026-03-29 | 78/100 | Good |
| 8e9fc5f | 2026-03-29 | 90/100 | Excellent |
| 924259d | 2026-03-29 | 68/100 | Acceptable |
| 2e08937 | 2026-03-28 | 78/100 | Good |
| da52e1f | 2026-03-28 | 82/100 | Good |
| 21d2ff4 | 2026-03-28 | 62/100 | Acceptable |
| 9663b17 | 2026-03-28 | 90/100 | Excellent |
| effec4a | 2026-03-28 | 62/100 | Acceptable |
| 6a127b2 | 2026-03-28 | 72/100 | Good |
| 4191125 | 2026-03-28 | 88/100 | Good |
| b17d52a | 2026-03-28 | 58/100 | Acceptable |
| a24b465 | 2026-03-28 | 85/100 | Good |
| 630efb9 | 2026-03-28 | 82/100 | Good |
| b0bc543 | 2026-03-28 | 88/100 | Good |
| 3c31909 | 2026-03-28 | 72/100 | Good |
| 44b8d08 | 2026-03-27 | 82/100 | Good |
| 4da90fc | 2026-03-27 | 90/100 | Excellent |
| b5d1356 | 2026-03-27 | 78/100 | Good |
| cdde6ae | 2026-03-27 | 85/100 | Good |
| d7a0bdc | 2026-03-26 | 82/100 | Good |
| 29dd54c | 2026-03-26 | 82/100 | Good |
| c0bebe3 | 2026-03-26 | 90/100 | Excellent |
| c2374c2 | 2026-03-26 | 88/100 | Good |
| 9c71eea | 2026-03-26 | 92/100 | Excellent |
| 258e45b | 2026-03-26 | 90/100 | Excellent |
| cc052c8 | 2026-03-26 | 80/100 | Good |
| dc6cde9 | 2026-03-26 | 82/100 | Good |
| 2b0f6cb | 2026-03-26 | 88/100 | Good |
| aa69a19 | 2026-03-26 | 75/100 | Good |
| b5f3eb7 | 2026-03-26 | 80/100 | Good |
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
