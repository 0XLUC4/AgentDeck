//! Copies the git-ignored files a new worktree needs (`.env`, local config) from the main checkout,
//! as listed in `<repo>/.worktreeinclude`: one path or glob per line, `#` comments, gitignore-like.

use std::fs;
use std::path::Path;

/// Never copied: rebuilt per worktree (or git's own).
const SKIPPED: [&str; 3] = ["node_modules", "target", ".git"];

/// Copies every existing match of `<main_root>/.worktreeinclude` into `worktree`, keeping files
/// already there. No list file = nothing to do.
pub fn prepare(main_root: &Path, worktree: &Path) -> Result<(), String> {
    let Ok(list) = fs::read_to_string(main_root.join(".worktreeinclude")) else {
        return Ok(());
    };
    let patterns: Vec<&str> = list
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.trim_matches('/'))
        .filter(|l| !l.is_empty() && !l.split('/').any(|part| part == ".."))
        .collect();
    let mut found = Vec::new();
    for pattern in &patterns {
        if pattern.contains(['*', '?']) {
            walk(main_root, "", pattern, &mut found);
        } else if main_root.join(pattern).exists() {
            found.push((*pattern).to_owned());
        }
    }
    for rel in found {
        copy(&main_root.join(&rel), &worktree.join(&rel)).map_err(|e| format!(".worktreeinclude {rel}: {e}"))?;
    }
    Ok(())
}

fn skipped(name: &std::ffi::OsStr) -> bool {
    SKIPPED.iter().any(|s| name == *s)
}

/// Collects paths under `root/dir` matching `pattern` (a pattern without `/` matches a name at any depth).
fn walk(root: &Path, dir: &str, pattern: &str, found: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(root.join(dir)) else { return };
    for entry in entries.flatten() {
        if skipped(&entry.file_name()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
        let subject = if pattern.contains('/') { &rel } else { &name };
        if glob_match(pattern, subject) {
            found.push(rel);
        } else if entry.file_type().is_ok_and(|t| t.is_dir()) {
            walk(root, &rel, pattern, found);
        }
    }
}

/// `*` = any run without `/`, `**` = any run, `?` = one character.
fn glob_match(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match p {
            [] => t.is_empty(),
            ['*', '*', rest @ ..] => {
                let after_slash = rest.strip_prefix(&['/']).unwrap_or(rest);
                (0..=t.len()).any(|i| go(after_slash, &t[i..]) || go(rest, &t[i..]))
            }
            ['*', rest @ ..] => (0..=t.len()).take_while(|&i| i == 0 || t[i - 1] != '/').any(|i| go(rest, &t[i..])),
            ['?', rest @ ..] => t.first().is_some_and(|&c| c != '/') && go(rest, &t[1..]),
            [c, rest @ ..] => t.first() == Some(c) && go(rest, &t[1..]),
        }
    }
    go(&pattern.chars().collect::<Vec<_>>(), &text.chars().collect::<Vec<_>>())
}

/// Copies a file or folder tree (minus [`SKIPPED`]); never overwrites what the checkout has.
fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            if !skipped(&entry.file_name()) {
                copy(&entry.path(), &to.join(entry.file_name()))?;
            }
        }
    } else if !to.exists() {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(from, to)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("*.env", "a.env") && !glob_match("*.env", "a/b.env"));
        assert!(glob_match("config/**/*.json", "config/a/b/c.json") && glob_match("config/**/*.json", "config/c.json"));
        assert!(glob_match(".env.?", ".env.1") && !glob_match(".env.?", ".env.10"));
    }

    #[test]
    fn copies_listed_files_only() {
        let base = std::env::temp_dir().join(format!("agentdeck-prep-{}", uuid::Uuid::new_v4()));
        let (main, wt) = (base.join("main"), base.join("wt"));
        for dir in ["conf/node_modules", "nested/deep", "other"] {
            fs::create_dir_all(main.join(dir)).unwrap();
        }
        fs::create_dir_all(&wt).unwrap();
        for (file, text) in [
            (".env", "SECRET=1"),
            ("conf/local.toml", "a"),
            ("conf/node_modules/x.js", "x"),
            ("nested/deep/k.key", "k"),
            ("other/skip.txt", "s"),
        ] {
            fs::write(main.join(file), text).unwrap();
        }
        fs::write(main.join(".worktreeinclude"), "# local files\n.env\n/conf/\n*.key\nmissing.txt\n../escape\n").unwrap();
        fs::write(wt.join(".env"), "KEPT").unwrap();

        prepare(&main, &wt).unwrap();
        assert_eq!(fs::read_to_string(wt.join(".env")).unwrap(), "KEPT");
        assert!(wt.join("conf/local.toml").is_file() && wt.join("nested/deep/k.key").is_file());
        assert!(!wt.join("conf/node_modules").exists() && !wt.join("other").exists());
        assert!(prepare(&base.join("none"), &wt).is_ok());
        let _ = fs::remove_dir_all(&base);
    }
}
