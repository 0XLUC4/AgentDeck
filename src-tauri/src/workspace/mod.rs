//! Workspace folder discovery: folders open in other apps, and git info for a folder.
//!
//! How `list_open_projects` finds folders (Windows only, one PowerShell call, ~0.5-1 s):
//! - Explorer: `Shell.Application.Windows()` gives each folder window's real path.
//! - VS Code / Cursor / Windsurf / VSCodium: UI Automation lists every top-level window
//!   of the running editor processes (`Get-Process` `MainWindowTitle` only shows one).
//!   A title looks like `file - folder - Visual Studio Code`, so it gives the folder NAME
//!   only. The name is resolved to a path with the folder URIs the editor keeps in
//!   `%APPDATA%\<app>\User\globalStorage\storage.json` (`backupWorkspaces`, `windowsState`,
//!   then `profileAssociations`, i.e. recently opened folders).
//!
//! Limits: a custom `window.title`, `.code-workspace` workspaces, remote (SSH/WSL)
//! folders and folders never recorded in storage.json are skipped. Two recent folders with
//! the same name resolve to the first existing one in the order above.
//! JetBrains IDEs are not detected.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agents::{find_repo_root, git};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenProject {
    pub app: String,
    pub name: String,
    pub path: String,
    pub is_git: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInfo {
    pub path: String,
    pub name: String,
    pub exists: bool,
    pub is_git: bool,
    pub branch: Option<String>,
    pub remote: Option<String>,
}

/// (process name, %APPDATA% folder, label shown to the user)
const EDITORS: &[(&str, &str, &str)] = &[
    ("Code", "Code", "VS Code"),
    ("Cursor", "Cursor", "Cursor"),
    ("Windsurf", "Windsurf", "Windsurf"),
    ("VSCodium", "VSCodium", "VSCodium"),
];

const PROBE_SCRIPT: &str = r#"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ErrorActionPreference = 'SilentlyContinue'
$explorer = @(foreach ($w in (New-Object -ComObject Shell.Application).Windows()) { try { $w.Document.Folder.Self.Path } catch {} })
$names = @{}
Get-Process Code,Cursor,Windsurf,VSCodium | ForEach-Object { $names[$_.Id] = $_.ProcessName }
$windows = @()
if ($names.Count) {
  Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
  $all = [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children, [Windows.Automation.Condition]::TrueCondition)
  $windows = @(foreach ($w in $all) { $c = $w.Current; if ($names.ContainsKey($c.ProcessId) -and $c.Name) { [pscustomobject]@{ app = $names[$c.ProcessId]; title = $c.Name } } })
}
ConvertTo-Json -Compress -Depth 3 @{ explorer = $explorer; windows = $windows }
"#;

#[derive(Debug, Default, Deserialize)]
struct Probe {
    #[serde(default)]
    explorer: Vec<Option<String>>,
    #[serde(default)]
    windows: Vec<EditorWindow>,
}

#[derive(Debug, Deserialize)]
struct EditorWindow {
    app: String,
    title: String,
}

/// Folders currently open in editors and Explorer, deduped by path, existing dirs only.
#[tauri::command(async)]
pub fn list_open_projects() -> Vec<OpenProject> {
    let probe = run_probe().unwrap_or_else(|e| {
        eprintln!("[workspace] probe failed: {e}");
        Probe::default()
    });
    let mut found = Vec::new();
    for (process, appdata_dir, label) in EDITORS {
        let titles: Vec<&str> = probe
            .windows
            .iter()
            .filter(|w| w.app.eq_ignore_ascii_case(process))
            .map(|w| w.title.as_str())
            .collect();
        if titles.is_empty() {
            continue;
        }
        let known = editor_folders(appdata_dir);
        for title in titles {
            if let Some(path) = resolve_title(title, &known) {
                found.push((label.to_string(), path));
            }
        }
    }
    for path in probe.explorer.into_iter().flatten() {
        if !path.starts_with("::{") {
            found.push(("Explorateur".to_string(), PathBuf::from(path)));
        }
    }
    dedupe(found)
        .into_iter()
        .filter(|(_, p)| p.is_dir())
        .map(|(app, path)| OpenProject {
            app,
            name: folder_name(&path),
            is_git: find_repo_root(&path).is_some(),
            path: path.to_string_lossy().into_owned(),
        })
        .collect()
}

