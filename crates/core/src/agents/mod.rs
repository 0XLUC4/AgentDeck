//! Agent definitions (Claude Code, Codex, Gemini CLI, Grok, OpenCode), install detection,
//! git worktrees and hook configuration.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use portable_pty::CommandBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub mod browser_guard;
pub mod worktree_prep;

use crate::hooks_server::{BROWSER_PORT_ENV, HookServer, TOKEN_HEADER, browser_server};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
    Gemini,
    Grok,
    #[serde(rename = "opencode")]
    OpenCode,
    Shell,
}

impl AgentKind {
    /// CLI binary name; `None` for a plain shell.
    pub fn binary(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("claude"),
            Self::Codex => Some("codex"),
            Self::Gemini => Some("gemini"),
            Self::Grok => Some("grok"),
            Self::OpenCode => Some("opencode"),
            Self::Shell => None,
        }
    }

    fn install_url(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("https://code.claude.com/docs/en/setup"),
            Self::Codex => Some("https://github.com/openai/codex#quickstart"),
            Self::Gemini => Some("https://github.com/google-gemini/gemini-cli#-installation"),
            Self::Grok => Some("https://docs.x.ai/build/overview"),
            Self::OpenCode => Some("https://opencode.ai/docs/"),
            Self::Shell => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub kind: AgentKind,
    pub installed: bool,
    pub path: Option<String>,
    pub install_url: Option<&'static str>,
}

/// Reports which agent CLIs are available on PATH.
pub fn detect_agents() -> Vec<AgentInfo> {
    [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Gemini,
        AgentKind::Grok,
        AgentKind::OpenCode,
        AgentKind::Shell,
    ]
    .into_iter()
    .map(|kind| {
        let path = kind.binary().and_then(resolve_binary);
        AgentInfo {
            kind,
            installed: kind == AgentKind::Shell || path.is_some(),
            install_url: if path.is_none() {
                kind.install_url()
            } else {
                None
            },
            path: path.map(|p| p.to_string_lossy().into_owned()),
        }
    })
    .collect()
}

/// Looks `bin` up on the real PATH (honoring PATHEXT on Windows).
fn resolve_binary(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_owned)
            .collect()
    } else {
        vec![String::new()]
    };
    // Grok's installer adds ~/.grok/bin to the user PATH, which an already-running app doesn't see yet.
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    let fallback = home.map(|h| {
        std::path::PathBuf::from(h)
            .join(".grok")
            .join("bin")
            .into_os_string()
    });
    find_in_path(bin, &path, &exts).or_else(|| find_in_path(bin, &fallback?, &exts))
}

/// First `dir/bin+ext` that is a file, dirs in PATH order then exts in order.
/// On Windows the extensionless npm `sh` shim is skipped since "" is not in PATHEXT.
fn find_in_path(bin: &str, path: &std::ffi::OsStr, exts: &[String]) -> Option<PathBuf> {
    std::env::split_paths(path).find_map(|dir| {
        exts.iter()
            .map(|ext| dir.join(format!("{bin}{}", ext.to_lowercase())))
            .find(|p| p.is_file())
    })
}

/// Builds the command that launches `kind` inside a PTY.
pub fn command_for(kind: AgentKind, args: &[String]) -> CommandBuilder {
    let mut cmd = match (kind.binary(), cfg!(windows)) {
        (Some(bin), true) => {
            let resolved = resolve_binary(bin);
            let ext = resolved
                .as_ref()
                .and_then(|p| p.extension())
                .map(|e| e.to_string_lossy().to_lowercase());
            match (resolved, ext.as_deref()) {
                (Some(p), Some("exe")) => CommandBuilder::new(p),
                (Some(p), Some("ps1")) => {
                    let mut c = CommandBuilder::new("powershell.exe");
                    c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
                    c.arg(p);
                    c
                }
                // npm installs agents as .cmd shims, which need cmd.exe.
                // Note: cmd.exe mangles quoted paths containing spaces; npm's default prefix has none.
                (resolved, _) => {
                    let mut c = CommandBuilder::new("cmd.exe");
                    c.arg("/c");
                    c.arg(resolved.map_or_else(|| bin.into(), PathBuf::into_os_string));
                    c
                }
            }
        }
        (Some(bin), false) => CommandBuilder::new(bin),
        (None, true) => CommandBuilder::new("powershell.exe"),
        (None, false) => {
            CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()))
        }
    };
    cmd.args(args);
    cmd
}

