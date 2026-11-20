//! Worktrees of the "Create worktree" dialog: on a chosen branch (existing, new, a pull request's),
//! in the same `<repo>-agentdeck-<id>` folder as pane worktrees (`agents::create_worktree`), prepared
//! and set up the same way. GitHub tasks come from the GitHub CLI.

use std::path::{Path, PathBuf};

use super::{base_branch, current_branch, gh, invalidate_status, repo_root, run};
use crate::logic::new_worktree::{GitHubKind, TaskSource, parse_branches, valid_branch};

/// Which branch the new worktree checks out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// A local branch, or a remote one (git then creates its tracking branch).
    Existing(String),
    /// A new branch from `base` ("" = the default branch).
    New { branch: String, base: String },
}

impl Start {
    pub fn branch(&self) -> &str {
        match self {
            Self::Existing(branch) | Self::New { branch, .. } => branch,
        }
    }
}

/// Local branches, then remote-only ones.
pub fn list_branches(cwd: &str) -> Result<Vec<String>, String> {
    let root = repo_root(cwd)?;
    let out = run(&root, &["for-each-ref", "--format=%(refname)", "refs/heads", "refs/remotes"], None)?;
    Ok(parse_branches(&out))
}

/// `main` or `master`, else the checked out branch.
pub fn default_branch(cwd: &str) -> Option<String> {
    let root = repo_root(cwd).ok()?;
    base_branch(&root).or_else(|| current_branch(&root))
}

fn has_ref(root: &Path, reference: &str) -> bool {
    run(root, &["rev-parse", "--verify", "--quiet", &format!("{reference}^{{commit}}")], None).is_ok()
}

/// Creates `<parent>/<repo>-agentdeck-<short>` (`parent` "" = next to the repository) on `start`,
/// copies the ignored files pane worktrees get, then runs the repo's setup (else `global_setup`).
pub fn create(cwd: &str, parent: &str, short: &str, start: &Start, global_setup: &str) -> Result<String, String> {
    if short.is_empty() || !short.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("invalid worktree id".into());
    }
    if !valid_branch(start.branch()) {
        return Err(format!("invalid branch name: {}", start.branch()));
    }
    let root = repo_root(cwd)?;
    let name = root.file_name().ok_or("repository has no folder name")?.to_string_lossy().into_owned();
    let parent = match parent.trim() {
        "" => root.parent().ok_or("repository has no parent folder")?.to_path_buf(),
        dir => PathBuf::from(dir),
    };
    if !parent.is_dir() {
        return Err(format!("folder not found: {}", parent.display()));
    }
    let target = parent.join(format!("{name}-agentdeck-{short}"));
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }
    let path = target.to_string_lossy().into_owned();
    match start {
        Start::Existing(branch) => {
            run(&root, &["worktree", "add", &path, branch], None)?;
        }
        Start::New { branch, base } => {
            if has_ref(&root, &format!("refs/heads/{branch}")) {
                return Err(format!("branch {branch} already exists: pick it under Branch"));
            }
            let base = match base.trim() {
                "" => base_branch(&root).unwrap_or_else(|| "HEAD".into()),
                base if valid_branch(base) => base.to_owned(),
                base => return Err(format!("invalid base branch: {base}")),
            };
            // Only on the remote: start from its remote-tracking branch.
            let base = if has_ref(&root, &base) { base } else { format!("origin/{base}") };
            run(&root, &["worktree", "add", "-b", branch, &path, &base], None)?;
        }
    }
    invalidate_status();
    crate::agents::worktree_prep::prepare(&root, &target)?;
    let setup = crate::agents::repo_setup_command(&root, global_setup);
    if !setup.trim().is_empty() {
        crate::agents::run_setup(setup.trim(), &target, &root)?;
    }
    Ok(path)
}

