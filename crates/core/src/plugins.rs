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
    /// Its SKILL.md.
    pub text: &'static str,
    /// Its other files (`scripts/x.js`, `reference/y.md`), by path relative to its folder.
    pub files: &'static [(&'static str, &'static str)],
}

impl BundledSkill {
    /// Version of the skill as shipped, compared with [`disk_digest`] of an installed copy.
    fn digest(&self) -> String {
        digest(std::iter::once(("SKILL.md".to_owned(), self.text.to_owned())).chain(self.files.iter().map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))).collect())
    }
}

pub struct Plugin {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Catalog section ("Browser", "Performance"...).
    pub category: &'static str,
    /// On until the user turns it off.
    pub default_on: bool,
    /// Ids of older plugins merged into this one: on when one of them was on, until the user picks.
    pub replaces: &'static [&'static str],
    pub skills: &'static [BundledSkill],
}

/// `skill!("plugin", "agentdeck-x", "scripts/y.js", ...)`: the skill's SKILL.md and the other files named.
macro_rules! skill {
    ($plugin:literal, $name:literal $(, $file:literal)*) => {
        BundledSkill {
            name: $name,
            text: include_str!(concat!("../plugins/", $plugin, "/", $name, "/SKILL.md")),
            files: &[$(($file, include_str!(concat!("../plugins/", $plugin, "/", $name, "/", $file)))),*],
        }
    };
}

pub const CATALOG: &[Plugin] = &[
    Plugin {
        id: "web-browser",
        name: "Web browser",
        description: "Agents test web pages for real in the AgentDeck browser panel: load, look, read console \
                      and network errors, use the page, accessibility audit, fix, check again. Phone-width audit \
                      script and full flow tests.",
        category: "Browser",
        default_on: true,
        replaces: &[],
        skills: &[
            skill!("web-browser", "agentdeck-web-check"),
            skill!("web-browser", "agentdeck-mobile-check", "scripts/mobile-audit.js"),
            skill!("web-browser", "agentdeck-form-flow"),
        ],
    },
    Plugin {
        id: "engineering",
        name: "Engineering",
        description: "Senior-engineer discipline: understand first, smallest complete change that fits the project, \
                      surgical diffs, no speculative code, then a self-review and fresh proof before saying done.",
        category: "Engineering",
        default_on: false,
        replaces: &["senior-dev", "lean-code", "code-review"],
        skills: &[skill!("engineering", "agentdeck-engineering")],
    },
    Plugin {
        id: "debugging",
        name: "Debugging",
        description: "No fix without a proven root cause: reproduce, trace the bad value to its origin, one \
                      hypothesis at a time, fix once at the source, regression test. Stops after three failed fixes.",
        category: "Quality",
        default_on: false,
        replaces: &[],
        skills: &[skill!("debugging", "agentdeck-debug")],
    },
    Plugin {
        id: "tests-first",
        name: "Tests first",
        description: "A test that fails for the right reason before any code, smallest change to pass, refactor \
                      while green. Real code over mocks, deterministic tests, never weakened to get green.",
        category: "Quality",
        default_on: false,
        replaces: &[],
        skills: &[skill!("tests-first", "agentdeck-tests-first")],
    },
    Plugin {
        id: "plan-first",
        name: "Plan first",
        description: "Big or vague tasks: one question at a time, a plan of small steps each with an exact check, \
                      risks, parallel parts ready for several agents, approval before risky work.",
        category: "Engineering",
        default_on: false,
        replaces: &[],
        skills: &[skill!("plan-first", "agentdeck-plan-first")],
    },
    Plugin {
        id: "security",
        name: "Security",
        description: "Secure by default and high-confidence reviews: injections, SSRF, IDOR, secrets, invented \
                      package names, fail-open errors, LLM prompt injection, with semgrep, gitleaks and audits.",
        category: "Quality",
        default_on: false,
        replaces: &[],
        skills: &[skill!("security", "agentdeck-security")],
    },
    Plugin {
        id: "performance",
        name: "Performance",
        description: "Measure before optimizing: performance traces with LCP subparts and INP in the browser \
                      panel, profilers and benchmarks for Rust, Node, Python, Go and SQL, before/after numbers.",
        category: "Performance",
        default_on: false,
        replaces: &[],
        skills: &[skill!("performance", "agentdeck-web-perf"), skill!("performance", "agentdeck-profile")],
    },
    Plugin {
        id: "git-workflow",
        name: "Git workflow",
        description: "Never lose work: state checked first, atomic Conventional Commits, rebase or merge, \
                      conflicts, pull requests with gh, finishing a branch, safe undo.",
        category: "Engineering",
        default_on: false,
        replaces: &[],
        skills: &[skill!("git-workflow", "agentdeck-git")],
    },
    Plugin {
        id: "web-research",
        name: "Web research",
        description: "Effort scaled to the question, parallel searches, primary sources first, pages read not \
                      skimmed, versions and dates checked, claims cited, fetched text treated as data.",
        category: "Research",
        default_on: false,
        replaces: &[],
        skills: &[skill!("web-research", "agentdeck-web-research")],
    },
    Plugin {
        id: "terse",
        name: "Terse replies",
        description: "Short answers that keep every fact: answer first, no filler or recaps, exact numbers and \
                      negations kept, code untouched, full sentences for warnings.",
        category: "Style",
        default_on: false,
        replaces: &[],
        skills: &[skill!("terse", "agentdeck-terse")],
    },
    Plugin {
        id: "3d",
        name: "3D",
        description: "3D that looks right and runs smooth. Blender via MCP or headless bpy scripts with rendered \
                      previews checked, version-safe APIs, glTF export. Three.js WebGPU, R3F, Godot, Unity, Bevy, Roblox.",
        category: "3D",
        default_on: false,
        replaces: &[],
        skills: &[skill!("3d", "agentdeck-3d", "reference/blender.md", "reference/realtime.md")],
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

/// Hash of a skill's files (path, text). A lone SKILL.md hashes as its text, as before skills had
/// other files: copies installed then still count as untouched.
fn digest(mut files: Vec<(String, String)>) -> String {
    if let [(path, text)] = files.as_slice()
        && path == "SKILL.md"
    {
        return hash(text);
    }
    files.sort();
    hash(&files.iter().map(|(path, text)| format!("{path}\n{text}\n")).collect::<String>())
}

/// [`digest`] of the files in an installed skill folder (marker aside); None when one cannot be read.
fn disk_digest(dir: &Path) -> Option<String> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) -> Option<()> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if entry.file_type().ok()?.is_dir() {
                walk(&entry.path(), &path, out)?;
            } else if path != MARKER {
                out.push((path, std::fs::read_to_string(entry.path()).ok()?));
            }
        }
        Some(())
    }
    let mut files = Vec::new();
    walk(dir, "", &mut files)?;
    Some(digest(files))
}