/// Runs git with `safe.directory` set for `repo` (D: is exFAT: no ownership info).
pub(crate) fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let safe = format!(
        "safe.directory={}",
        repo.to_string_lossy().replace('\\', "/")
    );
    let mut cmd = Command::new("git");
    cmd.arg("-c").arg(safe).arg("-C").arg(repo).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash from the GUI app
    }
    let out = cmd.output().map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Nearest ancestor of `cwd` containing `.git` (dir, or file for worktrees/submodules).
pub(crate) fn find_repo_root(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|d| d.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Main checkout of an AgentDeck worktree (`<parent>/<repo>-agentdeck-<id>`, see [`create_worktree_with`]),
/// `None` for any other folder.
pub fn agentdeck_worktree_root(cwd: &Path) -> Option<PathBuf> {
    let name = cwd.file_name()?.to_str()?;
    let (repo, _) = name.rsplit_once("-agentdeck-")?;
    let root = cwd.parent()?.join(repo);
    (cwd.join(".git").is_file() && root.join(".git").is_dir()).then_some(root)
}

/// Creates (or reuses) the dedicated git worktree for the pane and returns its path.
pub fn create_worktree(cwd: &str, pane_id: &str) -> Result<String, String> {
    create_worktree_with(cwd, pane_id, "")
}

/// Runs the user's worktree setup command (install deps, copy .env) through the shell in `dir`.
/// `AGENTDECK_ROOT` is the main checkout, where the git-ignored files live.
/// `command` run by the system shell (cmd on Windows, sh elsewhere), with no console window.
pub(crate) fn shell(command: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("cmd");
        // /S + outer quotes: cmd runs the text as typed, inner quotes included.
        cmd.args(["/D", "/S", "/C"]).raw_arg(format!("\"{command}\""));
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

fn run_setup(command: &str, dir: &Path, root: &Path) -> Result<(), String> {
    let out = shell(command)
        .current_dir(dir)
        .env("AGENTDECK_ROOT", root)
        .stdin(std::process::Stdio::null()) // a prompt must fail, not hang the spawn
        .output()
        .map_err(|e| format!("cannot run worktree setup: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let text = if stderr.trim().is_empty() { stdout } else { stderr };
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    Err(format!(
        "worktree setup failed ({}): {}. Restart the agent to continue without it.",
        out.status,
        lines[lines.len().saturating_sub(3)..].join(" | ")
    ))
}

/// [`create_worktree`], then `setup` (a shell command, "" = none) once in the worktree it just created.
/// A reused worktree is never set up again.
pub fn create_worktree_with(cwd: &str, pane_id: &str, setup: &str) -> Result<String, String> {
    let short: String = pane_id.chars().take(8).collect();
    if short.is_empty()
        || !short.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        || short.starts_with('-')
    {
        return Err("invalid pane id for worktree name".into());
    }
    let root =
        find_repo_root(Path::new(cwd)).ok_or_else(|| format!("not a git repository: {cwd}"))?;
    let name = root
        .file_name()
        .ok_or("repository has no folder name")?
        .to_string_lossy();
    let parent = root.parent().ok_or("repository has no parent folder")?;
    let target = parent.join(format!("{name}-agentdeck-{short}"));
    let target_str = target.to_string_lossy().into_owned();
    if target.is_dir() {
        return Ok(target_str); // restart / restored session: reuse this pane's worktree
    }
    let branch = format!("agentdeck/{short}");
    if !claim_spare(&root, &target, &branch) {
        git(&root, &["worktree", "add", &target_str, "-b", &branch])?;
        worktree_prep::prepare(&root, &target)?;
    }
    refill_spare(root.clone());
    if !setup.trim().is_empty() {
        run_setup(setup.trim(), &target, &root)?;
    }
    Ok(target_str)
}

/// Setup command for new worktrees of `main_root`: the repo's own `.agentdeck/setup.ps1` (Windows),
/// `setup.sh` or `setup` script when there is one, else the global `settings.worktree_setup`.
pub fn repo_setup_command(main_root: &Path, global: &str) -> String {
    let dir = main_root.join(".agentdeck");
    let ps1 = cfg!(windows).then_some("setup.ps1");
    for name in ps1.into_iter().chain(["setup.sh", "setup"]) {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        let path = path.to_string_lossy();
        return if name == "setup.ps1" {
            format!("powershell -NoProfile -ExecutionPolicy Bypass -File \"{path}\"")
        } else {
            format!("sh \"{}\"", path.replace('\\', "/"))
        };
    }
    global.to_owned()
}

/// One spare worktree per repo is made in the background after a worktree spawn, so the next
/// spawn renames it instead of waiting for `git worktree add`. Off in tests (no stray threads).
const SPARE_WORKTREE: bool = !cfg!(test);
/// An older spare (or one whose commit is no longer the repo's HEAD) is removed, not reused.
const SPARE_TTL: Duration = Duration::from_secs(5 * 60);

struct Spare {
    path: PathBuf,
    branch: String,
    head: String,
    made: Instant,
}

/// Spares by main checkout; `None` = being made.
static SPARES: Mutex<Vec<(PathBuf, Option<Spare>)>> = Mutex::new(Vec::new());

fn remove_worktree(root: &Path, path: &Path, branch: &str) {
    let _ = git(root, &["worktree", "remove", "--force", &path.to_string_lossy()]);
    let _ = git(root, &["branch", "-D", branch]);
}

/// Moves the repo's ready spare to `target` on `branch`: true when the worktree is there.
/// A stale spare (older than [`SPARE_TTL`], other commit) is removed instead.
fn claim_spare(root: &Path, target: &Path, branch: &str) -> bool {
    let spare = SPARES.lock().ok().and_then(|mut spares| {
        let i = spares.iter().position(|(r, s)| r == root && s.is_some())?;
        spares.remove(i).1
    });
    let Some(spare) = spare else { return false };
    let fresh = spare.made.elapsed() < SPARE_TTL && git(root, &["rev-parse", "HEAD"]).is_ok_and(|h| h == spare.head);
    if !fresh || git(root, &["worktree", "move", &spare.path.to_string_lossy(), &target.to_string_lossy()]).is_err() {
        remove_worktree(root, &spare.path, &spare.branch);
        return false;
    }
    if git(root, &["branch", "-m", &spare.branch, branch]).is_err() {
        remove_worktree(root, target, &spare.branch);
        return false;
    }
    true
}

/// App exit: removes the ready spares, which hold no work and would stay on disk.
/// shortcut: a spare still being made at exit is left behind, rare (needs a quit within seconds of a spawn).
pub fn drop_spares() {
    let spares = SPARES.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or_default();
    for (root, spare) in spares {
        if let Some(spare) = spare {
            remove_worktree(&root, &spare.path, &spare.branch);
        }
    }
}

/// Starts making the repo's spare worktree (branch `agentdeck/<new id>`) unless one exists.
fn refill_spare(root: PathBuf) {
    if !SPARE_WORKTREE {
        return;
    }
    match SPARES.lock() {
        Ok(mut spares) if !spares.iter().any(|(r, _)| *r == root) => spares.push((root.clone(), None)),
        _ => return,
    }
    std::thread::spawn(move || {
        let id: String = uuid::Uuid::new_v4().simple().to_string().chars().take(8).collect();
        let branch = format!("agentdeck/{id}");
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let path = root.parent().unwrap_or(&root).join(format!("{name}-agentdeck-{id}"));
        let made = git(&root, &["worktree", "add", &path.to_string_lossy(), "-b", &branch])
            .and_then(|_| worktree_prep::prepare(&root, &path))
            .and_then(|()| git(&path, &["rev-parse", "HEAD"]));
        if made.is_err() {
            remove_worktree(&root, &path, &branch);
        }
        if let Ok(mut spares) = SPARES.lock() {
            spares.retain(|(r, _)| *r != root);
            if let Ok(head) = made {
                spares.push((root, Some(Spare { path, branch, head, made: Instant::now() })));
            }
        }
    });
}

/// Extra CLI args that wire `kind` to the hook server (prepended to user args).
pub fn hook_args(kind: AgentKind, pane_id: &str, hooks: &HookServer) -> Vec<String> {
    match kind {
        // Codex appends its JSON payload as the last argv of `notify`, so curl gets it as --data-binary.
        // TOML literal strings ('...') and no spaces/& keep this intact through cmd.exe + the npm .cmd shim.
        AgentKind::Codex => vec![
            "-c".into(),
            format!(
                "notify=['curl','-s','-m','3','-H','{TOKEN_HEADER}:{}','-H','Content-Type:application/json','{}?pane={pane_id}','--data-binary']",
                hooks.token,
                hooks.url()
            ),
            // `notify` only reports finished turns: approvals come as OSC 9 terminal notifications.
            "-c".into(),
            "tui.notifications=['approval-requested']".into(),
            "-c".into(),
            "tui.notification_method='osc9'".into(),
        ],
        _ => Vec::new(),
    }
}

/// Merges AgentDeck hooks into the agent's config for `cwd` without clobbering user config.
pub fn prepare_hooks(kind: AgentKind, cwd: &str, hooks: &HookServer) -> Result<(), String> {
    match kind {
        AgentKind::Claude => update_settings(&claude_settings(cwd), |s| {
            merge_claude_hooks(s, &hooks.url())
        }),
        AgentKind::Gemini => {
            // Older builds wrote into the project file; drop only those entries. Non-fatal (JSONC is left alone).
            if let Err(e) =
                update_settings(&gemini_settings(cwd), |s| strip_hooks(s, is_ours_gemini))
            {
                eprintln!("gemini legacy hooks not removed: {e}");
            }
            Ok(())
        }
        // Codex: see `hook_args` (no global config edits).
        _ => Ok(()),
    }
}

/// Env var (and file) that load AgentDeck's Gemini hooks without touching the repo or `~/.gemini`.
/// Gemini reads its "system defaults" layer from this path, and hook arrays are concatenated across
/// layers (packages/cli/src/config/settings.ts `getSystemDefaultsPath`, settingsSchema.ts `MergeStrategy.CONCAT`).
/// The file is a copy of the real system defaults plus our hooks, so admin defaults still apply.
/// `None` (logged) when the real defaults are JSONC or unwritable: output-based idle detection still works.
pub fn gemini_env(app_data: &Path, hooks: &HookServer) -> Option<(&'static str, PathBuf)> {
    // One file per port so concurrent AgentDeck instances do not overwrite each other.
    let ours = app_data
        .join("gemini")
        .join(format!("system-defaults-{}.json", hooks.port));
    let result = gemini_system_defaults(&ours).and_then(|base| {
        crate::trust::edit_file(&ours, |_| {
            let text = std::fs::read_to_string(&base).unwrap_or_default();
            let mut settings: Value = if text.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&text)
                    .map_err(|e| format!("{}: not valid JSON: {e}", base.display()))?
            };
            if !settings.is_object() {
                return Err(format!("{}: not a JSON object", base.display()));
            }
            merge_gemini_hooks(&mut settings, &hooks.url());
            merge_gemini_browser(&mut settings);
            let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
            Ok(Some(
                text + "
",
            ))
        })
    });
    match result {
        Ok(()) => Some((GEMINI_DEFAULTS_ENV, ours)),
        Err(e) => {
            eprintln!("gemini hooks not installed: {e}");
            None
        }
    }
}

const GEMINI_DEFAULTS_ENV: &str = "GEMINI_CLI_SYSTEM_DEFAULTS_PATH";

/// The system defaults file Gemini would read without our override (mirrors `getSystemDefaultsPath`).
fn gemini_system_defaults(ours: &Path) -> Result<PathBuf, String> {
    let inherited = std::env::var_os(GEMINI_DEFAULTS_ENV).map(PathBuf::from);
    if let Some(path) = inherited.filter(|p| p != ours) {
        return Ok(path);
    }
    let settings = std::env::var_os("GEMINI_CLI_SYSTEM_SETTINGS_PATH").map_or_else(
        || {
            PathBuf::from(if cfg!(windows) {
                r"C:\ProgramData\gemini-cli\settings.json"
            } else if cfg!(target_os = "macos") {
                "/Library/Application Support/GeminiCli/settings.json"
            } else {
                "/etc/gemini-cli/settings.json"
            })
        },
        PathBuf::from,
    );
    let dir = settings
        .parent()
        .ok_or("system settings path has no folder")?;
    Ok(dir.join("system-defaults.json"))
}

/// Removes AgentDeck's hooks from `<cwd>/.claude/settings.local.json` and legacy `<cwd>/.gemini/settings.json` entries.
pub fn uninstall_hooks(cwd: String) -> Result<(), String> {
    update_settings(&claude_settings(&cwd), strip_claude_hooks)?;
    update_settings(&gemini_settings(&cwd), |s| strip_hooks(s, is_ours_gemini))
}

fn claude_settings(cwd: &str) -> PathBuf {
    Path::new(cwd).join(".claude").join("settings.local.json")
}

fn gemini_settings(cwd: &str) -> PathBuf {
    Path::new(cwd).join(".gemini").join("settings.json")
}

const CLAUDE_EVENTS: [&str; 10] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "Notification",
    "Stop",
    "StopFailure",
    "SubagentStart",
    "SubagentStop",
];
/// PermissionRequest is held by the hook server until the user answers in AgentDeck (hold: 590 s).
const PERMISSION_TIMEOUT_S: u64 = 600;

/// Read-modify-write of a JSON settings file (atomic replace); refuses to touch a file it cannot parse.
fn update_settings(path: &Path, edit: impl Fn(&mut Value)) -> Result<(), String> {
    crate::trust::edit_file(path, |text| {
        let mut settings: Value = if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?
        };
        if !settings.is_object() {
            return Err("not a JSON object".into());
        }
        let before = settings.clone();
        edit(&mut settings);
        if settings == before {
            return Ok(None);
        }
        let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
        Ok(Some(text + "\n"))
    })
}

