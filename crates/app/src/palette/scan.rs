//! Blocking reads of the finders, run off the UI thread: the files of a folder (git, else a walk)
//! and the project commands it offers (package.json scripts, Cargo).

use std::path::Path;
use std::process::Command;

use agentdeck_core::logic::project_commands::{self as pc, ProjectCommand};

use super::rank::FileEntry;

/// Files kept per pass: a huge folder still opens fast.
const MAX_FILES: usize = 50_000;
/// Never walked when git is not there (same list as the Files page).
const SKIPPED_FOLDERS: &[&str] = &[".git", "node_modules", "target", "dist", ".next"];

/// Paths printed by `git ls-files -z ...` in `root`, relative to it.
fn git_paths(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let safe = format!("safe.directory={}", root.to_string_lossy().replace('\\', "/"));
    let mut cmd = Command::new("git");
    cmd.arg("-c").arg(safe).arg("-C").arg(root).args(args).env("GIT_OPTIONAL_LOCKS", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    let mut paths: Vec<String> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    paths.dedup(); // a conflicted file is listed once per stage
    Ok(paths)
}

/// First pass (`ignored` false): tracked and untracked files, or a walk outside git. Second pass:
/// gitignored files; ignored folders (target/, node_modules/) are skipped as a whole.
pub fn list_files(root: &Path, ignored: bool) -> Vec<FileEntry> {
    let args: &[&str] = if ignored {
        &["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory"]
    } else {
        &["ls-files", "-z", "--cached", "--others", "--exclude-standard"]
    };
    match git_paths(root, args) {
        Ok(paths) => paths
            .into_iter()
            .filter(|p| !p.ends_with('/'))
            .take(MAX_FILES)
            .map(|path| FileEntry { path, ignored })
            .collect(),
        Err(_) if !ignored => walk(root),
        Err(_) => Vec::new(),
    }
}

/// Files under `root` without git, skipping build and dependency folders.
fn walk(root: &Path) -> Vec<FileEntry> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                if !SKIPPED_FOLDERS.contains(&name.as_str()) {
                    pending.push(entry.path());
                }
                continue;
            }
            let path = entry.path();
            let rel = path.strip_prefix(root).map_or(name, |p| p.to_string_lossy().replace('\\', "/"));
            found.push(FileEntry { path: rel, ignored: false });
            if found.len() >= MAX_FILES {
                return found;
            }
        }
    }
    found
}

/// `bin` can be started from the PATH (PATHEXT and npm's .ps1 shims on Windows).
fn on_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    let mut exts = vec![String::new()];
    if cfg!(windows) {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
        exts.extend(pathext.split(';').filter(|e| !e.is_empty()).map(str::to_lowercase));
        exts.push(".ps1".into());
    }
    std::env::split_paths(&path).any(|dir| exts.iter().any(|ext| dir.join(format!("{bin}{ext}")).is_file()))
}

/// What "Run project command…" offers in `dir`, and why some commands are missing.
#[derive(Debug, Default)]
pub struct Detected {
    pub commands: Vec<ProjectCommand>,
    pub problems: Vec<String>,
}

pub fn detect_commands(dir: &Path) -> Detected {
    let mut found = Detected::default();
    let package = dir.join("package.json");
    if package.is_file() {
        let scripts = std::fs::read_to_string(&package)
            .map_err(|e| format!("package.json: {e}"))
            .and_then(|text| pc::package_scripts(&text));
        match scripts {
            Ok(scripts) if scripts.is_empty() => found.problems.push("package.json declares no scripts.".into()),
            Ok(scripts) => {
                let (runner, lock) = pc::package_runner(|file| dir.join(file).is_file());
                if on_path(runner) {
                    found.commands.extend(pc::script_commands(runner, &scripts));
                } else {
                    found.problems.push(match lock {
                        Some(lock) => format!("{runner} is not installed (not found on PATH), and {lock} asks for it."),
                        None => format!("{runner} is not installed (not found on PATH)."),
                    });
                }
            }
            Err(e) => found.problems.push(e),
        }
    }
    if dir.join("Cargo.toml").is_file() {
        if on_path("cargo") {
            found.commands.extend(pc::cargo_commands());
        } else {
            found.problems.push("cargo is not installed (not found on PATH).".into());
        }
    }
    if found.commands.is_empty() && found.problems.is_empty() {
        found.problems.push(format!("No package.json or Cargo.toml in {}.", dir.display()));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::detect_commands;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("agentdeck-scan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn invalid_manifest_and_empty_folder_explain_themselves() {
        let dir = temp_dir("bad json");
        std::fs::write(dir.join("package.json"), "{ nope").unwrap();
        let found = detect_commands(&dir);
        assert!(found.commands.is_empty());
        assert!(found.problems[0].contains("not valid JSON"), "{:?}", found.problems);
        let empty = temp_dir("empty");
        assert!(detect_commands(&empty).problems[0].starts_with("No package.json"));
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(empty);
    }

    #[test]
    fn missing_package_manager_is_named() {
        let dir = temp_dir("no runner");
        std::fs::write(dir.join("package.json"), r#"{"scripts":{"dev":"vite"}}"#).unwrap();
        // A lock file for a manager that cannot be on the PATH under this name.
        std::fs::write(dir.join("bun.lockb"), "").unwrap();
        let found = detect_commands(&dir);
        if found.commands.is_empty() {
            assert!(found.problems[0].starts_with("bun is not installed"), "{:?}", found.problems);
        } else {
            assert_eq!(found.commands[0].display(), "bun run dev"); // bun happens to be installed
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
