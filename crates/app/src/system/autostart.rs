//! "Launch AgentDeck when Windows starts": the OS registration (HKCU Run key "AgentDeck", the same
//! entry tauri-plugin-autostart wrote) is the source of truth.

use auto_launch::{AutoLaunch, AutoLaunchBuilder};

fn launcher() -> Result<AutoLaunch, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = exe.to_str().ok_or("the app path is not valid UTF-8")?;
    AutoLaunchBuilder::new()
        .set_app_name("AgentDeck")
        .set_app_path(path)
        .build()
        .map_err(|e| e.to_string())
}

pub fn autostart_enabled() -> Result<bool, String> {
    launcher()?.is_enabled().map_err(|e| e.to_string())
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let launcher = launcher()?;
    if enabled { launcher.enable() } else { launcher.disable() }.map_err(|e| e.to_string())
}