/// Pull request or issue `number` of the project's GitHub repo (`gh pr view` / `gh issue view`),
/// with the pull request's head branch. Unknown kind: pull request first, then issue.
pub fn gh_task(cwd: &str, kind: Option<GitHubKind>, number: u64) -> Result<(TaskSource, Option<String>), String> {
    let root = repo_root(cwd)?;
    let n = number.to_string();
    let pr = || gh(&root, &["pr", "view", &n, "--json", "number,title,body,url,headRefName"]);
    let issue = || gh(&root, &["issue", "view", &n, "--json", "number,title,body,url"]);
    let (kind, out) = match kind {
        Some(GitHubKind::PullRequest) => (GitHubKind::PullRequest, pr()?),
        Some(GitHubKind::Issue) => (GitHubKind::Issue, issue()?),
        None => match pr() {
            Ok(out) => (GitHubKind::PullRequest, out),
            Err(_) => (GitHubKind::Issue, issue()?),
        },
    };
    parse_gh_task(kind, &out).ok_or_else(|| "unexpected answer from gh".into())
}

fn parse_gh_task(kind: GitHubKind, json: &str) -> Option<(TaskSource, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let head = v["headRefName"].as_str().map(str::to_owned);
    let task = TaskSource::GitHub {
        kind,
        number: v["number"].as_u64()?,
        title: v["title"].as_str()?.trim().to_owned(),
        url: v["url"].as_str().unwrap_or("").to_owned(),
        repo: None,
        body: v["body"].as_str().map(str::to_owned).filter(|b| !b.trim().is_empty()),
    };
    Some((task, head))
}

/// Local branch for pull request `number`: its head branch, unless that name is unusable or is the
/// default branch (a fork's `main`), then `pr-<number>`.
fn pr_local_branch(head: &str, number: u64, default: Option<&str>) -> String {
    if valid_branch(head) && Some(head) != default { head.to_owned() } else { format!("pr-{number}") }
}

/// The pull request's branch, fetched from `origin` (`pull/<n>/head`, forks too) when not local yet.
pub fn pr_branch(cwd: &str, number: u64, head: &str) -> Result<String, String> {
    let root = repo_root(cwd)?;
    let local = pr_local_branch(head, number, base_branch(&root).as_deref());
    if !has_ref(&root, &format!("refs/heads/{local}")) {
        run(&root, &["fetch", "origin", &format!("pull/{number}/head:{local}")], None)?;
    }
    Ok(local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gh_answers() {
        let json = r#"{"number":45,"title":" Fix tax ","body":"","url":"https://github.com/o/r/pull/45","headRefName":"fix/tax"}"#;
        let (task, head) = parse_gh_task(GitHubKind::PullRequest, json).unwrap();
        assert_eq!(head.as_deref(), Some("fix/tax"));
        assert_eq!(task.label(), "PR #45");
        assert_eq!(task.title(), "Fix tax");
        assert!(matches!(task, TaskSource::GitHub { body: None, .. }));
        assert_eq!(parse_gh_task(GitHubKind::Issue, "{}"), None);
    }

    #[test]
    fn names_the_pr_branch() {
        assert_eq!(pr_local_branch("fix/tax", 45, Some("main")), "fix/tax");
        assert_eq!(pr_local_branch("main", 45, Some("main")), "pr-45");
        assert_eq!(pr_local_branch("a b", 45, None), "pr-45");
    }

    #[test]
    fn creates_worktrees_on_new_and_existing_branches() {
        let tmp = std::env::temp_dir().join(format!("agentdeck-bwt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let repo = tmp.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let s = |p: &Path| p.to_string_lossy().into_owned();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "init"],
            vec!["branch", "feature"],
        ] {
            run(&repo, &args, None).unwrap();
        }
        let new = Start::New { branch: "acme-1-promo".into(), base: String::new() };
        let made = create(&s(&repo), "", "aaaa1111", &new, "").unwrap();
        assert!(made.ends_with("repo-agentdeck-aaaa1111"));
        assert_eq!(current_branch(Path::new(&made)).as_deref(), Some("acme-1-promo"));
        assert!(create(&s(&repo), "", "bbbb2222", &new, "").unwrap_err().contains("already exists"));
        let made = create(&s(&repo), &s(&tmp), "cccc3333", &Start::Existing("feature".into()), "").unwrap();
        assert_eq!(current_branch(Path::new(&made)).as_deref(), Some("feature"));
        assert!(create(&s(&repo), "", "dddd4444", &Start::Existing("-x".into()), "").is_err());
        assert_eq!(list_branches(&s(&repo)).unwrap(), ["acme-1-promo", "feature", "main"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
