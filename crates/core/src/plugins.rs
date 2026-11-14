//! AgentDeck plugins: skills bundled with the app, installed for every agent at once.
//!
//! A plugin is a set of skills (`plugins/<plugin>/<skill>/SKILL.md`, compiled in). Turned on, its
//! skills are copied into each agent's user skill folder; turned off, they are removed.
//!
//! - Names: every bundled skill is called `agentdeck-*`, so it never takes the name of a skill the
//!   user or an agent's own plugin system installed.
//! - Ownership: each copied folder holds [`MARKER`] with the plugin id and the hash of the SKILL.md
//!   written. A folder without it is not ours and is never touched (reported as a conflict).
//! - Updates: [`sync`] runs at start and on each toggle. A copy whose SKILL.md still has the hash we
//!   wrote is replaced when the app ships a new version; one the user edited is kept as is (reported).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::agents::AgentKind;

/// Ownership file inside each skill folder AgentDeck copied.
const MARKER: &str = ".agentdeck-plugin.json";
/// Prefix of every bundled skill name (and folder).
const PREFIX: &str = "agentdeck-";

pub struct BundledSkill {
    /// Folder and `name:` of the skill, always [`PREFIX`]-ed.
    pub name: &'static str,
    pub text: &'static str,
}

pub struct Plugin {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Catalog section ("Browser", "Performance"...).
    pub category: &'static str,
    /// On until the user turns it off.
    pub default_on: bool,
    pub skills: &'static [BundledSkill],
}

macro_rules! skill {
    ($plugin:literal, $name:literal) => {
        BundledSkill { name: $name, text: include_str!(concat!("../plugins/", $plugin, "/", $name, "/SKILL.md")) }
    };
}

pub const CATALOG: &[Plugin] = &[
    Plugin {
        id: "web-browser",
        name: "Web browser",
        description: "Agents test web pages for real in the AgentDeck browser panel: load, look, read console \
                      and network errors, use the page, fix, check again. Phone-width checks and full flow tests.",
        category: "Browser",
        default_on: true,
        skills: &[
            skill!("web-browser", "agentdeck-web-check"),
            skill!("web-browser", "agentdeck-mobile-check"),
            skill!("web-browser", "agentdeck-form-flow"),
        ],
    },
    Plugin {
        id: "performance",
        name: "Performance",
        description: "Measure before optimizing: Lighthouse and performance traces in the browser panel, \
                      profilers and benchmarks for Rust, Node, Python, Go and SQL, before/after numbers.",
        category: "Performance",
        default_on: false,
        skills: &[skill!("performance", "agentdeck-web-perf"), skill!("performance", "agentdeck-profile")],
    },
    Plugin {
        id: "code-review",
        name: "Self-review",
        description: "Agents review their own diff before saying done: callers, edge cases, errors, security, \
                      leftovers, and the project's checks.",
        category: "Quality",
        default_on: false,
        skills: &[skill!("code-review", "agentdeck-self-review")],
    },
    Plugin {
        id: "debugging",
        name: "Debugging",
        description: "Root cause from evidence: reproduce, narrow down, prove the cause, fix it once, prove the fix.",
        category: "Quality",
        default_on: false,
        skills: &[skill!("debugging", "agentdeck-debug")],
    },
];

/// FNV-1a: stable across builds (unlike `DefaultHasher`), enough to spot an edited file.
fn hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.replace("\r\n", "\n").bytes() {
        h = (h ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// The user skill folders to fill, one per agent family that is installed (its config folder exists):
/// `~/.claude/skills` (Claude Code, also read by OpenCode), `~/.agents/skills` (shared by Codex and
/// Gemini), `~/.grok/skills`. One copy per folder, so no agent sees a skill twice.
fn roots(home: &Path) -> Vec<PathBuf> {
    let has = |dir: &str| home.join(dir).is_dir();
    let mut roots = Vec::new();
    if has(".claude") || has(".config/opencode") {
        roots.push(home.join(".claude").join("skills"));
    }
    if has(".codex") || has(".gemini") {
        roots.push(home.join(".agents").join("skills"));
    }
    if has(".grok") {
        roots.push(home.join(".grok").join("skills"));
    }
    roots
}

/// The marker of an AgentDeck-owned folder: (plugin id, hash written).
fn marker(dir: &Path) -> Option<(String, String)> {
    let value: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(MARKER)).ok()?).ok()?;
    Some((value["plugin"].as_str()?.to_owned(), value["hash"].as_str()?.to_owned()))
}

/// The folder's SKILL.md is still exactly what AgentDeck wrote.
fn untouched(dir: &Path, written: &str) -> bool {
    std::fs::read_to_string(dir.join("SKILL.md")).is_ok_and(|text| hash(&text) == written)
}

fn write(dir: &Path, plugin: &str, skill: &BundledSkill) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("SKILL.md"), skill.text)?;
    let marker = json!({ "plugin": plugin, "hash": hash(skill.text) });
    std::fs::write(dir.join(MARKER), marker.to_string())
}

/// Brings every root in line with the plugins `enabled` says are on. Returns notes for the user
/// (conflicts, edited copies kept, write errors); empty when all went as planned.
pub fn sync(enabled: &dyn Fn(&Plugin) -> bool) -> Vec<String> {
    home().map_or_else(|| vec!["Plugins: home folder not found".to_owned()], |home| sync_roots(&roots(&home), enabled))
}

