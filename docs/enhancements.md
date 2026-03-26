# Technical Debt & Enhancement Log

**Last Updated:** 2026-03-25
**Total Active Issues:** 2
**Resolved This Month:** 9

---

## Active Issues

### Critical

_None_

### High

_None_

### Medium

#### M-003: PluginStore instantiated as concrete dependency in PluginRegistry methods
- **File(s):** `src-tauri/src/plugin.rs` (add_plugin, remove_plugin, load_plugins)
- **Principle:** DIP
- **Description:** `PluginStore::new()` is constructed inline within `PluginRegistry` methods. While `PluginStore` has a `with_dir()` constructor for tests, the `PluginRegistry` itself cannot be tested for add/remove behavior without hitting the real filesystem. Injecting the store or passing it as a parameter would improve testability.
- **Suggested Fix:** Accept a `PluginStore` reference (or a trait) in `PluginRegistry::new()` / `load_plugins()`, or store it as a field.
- **Detected:** 2026-03-25 (commit e4ca382)

### Low

#### L-002: Settings stored/loaded as raw serde_json::Value
- **File(s):** `src-tauri/src/commands.rs` (get_settings, save_settings)
- **Principle:** Type Safety / OCP
- **Description:** `save_settings` replaces the entire config.json with whatever JSON is passed from the frontend. As settings grow, this risks accidental key loss and lacks compile-time validation. A typed `Settings` struct would be safer.
- **Suggested Fix:** Define a `Settings` struct in the shared crate with typed fields, serialize/deserialize through it.
- **Detected:** 2026-03-25 (commit e4ca382)

---

## Resolved Issues

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
| 102813b | 2026-03-25 | 88/100 | Good |
| 6ebae60 | 2026-03-25 | 58/100 | Acceptable |
| e4ca382 | 2026-03-25 | 82/100 | Good |
| ba492ce | 2026-03-25 | 42/100 | Needs Improvement |
