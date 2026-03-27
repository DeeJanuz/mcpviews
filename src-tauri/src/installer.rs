use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::Manager;

/// Check if first-run setup has been completed.
/// Returns true if ~/.mcpviews/.setup-complete exists.
pub fn check_first_run() -> bool {
    dirs::home_dir()
        .map(|home| home.join(".mcpviews").join(".setup-complete").exists())
        .unwrap_or(false)
}

/// Resolve the bundled script path from Tauri resources.
/// Returns the path to setup-integrations.sh (Linux/macOS) or setup-integrations.ps1 (Windows).
pub fn get_script_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let script_name = if cfg!(target_os = "windows") {
        "scripts/setup-integrations.ps1"
    } else {
        "scripts/setup-integrations.sh"
    };

    app.path()
        .resolve(script_name, tauri::path::BaseDirectory::Resource)
        .ok()
        .filter(|p: &PathBuf| p.exists())
}

/// Open a terminal window running the installer script.
/// Uses std::process::Command to spawn a visible terminal window.
pub fn open_installer_terminal(script_path: &Path) -> Result<(), String> {
    let script = script_path
        .to_str()
        .ok_or_else(|| "Invalid script path encoding".to_string())?;

    #[cfg(target_os = "linux")]
    {
        let terminals: &[(&str, &[&str])] = &[
            ("x-terminal-emulator", &["-e"]),
            ("gnome-terminal", &["--"]),
            ("konsole", &["-e"]),
            ("xfce4-terminal", &["-e"]),
            ("xterm", &["-e"]),
        ];

        for (terminal, args) in terminals {
            if which_exists(terminal) {
                let mut cmd_args: Vec<&str> = args.to_vec();
                cmd_args.push(script);

                return Command::new(terminal)
                    .args(&cmd_args)
                    .spawn()
                    .map(|_| ())
                    .map_err(|e| format!("Failed to spawn {}: {}", terminal, e));
            }
        }

        Err("No supported terminal emulator found".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-a", "Terminal", script])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open Terminal.app: {}", e))
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/c", "start", "powershell.exe", "-ExecutionPolicy", "Bypass", "-File", script])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to start PowerShell: {}", e))
    }
}

#[cfg(target_os = "linux")]
fn which_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
