//! Each agent's skills (`<root>/<name>/SKILL.md`) and notes (instruction + memory Markdown files):
//! list, install skills from a GitHub link, remove, read and edit. Also saves pasted images.
//! Only paths this module lists can be read or written: the frontend echoes them back.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::agents::AgentKind;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    /// "user" (all projects) or "project".
    pub scope: &'static str,
    /// The SKILL.md file.
    pub path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteFile {
    pub label: String,
    pub path: String,
    pub exists: bool,
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or_else(|| "home folder not found".into())
}

/// Folder name of the agent's config under home and in a project.
fn config_dir(agent: AgentKind) -> Result<&'static str, String> {
    match agent {
        AgentKind::Claude => Ok(".claude"),
        AgentKind::Codex => Ok(".codex"),
        AgentKind::Gemini => Ok(".gemini"),
        AgentKind::Grok => Ok(".grok"),
        AgentKind::Shell => Err("a shell has no skills or notes".into()),
    }
}

/// Skill folders the agent reads, per scope; the first of each scope is where installs go.
fn skill_roots(agent: AgentKind, cwd: &Path) -> Result<Vec<(&'static str, PathBuf)>, String> {
    let home = home()?;
    let dir = config_dir(agent)?;
    let mut roots = vec![
        ("user", home.join(dir).join("skills")),
        ("project", cwd.join(dir).join("skills")),
    ];
    // Codex and Gemini also read the shared `.agents/skills` folders.
    if matches!(agent, AgentKind::Codex | AgentKind::Gemini) {
        roots.push(("user", home.join(".agents").join("skills")));
        roots.push(("project", cwd.join(".agents").join("skills")));
    }
    Ok(roots)
}

/// One plain path segment: no separators, no `..`, nothing git could read as an option.
fn plain(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// `description:` from the SKILL.md front matter (single line, or a `>`/`|` block).
fn description(skill_md: &str) -> String {
    let mut lines = skill_md.lines();
    if lines.next().map(str::trim) != Some("---") {
        return String::new();
    }
    let front: Vec<&str> = lines.take_while(|l| l.trim() != "---").collect();
    let Some(i) = front.iter().position(|l| l.starts_with("description:")) else {
        return String::new();
    };
    let value = front[i]["description:".len()..].trim();
    if !value.is_empty() && !value.starts_with(['>', '|']) {
        return value.trim_matches(['"', '\'']).to_string();
    }
    front[i + 1..]
        .iter()
        .take_while(|l| l.starts_with([' ', '\t']))
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

fn list_skills(agent: AgentKind, cwd: &Path) -> Result<Vec<SkillInfo>, String> {
    let mut skills = Vec::new();
    for (scope, root) in skill_roots(agent, cwd)? {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path().join("SKILL.md");
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            skills.push(SkillInfo {
                name: entry.file_name().to_string_lossy().into_owned(),
                description: description(&text),
                scope,
                path: path.to_string_lossy().into_owned(),
            });
        }
    }
    skills.sort_by_key(|s| s.name.to_lowercase());
    Ok(skills)
}

#[tauri::command(async)]
pub fn skills_list(agent: AgentKind, cwd: String) -> Result<Vec<SkillInfo>, String> {
    list_skills(agent, Path::new(&cwd))
}

/// Deletes the skill folder holding `path` (a SKILL.md that `skills_list` returned).
#[tauri::command(async)]
pub fn skills_remove(agent: AgentKind, cwd: String, path: String) -> Result<(), String> {
    let skill = list_skills(agent, Path::new(&cwd))?
        .into_iter()
        .find(|s| s.path == path)
        .ok_or("not an installed skill")?;
    let dir = Path::new(&skill.path)
        .parent()
        .ok_or("not an installed skill")?;
    std::fs::remove_dir_all(dir).map_err(|e| format!("cannot remove {}: {e}", skill.name))
}

/// Claude's auto-memory folder for a project: `~/.claude/projects/<cwd with non-alphanumerics as ->/memory`.
fn claude_memory_dir(home: &Path, cwd: &Path) -> PathBuf {
    let slug: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    home.join(".claude")
        .join("projects")
        .join(slug)
        .join("memory")
}

fn markdown_files(dir: &Path, label: &str, out: &mut Vec<NoteFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "md"))
        .collect();
    files.sort();
    for path in files {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        out.push(NoteFile {
            label: format!("{label} · {name}"),
            path: path.to_string_lossy().into_owned(),
            exists: true,
        });
    }
}

