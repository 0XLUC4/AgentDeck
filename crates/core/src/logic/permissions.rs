//! Permission rules answered by AgentDeck without asking (`components/Permission/autoAllow.ts`):
//! "Read", "Bash(pnpm test)", "Bash(pnpm test *)". Shell commands only match when they are a single
//! plain command: a rule for `pnpm test` must never approve `pnpm test && rm -rf ~`.

use super::settings::AutoAllowRule;

const SHELL_TOOLS: [&str; 2] = ["Bash", "PowerShell"];
/// Chaining, substitution, redirection, newlines: anything that could run more than the approved command.
const SHELL_META: &[char] = &[';', '&', '|', '`', '$', '<', '>', '\r', '\n', '(', ')', '{', '}'];

fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

pub fn inside_folder(cwd: &str, folder: &str) -> bool {
    if folder.is_empty() {
        return true;
    }
    let (base, path) = (normalize(folder), normalize(cwd));
    path == base || path.starts_with(&format!("{base}/"))
}

/// Splits "Bash(pnpm test)" into tool + content; a bare "Read" has no content. None when malformed.
pub fn parse_rule(rule: &str) -> Option<(String, Option<String>)> {
    let rule = rule.trim();
    let valid_tool = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-');
    match rule.split_once('(') {
        None => valid_tool(rule).then(|| (rule.to_owned(), None)),
        Some((tool, rest)) => {
            let content = rest.strip_suffix(')')?;
            (valid_tool(tool) && !content.is_empty())
                .then(|| (tool.to_owned(), Some(content.trim().to_owned())))
        }
    }
}

/// True when `rule` covers this request. Shell rules need content; other tools match by name only.
pub fn rule_matches(rule: &str, tool_name: &str, detail: &str) -> bool {
    let Some((tool, content)) = parse_rule(rule) else {
        return false;
    };
    if tool != tool_name {
        return false;
    }
    if !SHELL_TOOLS.contains(&tool_name) {
        return content.is_none();
    }
    let Some(content) = content else {
        return false; // never "any shell command"
    };
    let command = detail.trim();
    if command.contains(SHELL_META) {
        return false;
    }
    match content.strip_suffix(" *") {
        Some(prefix) => {
            let prefix = prefix.trim();
            command == prefix || command.starts_with(&format!("{prefix} "))
        }
        None => command == content,
    }
}

pub fn auto_allowed<'a>(
    tool_name: &str,
    detail: &str,
    cwd: &str,
    rules: &'a [AutoAllowRule],
) -> Option<&'a AutoAllowRule> {
    rules
        .iter()
        .find(|r| inside_folder(cwd, &r.folder) && rule_matches(&r.rule, tool_name, detail))
}

/// Rule the "Auto-allow here" button adds: the exact shell command, or the tool name. None if not allowed.
pub fn rule_for(tool_name: &str, detail: &str) -> Option<String> {
    if !SHELL_TOOLS.contains(&tool_name) {
        return Some(tool_name.to_owned());
    }
    let command = detail.trim();
    (!command.is_empty() && !command.contains(SHELL_META)).then(|| format!("{tool_name}({command})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_rules_are_strict() {
        assert!(rule_matches("Bash(pnpm test)", "Bash", " pnpm test "));
        assert!(!rule_matches("Bash(pnpm test)", "Bash", "pnpm test && rm -rf ~"));
        assert!(!rule_matches("Bash(pnpm test)", "Bash", "pnpm test --watch"));
        assert!(rule_matches("Bash(pnpm test *)", "Bash", "pnpm test --watch"));
        assert!(rule_matches("Bash(pnpm test *)", "Bash", "pnpm test"));
        assert!(!rule_matches("Bash(pnpm test *)", "Bash", "pnpm testx"));
        assert!(!rule_matches("Bash", "Bash", "ls"));
        assert!(!rule_matches("Bash(ls)", "Bash", "ls $(whoami)"));
    }

    #[test]
    fn other_tools_by_name() {
        assert!(rule_matches("Read", "Read", "{}"));
        assert!(!rule_matches("Read(x)", "Read", "{}"));
        assert!(!rule_matches("Edit", "Read", "{}"));
        assert!(parse_rule("Bad Rule").is_none());
    }

    #[test]
    fn folders_and_rule_for() {
        let rules = vec![AutoAllowRule {
            rule: "Read".into(),
            folder: "D:\\p".into(),
        }];
        assert!(auto_allowed("Read", "", "d:/p/sub", &rules).is_some());
        assert!(auto_allowed("Read", "", "D:\\px", &rules).is_none());
        assert_eq!(rule_for("Bash", "npm test").as_deref(), Some("Bash(npm test)"));
        assert_eq!(rule_for("Bash", "a; b"), None);
        assert_eq!(rule_for("Edit", "x").as_deref(), Some("Edit"));
    }
}
