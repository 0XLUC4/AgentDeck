//! Update check for the installed agent CLIs: `<cli> --version` against the npm registry's latest
//! release, and the command that updates each one the way it was installed.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use super::{AgentKind, detect_agents};

/// An installed CLI with a newer release. `command`: program + arguments that update it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliUpdate {
    pub kind: AgentKind,
    pub current: String,
    pub latest: String,
    pub command: Vec<String>,
}

/// npm package each CLI is published as (its native installers ship the same version numbers).
// shortcut: Grok's CLI comes from xAI's installer with no registry to ask; add it here once it has one.
pub fn npm_package(kind: AgentKind) -> Option<&'static str> {
    match kind {
        AgentKind::Claude => Some("@anthropic-ai/claude-code"),
        AgentKind::Codex => Some("@openai/codex"),
        AgentKind::Gemini => Some("@google/gemini-cli"),
        AgentKind::OpenCode => Some("opencode-ai"),
        AgentKind::Grok | AgentKind::Shell => None,
        AgentKind::Other(spec) => spec.npm,
    }
}

/// First `major.minor.patch` in `text` ("codex-cli 0.170.0", "2.1.322 (Claude Code)").
pub fn parse_version(text: &str) -> Option<[u64; 3]> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.')).find_map(|word| {
        let mut parts = word.split('.').map(|p| p.parse::<u64>().ok());
        let version = [parts.next()??, parts.next()??, parts.next()??];
        Some(version)
    })
}

/// Whether `latest` is a newer release than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// Installed CLIs that have a newer release and a known way to update. Blocking (processes + HTTP).
pub fn check_updates() -> Vec<CliUpdate> {
    let Ok(client) = reqwest::blocking::Client::builder().timeout(Duration::from_secs(15)).build() else {
        return Vec::new();
    };
    detect_agents()
        .into_iter()
        .filter_map(|agent| {
            let package = npm_package(agent.kind)?;
            let bin = Path::new(agent.path.as_deref()?);
            let current = run(bin, &["--version"]).ok().and_then(|out| parse_version(&out))?;
            let latest = latest_version(&client, package)?;
            let current = format!("{}.{}.{}", current[0], current[1], current[2]);
            if !is_newer(&latest, &current) {
                return None;
            }
            let command = update_command(agent.kind, bin, package)?;
            Some(CliUpdate { kind: agent.kind, current, latest, command })
        })
        .collect()
}

/// Runs the update command; `Err` carries the tail of its output.
pub fn run_update(update: &CliUpdate) -> Result<(), String> {
    let (program, args) = update.command.split_first().ok_or("empty update command")?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run(Path::new(program), &args).map(|_| ())
}

fn latest_version(client: &reqwest::blocking::Client, package: &str) -> Option<String> {
    let json: serde_json::Value = client
        .get(format!("https://registry.npmjs.org/{package}/latest"))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::json)
        .map_err(|e| eprintln!("cli update: {package}: {e}"))
        .ok()?;
    json["version"].as_str().map(str::to_owned)
}

/// npm global install when the package sits in the `node_modules` next to the shim (Windows) or
/// the binary resolves into one (Unix symlink); otherwise the CLI's own updater, if it has one.
fn update_command(kind: AgentKind, bin: &Path, package: &str) -> Option<Vec<String>> {
    let beside = bin.parent().is_some_and(|dir| dir.join("node_modules").join(package).is_dir());
    let linked = std::fs::canonicalize(bin)
        .is_ok_and(|real| real.to_string_lossy().replace('\\', "/").contains(&format!("node_modules/{package}/")));
    if beside || linked {
        return Some(["npm", "install", "-g", &format!("{package}@latest")].map(str::to_owned).to_vec());
    }
    let own = match kind {
        AgentKind::Claude => "update",
        AgentKind::OpenCode => "upgrade",
        _ => return None,
    };
    Some(vec![bin.to_string_lossy().into_owned(), own.to_owned()])
}

/// Runs `program args` without a console window and returns stdout. On Windows an npm shim runs its
/// target directly (as agents are launched); other batch files and bare names (`npm`) go through
/// `cmd /C`, which resolves PATHEXT.
fn run(program: &Path, args: &[&str]) -> Result<String, String> {
    let shim = program.extension().is_none_or(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    let target = if cfg!(windows) && shim { super::npm_shim_target(program) } else { None };
    let mut cmd = if let Some((exe, script)) = target {
        let mut cmd = Command::new(exe);
        cmd.args(script);
        cmd
    } else if cfg!(windows) && shim {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(program);
        cmd
    } else {
        Command::new(program)
    };
    cmd.args(args).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("cannot run {}: {e}", program.display()))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let text = String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).into_owned();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    Err(lines[lines.len().saturating_sub(3)..].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("2.1.322 (Claude Code)"), Some([2, 1, 322]));
        assert_eq!(parse_version("codex-cli 0.170.0"), Some([0, 170, 0]));
        assert_eq!(parse_version("v1.2.25\n"), Some([1, 2, 25]));
        assert_eq!(parse_version("0.70.0-preview.1"), Some([0, 70, 0]));
        assert_eq!(parse_version("no version"), None);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("2.1.329", "2.1.322"));
        assert!(is_newer("1.19.6", "1.2.25"));
        assert!(!is_newer("0.170.0", "0.170.0"));
        assert!(!is_newer("0.9.0", "0.10.0"));
        assert!(!is_newer("garbage", "1.0.0"));
    }

    #[test]
    fn npm_installs_update_through_npm() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/@openai/codex")).unwrap();
        let bin = dir.path().join("codex.cmd");
        std::fs::write(&bin, "").unwrap();
        assert_eq!(
            update_command(AgentKind::Codex, &bin, "@openai/codex").unwrap(),
            ["npm", "install", "-g", "@openai/codex@latest"]
        );
        // A native Claude install updates itself; a native Codex has no updater.
        let native = dir.path().join("bin").join("claude.exe");
        assert_eq!(update_command(AgentKind::Claude, &native, "@anthropic-ai/claude-code").unwrap()[1], "update");
        assert_eq!(update_command(AgentKind::Codex, &native, "@openai/codex"), None);
    }
}