/// The folder's files are still exactly what AgentDeck wrote.
fn untouched(dir: &Path, written: &str) -> bool {
    disk_digest(dir).is_some_and(|now| now == written)
}

/// Writes the skill afresh: files dropped from a newer version must not linger (only called on a
/// new folder or an untouched copy).
fn write(dir: &Path, plugin: &str, skill: &BundledSkill) -> std::io::Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("SKILL.md"), skill.text)?;
    for (path, text) in skill.files {
        let file = dir.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file, text)?;
    }
    let marker = json!({ "plugin": plugin, "hash": skill.digest() });
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
                Some((_, written)) if written == skill.digest() => continue, // up to date
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

    /// Limits from Anthropic's skill authoring guide, and links between skills that still exist.
    #[test]
    fn catalog_skills_follow_the_authoring_rules() {
        let names: Vec<&str> = CATALOG.iter().flat_map(|p| p.skills.iter().map(|s| s.name)).collect();
        let ids: Vec<&str> = CATALOG.iter().map(|p| p.id).collect();
        for plugin in CATALOG {
            assert!(plugin.replaces.iter().all(|old| !ids.contains(old)), "{} replaces a live plugin", plugin.id);
            for skill in plugin.skills {
                let text = skill.text.replace("\r\n", "\n");
                let description = text.lines().find_map(|l| l.strip_prefix("description: ")).unwrap_or_default();
                assert!(description.len() <= 1024 && !description.contains('<'), "{}: description", skill.name);
                assert!(description.contains("Use "), "{}: description says when to use it", skill.name);
                assert!(text.lines().count() < 500, "{}: SKILL.md over 500 lines", skill.name);
                let all = std::iter::once(skill.text).chain(skill.files.iter().map(|(_, t)| *t));
                for body in all {
                    for (at, _) in body.match_indices("`agentdeck-") {
                        let rest = &body[at + 1..];
                        let named = &rest[..rest.find('`').unwrap_or(rest.len())];
                        assert!(names.contains(&named), "{} links to missing skill {named}", skill.name);
                    }
                }
                for (path, _) in skill.files {
                    assert!(skill.text.contains(path), "{}: {path} is never mentioned in SKILL.md", skill.name);
                }
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
        write(&web, "web-browser", &BundledSkill { name: "agentdeck-web-check", text: "old", files: &[] }).unwrap();
        sync_roots(&roots, &only_web);
        assert!(std::fs::read_to_string(web.join("SKILL.md")).unwrap().contains("agentdeck-web-check"));

        // An edited copy is kept when an update comes, and when its plugin is off.
        write(&web, "web-browser", &BundledSkill { name: "agentdeck-web-check", text: "old", files: &[] }).unwrap();
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
    fn skills_with_files_install_update_and_detect_edits() {
        let root = temp();
        let dir = root.join("agentdeck-x");
        let v1 = BundledSkill { name: "agentdeck-x", text: "v1", files: &[("scripts/a.js", "a1"), ("old.md", "o")] };
        write(&dir, "p", &v1).unwrap();
        assert!(untouched(&dir, &v1.digest()));
        assert_eq!(std::fs::read_to_string(dir.join("scripts/a.js")).unwrap(), "a1");
        // A new version replaces every file and drops the ones it no longer has.
        let v2 = BundledSkill { name: "agentdeck-x", text: "v1", files: &[("scripts/a.js", "a2")] };
        assert_ne!(v1.digest(), v2.digest());
        write(&dir, "p", &v2).unwrap();
        assert!(!dir.join("old.md").exists() && untouched(&dir, &v2.digest()));
        // Editing any file counts as an edit.
        std::fs::write(dir.join("scripts/a.js"), "mine").unwrap();
        assert!(!untouched(&dir, &v2.digest()));
        // A lone SKILL.md keeps the old hash: copies installed by older versions stay untouched.
        assert_eq!(BundledSkill { name: "agentdeck-y", text: "t", files: &[] }.digest(), hash("t"));
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
