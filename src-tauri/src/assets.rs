//! 3D assets of a workspace: listing model files and reading them for the in-app preview.
//! `.blend` files are converted to GLB with the installed Blender (headless) since browsers cannot read them.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use tauri::Manager;

const MODEL_EXTS: &[&str] = &["glb", "gltf", "obj", "fbx", "stl", "blend"];
/// Images the webview can show; also where missing model textures are searched.
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg", "tga"];
const SKIP_DIRS: &[&str] = &["node_modules", "target", "dist", "build", "__pycache__"];
const MAX_FILES: usize = 2000;
const MAX_DEPTH: usize = 8;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetFile {
    pub path: String,
    /// Path relative to the workspace, for display.
    pub rel: String,
    pub ext: String,
    pub size: u64,
}

/// Files with one of `exts` under `dir` (hidden, dependency and build folders skipped; capped at 2000).
pub(crate) fn walk(root: &Path, dir: &Path, depth: usize, exts: &[&str], out: &mut Vec<AssetFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth < MAX_DEPTH && !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                walk(root, &path, depth + 1, exts, out);
            }
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !exts.contains(&ext.as_str()) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(AssetFile {
            path: path.to_string_lossy().into_owned(),
            rel,
            ext,
            size,
        });
    }
}

fn export_dir() -> PathBuf {
    std::env::temp_dir().join("agentdeck-assets")
}

/// Model and image files under the workspace folder (hidden, dependency and build folders skipped).
/// Also lets the webview load files of that folder through the asset protocol (textures next to the models).
#[tauri::command(async)]
pub fn assets_list(app: tauri::AppHandle, cwd: String) -> Result<Vec<AssetFile>, String> {
    let root = PathBuf::from(&cwd);
    if !root.is_dir() {
        return Err(format!("{cwd} is not a folder"));
    }
    let scope = app.asset_protocol_scope();
    scope
        .allow_directory(&root, true)
        .map_err(|e| e.to_string())?;
    scope
        .allow_directory(export_dir(), true)
        .map_err(|e| e.to_string())?;
    Ok(list_models(&root))
}

fn list_models(root: &Path) -> Vec<AssetFile> {
    let mut files = Vec::new();
    let exts: Vec<&str> = MODEL_EXTS.iter().chain(IMAGE_EXTS).copied().collect();
    walk(root, root, 0, &exts, &mut files);
    files.sort_by_key(|f| f.rel.to_lowercase());
    files
}

/// The file must sit inside the workspace: the webview only reads model files of the folder it shows.
pub(crate) fn inside(cwd: &str, path: &str) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(cwd).map_err(|e| format!("cannot open {cwd}: {e}"))?;
    let file = std::fs::canonicalize(path).map_err(|e| format!("cannot open {path}: {e}"))?;
    if !file.starts_with(&root) {
        return Err("file is outside the workspace".into());
    }
    Ok(file)
}

fn find_blender() -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join("blender.exe"))
            .find(|p| p.is_file())
    });
    if on_path.is_some() {
        return on_path;
    }
    // Default installer location: one folder per version, take the newest.
    let mut installs: Vec<PathBuf> = std::fs::read_dir(r"C:\Program Files\Blender Foundation")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("blender.exe"))
        .filter(|p| p.is_file())
        .collect();
    installs.sort();
    installs.pop().or_else(steam_blender)
}

/// Blender installed through Steam, in any Steam library (listed in libraryfolders.vdf).
fn steam_blender() -> Option<PathBuf> {
    let steam = Path::new(r"C:\Program Files (x86)\Steam");
    let vdf =
        std::fs::read_to_string(steam.join(r"steamapps\libraryfolders.vdf")).unwrap_or_default();
    // Lines look like `"path"  "D:\\SteamLibrary"`: backslashes are doubled in the file.
    let libraries = vdf
        .lines()
        .filter_map(|l| l.trim().strip_prefix("\"path\""))
        .map(|v| PathBuf::from(v.trim().trim_matches('"').replace("\\\\", "\\")));
    std::iter::once(steam.to_path_buf())
        .chain(libraries)
        .map(|lib| lib.join(r"steamapps\common\Blender\blender.exe"))
        .find(|p| p.is_file())
}

/// Relinks textures missing at their saved path by searching the workspace, then writes a GLB with textures
/// embedded and vertex colors kept (that option only exists in Blender 4.2+). Args after `--`: out, workspace.
const EXPORT_SCRIPT: &str = "import bpy, sys
out, root = sys.argv[-2], sys.argv[-1]
try:
    bpy.ops.file.find_missing_files(directory=root)
except Exception as e:
    print('find_missing_files failed:', e)
try:
    bpy.ops.export_scene.gltf(filepath=out, export_format='GLB', export_apply=True, export_vertex_color='ACTIVE')
except TypeError:
    bpy.ops.export_scene.gltf(filepath=out, export_format='GLB', export_apply=True)
";

/// Exports the .blend to a cached GLB in %TEMP% (keyed by path + modification time).
// shortcut: cached GLBs are never cleaned; add cleanup if %TEMP%/agentdeck-assets ever grows noticeably.
fn blend_to_glb(file: &Path, workspace: &Path) -> Result<PathBuf, String> {
    let modified = std::fs::metadata(file).and_then(|m| m.modified()).ok();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // The version tag invalidates GLBs exported with older export options.
    (file, modified, "v3").hash(&mut hasher);
    let dir = export_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let out = dir.join(format!("{:x}.glb", hasher.finish()));
    if out.is_file() {
        return Ok(out);
    }
    let blender =
        find_blender().ok_or("Blender not found: install it or add blender.exe to PATH")?;
    let mut cmd = Command::new(blender);
    cmd.arg("-b")
        .arg(file)
        .arg("--python-expr")
        .arg(EXPORT_SCRIPT)
        .arg("--")
        .arg(&out)
        .arg(workspace);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let result = cmd
        .output()
        .map_err(|e| format!("cannot run Blender: {e}"))?;
    if !out.is_file() {
        let log = String::from_utf8_lossy(&result.stdout);
        let reason = log
            .lines()
            .rev()
            .find(|l| l.contains("Error"))
            .unwrap_or("no GLB written");
        return Err(format!("Blender export failed: {}", reason.trim()));
    }
    Ok(out)
}

/// File the preview loads (through the asset protocol): the model itself, or the GLB export of a .blend.
#[tauri::command(async)]
pub fn asset_source(cwd: String, path: String) -> Result<String, String> {
    let file = inside(&cwd, &path)?;
    let is_blend = file
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("blend"));
    let source = if is_blend {
        blend_to_glb(&file, Path::new(&cwd))?
    } else {
        file
    };
    Ok(source.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_models_and_guards_the_workspace() {
        let root =
            std::env::temp_dir().join(format!("agentdeck-assets-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("models")).unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(root.join("models/ship.GLB"), b"x").unwrap();
        std::fs::write(root.join("node_modules/dep.glb"), b"x").unwrap();
        std::fs::write(root.join("readme.md"), b"x").unwrap();
        std::fs::write(root.join("models/ship.png"), b"x").unwrap();
        let files = list_models(&root);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].ext, "glb");
        assert_eq!(files[1].ext, "png");
        assert!(inside(&root.to_string_lossy(), &files[0].path).is_ok());
        let models = root.join("models");
        assert!(
            inside(
                &models.to_string_lossy(),
                &root.join("readme.md").to_string_lossy()
            )
            .is_err()
        );
        std::fs::remove_dir_all(root).ok();
    }
}
