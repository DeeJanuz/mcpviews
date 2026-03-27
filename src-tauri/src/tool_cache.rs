use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

const CACHE_TTL_SECS: u64 = 300; // 5 minutes

pub struct CachedPluginTools {
    pub tools: Vec<Value>,
    pub fetched_at: Option<Instant>,
    pub refresh_pending: bool,
}

impl CachedPluginTools {
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            fetched_at: None,
            refresh_pending: false,
        }
    }
}

pub struct ToolCache {
    pub entries: Vec<CachedPluginTools>,
    pub tool_index: HashMap<String, usize>,
}

impl ToolCache {
    pub fn new(plugin_count: usize) -> Self {
        let entries = (0..plugin_count).map(|_| CachedPluginTools::new()).collect();
        Self {
            entries,
            tool_index: HashMap::new(),
        }
    }

    /// Return indices of plugins whose tool cache is stale or empty.
    /// Takes a closure to check if the plugin at index i has MCP config.
    pub fn stale_indices(&self, has_mcp: impl Fn(usize) -> bool) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(i, entry)| {
                has_mcp(*i)
                    && !entry.refresh_pending
                    && match entry.fetched_at {
                        None => true,
                        Some(t) => t.elapsed().as_secs() > CACHE_TTL_SECS,
                    }
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn mark_pending(&mut self, idx: usize) {
        if let Some(entry) = self.entries.get_mut(idx) {
            entry.refresh_pending = true;
        }
    }

    pub fn clear_pending(&mut self, idx: usize) {
        if let Some(entry) = self.entries.get_mut(idx) {
            entry.refresh_pending = false;
        }
    }

    /// Return all cached tools across all plugins
    pub fn all_tools(&self) -> Vec<Value> {
        self.entries
            .iter()
            .flat_map(|e| e.tools.clone())
            .collect()
    }

    /// Apply fetched tools for a plugin: prefix names, update index, set timestamp
    pub fn apply(&mut self, idx: usize, prefix: &str, tools: Vec<Value>) {
        let mut index_updates: Vec<(String, usize)> = Vec::new();
        let prefixed_tools: Vec<Value> = tools
            .into_iter()
            .map(|mut tool| {
                if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                    let prefixed = format!("{}{}", prefix, name);
                    if let Some(obj) = tool.as_object_mut() {
                        obj.insert("name".to_string(), Value::String(prefixed.clone()));
                    }
                    index_updates.push((prefixed, idx));
                }
                tool
            })
            .collect();

        for (name, plugin_idx) in index_updates {
            self.tool_index.insert(name, plugin_idx);
        }
        if let Some(entry) = self.entries.get_mut(idx) {
            entry.tools = prefixed_tools;
            entry.fetched_at = Some(Instant::now());
            entry.refresh_pending = false;
        }
    }

