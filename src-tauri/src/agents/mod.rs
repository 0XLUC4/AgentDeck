//! Agent definitions (Claude Code, Codex, Gemini CLI), install detection,
//! git worktrees and hook configuration.

use std::path::{Path, PathBuf};
use std::process::Command;

use portable_pty::CommandBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::hooks_server::{HookServer, TOKEN_HEADER};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
    Gemini,
    Grok,
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
            Self::Shell => None,
        }
    }

    fn install_url(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("https://code.claude.com/docs/en/setup"),
            Self::Codex => Some("https://github.com/openai/codex#quickstart"),
            Self::Gemini => Some("https://github.com/google-gemini/gemini-cli#-installation"),
            Self::Grok => Some("https://docs.x.ai/build/overview"),
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
#[tauri::command(async)] // PATH scan: off the main thread
pub fn detect_agents() -> Vec<AgentInfo> {
    [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Gemini,
        AgentKind::Grok,
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

/// Creates (or reuses) the dedicated git worktree for the pane and returns its path.
pub fn create_worktree(cwd: &str, pane_id: &str) -> Result<String, String> {
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
    git(
        &root,
        &[
            "worktree",
            "add",
            &target_str,
            "-b",
            &format!("agentdeck/{short}"),
        ],
    )?;
    Ok(target_str)
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
            // Non-fatal: Gemini settings may be JSONC we refuse to rewrite; output-based idle detection still works.
            let url = hooks.url();
            if let Err(e) = update_settings(&gemini_settings(cwd), |s| merge_gemini_hooks(s, &url))
            {
                eprintln!("gemini hooks not installed: {e}");
            }
            Ok(())
        }
        // Codex: see `hook_args` (no global config edits).
        _ => Ok(()),
    }
}

/// Removes AgentDeck's hooks from `<cwd>/.claude/settings.local.json` and `<cwd>/.gemini/settings.json`.
#[tauri::command]
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn gemini_settings_file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("agentdeck-gem-{}", uuid::Uuid::new_v4()));
        let file = dir.join(".gemini").join("settings.json");
        let cwd = dir.to_string_lossy().into_owned();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{ "theme": "Dracula" }"#).unwrap();
        let hooks = HookServer {
            port: 47800,
            token: "tok".into(),
        };
        prepare_hooks(AgentKind::Gemini, &cwd, &hooks).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains(GEMINI_HOOK_NAME) && !text.contains("tok\""));
        uninstall_hooks(cwd.clone()).unwrap();
        let back: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(back, json!({ "theme": "Dracula" }));

        // JSONC (comments) is left untouched and does not block the pane.
        std::fs::write(&file, "// mine\n{}").unwrap();
        prepare_hooks(AgentKind::Gemini, &cwd, &hooks).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "// mine\n{}");
        std::fs::remove_dir_all(&dir).unwrap();
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
        let _ = std::fs::remove_dir_all(&base);
    }
}