/// Our Claude hook groups are recognized by the AgentDeck token header (no extra keys: Claude validates settings).
fn is_ours(group: &Value) -> bool {
    group["hooks"]
        .as_array()
        .is_some_and(|hs| hs.iter().any(|h| h["headers"].get(TOKEN_HEADER).is_some()))
}

/// Our Gemini hook groups carry a hook named `GEMINI_HOOK_NAME`.
fn is_ours_gemini(group: &Value) -> bool {
    group["hooks"]
        .as_array()
        .is_some_and(|hs| hs.iter().any(|h| h["name"] == GEMINI_HOOK_NAME))
}

fn strip_claude_hooks(settings: &mut Value) {
    strip_hooks(settings, is_ours);
}

/// Drops hook groups matching `ours`, then empty event lists and an empty `hooks` key.
fn strip_hooks(settings: &mut Value, ours: fn(&Value) -> bool) {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    for groups in hooks.values_mut() {
        if let Some(arr) = groups.as_array_mut() {
            arr.retain(|g| !ours(g));
        }
    }
    hooks.retain(|_, groups| groups.as_array().is_none_or(|a| !a.is_empty()));
    if hooks.is_empty() {
        settings
            .as_object_mut()
            .expect("checked object")
            .remove("hooks");
    }
}

