use mcp_mux_shared::plugins_dir;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RendererInfo {
    pub plugin_name: String,
    pub file_name: String,
    pub url: String,
}

/// Scan all installed plugin directories for renderer JS files.
/// Looks for files in {plugin_dir}/renderers/*.js
pub fn scan_plugin_renderers() -> Vec<RendererInfo> {
    let dir = plugins_dir();
    if !dir.exists() {
        return Vec::new();
    }

    let mut renderers = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let plugin_name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => continue,
            };

            let renderers_dir = path.join("renderers");
            if !renderers_dir.is_dir() {
                continue;
            }

            if let Ok(renderer_entries) = std::fs::read_dir(&renderers_dir) {
                for renderer_entry in renderer_entries.flatten() {
                    let renderer_path = renderer_entry.path();
                    if renderer_path.extension().and_then(|e| e.to_str()) == Some("js") {
                        let file_name = renderer_entry.file_name().to_string_lossy().to_string();
                        renderers.push(RendererInfo {
                            plugin_name: plugin_name.clone(),
                            file_name: file_name.clone(),
                            url: format!(
                                "plugin://localhost/{}/renderers/{}",
                                plugin_name, file_name
                            ),
                        });
                    }
                }
            }
        }
    }

    renderers
}
