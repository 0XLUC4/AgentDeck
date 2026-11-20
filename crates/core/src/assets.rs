//! 3D assets of a workspace: listing model files and reading them for the in-app preview.
//! `.blend` files are converted to GLB with the installed Blender (headless) since browsers cannot read them.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

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
pub fn assets_list(cwd: String) -> Result<Vec<AssetFile>, String> {
    let root = PathBuf::from(&cwd);
    if !root.is_dir() {
        return Err(format!("{cwd} is not a folder"));
    }
    Ok(list_models(&root))
}

fn list_models(root: &Path) -> Vec<AssetFile> {
    let mut files = Vec::new();
    let exts: Vec<&str> = MODEL_EXTS.iter().chain(IMAGE_EXTS).copied().collect();
    walk(root, root, 0, &exts, &mut files);
    files.sort_by_key(|f| f.rel.to_lowercase());
    files
}

/// Markdown reports, listed among the changed deliverables only.
const MARKDOWN_EXTS: &[&str] = &["md", "markdown"];

/// Deliverables changed in a Git worktree, for the "Changed deliverables" view.
#[derive(Debug)]
pub struct ChangedAssets {
    /// Root of the worktree: the files' `rel` are relative to it, previews read under it.
    pub root: String,
    pub files: Vec<AssetFile>,
}

/// Paths `git status --porcelain -z --no-renames` reports as modified, added or untracked; deleted
/// ones (in the index or the working tree) are left out.
fn changed_paths(porcelain: &str) -> Vec<&str> {
    porcelain
        .split('\0')
        .filter(|record| record.len() > 3)
        .filter(|record| !record[..2].contains('D'))
        .map(|record| &record[3..])
        .collect()
}