/// Idempotent: drops previous AgentDeck entries, then appends fresh ones (port may have changed).
/// Token and pane id come from the PTY env at runtime, so the file holds no secret.
fn merge_claude_hooks(settings: &mut Value, url: &str) {
    strip_claude_hooks(settings);
    let ours = json!({ "hooks": [{
        "type": "http",
        "url": format!("{url}?agent=claude"),
        "timeout": 5,
        "headers": { TOKEN_HEADER: "$AGENTDECK_HOOK_TOKEN", "X-AgentDeck-Pane": "$AGENTDECK_PANE_ID" },
        "allowedEnvVars": ["AGENTDECK_HOOK_TOKEN", "AGENTDECK_PANE_ID"],
    }]});
    let obj = settings.as_object_mut().expect("checked object");
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    for event in CLAUDE_EVENTS {
        let groups = hooks
            .as_object_mut()
            .expect("object")
            .entry(event)
            .or_insert_with(|| json!([]));
        if let Some(arr) = groups.as_array_mut() {
            let mut group = ours.clone();
            if event == "PermissionRequest" {
                group["hooks"][0]["timeout"] = json!(PERMISSION_TIMEOUT_S);
            }
            arr.push(group);
        }
    }
}

/// https://github.com/google-gemini/gemini-cli/blob/main/docs/hooks/reference.md
const GEMINI_EVENTS: [&str; 6] = [
    "SessionStart",
    "BeforeAgent",
    "BeforeTool",
    "AfterTool",
    "AfterAgent",
    "Notification",
];
const GEMINI_HOOK_NAME: &str = "agentdeck-status";

