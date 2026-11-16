//! Bundled ConPTY: Windows Terminal's conpty.dll + OpenConsole.exe (see `assets/conpty/README.md`),
//! faster than the console host built into Windows. Both are embedded so the app stays a single
//! exe, and extracted once per version.

use std::path::{Path, PathBuf};
use std::sync::Once;

const VERSION: &str = "1.25.260930003";
const FILES: [(&str, &[u8]); 2] = [
    ("conpty.dll", include_bytes!("../../assets/conpty/conpty.dll")),
    ("OpenConsole.exe", include_bytes!("../../assets/conpty/OpenConsole.exe")),
];

/// Loads the bundled conpty.dll before portable-pty first resolves "conpty.dll": a loaded module
/// of that name wins its lookup. Runs once; on any failure Windows' own ConPTY stays in use.
/// `AGENTDECK_SYSTEM_CONPTY=1` keeps Windows' own one.
pub fn preload() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("AGENTDECK_SYSTEM_CONPTY").is_some() {
            return;
        }
        if let Err(e) = extract().and_then(|dir| load(&dir.join("conpty.dll"))) {
            eprintln!("bundled ConPTY unavailable, using Windows' own: {e}");
        }
    });
}

/// Writes the files to `%LOCALAPPDATA%/com.agentdeck.app/conpty/<version>` unless already there.
/// conpty.dll starts the OpenConsole.exe found next to it.
fn extract() -> Result<PathBuf, String> {
    let dir = dirs::data_local_dir()
        .ok_or("no local app data directory")?
        .join(crate::APP_IDENTIFIER)
        .join("conpty")
        .join(VERSION);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (name, bytes) in FILES {
        let path = dir.join(name);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64) {
            continue;
        }
        // Temp file + rename: another instance never sees (or loads) a half-written file.
        let tmp = dir.join(format!("{name}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, bytes)
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("{}: {e}", path.display())
            })?;
    }
    Ok(dir)
}

/// Loads the dll for the whole process lifetime (never freed: portable-pty keeps its functions).
fn load(dll: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;
    let wide: Vec<u16> = dll.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: NUL-terminated path; the returned module is intentionally never freed.
    let module = unsafe { LoadLibraryW(wide.as_ptr()) };
    if module.is_null() {
        return Err(format!("{}: {}", dll.display(), std::io::Error::last_os_error()));
    }
    Ok(())
}
