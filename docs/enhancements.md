# Technical Debt & Enhancement Log

**Last Updated:** 2026-03-28
**Total Active Issues:** 7
**Resolved This Month:** 33

---

## Active Issues

### Critical

_None_

### High

_None_

### Medium

#### M-018: No tests for drawer-stack, invocation-registry, or mcpview:// URI parsing
- **File(s):** `public/renderers/drawer-stack.js`, `public/renderers/invocation-registry.js`, `public/renderers/shared.js`
- **Principle:** Testability
- **Description:** Three new frontend modules totaling ~250 lines were added with zero unit tests. The `globToRegex` function, mcpview:// URI parser, drawer stack open/close lifecycle, and `autoDetectLinks` DOM mutation are all testable. Pure functions like `globToRegex` and the URI parser can be extracted and tested directly.
- **Suggested Fix:** Extract `globToRegex` and `parseMcpviewUri` into a utils module; add vitest tests covering glob matching, URI parsing edge cases, and drawer stack push/pop behavior.
- **Detected:** 2026-03-28 (commit 21d2ff4)

#### M-019: get_renderer_registry test duplicates filtering logic instead of calling the function
- **File(s):** `src-tauri/src/commands.rs`
- **Principle:** DRY / Testability
- **Description:** The test for `get_renderer_registry` (line ~555) duplicates the entire filtering/json-building loop from the command function (line ~404) rather than calling the actual function through a helper. The test validates a copy of the logic, not the real implementation -- a bug in one may not appear in the other.
- **Suggested Fix:** Extract the filtering logic into a standalone `fn collect_invocable_renderers(manifests: &[PluginManifest]) -> Vec<serde_json::Value>` and test that directly, or find a way to call `get_renderer_registry` with a mock State.
- **Detected:** 2026-03-28 (commit 21d2ff4)

### Low

#### L-017: display_mode is stringly-typed Option<String> instead of an enum
- **File(s):** `shared/src/lib.rs`
- **Principle:** OCP / Type Safety
- **Description:** `display_mode: Option<String>` on `RendererDef` accepts any string, but the system only supports "drawer", "modal", and "replace". Invalid values silently fall through to a default. An enum would provide compile-time validation and exhaustive matching.
- **Suggested Fix:** Define `enum DisplayMode { Drawer, Modal, Replace }` with `serde` rename attributes and use `Option<DisplayMode>` on the struct.
- **Detected:** 2026-03-28 (commit 21d2ff4)

#### L-014: Large inline documentation strings in builtin_renderer_definitions()
- **File(s):** `src-tauri/src/mcp_tools.rs`
- **Principle:** SRP / Maintainability
- **Description:** The `structured_data` renderer rule is a ~90-line raw string literal embedded in `builtin_renderer_definitions()`. As renderer documentation grows, this function becomes harder to navigate and mixes documentation content with code structure. Other renderers will follow the same pattern.
- **Suggested Fix:** Extract long rule text into constants, a dedicated module, or use `include_str!` to load from embedded files.
- **Detected:** 2026-03-28 (commit 6a127b2)

#### L-015: Fragile positional index assertions in collect_rules tests
- **File(s):** `src-tauri/src/mcp_tools.rs`
- **Principle:** Maintainability
- **Description:** All `collect_rules` tests use hardcoded `rules[0]` / `rules[1]` positional indexing. Adding any new cross-cutting rule requires updating indices in every test. The renderer_selection addition already caused this cascade.
- **Suggested Fix:** Use `rules.iter().find(|r| r["name"] == "target_name")` instead of positional indexing.
- **Detected:** 2026-03-28 (commit 6a127b2)

#### L-016: Duplicated renderer hint iteration in builtin_tool_definitions
- **File(s):** `src-tauri/src/mcp_tools.rs`
- **Principle:** DRY
- **Description:** The `renderers.iter().filter_map(|r| r.data_hint.as_ref().map(...))` pattern is duplicated identically in both `push_content` and `push_review` tool definitions.
- **Suggested Fix:** Extract a helper function like `build_data_description(renderers: &[RendererDef], prefix: &str) -> String`.
- **Detected:** 2026-03-28 (commit 6a127b2)

#### L-011: PluginStore reconstructed via with_dir instead of reused in AppState
- **File(s):** `src-tauri/src/state.rs`
- **Principle:** DRY
- **Description:** Both `new_with_store()` (line 35) and `reload_plugins()` (line 64) call `PluginStore::with_dir(self.plugin_store.dir().to_path_buf())` to create a fresh store from the path, rather than passing or cloning the stored `plugin_store` field directly. If `PluginStore` gains configuration beyond the directory path, these reconstructions would silently lose it.
- **Suggested Fix:** If `PluginStore` implements `Clone`, use `self.plugin_store.clone()`. Otherwise, add a `PluginStore::clone_fresh()` method that preserves all configuration.
- **Detected:** 2026-03-26 (commit 2b0f6cb)

---

## Resolved Issues

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