fn sync_roots(roots: &[PathBuf], enabled: &dyn Fn(&Plugin) -> bool) -> Vec<String> {
    let mut notes = Vec::new();
    let wanted: Vec<(&Plugin, &BundledSkill)> =
        CATALOG.iter().filter(|p| enabled(p)).flat_map(|p| p.skills.iter().map(move |s| (p, s))).collect();
    for root in roots {
        let shown = root.display();
        for (plugin, skill) in &wanted {
            let dir = root.join(skill.name);
            let result = match marker(&dir) {
                None if dir.exists() => {
                    notes.push(format!("{shown}: a skill named {} is not AgentDeck's, left as is", skill.name));
                    continue;
                }
                None => write(&dir, plugin.id, skill),
                Some((_, written)) if written == hash(skill.text) => continue, // up to date
                Some((_, written)) if untouched(&dir, &written) => write(&dir, plugin.id, skill), // update
                Some(_) => {
                    notes.push(format!("{shown}/{}: edited by you, not updated", skill.name));
                    continue;
                }
            };
            if let Err(e) = result {
                notes.push(format!("{shown}/{}: {e}", skill.name));
            }
        }
        // Ours but no longer wanted (plugin off, skill dropped from the catalog): removed unless edited.
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let dir = entry.path();
            if !name.starts_with(PREFIX) || wanted.iter().any(|(_, s)| s.name == name) {
                continue;
            }
            let Some((_, written)) = marker(&dir) else { continue };
            if !untouched(&dir, &written) {
                notes.push(format!("{shown}/{name}: edited by you, kept although its plugin is off"));
            } else if let Err(e) = std::fs::remove_dir_all(&dir) {
                notes.push(format!("{shown}/{name}: {e}"));
            }
        }
    }
    notes
}

/// Installs the skills of a GitHub link for every installed agent (user scope). Returns the skill names.
pub fn install_everywhere(url: &str) -> Result<Vec<String>, String> {
    let home = home().ok_or("home folder not found")?;
    let cwd = home.to_string_lossy().into_owned();
    let agents: Vec<AgentKind> = [(AgentKind::Claude, ".claude"), (AgentKind::Codex, ".codex"), (AgentKind::Gemini, ".gemini")]
        .into_iter()
        .filter(|(_, dir)| home.join(dir).is_dir())
        .map(|(agent, _)| agent)
        .collect();
    if agents.is_empty() {
        return Err("no agent installed (no ~/.claude, ~/.codex or ~/.gemini folder)".into());
    }
    let mut names = Vec::new();
    for agent in agents {
        for name in crate::skills::skills_install(agent, cwd.clone(), url.to_owned(), "user".into())? {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agentdeck-plugins-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn catalog_skills_are_prefixed_and_named_like_their_folder() {
        for plugin in CATALOG {
            for skill in plugin.skills {
                assert!(skill.name.starts_with(PREFIX), "{}", skill.name);
                let text = skill.text.replace("\r\n", "\n");
                assert!(text.contains(&format!("name: {}\n", skill.name)), "{}", skill.name);
                assert!(text.contains("description: "), "{}", skill.name);
            }
        }
    }

    #[test]
    fn installs_updates_keeps_edits_and_removes() {
        let root = temp();
        let roots = [root.clone()];
        let only_web = |p: &Plugin| p.id == "web-browser";
        let web = root.join("agentdeck-web-check");

        // A foreign folder with one of our names is never touched.
        std::fs::create_dir_all(root.join("agentdeck-form-flow")).unwrap();
        std::fs::write(root.join("agentdeck-form-flow/SKILL.md"), "mine").unwrap();
        let notes = sync_roots(&roots, &only_web);
        assert!(web.join("SKILL.md").is_file() && web.join(MARKER).is_file());
        assert_eq!(std::fs::read_to_string(root.join("agentdeck-form-flow/SKILL.md")).unwrap(), "mine");
        assert_eq!(notes.len(), 1, "{notes:?}");

        // An older untouched copy is updated.
        write(&web, "web-browser", &BundledSkill { name: "agentdeck-web-check", text: "old" }).unwrap();
        sync_roots(&roots, &only_web);
        assert!(std::fs::read_to_string(web.join("SKILL.md")).unwrap().contains("agentdeck-web-check"));

        // An edited copy is kept when an update comes, and when its plugin is off.
        write(&web, "web-browser", &BundledSkill { name: "agentdeck-web-check", text: "old" }).unwrap();
        std::fs::write(web.join("SKILL.md"), "edited").unwrap();
        assert!(sync_roots(&roots, &only_web).iter().any(|n| n.contains("not updated")));
        assert!(sync_roots(&roots, &|_| false).iter().any(|n| n.contains("kept")));
        assert_eq!(std::fs::read_to_string(web.join("SKILL.md")).unwrap(), "edited");

        // Untouched copies go when the plugin is turned off.
        std::fs::remove_dir_all(&web).unwrap();
        sync_roots(&roots, &only_web);
        sync_roots(&roots, &|_| false);
        assert!(!web.exists() && !root.join("agentdeck-mobile-check").exists());
        assert!(root.join("agentdeck-form-flow").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn roots_follow_installed_agents() {
        let home = temp();
        assert!(roots(&home).is_empty());
        std::fs::create_dir_all(home.join(".gemini")).unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        assert_eq!(roots(&home), [home.join(".claude").join("skills"), home.join(".agents").join("skills")]);
        assert_eq!(hash("a\r\nb"), hash("a\nb"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