/// Gemini hooks are command-only (payload on stdin), run by PowerShell on Windows and bash elsewhere.
/// Token and pane id are read from the PTY env at run time, so the file holds no secret.
fn gemini_hook_command(url: &str) -> String {
    let url = format!("{url}?agent=gemini");
    if cfg!(windows) {
        // `curl.exe`, not the PowerShell `curl` alias; "@-" quoted so PowerShell does not splat it.
        format!(
            "curl.exe -s -m 3 -o NUL -H \"{TOKEN_HEADER}: $env:AGENTDECK_HOOK_TOKEN\" -H \"X-AgentDeck-Pane: $env:AGENTDECK_PANE_ID\" -H \"Content-Type: application/json\" --data-binary \"@-\" \"{url}\""
        )
    } else {
        format!(
            "curl -s -m 3 -o /dev/null -H \"{TOKEN_HEADER}: $AGENTDECK_HOOK_TOKEN\" -H \"X-AgentDeck-Pane: $AGENTDECK_PANE_ID\" -H \"Content-Type: application/json\" --data-binary @- '{url}'"
        )
    }
}

/// Idempotent: drops previous AgentDeck entries, then appends fresh ones (port may have changed).
fn merge_gemini_hooks(settings: &mut Value, url: &str) {
    strip_hooks(settings, is_ours_gemini);
    let ours = json!({ "hooks": [{
        "type": "command",
        "name": GEMINI_HOOK_NAME,
        "command": gemini_hook_command(url),
        "timeout": 5000,
    }]});
    let obj = settings.as_object_mut().expect("checked object");
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    for event in GEMINI_EVENTS {
        let groups = hooks
            .as_object_mut()
            .expect("object")
            .entry(event)
            .or_insert_with(|| json!([]));
        if let Some(arr) = groups.as_array_mut() {
            arr.push(ours.clone());
        }
    }
}

