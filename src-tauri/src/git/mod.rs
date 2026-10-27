//! Git panel backend: status and diffs of a pane's working tree, commit, merge/removal of
//! AgentDeck worktrees, task checkpoints and comparison of two panes' trees.
//!
//! Every view is computed from a *snapshot tree*: the working tree (untracked files included,
//! ignored files excluded) written to the object store through a temporary index, so the real
//! index and working tree are never touched. git always runs directly (no shell), paths after `--`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::agents::find_repo_root;

/// Branch prefix of the worktrees created by `agents::create_worktree`.
const BRANCH_PREFIX: &str = "agentdeck/";
const CHECKPOINT_REFS: &str = "refs/agentdeck/checkpoints";
const CHECKPOINT_SUBJECT: &str = "agentdeck checkpoint";
const CHECKPOINTS_KEPT: usize = 50;
// Note: SHA-1 empty tree only; SHA-256 repos without any commit can't show a status yet.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    /// `A`, `M`, `D` or `T` (untracked files show as `A`).
    pub status: String,
    /// `None` for binary files.
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub root: String,
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    /// `main` or `master`, when one exists.
    pub base: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Linked worktree on an `agentdeck/*` branch: merge and removal are allowed.
    pub agent_worktree: bool,
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub sha: String,
    /// Creation time, ms since the Unix epoch.
    pub ts: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    /// Snapshot trees to pass back to `git_diff_trees`.
    pub from: String,
    pub to: String,
    pub files: Vec<FileChange>,
}