/// Existence and git details (branch, origin) of `path`; subfolders of a repo count as git.
#[tauri::command(async)]
pub fn inspect_folder(path: String) -> Result<FolderInfo, String> {
    let dir = PathBuf::from(path.trim());
    if dir.as_os_str().is_empty() {
        return Err("empty path".into());
    }
    let exists = dir.is_dir();
    let root = exists.then(|| find_repo_root(&dir)).flatten();
    let (branch, remote) = match &root {
        Some(root) => (
            git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
                .ok()
                .filter(|b| !b.is_empty() && b != "HEAD"),
            git(root, &["remote", "get-url", "origin"])
                .ok()
                .filter(|r| !r.is_empty()),
        ),
        None => (None, None),
    };
    Ok(FolderInfo {
        name: folder_name(&dir),
        path: dir.to_string_lossy().into_owned(),
        exists,
        is_git: root.is_some(),
        branch,
        remote,
    })
}

#[cfg(windows)]
fn run_probe() -> Result<Probe, String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", PROBE_SCRIPT])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| format!("cannot run powershell: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json = stdout.trim().trim_start_matches('\u{feff}');
    if json.is_empty() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    serde_json::from_str(json).map_err(|e| format!("bad probe output: {e}"))
}

#[cfg(not(windows))]
fn run_probe() -> Result<Probe, String> {
    Ok(Probe::default())
}

/// Local folder paths an editor remembers, most-likely-open first.
fn editor_folders(appdata_dir: &str) -> Vec<PathBuf> {
    let Some(appdata) = std::env::var_os("APPDATA") else {
        return Vec::new();
    };
    let file = Path::new(&appdata)
        .join(appdata_dir)
        .join("User")
        .join("globalStorage")
        .join("storage.json");
    match std::fs::read_to_string(&file) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(json) => storage_folders(&json),
            Err(e) => {
                eprintln!("[workspace] {}: {e}", file.display());
                Vec::new()
            }
        },
        Err(_) => Vec::new(), // editor installed elsewhere / never used
    }
}

/// Folder URIs from a VS Code-family storage.json, in priority order, as local paths.
fn storage_folders(json: &Value) -> Vec<PathBuf> {
    let mut uris: Vec<&str> = Vec::new();
    if let Some(folders) = json["backupWorkspaces"]["folders"].as_array() {
        uris.extend(folders.iter().filter_map(|f| f["folderUri"].as_str()));
    }
    let state = &json["windowsState"];
    uris.extend(state["lastActiveWindow"]["folder"].as_str());
    if let Some(windows) = state["openedWindows"].as_array() {
        uris.extend(windows.iter().filter_map(|w| w["folder"].as_str()));
    }
    if let Some(map) = json["profileAssociations"]["workspaces"].as_object() {
        uris.extend(map.keys().map(String::as_str));
    }
    uris.into_iter().filter_map(uri_to_path).collect()
}

/// `file:///d%3A/my%20dir` -> `D:\my dir`; `file://host/share` -> `\\host\share`.
/// Non-file URIs (vscode-remote://, ...) give `None`.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = percent_decode(rest)?;
    let path = match decoded.strip_prefix('/') {
        Some(local) => {
            let mut chars: Vec<char> = local.chars().collect();
            if chars.len() >= 2 && chars[1] == ':' {
                chars[0] = chars[0].to_ascii_uppercase();
            }
            chars.into_iter().collect::<String>()
        }
        None => format!("//{decoded}"),
    };
    Some(PathBuf::from(path.replace('/', "\\")))
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Possible folder names in an editor title, most likely first.
/// `● file.ts - my-app - Visual Studio Code` -> `["my-app", "● file.ts"]`.
fn title_segments(title: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = title.split(" - ").map(str::trim).collect();
    parts.pop(); // app name (may carry "[Administrator]")
    parts.reverse();
    parts.retain(|p| !p.is_empty());
    parts
}