/// Instruction files (listed even when missing, so they can be created) and existing memory files.
// shortcut: Grok's per-workspace memory folder is a hash we can't derive, so only its global memory is listed.
fn list_notes(agent: AgentKind, cwd: &Path) -> Result<Vec<NoteFile>, String> {
    let home = home()?;
    let dir = config_dir(agent)?;
    let file = match agent {
        AgentKind::Claude => "CLAUDE.md",
        AgentKind::Gemini => "GEMINI.md",
        _ => "AGENTS.md",
    };
    let mut notes: Vec<NoteFile> = [
        ("Instructions (all projects)", home.join(dir).join(file)),
        ("Instructions (project)", cwd.join(file)),
    ]
    .into_iter()
    .map(|(label, path)| NoteFile {
        label: format!("{label} · {file}"),
        exists: path.is_file(),
        path: path.to_string_lossy().into_owned(),
    })
    .collect();
    match agent {
        AgentKind::Claude => markdown_files(
            &claude_memory_dir(&home, cwd),
            "Memory (project)",
            &mut notes,
        ),
        AgentKind::Grok => {
            let global = home.join(".grok").join("memory-v2").join("global");
            markdown_files(&global, "Memory (all projects)", &mut notes);
            markdown_files(&global.join("topics"), "Memory topic", &mut notes);
        }
        _ => {}
    }
    Ok(notes)
}

#[tauri::command(async)]
pub fn notes_list(agent: AgentKind, cwd: String) -> Result<Vec<NoteFile>, String> {
    list_notes(agent, Path::new(&cwd))
}

/// A note or SKILL.md this module listed for the agent: the only files the editor may touch.
fn editable(agent: AgentKind, cwd: &str, path: &str) -> Result<PathBuf, String> {
    let cwd = Path::new(cwd);
    let listed = list_notes(agent, cwd)?.iter().any(|n| n.path == path)
        || list_skills(agent, cwd)?.iter().any(|s| s.path == path);
    if listed {
        Ok(PathBuf::from(path))
    } else {
        Err(format!("not an editable file: {path}"))
    }
}

/// The file's text, or "" when it does not exist yet.
#[tauri::command(async)]
pub fn agent_file_read(agent: AgentKind, cwd: String, path: String) -> Result<String, String> {
    let path = editable(agent, &cwd, &path)?;
    match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        other => other.map_err(|e| format!("cannot read {}: {e}", path.display())),
    }
}

#[tauri::command(async)]
pub fn agent_file_write(
    agent: AgentKind,
    cwd: String,
    path: String,
    text: String,
) -> Result<(), String> {
    let path = editable(agent, &cwd, &path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, text).map_err(|e| format!("cannot save {}: {e}", path.display()))
}

/// GitHub source: repo, optional branch/tag, optional folder inside the repo.
#[derive(Debug, PartialEq)]
struct Source {
    owner: String,
    repo: String,
    git_ref: Option<String>,
    subdir: Option<String>,
}

/// Accepts `owner/repo`, `https://github.com/owner/repo(.git)` and `.../tree/<ref>/<folder>`.
// shortcut: a branch name containing `/` in a tree URL is read as ref + folder; paste the repo URL then.
fn parse_source(input: &str) -> Result<Source, String> {
    let bad = || format!("not a GitHub link: {input}");
    let path = input
        .trim()
        .trim_end_matches('/')
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_start_matches("github.com/");
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 || !plain(parts[0]) || !plain(parts[1].trim_end_matches(".git")) {
        return Err(bad());
    }
    let mut source = Source {
        owner: parts[0].into(),
        repo: parts[1].trim_end_matches(".git").into(),
        git_ref: None,
        subdir: None,
    };
    if parts.len() > 2 {
        if !matches!(parts[2], "tree" | "blob")
            || parts.len() < 4
            || !parts[3..].iter().all(|p| plain(p))
        {
            return Err(bad());
        }
        source.git_ref = Some(parts[3].into());
        // A blob link to SKILL.md means its folder.
        let folder: Vec<&str> = parts[4..]
            .iter()
            .copied()
            .filter(|p| *p != "SKILL.md")
            .collect();
        source.subdir = (!folder.is_empty()).then(|| folder.join("/"));
    }
    Ok(source)
}