/// Runs git in `dir` (which must be a worktree root, for `safe.directory`), optionally on another index.
fn run(dir: &Path, args: &[&str], index: Option<&Path>) -> Result<String, String> {
    let safe = format!(
        "safe.directory={}",
        dir.to_string_lossy().replace('\\', "/")
    );
    let mut cmd = Command::new("git");
    cmd.arg("-c").arg(safe).arg("-C").arg(dir).args(args);
    if let Some(index) = index {
        cmd.env("GIT_INDEX_FILE", index);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("cannot run git: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    // `git commit` reports "nothing to commit" on stdout.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let msg = if stderr.trim().is_empty() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        stderr.into_owned()
    };
    Err(format!("git {}: {}", args.join(" "), msg.trim()))
}

fn run_trim(dir: &Path, args: &[&str]) -> Result<String, String> {
    run(dir, args, None).map(|s| s.trim().to_string())
}

/// Validates `cwd` (existing directory inside a git checkout) and returns the checkout root.
fn repo_root(cwd: &str) -> Result<PathBuf, String> {
    let dir = Path::new(cwd);
    if cwd.is_empty() || !dir.is_dir() {
        return Err(format!("folder not found: {cwd}"));
    }
    find_repo_root(dir).ok_or_else(|| format!("not a Git repository: {cwd}"))
}

fn is_hex_sha(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn check_pane_id(pane_id: &str) -> Result<(), String> {
    let ok = !pane_id.is_empty()
        && pane_id.len() <= 64
        && pane_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok {
        Ok(())
    } else {
        Err("invalid pane id".into())
    }
}

fn head(root: &Path) -> Option<String> {
    run_trim(root, &["rev-parse", "--verify", "--quiet", "HEAD"]).ok()
}

fn current_branch(root: &Path) -> Option<String> {
    run_trim(root, &["symbolic-ref", "--short", "-q", "HEAD"]).ok()
}

fn base_branch(root: &Path) -> Option<String> {
    ["main", "master"]
        .into_iter()
        .find(|b| {
            run(
                root,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{b}"),
                ],
                None,
            )
            .is_ok()
        })
        .map(str::to_owned)
}

/// Root of the main checkout (first entry of `git worktree list`).
fn main_worktree(root: &Path) -> Result<PathBuf, String> {
    let list = run(root, &["worktree", "list", "--porcelain"], None)?;
    list.lines()
        .next()
        .and_then(|l| l.strip_prefix("worktree "))
        .map(PathBuf::from)
        .ok_or_else(|| "main repository not found".into())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Branch of a linked AgentDeck worktree plus the main checkout root, or why it isn't one.
fn agent_worktree(root: &Path) -> Result<(String, PathBuf), String> {
    let branch = current_branch(root)
        .filter(|b| b.starts_with(BRANCH_PREFIX))
        .ok_or("only for AgentDeck worktrees (agentdeck/* branch)")?;
    let main = main_worktree(root)?;
    if same_dir(&main, root) {
        return Err("this folder is the main repository, not a worktree".into());
    }
    Ok((branch, main))
}

/// Writes the whole working tree (untracked included, ignored excluded) as a tree object.
fn snapshot_tree(root: &Path) -> Result<String, String> {
    let index = std::env::temp_dir().join(format!("agentdeck-index-{}", uuid::Uuid::new_v4()));
    let real = run_trim(
        root,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?;
    // Seeding from the real index keeps git's stat cache: only changed files get hashed.
    if Path::new(&real).is_file() {
        std::fs::copy(&real, &index).map_err(|e| format!("copy index: {e}"))?;
    }
    let tree = run(root, &["add", "-A"], Some(&index))
        .and_then(|_| run(root, &["write-tree"], Some(&index)));
    let _ = std::fs::remove_file(&index);
    Ok(tree?.trim().to_string())
}

/// Parses `git diff --name-status -z` output: `X\0path\0` records.
fn parse_name_status(out: &str) -> Vec<(String, String)> {
    let mut parts = out.split('\0').filter(|s| !s.is_empty());
    let mut entries = Vec::new();
    while let (Some(status), Some(path)) = (parts.next(), parts.next()) {
        entries.push((status.to_string(), path.to_string()));
    }
    entries
}

/// Parses `git diff --numstat -z --no-renames` output: `add\tdel\tpath\0` (`-` for binary).
fn parse_numstat(out: &str) -> HashMap<String, (Option<u32>, Option<u32>)> {
    out.split('\0')
        .filter_map(|rec| {
            let mut f = rec.splitn(3, '\t');
            let (add, del, path) = (f.next()?, f.next()?, f.next()?);
            Some((path.to_string(), (add.parse().ok(), del.parse().ok())))
        })
        .collect()
}

/// Files changed between two tree-ish, with line counts.
fn changes(root: &Path, from: &str, to: &str) -> Result<Vec<FileChange>, String> {
    let names = run(
        root,
        &["diff", "--name-status", "-z", "--no-renames", from, to],
        None,
    )?;
    let counts = parse_numstat(&run(
        root,
        &["diff", "--numstat", "-z", "--no-renames", from, to],
        None,
    )?);
    Ok(parse_name_status(&names)
        .into_iter()
        .map(|(status, path)| {
            let (additions, deletions) = counts.get(&path).copied().unwrap_or((None, None));
            FileChange {
                path,
                status,
                additions,
                deletions,
            }
        })
        .collect())
}

fn unified_diff(root: &Path, from: &str, to: &str, path: &str) -> Result<String, String> {
    if path.is_empty() {
        return Err("missing file".into());
    }
    run(
        root,
        &["diff", "--no-color", "--no-ext-diff", from, to, "--", path],
        None,
    )
}

fn status(cwd: &str) -> Result<GitStatus, String> {
    let root = repo_root(cwd)?;
    let branch = current_branch(&root);
    let base = base_branch(&root);
    let (mut ahead, mut behind) = (0, 0);
    if let (Some(b), Some(br)) = (&base, &branch)
        && b != br
        && let Ok(out) = run_trim(
            &root,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{b}...HEAD"),
            ],
        )
    {
        let mut n = out.split_whitespace().map(|n| n.parse().unwrap_or(0));
        behind = n.next().unwrap_or(0);
        ahead = n.next().unwrap_or(0);
    }
    let tree = snapshot_tree(&root)?;
    let from = head(&root).unwrap_or_else(|| EMPTY_TREE.into());
    Ok(GitStatus {
        root: root.to_string_lossy().into_owned(),
        agent_worktree: agent_worktree(&root).is_ok(),
        files: changes(&root, &from, &tree)?,
        branch,
        base,
        ahead,
        behind,
    })
}

fn commit(cwd: &str, message: &str) -> Result<String, String> {
    let root = repo_root(cwd)?;
    let message = message.trim();
    if message.is_empty() {
        return Err("empty commit message".into());
    }
    run(&root, &["add", "-A"], None)?;
    run(&root, &["commit", "-q", "-m", message], None)?;
    run_trim(&root, &["rev-parse", "--short", "HEAD"])
}

fn merge_worktree(cwd: &str) -> Result<String, String> {
    let root = repo_root(cwd)?;
    let (branch, main) = agent_worktree(&root)?;
    if !run_trim(&root, &["status", "--porcelain"])?.is_empty() {
        return Err(format!(
            "{branch} has uncommitted changes: commit them first"
        ));
    }
    let dirty = run_trim(&main, &["status", "--porcelain", "--untracked-files=no"])?;
    if !dirty.is_empty() {
        return Err(format!(
            "the main repository ({}) has uncommitted changes, merge refused:\n{dirty}",
            main.display()
        ));
    }
    let target = current_branch(&main).unwrap_or_else(|| "HEAD".into());
    if let Err(err) = run(&main, &["merge", "--no-ff", "--no-edit", &branch], None) {
        let conflicts = run_trim(&main, &["diff", "--name-only", "--diff-filter=U"])
            .unwrap_or_default()
            .replace('\n', ", ");
        let _ = run(&main, &["merge", "--abort"], None);
        return Err(if conflicts.is_empty() {
            format!("cannot merge {branch} into {target}: {err}")
        } else {
            format!("conflicts merging {branch} into {target} (merge aborted): {conflicts}")
        });
    }
    Ok(format!("{branch} merged into {target}"))
}

/// Pushes the worktree's branch to `origin` and opens a pull request with the GitHub CLI; returns its URL.
fn create_pr(cwd: &str, title: &str, body: &str) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() || title.len() > 256 || title.contains(char::is_control) {
        return Err("title: 1 to 256 characters, single line".into());
    }
    let root = repo_root(cwd)?;
    let (branch, main) = agent_worktree(&root)?;
    if !run_trim(&root, &["status", "--porcelain"])?.is_empty() {
        return Err(format!(
            "{branch} has uncommitted changes: commit them first"
        ));
    }
    let base = current_branch(&main)
        .or_else(|| base_branch(&root))
        .ok_or("no base branch found")?;
    // Fail before pushing when the GitHub CLI is missing or logged out.
    gh(&root, &["auth", "status"])?;
    run(&root, &["push", "-u", "origin", &branch], None)?;
    let out = gh(
        &root,
        &[
            "pr", "create", "--head", &branch, "--base", &base, "--title", title, "--body", body,
        ],
    )?;
    // gh prints the PR URL last.
    Ok(out.lines().last().unwrap_or_default().to_string())
}

fn gh(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("gh");
    cmd.current_dir(root).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd
        .output()
        .map_err(|e| format!("cannot run gh (install the GitHub CLI): {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    Err(format!(
        "gh {}: {}",
        args[0],
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

fn remove_worktree(cwd: &str) -> Result<(), String> {
    let root = repo_root(cwd)?;
    let (branch, main) = agent_worktree(&root)?;
    run(
        &main,
        &["worktree", "remove", &root.to_string_lossy()],
        None,
    )?;
    run(&main, &["branch", "-D", &branch], None).map(drop)
}

/// Directory where a pane's agent runs: its worktree when it has one (see `agents::create_worktree`).
fn pane_dir(cwd: &str, pane_id: &str, worktree: bool) -> String {
    let short: String = pane_id.chars().take(8).collect();
    let target = worktree
        .then(|| find_repo_root(Path::new(cwd)))
        .flatten()
        .and_then(|root| {
            let name = root.file_name()?.to_string_lossy().into_owned();
            Some(root.parent()?.join(format!("{name}-agentdeck-{short}")))
        })
        .filter(|p| p.is_dir());
    target.map_or_else(|| cwd.to_string(), |p| p.to_string_lossy().into_owned())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn checkpoint_refs(root: &Path, pane_id: &str) -> Result<Vec<(String, Checkpoint)>, String> {
    let out = run(
        root,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            &format!("{CHECKPOINT_REFS}/{pane_id}/"),
        ],
        None,
    )?;
    let mut refs: Vec<(String, Checkpoint)> = out
        .lines()
        .filter_map(|line| {
            let (name, sha) = line.split_once(' ')?;
            let ts = name.rsplit('/').next()?.parse().ok()?;
            Some((
                name.to_string(),
                Checkpoint {
                    sha: sha.to_string(),
                    ts,
                },
            ))
        })
        .collect();
    refs.sort_by_key(|(_, c)| std::cmp::Reverse(c.ts));
    Ok(refs)
}

/// Records the exact working tree state under `refs/agentdeck/checkpoints/<pane>/<ts>`.
/// A clean tree records HEAD itself; otherwise a commit (parent HEAD) holding the snapshot.
fn checkpoint(cwd: &str, pane_id: &str) -> Result<Option<String>, String> {
    check_pane_id(pane_id)?;
    let Ok(root) = repo_root(cwd) else {
        return Ok(None); // not a repo: nothing to protect
    };
    let tree = snapshot_tree(&root)?;
    let head = head(&root);
    let head_tree = head
        .as_ref()
        .and_then(|h| run_trim(&root, &["rev-parse", &format!("{h}^{{tree}}")]).ok());
    let sha = match head {
        Some(h) if head_tree.as_deref() == Some(tree.as_str()) => h,
        _ => {
            let mut args = vec![
                "-c",
                "user.name=AgentDeck",
                "-c",
                "user.email=agentdeck@localhost",
                "commit-tree",
                &tree,
                "-m",
                CHECKPOINT_SUBJECT,
            ];
            if let Some(h) = &head {
                args.extend(["-p", h]);
            }
            run_trim(&root, &args)?
        }
    };
    let refs = checkpoint_refs(&root, pane_id)?;
    // Same millisecond as the previous one: bump so refs stay unique and ordered.
    let ts = refs.first().map_or(0, |(_, c)| c.ts + 1).max(now_ms());
    run(
        &root,
        &[
            "update-ref",
            &format!("{CHECKPOINT_REFS}/{pane_id}/{ts}"),
            &sha,
        ],
        None,
    )?;
    for (name, _) in refs.iter().skip(CHECKPOINTS_KEPT - 1) {
        let _ = run(&root, &["update-ref", "-d", name], None);
    }
    Ok(Some(sha))
}

/// Puts the working tree back exactly as in checkpoint `sha` (tracked + untracked, ignored files
/// untouched) and HEAD back to where it was. The current state is checkpointed first.
/// Returns the safety checkpoint's sha.
fn restore_checkpoint(cwd: &str, pane_id: &str, sha: &str) -> Result<Option<String>, String> {
    check_pane_id(pane_id)?;
    if !is_hex_sha(sha) {
        return Err("invalid restore point".into());
    }
    let root = repo_root(cwd)?;
    if !checkpoint_refs(&root, pane_id)?
        .iter()
        .any(|(_, c)| c.sha == sha)
    {
        return Err("unknown restore point".into());
    }
    let info = run_trim(&root, &["log", "-1", "--format=%P%n%s", sha])?;
    let (parents, subject) = info.split_once('\n').unwrap_or((&info, ""));
    // A snapshot commit sits on the HEAD of its time; a clean checkpoint *is* that HEAD.
    let base = if subject == CHECKPOINT_SUBJECT {
        parents.split_whitespace().next().map(str::to_owned)
    } else {
        Some(sha.to_string())
    };
    let backup = checkpoint(cwd, pane_id)?;
    run(&root, &["read-tree", "--reset", "-u", sha], None)?;
    run(&root, &["clean", "-fdq"], None)?; // index == checkpoint: drops files created since
    if let Some(base) = base {
        run(&root, &["reset", "-q", &base], None)?; // mixed: snapshot-only files become untracked again
    }
    Ok(backup)
}

fn compare(cwd: &str, other_cwd: &str) -> Result<Comparison, String> {
    let (a, b) = (repo_root(cwd)?, repo_root(other_cwd)?);
    let common = |root: &Path| {
        run_trim(
            root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
    };
    if !same_dir(Path::new(&common(&a)?), Path::new(&common(&b)?)) {
        return Err("the two panes are not in the same repository".into());
    }
    let (from, to) = (snapshot_tree(&a)?, snapshot_tree(&b)?);
    Ok(Comparison {
        files: changes(&a, &from, &to)?,
        from,
        to,
    })
}

// ---- Tauri commands (run on the async pool: git can be slow on big repos) ----

#[tauri::command(async)]
pub fn git_status(cwd: String) -> Result<GitStatus, String> {
    status(&cwd)
}

/// Unified diff of one file: HEAD vs current working tree (untracked files included).
#[tauri::command(async)]
pub fn git_diff(cwd: String, path: String) -> Result<String, String> {
    let root = repo_root(&cwd)?;
    let tree = snapshot_tree(&root)?;
    let from = head(&root).unwrap_or_else(|| EMPTY_TREE.into());
    unified_diff(&root, &from, &tree, &path)
}

/// Unified diff of one file between two trees returned by `git_compare`.
#[tauri::command(async)]
pub fn git_diff_trees(
    cwd: String,
    from: String,
    to: String,
    path: String,
) -> Result<String, String> {
    if !is_hex_sha(&from) || !is_hex_sha(&to) {
        return Err("invalid tree".into());
    }
    unified_diff(&repo_root(&cwd)?, &from, &to, &path)
}

/// Stages everything and commits; returns the short sha.
#[tauri::command(async)]
pub fn git_commit(cwd: String, message: String) -> Result<String, String> {
    commit(&cwd, &message)
}

#[tauri::command(async)]
pub fn git_merge_worktree(cwd: String) -> Result<String, String> {
    merge_worktree(&cwd)
}

#[tauri::command(async)]
pub fn git_create_pr(cwd: String, title: String, body: String) -> Result<String, String> {
    create_pr(&cwd, &title, &body)
}

#[tauri::command(async)]
pub fn git_remove_worktree(cwd: String) -> Result<(), String> {
    remove_worktree(&cwd)
}

#[tauri::command(async)]
pub fn git_pane_dir(cwd: String, pane_id: String, worktree: bool) -> String {
    pane_dir(&cwd, &pane_id, worktree)
}

#[tauri::command(async)]
pub fn git_checkpoint(cwd: String, pane_id: String) -> Result<Option<String>, String> {
    checkpoint(&cwd, &pane_id)
}

#[tauri::command(async)]
pub fn git_list_checkpoints(cwd: String, pane_id: String) -> Result<Vec<Checkpoint>, String> {
    check_pane_id(&pane_id)?;
    let Ok(root) = repo_root(&cwd) else {
        return Ok(Vec::new());
    };
    Ok(checkpoint_refs(&root, &pane_id)?
        .into_iter()
        .map(|(_, c)| c)
        .collect())
}

#[tauri::command(async)]
pub fn git_restore_checkpoint(
    cwd: String,
    pane_id: String,
    sha: String,
) -> Result<Option<String>, String> {
    restore_checkpoint(&cwd, &pane_id, &sha)
}

#[tauri::command(async)]
pub fn git_compare(cwd: String, other_cwd: String) -> Result<Comparison, String> {
    compare(&cwd, &other_cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fresh repo with one commit (`a.txt` = "one\n"), or None when git is missing.
    fn temp_repo() -> Option<(PathBuf, PathBuf)> {
        let base = std::env::temp_dir().join(format!("agentdeck-git-{}", uuid::Uuid::new_v4()));
        let repo = base.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        run(&repo, &["init", "-q", "-b", "main"], None).ok()?;
        run(&repo, &["config", "user.name", "t"], None).unwrap();
        run(&repo, &["config", "user.email", "t@t"], None).unwrap();
        // Byte-exact assertions: no CRLF conversion from a global core.autocrlf.
        run(&repo, &["config", "core.autocrlf", "false"], None).unwrap();
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        run(&repo, &["add", "-A"], None).unwrap();
        run(&repo, &["commit", "-q", "-m", "init"], None).unwrap();
        Some((base, repo))
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn parsers() {
        assert_eq!(
            parse_name_status("M\0a b.txt\0A\0new\0"),
            vec![("M".into(), "a b.txt".into()), ("A".into(), "new".into())]
        );
        let n = parse_numstat("3\t1\ta.txt\0-\t-\timg.png\0");
        assert_eq!(n["a.txt"], (Some(3), Some(1)));
        assert_eq!(n["img.png"], (None, None));
        assert!(is_hex_sha(EMPTY_TREE) && !is_hex_sha("HEAD") && !is_hex_sha("--x"));
        assert!(check_pane_id("abc-123").is_ok() && check_pane_id("../x").is_err());
    }

    #[test]
    fn status_lists_tracked_and_untracked() {
        let Some((base, repo)) = temp_repo() else {
            return;
        };
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(repo.join("new.txt"), "x\ny\nz\n").unwrap();
        let st = status(&s(&repo)).unwrap();
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.base.as_deref(), Some("main"));
        assert!(!st.agent_worktree);
        let a = st.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!(
            (a.status.as_str(), a.additions, a.deletions),
            ("M", Some(1), Some(0))
        );
        let n = st.files.iter().find(|f| f.path == "new.txt").unwrap();
        assert_eq!((n.status.as_str(), n.additions), ("A", Some(3)));
        // The real index was not touched: new.txt is still untracked.
        assert!(
            run_trim(&repo, &["status", "--porcelain"])
                .unwrap()
                .contains("?? new.txt")
        );
        assert!(git_diff(s(&repo), "new.txt".into()).unwrap().contains("+z"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn checkpoint_restore_is_exact() {
        let Some((base, repo)) = temp_repo() else {
            return;
        };
        let head0 = head(&repo).unwrap();
        std::fs::write(repo.join("a.txt"), "changed\n").unwrap();
        std::fs::write(repo.join("u.txt"), "untracked\n").unwrap();
        let cp = checkpoint(&s(&repo), "pane-1").unwrap().unwrap();
        assert_ne!(cp, head0);

        // The "agent" edits, deletes, creates and commits.
        std::fs::write(repo.join("a.txt"), "agent\n").unwrap();
        std::fs::remove_file(repo.join("u.txt")).unwrap();
        std::fs::write(repo.join("v.txt"), "new\n").unwrap();
        run(&repo, &["add", "-A"], None).unwrap();
        run(&repo, &["commit", "-q", "-m", "agent"], None).unwrap();
        std::fs::write(repo.join("w.txt"), "loose\n").unwrap();

        let backup = restore_checkpoint(&s(&repo), "pane-1", &cp).unwrap();
        assert!(backup.is_some());
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "changed\n"
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("u.txt")).unwrap(),
            "untracked\n"
        );
        assert!(!repo.join("v.txt").exists() && !repo.join("w.txt").exists());
        assert_eq!(head(&repo).unwrap(), head0);
        let porcelain = run(&repo, &["status", "--porcelain"], None).unwrap();
        assert!(porcelain.contains("?? u.txt") && porcelain.contains(" M a.txt"));

        let list = git_list_checkpoints(s(&repo), "pane-1".into()).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].sha, cp); // newest first: [0] is the pre-restore backup
        assert!(restore_checkpoint(&s(&repo), "pane-1", EMPTY_TREE).is_err());

        // Clean tree: the checkpoint is HEAD itself.
        run(&repo, &["add", "-A"], None).unwrap();
        run(&repo, &["commit", "-q", "-m", "x"], None).unwrap();
        assert_eq!(checkpoint(&s(&repo), "pane-1").unwrap(), head(&repo));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn worktree_merge_refuses_dirty_main_then_merges_and_removes() {
        let Some((base, repo)) = temp_repo() else {
            return;
        };
        let wt = crate::agents::create_worktree(&s(&repo), "abcdef12").unwrap();
        let wt = PathBuf::from(wt);
        assert_eq!(pane_dir(&s(&repo), "abcdef12", true), s(&wt));
        std::fs::write(wt.join("feature.txt"), "f\n").unwrap();
        assert!(merge_worktree(&s(&wt)).unwrap_err().contains("uncommitted"));
        commit(&s(&wt), "feat: add feature").unwrap();
        let st = status(&s(&wt)).unwrap();
        assert!(st.agent_worktree);
        assert_eq!((st.ahead, st.behind), (1, 0));

        let cmp = compare(&s(&repo), &s(&wt)).unwrap();
        assert_eq!(cmp.files.len(), 1);
        assert!(
            git_diff_trees(s(&repo), cmp.from, cmp.to, "feature.txt".into())
                .unwrap()
                .contains("+f")
        );

        std::fs::write(repo.join("a.txt"), "dirty\n").unwrap();
        let err = merge_worktree(&s(&wt)).unwrap_err();
        assert!(err.contains("main repository"), "{err}");
        assert!(!repo.join("feature.txt").exists());

        run(&repo, &["checkout", "-q", "--", "a.txt"], None).unwrap();
        merge_worktree(&s(&wt)).unwrap();
        assert!(repo.join("feature.txt").exists());
        assert!(merge_worktree(&s(&repo)).is_err()); // main checkout is not an agent worktree

        remove_worktree(&s(&wt)).unwrap();
        assert!(!wt.exists());
        assert!(
            run(
                &repo,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "refs/heads/agentdeck/abcdef12"
                ],
                None
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn merge_conflict_is_aborted() {
        let Some((base, repo)) = temp_repo() else {
            return;
        };
        let wt = PathBuf::from(crate::agents::create_worktree(&s(&repo), "c0ffee00").unwrap());
        std::fs::write(wt.join("a.txt"), "agent\n").unwrap();
        commit(&s(&wt), "agent").unwrap();
        std::fs::write(repo.join("a.txt"), "user\n").unwrap();
        commit(&s(&repo), "user").unwrap();
        let err = merge_worktree(&s(&wt)).unwrap_err();
        assert!(err.contains("conflicts") && err.contains("a.txt"), "{err}");
        assert!(
            run_trim(&repo, &["status", "--porcelain"])
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
