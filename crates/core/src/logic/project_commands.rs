//! "Run project command…": the commands a worktree offers (root `package.json` scripts with the
//! project's package manager, usual Cargo commands) and the shell arguments that run one in a
//! dedicated terminal pane. Pure: the app reads the files and checks the PATH.

/// One runnable command: `program` and its arguments, kept separate so their limits survive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl ProjectCommand {
    fn new(program: &str, args: &[&str]) -> Self {
        Self { program: program.to_owned(), args: args.iter().map(|a| (*a).to_owned()).collect() }
    }

    /// As shown to the user before launch: "pnpm run dev".
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_str()).chain(self.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ")
    }
}

/// Lock file -> package manager, same order as the post-merge check of the git module.
const RUNNERS: [(&str, &str); 4] = [("pnpm-lock.yaml", "pnpm"), ("yarn.lock", "yarn"), ("bun.lock", "bun"), ("bun.lockb", "bun")];

/// Package manager of a JS project from the files at its root (`has_file(name)`); npm by default.
/// Returns (runner, lock file that chose it).
pub fn package_runner(has_file: impl Fn(&str) -> bool) -> (&'static str, Option<&'static str>) {
    RUNNERS
        .into_iter()
        .find(|(lock, _)| has_file(lock))
        .map_or(("npm", None), |(lock, runner)| (runner, Some(lock)))
}

/// Script names of a `package.json`, in declared order. Err: not valid JSON, or `scripts` is not an object.
pub fn package_scripts(json: &str) -> Result<Vec<String>, String> {
    let package: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("package.json is not valid JSON: {e}"))?;
    match &package["scripts"] {
        serde_json::Value::Null => Ok(Vec::new()),
        serde_json::Value::Object(scripts) => Ok(scripts.iter().filter(|(_, v)| v.is_string()).map(|(k, _)| k.clone()).collect()),
        _ => Err("package.json: \"scripts\" is not an object".to_owned()),
    }
}

/// `<runner> run <script>` for each script.
pub fn script_commands(runner: &str, scripts: &[String]) -> Vec<ProjectCommand> {
    scripts.iter().map(|name| ProjectCommand::new(runner, &["run", name])).collect()
}

/// The usual Cargo commands, offered when `Cargo.toml` exists.
pub fn cargo_commands() -> Vec<ProjectCommand> {
    ["check", "test", "build", "run", "clippy", "fmt"].iter().map(|sub| ProjectCommand::new("cargo", &[sub])).collect()
}

/// PowerShell single-quoted literal: no expansion, `'` doubled.
fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// POSIX shell single-quoted literal.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Arguments of the shell pane (PowerShell on Windows, `$SHELL` elsewhere) that run `command` and
/// keep the terminal open afterwards: its output stays readable, Ctrl+C stops a server and leaves a
/// prompt. Each argument is quoted on its own, so spaces and quotes stay inside it.
pub fn shell_args(command: &ProjectCommand, windows: bool) -> Vec<String> {
    let words = std::iter::once(&command.program).chain(&command.args);
    if windows {
        let line = words.map(|w| ps_quote(w)).collect::<Vec<_>>().join(" ");
        vec!["-NoExit".into(), "-Command".into(), format!("& {line}")]
    } else {
        let line = words.map(|w| sh_quote(w)).collect::<Vec<_>>().join(" ");
        vec!["-c".into(), format!("{line}; exec \"${{SHELL:-/bin/sh}}\"")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_follows_the_lock_file() {
        assert_eq!(package_runner(|f| f == "pnpm-lock.yaml"), ("pnpm", Some("pnpm-lock.yaml")));
        assert_eq!(package_runner(|f| f == "bun.lockb"), ("bun", Some("bun.lockb")));
        assert_eq!(package_runner(|_| false), ("npm", None));
    }

    #[test]
    fn scripts_keep_their_order_and_reject_bad_manifests() {
        let json = r#"{"scripts":{"dev":"vite","test":"vitest","build":"vite build","bad":1}}"#;
        assert_eq!(package_scripts(json).unwrap(), ["dev", "test", "build"]);
        assert_eq!(package_scripts(r#"{"name":"x"}"#).unwrap(), Vec::<String>::new());
        assert!(package_scripts("{ nope").is_err());
        assert!(package_scripts(r#"{"scripts":[]}"#).is_err());
    }

    #[test]
    fn commands_display_as_typed() {
        let cmds = script_commands("pnpm", &["dev".into()]);
        assert_eq!(cmds[0].display(), "pnpm run dev");
        assert_eq!(cargo_commands().iter().map(ProjectCommand::display).collect::<Vec<_>>()[..2], ["cargo check", "cargo test"]);
    }

    #[test]
    fn shell_args_keep_argument_limits() {
        let cmd = ProjectCommand::new("pnpm", &["run", "it's dev"]);
        assert_eq!(shell_args(&cmd, true), ["-NoExit", "-Command", "& 'pnpm' 'run' 'it''s dev'"]);
        let unix = shell_args(&cmd, false);
        assert_eq!(unix[0], "-c");
        assert!(unix[1].starts_with(r"'pnpm' 'run' 'it'\''s dev'; exec"));
    }
}