/// The pane's browser panel server; Gemini expands the port from the pane's env, so one file serves every pane.
fn merge_gemini_browser(settings: &mut Value) {
    let url = format!("http://127.0.0.1:${{{BROWSER_PORT_ENV}}}");
    let obj = settings.as_object_mut().expect("checked object");
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    if !servers.is_object() {
        *servers = json!({});
    }
    servers["browser"] = browser_server(&url);
    // Gemini has no blocking hook: the user's other browser servers are left out of AgentDeck's agents.
    let excluded = browser_guard::gemini_excluded();
    if excluded.is_empty() {
        return;
    }
    let mcp = obj.entry("mcp").or_insert_with(|| json!({}));
    if !mcp.is_object() {
        *mcp = json!({});
    }
    let list = mcp.as_object_mut().expect("object").entry("excluded").or_insert_with(|| json!([]));
    if let Some(list) = list.as_array_mut() {
        for name in excluded {
            if !list.iter().any(|v| v == name.as_str()) {
                list.push(json!(name));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_setup_script_wins_over_global() {
        let root = std::env::temp_dir().join(format!("agentdeck-setup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".agentdeck")).unwrap();
        assert_eq!(repo_setup_command(&root, "pnpm install"), "pnpm install");
        std::fs::write(root.join(".agentdeck/setup.sh"), "pnpm i").unwrap();
        let cmd = repo_setup_command(&root, "pnpm install");
        assert!(cmd.starts_with("sh \"") && cmd.ends_with("/.agentdeck/setup.sh\"") && !cmd.contains('\\'), "{cmd}");
        if cfg!(windows) {
            std::fs::write(root.join(".agentdeck/setup.ps1"), "pnpm i").unwrap();
            assert!(repo_setup_command(&root, "").contains("-File"));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_is_idempotent_and_keeps_user_hooks() {
        let user = json!({ "type": "command", "command": "echo hi" });
        let mut s = json!({
            "permissions": { "allow": ["Bash(ls)"] },
            "hooks": { "Stop": [{ "hooks": [user] }] }
        });
        merge_claude_hooks(&mut s, "http://127.0.0.1:1/hook");
        let once = s.clone();
        merge_claude_hooks(&mut s, "http://127.0.0.1:1/hook");
        assert_eq!(s, once);
        assert_eq!(s["permissions"]["allow"][0], "Bash(ls)");
        assert_eq!(s["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(s["hooks"]["Stop"][0]["hooks"][0], user);
        assert_eq!(s["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
        assert_eq!(s["hooks"]["PermissionRequest"].as_array().unwrap().len(), 1);
        assert_eq!(
            s["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"],
            600
        );
        assert_eq!(s["hooks"]["Stop"][1]["hooks"][0]["timeout"], 5);

        // New port replaces, never duplicates.
        merge_claude_hooks(&mut s, "http://127.0.0.1:2/hook");
        assert_eq!(s["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(
            s["hooks"]["Stop"][1]["hooks"][0]["url"],
            "http://127.0.0.1:2/hook?agent=claude"
        );

        strip_claude_hooks(&mut s);
        assert_eq!(s["hooks"], json!({ "Stop": [{ "hooks": [user] }] }));
    }

    #[test]
    fn strip_removes_hooks_key_when_only_ours() {
        let mut s = json!({ "model": "opus" });
        merge_claude_hooks(&mut s, "http://127.0.0.1:1/hook");
        strip_claude_hooks(&mut s);
        assert_eq!(s, json!({ "model": "opus" }));
    }

    #[test]
    fn settings_file_roundtrip_and_invalid_json_untouched() {
        let dir = std::env::temp_dir().join(format!("agentdeck-test-{}", uuid::Uuid::new_v4()));
        let file = dir.join(".claude").join("settings.local.json");
        let cwd = dir.to_string_lossy().into_owned();
        update_settings(&file, |s| merge_claude_hooks(s, "http://127.0.0.1:1/hook")).unwrap();
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("X-AgentDeck-Token")
        );
        uninstall_hooks(cwd.clone()).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), "{}");

        std::fs::write(&file, "{ broken").unwrap();
        assert!(update_settings(&file, |s| merge_claude_hooks(s, "x")).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ broken");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gemini_merge_is_idempotent_and_keeps_user_settings() {
        let user = json!({ "type": "command", "name": "mine", "command": "echo hi" });
        let mut s = json!({
            "model": { "name": "gemini-2.5-pro" },
            "hooks": { "AfterAgent": [{ "hooks": [user] }] },
            "hooksConfig": { "enabled": true }
        });
        merge_gemini_hooks(&mut s, "http://127.0.0.1:1/hook");
        let once = s.clone();
        merge_gemini_hooks(&mut s, "http://127.0.0.1:1/hook");
        assert_eq!(s, once);
        assert_eq!(s["model"]["name"], "gemini-2.5-pro");
        assert_eq!(s["hooksConfig"]["enabled"], true);
        assert_eq!(s["hooks"]["AfterAgent"].as_array().unwrap().len(), 2);
        assert_eq!(s["hooks"]["AfterAgent"][0]["hooks"][0], user);
        for event in GEMINI_EVENTS {
            let ours = &s["hooks"][event].as_array().unwrap().last().unwrap()["hooks"][0];
            assert_eq!(ours["type"], "command");
            assert_eq!(ours["timeout"], 5000);
        }

        // New port replaces, never duplicates.
        merge_gemini_hooks(&mut s, "http://127.0.0.1:2/hook");
        assert_eq!(s["hooks"]["AfterAgent"].as_array().unwrap().len(), 2);
        let command = s["hooks"]["AfterAgent"][1]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(command.contains("http://127.0.0.1:2/hook?agent=gemini"));
        assert!(command.contains("AGENTDECK_HOOK_TOKEN") && command.contains("AGENTDECK_PANE_ID"));

        strip_hooks(&mut s, is_ours_gemini);
        assert_eq!(s["hooks"], json!({ "AfterAgent": [{ "hooks": [user] }] }));
        strip_hooks(&mut s, is_ours_gemini);
        assert_eq!(s["hooks"], json!({ "AfterAgent": [{ "hooks": [user] }] }));
    }

    #[test]
    fn gemini_legacy_project_hooks_are_cleaned() {
        let dir = std::env::temp_dir().join(format!("agentdeck-gem-{}", uuid::Uuid::new_v4()));
        let file = dir.join(".gemini").join("settings.json");
        let cwd = dir.to_string_lossy().into_owned();
        let hooks = HookServer {
            port: 47800,
            token: "tok".into(),
        };
        // Absent file is not created.
        prepare_hooks(AgentKind::Gemini, &cwd, &hooks).unwrap();
        assert!(!file.exists());

        // An old project entry is removed on spawn and on uninstall; user settings are kept byte-for-byte when untouched.
        let mut legacy = json!({ "theme": "Dracula" });
        merge_gemini_hooks(&mut legacy, "http://127.0.0.1:1/hook");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, legacy.to_string()).unwrap();
        prepare_hooks(AgentKind::Gemini, &cwd, &hooks).unwrap();
        let back: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(back, json!({ "theme": "Dracula" }));
        std::fs::write(&file, legacy.to_string()).unwrap();
        uninstall_hooks(cwd.clone()).unwrap();
        let back: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(back, json!({ "theme": "Dracula" }));

        // JSONC (comments) is left untouched and does not block the pane.
        std::fs::write(
            &file,
            "// mine
{}",
        )
        .unwrap();
        prepare_hooks(AgentKind::Gemini, &cwd, &hooks).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "// mine
{}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gemini_env_writes_app_data_file_only() {
        let dir = std::env::temp_dir().join(format!("agentdeck-gemenv-{}", uuid::Uuid::new_v4()));
        let hooks = HookServer {
            port: 47801,
            token: "tok".into(),
        };
        let (key, path) = gemini_env(&dir, &hooks).expect("installed");
        assert_eq!(key, "GEMINI_CLI_SYSTEM_DEFAULTS_PATH");
        assert!(path.starts_with(&dir) && path.ends_with("system-defaults-47801.json"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(GEMINI_HOOK_NAME)
                && text.contains(":47801/hook")
                && !text.contains("tok\"")
                && text.contains("127.0.0.1:${AGENTDECK_BROWSER_PORT}")
        );
        // Idempotent: a second pane on the same port yields the same file.
        gemini_env(&dir, &hooks).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gemini_defaults_base_skips_our_own_file() {
        let ours = Path::new("/x/ours.json");
        // Without the env var (or when it already points at our file) the platform default is used.
        if std::env::var_os(GEMINI_DEFAULTS_ENV).is_none_or(|p| p == ours.as_os_str()) {
            let base = gemini_system_defaults(ours).unwrap();
            assert!(base.ends_with("system-defaults.json") && base != ours);
        }
    }

    #[test]
    fn find_in_path_honors_ext_order() {
        let dir = std::env::temp_dir().join(format!("agentdeck-path-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("claude"), "").unwrap(); // npm sh shim
        std::fs::write(dir.join("claude.cmd"), "").unwrap();
        let path = std::env::join_paths([Path::new("/nonexistent"), &dir]).unwrap();
        let exts = vec![".EXE".to_string(), ".CMD".to_string()];
        assert_eq!(
            find_in_path("claude", &path, &exts),
            Some(dir.join("claude.cmd"))
        );
        assert_eq!(find_in_path("codex", &path, &exts), None);
        assert_eq!(
            find_in_path("claude", &path, &[String::new()]),
            Some(dir.join("claude"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn codex_notify_arg_is_shell_safe_toml() {
        let hooks = HookServer {
            port: 47800,
            token: "tok".into(),
        };
        let args = hook_args(AgentKind::Codex, "abc-123", &hooks);
        assert_eq!(args[0], "-c");
        assert!(!args[1].contains([' ', '&', '"', '%', '^', '|', '<', '>']));
        let doc: toml_edit::DocumentMut = args[1].parse().unwrap();
        let notify = doc["notify"].as_array().unwrap();
        assert_eq!(notify.get(0).unwrap().as_str(), Some("curl"));
        assert_eq!(
            notify.iter().last().unwrap().as_str(),
            Some("--data-binary")
        );
        assert!(hook_args(AgentKind::Claude, "x", &hooks).is_empty());
    }

    #[test]
    fn worktree_rejects_non_repo_and_bad_ids() {
        assert!(create_worktree("C:/", "-bad").is_err());
        let tmp = std::env::temp_dir().join(format!("agentdeck-norepo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        // Temp dir is normally outside any repo.
        if find_repo_root(&tmp).is_none() {
            let err = create_worktree(&tmp.to_string_lossy(), "abcdef12").unwrap_err();
            assert!(err.contains("not a git repository"));
        }
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn worktree_created_next_to_repo() {
        let base = std::env::temp_dir().join(format!("agentdeck-wt-{}", uuid::Uuid::new_v4()));
        let repo = base.join("proj");
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        if git(&repo, &["init", "-q"]).is_err() {
            return; // git not installed
        }
        git(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        )
        .unwrap();
        let path = create_worktree(&repo.join("sub").to_string_lossy(), "abcdef12-3456").unwrap();
        assert_eq!(Path::new(&path), base.join("proj-agentdeck-abcdef12"));
        assert!(Path::new(&path).join(".git").exists());
        assert_eq!(agentdeck_worktree_root(Path::new(&path)), Some(repo.clone()));
        assert_eq!(agentdeck_worktree_root(&repo), None);

        // Setup runs once in the new worktree, with the main checkout in AGENTDECK_ROOT.
        let write_root = if cfg!(windows) {
            "echo %AGENTDECK_ROOT%> root.txt"
        } else {
            "echo \"$AGENTDECK_ROOT\" > root.txt"
        };
        let cwd = repo.to_string_lossy();
        let made = PathBuf::from(create_worktree_with(&cwd, "setup001", write_root).unwrap());
        let seen = std::fs::read_to_string(made.join("root.txt")).unwrap();
        assert_eq!(Path::new(seen.trim()), repo);
        // Reused worktree: the command (failing here) is not run again.
        assert!(create_worktree_with(&cwd, "setup001", "exit 3").is_ok());
        let err = create_worktree_with(&cwd, "setup002", "echo no deps 1>&2 && exit 3").unwrap_err();
        assert!(err.contains("worktree setup failed") && err.contains("no deps"), "{err}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