/// Folders holding a SKILL.md, at most a few levels down (repos often keep them in `skills/`).
fn find_skills(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    if dir.join("SKILL.md").is_file() {
        out.push(dir.to_path_buf());
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && entry.file_name() != ".git" {
            find_skills(&path, depth - 1, out);
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_name() == ".git" {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn install_from(source: &Source, tmp: &Path, root: &Path) -> Result<Vec<String>, String> {
    let repo_url = format!("https://github.com/{}/{}.git", source.owner, source.repo);
    let tmp_str = tmp.to_string_lossy();
    let mut args = vec!["clone", "--depth", "1"];
    if let Some(git_ref) = &source.git_ref {
        args.extend(["--branch", git_ref]);
    }
    args.extend(["--", &repo_url, &tmp_str]);
    crate::agents::git(&std::env::temp_dir(), &args)?;

    let base = source
        .subdir
        .as_ref()
        .map_or(tmp.to_path_buf(), |s| tmp.join(s));
    let mut found = Vec::new();
    find_skills(&base, 3, &mut found);
    if found.is_empty() {
        return Err("no SKILL.md found at that link".into());
    }
    let mut names = Vec::new();
    for dir in found {
        // The repo root has the temp folder's name: use the repo's instead.
        let name = if dir == tmp {
            source.repo.clone()
        } else {
            dir.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        if !plain(&name) {
            continue;
        }
        let target = root.join(&name);
        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(|e| format!("cannot update {name}: {e}"))?;
        }
        copy_dir(&dir, &target).map_err(|e| format!("cannot install {name}: {e}"))?;
        names.push(name);
    }
    Ok(names)
}

/// Shallow-clones the repo and copies every skill found into the agent's folder for `scope`
/// (replacing same-named ones). Returns their names.
#[tauri::command(async)]
pub fn skills_install(
    agent: AgentKind,
    cwd: String,
    url: String,
    scope: String,
) -> Result<Vec<String>, String> {
    let source = parse_source(&url)?;
    let root = skill_roots(agent, Path::new(&cwd))?
        .into_iter()
        .find(|(s, _)| *s == scope)
        .map(|(_, root)| root)
        .ok_or_else(|| format!("unknown skill scope: {scope}"))?;
    let tmp = std::env::temp_dir().join(format!("agentdeck-skill-{}", uuid::Uuid::new_v4()));
    let result = install_from(&source, &tmp, &root);
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Saves a pasted image (raw request body) to the temp folder; returns its path for the agent.
// shortcut: files stay in %TEMP%/agentdeck-paste; add cleanup if it ever grows noticeably.
#[tauri::command(async)]
pub fn save_paste_image(request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected raw image bytes".into());
    };
    let ext = match request.headers().get("x-ext").and_then(|v| v.to_str().ok()) {
        Some(e @ ("png" | "jpg" | "gif" | "webp" | "bmp")) => e,
        _ => "png",
    };
    let dir = std::env::temp_dir().join("agentdeck-paste");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()));
    std::fs::write(&path, bytes).map_err(|e| format!("cannot save image: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_links() {
        let s = parse_source("https://github.com/anthropics/skills/tree/main/skills/pdf").unwrap();
        assert_eq!(
            (s.owner.as_str(), s.repo.as_str()),
            ("anthropics", "skills")
        );
        assert_eq!(s.git_ref.as_deref(), Some("main"));
        assert_eq!(s.subdir.as_deref(), Some("skills/pdf"));
        let s = parse_source("owner/repo.git").unwrap();
        assert_eq!((s.repo.as_str(), s.git_ref, s.subdir), ("repo", None, None));
        let s = parse_source("github.com/o/r/blob/dev/x/SKILL.md").unwrap();
        assert_eq!(s.subdir.as_deref(), Some("x"));
        assert!(parse_source("https://github.com/o/r/tree/main/../etc").is_err());
        assert!(parse_source("--upload-pack=x/r").is_err());
        assert!(parse_source("o/r/tree/--branch").is_err());
        assert!(parse_source("nope").is_err());
    }

    #[test]
    fn reads_description() {
        assert_eq!(
            description("---\nname: a\ndescription: \"Hi there\"\n---\nbody"),
            "Hi there"
        );
        assert_eq!(
            description("---\ndescription: >\n  one\n  two\nname: a\n---"),
            "one two"
        );
        assert_eq!(description("no front matter"), "");
    }

    #[test]
    fn claude_memory_slug() {
        let dir = claude_memory_dir(Path::new("H"), Path::new("D:\\agentdeck"));
        assert!(dir.ends_with(Path::new("projects").join("D--agentdeck").join("memory")));
    }

    #[test]
    fn editor_only_touches_listed_files() {
        let cwd = std::env::temp_dir().to_string_lossy().into_owned();
        assert!(
            editable(
                AgentKind::Codex,
                &cwd,
                &Path::new(&cwd).join("AGENTS.md").to_string_lossy()
            )
            .is_ok()
        );
        assert!(
            editable(
                AgentKind::Codex,
                &cwd,
                &Path::new(&cwd).join("secret.txt").to_string_lossy()
            )
            .is_err()
        );
        assert!(editable(AgentKind::Shell, &cwd, "x").is_err());
    }
}