fn resolve_title(title: &str, known: &[PathBuf]) -> Option<PathBuf> {
    title_segments(title).into_iter().find_map(|segment| {
        known
            .iter()
            .find(|p| folder_name(p).eq_ignore_ascii_case(segment) && p.is_dir())
            .cloned()
    })
}

/// Keeps the first entry per path (case-insensitive, trailing separator ignored).
fn dedupe(items: Vec<(String, PathBuf)>) -> Vec<(String, PathBuf)> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|(_, p)| {
            let key = p
                .to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .to_lowercase();
            seen.insert(key)
        })
        .collect()
}

fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_file_uris() {
        assert_eq!(
            uri_to_path("file:///d%3A/agentdeck"),
            Some(PathBuf::from(r"D:\agentdeck"))
        );
        assert_eq!(
            uri_to_path("file:///c%3A/Users/Luca/Desktop/c%2B%2Bpro"),
            Some(PathBuf::from(r"C:\Users\Luca\Desktop\c++pro"))
        );
        assert_eq!(
            uri_to_path("file:///c%3A/my%20projet/%C3%A9t%C3%A9"),
            Some(PathBuf::from(r"C:\my projet\été"))
        );
        assert_eq!(
            uri_to_path("file://server/share/x"),
            Some(PathBuf::from(r"\\server\share\x"))
        );
        assert_eq!(uri_to_path("vscode-remote://ssh-remote%2Bhost/x"), None);
        assert_eq!(uri_to_path("file:///bad%2"), None);
    }

    #[test]
    fn parses_title_segments() {
        assert_eq!(
            title_segments("Welcome - src-tauri - Visual Studio Code"),
            vec!["src-tauri", "Welcome"]
        );
        assert_eq!(
            title_segments("● lib.rs - agentdeck - Cursor [Administrator]"),
            vec!["agentdeck", "● lib.rs"]
        );
        assert_eq!(title_segments("agentdeck - Windsurf"), vec!["agentdeck"]);
        assert!(title_segments("Visual Studio Code").is_empty());
    }

    #[test]
    fn reads_storage_folders_in_priority_order() {
        let storage = json!({
            "backupWorkspaces": { "folders": [{ "folderUri": "file:///d%3A/a" }] },
            "windowsState": {
                "lastActiveWindow": { "folder": "file:///d%3A/b" },
                "openedWindows": [{ "folder": "vscode-remote://wsl/x" }]
            },
            "profileAssociations": { "workspaces": { "file:///c%3A/c": "__default__profile__" } }
        });
        assert_eq!(
            storage_folders(&storage),
            vec![
                PathBuf::from(r"D:\a"),
                PathBuf::from(r"D:\b"),
                PathBuf::from(r"C:\c")
            ]
        );
    }

    #[test]
    fn dedupes_by_path_case_insensitive() {
        let items = vec![
            ("VS Code".to_string(), PathBuf::from(r"D:\agentdeck")),
            ("Explorateur".to_string(), PathBuf::from(r"d:\AgentDeck\")),
            ("Explorateur".to_string(), PathBuf::from(r"D:\other")),
        ];
        let out = dedupe(items);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "VS Code");
        assert_eq!(out[1].1, PathBuf::from(r"D:\other"));
    }

    #[test]
    fn inspects_missing_folder() {
        let info = inspect_folder(r"Z:\definitely\missing\dir".into()).unwrap();
        assert!(!info.exists && !info.is_git && info.branch.is_none());
        assert_eq!(info.name, "dir");
    }

    /// Live check on a dev machine: `cargo test live_probe -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_probe() {
        let start = std::time::Instant::now();
        println!("{:#?}", list_open_projects());
        println!("{:?}", inspect_folder(env!("CARGO_MANIFEST_DIR").into()));
        println!("took {:?}", start.elapsed());
    }
}