/// Images, 3D models and Markdown files modified or untracked in the Git worktree holding `dir`
/// (ignored and deleted files left out). Git only tells which worktree a file changed in, not
/// which agent wrote it. Blocking: call off the UI thread.
pub fn changed_deliverables(dir: String) -> Result<ChangedAssets, String> {
    let start = Path::new(&dir);
    if !start.is_dir() {
        return Err(format!("{dir} is not a folder"));
    }
    let root = crate::agents::find_repo_root(start).ok_or_else(|| format!("not a Git repository: {dir}"))?;
    let mut cmd = Command::new("git");
    cmd.arg("-c")
        .arg(format!("safe.directory={}", root.to_string_lossy().replace('\\', "/")))
        .arg("-C")
        .arg(&root)
        .args(["status", "--porcelain", "-z", "--no-renames", "--untracked-files=all"])
        .env("GIT_OPTIONAL_LOCKS", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git status: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let porcelain = String::from_utf8_lossy(&out.stdout);
    let exts: Vec<&str> = MODEL_EXTS.iter().chain(IMAGE_EXTS).chain(MARKDOWN_EXTS).copied().collect();
    let mut files: Vec<AssetFile> = changed_paths(&porcelain)
        .into_iter()
        .filter_map(|rel| {
            let rel = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
            let path = root.join(&rel);
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            // Deleted since git looked, or a folder (submodule): not a deliverable.
            let meta = std::fs::metadata(&path).ok().filter(std::fs::Metadata::is_file)?;
            exts.contains(&ext.as_str()).then(|| AssetFile {
                path: path.to_string_lossy().into_owned(),
                rel,
                ext,
                size: meta.len(),
            })
        })
        .collect();
    files.sort_by_key(|f| f.rel.to_lowercase());
    Ok(ChangedAssets { root: root.to_string_lossy().into_owned(), files })
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

/// Run by Blender before the export: an empty scene with the FBX imported. The export script
/// reads the last two arguments, so the FBX path is the third from the end.
const IMPORT_FBX: &str = "import bpy, sys
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.fbx(filepath=sys.argv[-3])";

/// Exports the .blend or .fbx to a cached GLB in %TEMP% (keyed by path + modification time).
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
    // A .blend is opened as the scene; an FBX is imported into an empty one.
    let fbx = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("fbx"));
    cmd.arg("-b");
    if fbx {
        cmd.arg("--python-expr").arg(IMPORT_FBX);
    } else {
        cmd.arg(file);
    }
    cmd.arg("--python-expr").arg(EXPORT_SCRIPT).arg("--");
    if fbx {
        cmd.arg(file);
    }
    cmd.arg(&out).arg(workspace);
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
pub fn asset_source(cwd: String, path: String) -> Result<String, String> {
    let file = inside(&cwd, &path)?;
    // Blender reads both and hands back a GLB the viewer can load.
    let is_blend = file
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("blend") || e.eq_ignore_ascii_case("fbx"));
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

    #[test]
    fn changed_paths_skip_deletions() {
        let out = " M models/ship.glb\0?? shots/a.png\0D  old.png\0 D gone.md\0AD tmp.md\0A  docs/r.md\0";
        assert_eq!(changed_paths(out), ["models/ship.glb", "shots/a.png", "docs/r.md"]);
        assert!(changed_paths("").is_empty());
    }

    fn git(dir: &Path, args: &[&str]) -> bool {
        Command::new("git").arg("-C").arg(dir).args(args).output().is_ok_and(|o| o.status.success())
    }

    /// A repo with one commit: a tracked model, a tracked image to delete, a tracked source file.
    /// None when git cannot be run.
    fn temp_repo(base: &Path, name: &str) -> Option<PathBuf> {
        let repo = base.join(name);
        std::fs::create_dir_all(repo.join("models")).unwrap();
        std::fs::write(repo.join("models/ship.obj"), "v 0 0 0\n").unwrap();
        std::fs::write(repo.join("old.png"), b"x").unwrap();
        std::fs::write(repo.join("main.rs"), "fn main() {}\n").unwrap();
        let ok = git(&repo, &["init", "-q"])
            && git(&repo, &["config", "user.name", "t"])
            && git(&repo, &["config", "user.email", "t@t"])
            && git(&repo, &["add", "."])
            && git(&repo, &["commit", "-q", "-m", "init"]);
        ok.then_some(repo)
    }

    /// Brief scenario: same file names in two worktrees, a modified tracked model, an untracked
    /// capture and report; deleted files and other formats left out; each list points to its own copy.
    #[test]
    fn lists_changed_deliverables_per_worktree() {
        let base = std::env::temp_dir().join(format!("agentdeck-changed-{}", uuid::Uuid::new_v4()));
        let Some(one) = temp_repo(&base, "one") else {
            eprintln!("git not available: skipped");
            let _ = std::fs::remove_dir_all(&base);
            return;
        };
        // A linked worktree of the same repo, as AgentDeck creates for an agent.
        let two = base.join("two");
        assert!(git(&one, &["worktree", "add", "-q", "-b", "agentdeck/two", &two.to_string_lossy()]));
        for repo in [&one, &two] {
            std::fs::write(repo.join("models/ship.obj"), "v 1 1 1\n").unwrap(); // modified, tracked
            std::fs::create_dir_all(repo.join("out")).unwrap();
            std::fs::write(repo.join("out/capture.png"), b"png").unwrap(); // untracked
            std::fs::write(repo.join("out/report.md"), "# Report\n").unwrap(); // untracked
            std::fs::write(repo.join("main.rs"), "fn main() { }\n").unwrap(); // not a deliverable
            std::fs::remove_file(repo.join("old.png")).unwrap(); // deleted
        }
        std::fs::write(two.join("out/report.md"), "# Second\n").unwrap();
        let sep = std::path::MAIN_SEPARATOR;
        // Asked from a subfolder: the whole worktree is listed, relative to its root.
        let listed = changed_deliverables(one.join("out").to_string_lossy().into_owned()).unwrap();
        let rels: Vec<&str> = listed.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, [format!("models{sep}ship.obj"), format!("out{sep}capture.png"), format!("out{sep}report.md")]);
        assert_eq!(Path::new(&listed.root), one.as_path());
        let other = changed_deliverables(two.to_string_lossy().into_owned()).unwrap();
        let report = other.files.iter().find(|f| f.ext == "md").unwrap();
        assert!(report.path.starts_with(&*two.to_string_lossy()));
        assert_eq!(std::fs::read_to_string(&report.path).unwrap(), "# Second\n");
        assert!(inside(&other.root, &report.path).is_ok());

        // No changes: an empty list, not an error.
        assert!(git(&one, &["add", "-A"]) && git(&one, &["commit", "-q", "-m", "work"]));
        assert!(changed_deliverables(one.to_string_lossy().into_owned()).unwrap().files.is_empty());
        // Outside Git: an error the view shows; the usual gallery still lists the folder.
        let plain = base.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("a.png"), b"x").unwrap();
        // A temp dir inside a repo would be found as one: only check when it is not.
        if crate::agents::find_repo_root(&plain).is_none() {
            assert!(changed_deliverables(plain.to_string_lossy().into_owned()).unwrap_err().contains("not a Git repository"));
        }
        assert_eq!(assets_list(plain.to_string_lossy().into_owned()).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }
}
