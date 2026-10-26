//! Pre-accepts each agent's "trust this folder" prompt for folders opened in AgentDeck
//! (opt-in setting). Every edit keeps the rest of the agent's config untouched.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::agents::AgentKind;

/// Marks `cwd` as trusted in the agent's own config. Failures are logged, never fatal:
/// the agent then just shows its usual prompt.
pub fn trust_folder(agent: AgentKind, cwd: &str) {
    let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) else {
        return;
    };
    let home = PathBuf::from(home);
    let (given, canonical) = spellings(cwd);
    let result = match agent {
        // Claude keys projects by cwd with forward slashes; write both spellings so
        // a case difference between the typed and the on-disk path still matches.
        AgentKind::Claude => {
            let mut keys = vec![slashes(&given)];
            keys.extend(canonical.as_deref().map(slashes));
            keys.dedup();
            edit_file(&home.join(".claude.json"), |text| {
                trust_json(text, &keys, true)
            })
        }
        AgentKind::Codex => {
            let key = codex_key(canonical.as_deref().unwrap_or(&given));
            edit_file(&home.join(".codex").join("config.toml"), |text| {
                trust_codex(text, &key)
            })
        }
        // Gemini normalizes keys itself (realpath + normalizePath) on load and on match.
        AgentKind::Gemini => {
            let key = canonical.unwrap_or(given).to_string_lossy().into_owned();
            edit_file(&home.join(".gemini").join("trustedFolders.json"), |text| {
                trust_json(text, std::slice::from_ref(&key), false)
            })
        }
        // Grok: no documented file format; pty sets GROK_FOLDER_TRUST=0 instead.
        AgentKind::Grok | AgentKind::Shell => Ok(()),
    };
    if let Err(e) = result {
        eprintln!("trust: could not trust {cwd} for {agent:?}: {e}");
    }
}

/// The absolute path as given, and its canonical form (no `\\?\` prefix) when it exists.
fn spellings(cwd: &str) -> (PathBuf, Option<PathBuf>) {
    let given = std::path::absolute(cwd).unwrap_or_else(|_| PathBuf::from(cwd));
    let canonical = std::fs::canonicalize(&given).ok().map(|p| {
        let s = p.to_string_lossy();
        PathBuf::from(match s.strip_prefix(r"\\?\UNC\") {
            Some(rest) => format!(r"\\{rest}"),
            None => s.strip_prefix(r"\\?\").unwrap_or(&s).to_owned(),
        })
    });
    (given, canonical)
}

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Codex matches Windows keys case-insensitively and writes them lowercase itself.
fn codex_key(path: &Path) -> String {
    let key = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        key.to_ascii_lowercase()
    } else {
        key
    }
}

/// Read-modify-write with an atomic replace. `edit` returns None when nothing changes.
/// A parse error is retried once: the agent may be rewriting the file right now.
fn edit_file(
    path: &Path,
    edit: impl Fn(&str) -> Result<Option<String>, String>,
) -> Result<(), String> {
    let read = || match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    };
    let updated = match edit(&read()?) {
        Ok(updated) => updated,
        Err(_) => {
            std::thread::sleep(std::time::Duration::from_millis(150));
            edit(&read()?).map_err(|e| format!("{}: {e}", path.display()))?
        }
    };
    let Some(text) = updated else { return Ok(()) };
    let dir = path.parent().ok_or("no parent folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(
        ".{}.agentdeck-tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// Claude (`nested`): `projects[key].hasTrustDialogAccepted = true`.
/// Gemini (flat): `{ key: "TRUST_FOLDER" }`.
fn trust_json(text: &str, keys: &[String], nested: bool) -> Result<Option<String>, String> {
    let mut root: Value = if text.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str(text).map_err(|e| e.to_string())?
    };
    let before = root.clone();
    let obj = root.as_object_mut().ok_or("not a JSON object")?;
    let target = if nested {
        obj.entry("projects")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .ok_or("projects is not an object")?
    } else {
        obj
    };
    for key in keys {
        if nested {
            let project = target
                .entry(key.as_str())
                .or_insert_with(|| Value::Object(Map::new()));
            project
                .as_object_mut()
                .ok_or("project entry is not an object")?
                .insert("hasTrustDialogAccepted".into(), Value::Bool(true));
        } else {
            target.insert(key.clone(), Value::String("TRUST_FOLDER".into()));
        }
    }
    if root == before {
        return Ok(None);
    }
    let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    Ok(Some(text + "\n"))
}

