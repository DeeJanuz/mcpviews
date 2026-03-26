// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth;
mod commands;
mod http_server;
mod installer;
mod mcp;
mod mcp_session;
mod mcp_tools;
mod plugin;
mod registry;
mod renderer_scanner;
mod tool_cache;
mod review;
mod session;
mod state;

use std::sync::Arc;
use state::AppState;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Manager,
};
use tauri_plugin_autostart::MacosLauncher;

fn mime_from_extension(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("js") => "application/javascript",
        Some("mjs") => "application/javascript",
        Some("css") => "text/css",
        Some("html") | Some("htm") => "text/html",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        _ => "application/octet-stream",
    }
}

fn main() {
    let app_state = Arc::new(AppState::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .register_uri_scheme_protocol("plugin", |_ctx, request| {
            let uri = request.uri().to_string();
            // URI format: plugin://localhost/{plugin_name}/{path...}
            let path = uri
                .strip_prefix("plugin://localhost/")
                .or_else(|| uri.strip_prefix("plugin://localhost"))
                .unwrap_or("");

            let mut parts = path.splitn(2, '/');
            let plugin_name = parts.next().unwrap_or("");
            let file_path = parts.next().unwrap_or("");

            if plugin_name.is_empty() || file_path.is_empty() {
                return tauri::http::Response::builder()
                    .status(404)
                    .body(Vec::new())
                    .unwrap();
            }

            // Path traversal protection
            if file_path.contains("..") {
                return tauri::http::Response::builder()
                    .status(403)
                    .body(b"Forbidden: path traversal".to_vec())
                    .unwrap();
            }

            let plugins_dir = mcp_mux_shared::plugins_dir();
            let full_path = plugins_dir.join(plugin_name).join(file_path);

            match std::fs::read(&full_path) {
                Ok(contents) => {
                    let mime = mime_from_extension(&full_path);
                    tauri::http::Response::builder()
                        .status(200)
                        .header("Content-Type", mime)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(contents)
                        .unwrap()
                }
                Err(_) => {
                    tauri::http::Response::builder()
                        .status(404)
                        .body(b"Not found".to_vec())
                        .unwrap()
                }
            }
        })
        .manage(app_state.clone())
        .invoke_handler(tauri::generate_handler![
            commands::get_sessions,
            commands::submit_decision,
            commands::dismiss_session,
            commands::get_health,
            commands::list_plugins,
            commands::install_plugin,
            commands::uninstall_plugin,
            commands::install_plugin_from_file,
            commands::install_plugin_from_registry,
            commands::install_plugin_from_zip,
            commands::fetch_registry,
            commands::start_plugin_auth,
            commands::store_plugin_token,
            commands::get_settings,
            commands::save_settings,
            commands::get_plugin_renderers,
            commands::get_registry_sources,
            commands::add_registry_source,
            commands::remove_registry_source,
            commands::toggle_registry_source,
            commands::update_plugin,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Hide to tray instead of quitting
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let state = app_state.clone();

            // Spawn the axum HTTP server on a dedicated thread with its own tokio runtime
            std::thread::Builder::new()
                .name("http-server".into())
                .spawn(move || {
                    let rt = tokio::runtime::Runtime::new()
                        .expect("Failed to create tokio runtime");
                    rt.block_on(async move {
                        http_server::start_http_server(state, handle).await;
                    });
                })
                .expect("Failed to spawn HTTP thread");

            // Build system tray menu
            let show_item = MenuItemBuilder::with_id("show", "Show Window").build(app)?;
            let manage_plugins_item = MenuItemBuilder::with_id("manage_plugins", "Manage Plugins").build(app)?;
            let setup_item = MenuItemBuilder::with_id("setup_integrations", "Setup Agent Integrations").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let tray_menu = MenuBuilder::new(app)
                .item(&show_item)
                .item(&manage_plugins_item)
                .item(&setup_item)
                .separator()
                .item(&quit_item)
                .build()?;

            // Create tray icon
            let icon = app.default_window_icon().cloned().unwrap_or_else(|| {
                Image::new_owned(vec![99; 16 * 16 * 4], 16, 16)
            });

            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .menu(&tray_menu)
                .tooltip("MCP Mux")
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "manage_plugins" => {
                        if let Some(window) = app.get_webview_window("plugin-manager") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        } else {
                            let _ = tauri::WebviewWindowBuilder::new(
                                app,
                                "plugin-manager",
                                tauri::WebviewUrl::App("plugin-manager.html".into()),
                            )
                            .title("MCP Mux - Plugin Manager")
                            .inner_size(800.0, 600.0)
                            .build();
                        }
                    }
                    "setup_integrations" => {
                        if let Some(script) = installer::get_script_path(app) {
                            let _ = installer::open_installer_terminal(&script);
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click { .. } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // First-run agent integration setup
            if !installer::check_first_run() {
                if let Some(script) = installer::get_script_path(&app.handle().clone()) {
                    let _ = installer::open_installer_terminal(&script);
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running MCP Mux");
}