    /// Rebuild the tool_index from scratch
    pub fn rebuild_index(&mut self) {
        self.tool_index.clear();
        for (idx, entry) in self.entries.iter().enumerate() {
            for tool in &entry.tools {
                if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                    self.tool_index.insert(name.to_string(), idx);
                }
            }
        }
    }

    /// Add a new empty entry (when a plugin is added)
    pub fn push(&mut self) {
        self.entries.push(CachedPluginTools::new());
    }

    /// Remove an entry at index (when a plugin is removed)
    pub fn remove(&mut self, idx: usize) {
        if idx < self.entries.len() {
            self.entries.remove(idx);
        }
    }

    /// Get cached tools for a plugin by index
    pub fn plugin_tools(&self, idx: usize) -> Option<&[Value]> {
        self.entries.get(idx).map(|e| e.tools.as_slice())
    }

    /// Get tool count for a plugin
    pub fn tool_count(&self, idx: usize) -> usize {
        self.entries.get(idx).map(|e| e.tools.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_empty_entries() {
        let cache = ToolCache::new(3);
        assert_eq!(cache.entries.len(), 3);
        for entry in &cache.entries {
            assert!(entry.tools.is_empty());
            assert!(entry.fetched_at.is_none());
            assert!(!entry.refresh_pending);
        }
    }

    #[test]
    fn test_stale_indices_returns_indices_with_mcp_and_empty_cache() {
        let cache = ToolCache::new(3);
        // indices 0 and 2 have mcp, index 1 does not
        let stale = cache.stale_indices(|i| i == 0 || i == 2);
        assert_eq!(stale, vec![0, 2]);
    }

    #[test]
    fn test_stale_indices_skips_pending() {
        let mut cache = ToolCache::new(2);
        cache.mark_pending(0);
        let stale = cache.stale_indices(|_| true);
        assert_eq!(stale, vec![1]);
    }

    #[test]
    fn test_stale_indices_skips_no_mcp() {
        let cache = ToolCache::new(2);
        let stale = cache.stale_indices(|_| false);
        assert!(stale.is_empty());
    }

    #[test]
    fn test_mark_pending() {
        let mut cache = ToolCache::new(1);
        assert!(!cache.entries[0].refresh_pending);
        cache.mark_pending(0);
        assert!(cache.entries[0].refresh_pending);
    }

    #[test]
    fn test_clear_pending() {
        let mut cache = ToolCache::new(1);
        cache.mark_pending(0);
        assert!(cache.entries[0].refresh_pending);
        cache.clear_pending(0);
        assert!(!cache.entries[0].refresh_pending);
    }

    #[test]
    fn test_all_tools_empty() {
        let cache = ToolCache::new(2);
        assert!(cache.all_tools().is_empty());
    }

    #[test]
    fn test_apply_prefixes_tools() {
        let mut cache = ToolCache::new(1);
        let tools = vec![
            serde_json::json!({"name": "read", "description": "Read a file"}),
            serde_json::json!({"name": "write", "description": "Write a file"}),
        ];
        cache.apply(0, "github__", tools);

        assert_eq!(cache.tool_count(0), 2);
        assert!(cache.tool_index.contains_key("github__read"));
        assert!(cache.tool_index.contains_key("github__write"));
        assert_eq!(*cache.tool_index.get("github__read").unwrap(), 0);

        let all = cache.all_tools();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].get("name").unwrap().as_str().unwrap(), "github__read");
    }

    #[test]
    fn test_apply_then_all_tools() {
        let mut cache = ToolCache::new(2);
        cache.apply(
            0,
            "a__",
            vec![serde_json::json!({"name": "foo"})],
        );
        cache.apply(
            1,
            "b__",
            vec![serde_json::json!({"name": "bar"})],
        );
        let all = cache.all_tools();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_rebuild_index() {
        let mut cache = ToolCache::new(2);
        cache.apply(0, "a__", vec![serde_json::json!({"name": "x"})]);
        cache.apply(1, "b__", vec![serde_json::json!({"name": "y"})]);

        // Clear and rebuild
        cache.tool_index.clear();
        assert!(cache.tool_index.is_empty());
        cache.rebuild_index();
        assert_eq!(cache.tool_index.len(), 2);
        assert_eq!(*cache.tool_index.get("a__x").unwrap(), 0);
        assert_eq!(*cache.tool_index.get("b__y").unwrap(), 1);
    }

    #[test]
    fn test_push_adds_entry() {
        let mut cache = ToolCache::new(1);
        assert_eq!(cache.entries.len(), 1);
        cache.push();
        assert_eq!(cache.entries.len(), 2);
    }

    #[test]
    fn test_remove_entry() {
        let mut cache = ToolCache::new(3);
        cache.remove(1);
        assert_eq!(cache.entries.len(), 2);
    }

    #[test]
    fn test_plugin_tools_returns_slice_for_valid_index() {
        let mut cache = ToolCache::new(1);
        cache.apply(
            0,
            "p__",
            vec![
                serde_json::json!({"name": "a", "description": "Tool A"}),
                serde_json::json!({"name": "b", "description": "Tool B"}),
            ],
        );
        let tools = cache.plugin_tools(0);
        assert!(tools.is_some());
        let tools = tools.unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].get("name").unwrap().as_str().unwrap(), "p__a");
    }

    #[test]
    fn test_plugin_tools_returns_none_for_invalid_index() {
        let cache = ToolCache::new(1);
        assert!(cache.plugin_tools(5).is_none());
    }

    #[test]
    fn test_plugin_tools_returns_empty_slice_for_unfilled_entry() {
        let cache = ToolCache::new(2);
        let tools = cache.plugin_tools(1);
        assert!(tools.is_some());
        assert!(tools.unwrap().is_empty());
    }

    #[test]
    fn test_tool_count() {
        let mut cache = ToolCache::new(1);
        assert_eq!(cache.tool_count(0), 0);
        cache.apply(
            0,
            "p__",
            vec![
                serde_json::json!({"name": "a"}),
                serde_json::json!({"name": "b"}),
                serde_json::json!({"name": "c"}),
            ],
        );
        assert_eq!(cache.tool_count(0), 3);
    }
}