/// Codex: `[projects.'<key>'] trust_level = "trusted"`, formatting and comments kept.
fn trust_codex(text: &str, key: &str) -> Result<Option<String>, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let projects = doc
        .entry("projects")
        .or_insert_with(|| {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true); // render as [projects.'path'] headers only
            toml_edit::Item::Table(table)
        })
        .as_table_mut()
        .ok_or("projects is not a table")?;
    // An existing entry may differ only in case (Windows): reuse it.
    let existing = projects
        .iter()
        .map(|(k, _)| k.to_owned())
        .find(|k| k == key || (cfg!(windows) && k.eq_ignore_ascii_case(key)));
    let entry = projects
        .entry(existing.as_deref().unwrap_or(key))
        .or_insert_with(toml_edit::table)
        .as_table_like_mut()
        .ok_or("project entry is not a table")?;
    if entry.get("trust_level").and_then(|v| v.as_str()) == Some("trusted") {
        return Ok(None);
    }
    entry.insert("trust_level", toml_edit::value("trusted"));
    Ok(Some(doc.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_merge_keeps_other_keys_and_is_idempotent() {
        let text = r#"{"numStartups":3,"projects":{"C:/a":{"allowedTools":["x"],"hasTrustDialogAccepted":false}},"z":{}}"#;
        let keys = ["C:/a".to_owned(), "D:/b".to_owned()];
        let out = trust_json(text, &keys, true).unwrap().unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["numStartups"], 3);
        assert_eq!(v["z"], serde_json::json!({}));
        assert_eq!(v["projects"]["C:/a"]["allowedTools"][0], "x");
        assert_eq!(v["projects"]["C:/a"]["hasTrustDialogAccepted"], true);
        assert_eq!(v["projects"]["D:/b"]["hasTrustDialogAccepted"], true);
        // Key order is preserved (serde_json preserve_order).
        assert!(out.find("numStartups").unwrap() < out.find("\"z\"").unwrap());
        assert_eq!(trust_json(&out, &keys, true).unwrap(), None);
    }

    #[test]
    fn json_creates_missing_file_and_rejects_garbage() {
        let out = trust_json("", &["/p".to_owned()], false).unwrap().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&out).unwrap(),
            serde_json::json!({"/p": "TRUST_FOLDER"})
        );
        assert!(trust_json("{\"half", &["/p".to_owned()], true).is_err());
        assert!(trust_json("[]", &["/p".to_owned()], true).is_err());
    }

    #[test]
    fn codex_merge_keeps_comments_and_is_idempotent() {
        let text = "# my config\nmodel = \"o3\" # inline\n\n[projects.'c:\\users\\x']\ntrust_level = \"trusted\"\n";
        let out = trust_codex(text, r"d:\proj").unwrap().unwrap();
        assert!(out.starts_with("# my config\nmodel = \"o3\" # inline\n"));
        assert!(out.contains("[projects.'c:\\users\\x']"));
        assert!(out.contains("[projects.'d:\\proj']\ntrust_level = \"trusted\""));
        assert_eq!(trust_codex(&out, r"d:\proj").unwrap(), None);
        assert_eq!(trust_codex(text, r"c:\users\x").unwrap(), None);
        let fresh = trust_codex("", "/home/u/p").unwrap().unwrap();
        assert_eq!(
            fresh,
            "[projects.\"/home/u/p\"]\ntrust_level = \"trusted\"\n"
        );
    }

    #[test]
    fn edit_file_writes_atomically_and_skips_unchanged() {
        let dir = std::env::temp_dir().join(format!("agentdeck-trust-{}", uuid::Uuid::new_v4()));
        let path = dir.join("sub").join("f.json");
        let keys = ["/p".to_owned()];
        edit_file(&path, |t| trust_json(t, &keys, false)).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.contains("TRUST_FOLDER"));
        edit_file(&path, |t| trust_json(t, &keys, false)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        ); // no temp left
        std::fs::write(&path, "not json").unwrap();
        assert!(edit_file(&path, |t| trust_json(t, &keys, false)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
