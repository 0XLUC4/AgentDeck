//! Persistence (`src/store/persist.ts` + tauri-plugin-store): plain JSON files in the app data dir.
//! Same file names and keys as the Tauri build (`store.json` "deck", `settings.json` "settings",
//! `templates.json` "templates"), so existing data loads unchanged.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::logic::deck::{
    DEFAULT_WORKSPACE_NAME, DeckState, Pane, SavedSession, Tab, Workspace, new_id, new_tab,
    tab_to_session,
};
use crate::logic::settings::{Settings, merge_settings};
use crate::logic::templates::{SessionTemplate, parse_templates};

/// One JSON object file of key -> value (what tauri-plugin-store wrote).
pub struct JsonStore {
    path: PathBuf,
    data: Map<String, Value>,
}

impl JsonStore {
    /// Loads `dir/name`; a missing file is an empty store, a corrupt one an error.
    pub fn load(dir: &Path, name: &str) -> Result<Self, String> {
        let path = dir.join(name);
        let data = match std::fs::read_to_string(&path) {
            Ok(text) => {
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Ok(Self { path, data })
    }

    pub fn get(&self, key: &str) -> Value {
        self.data.get(key).cloned().unwrap_or(Value::Null)
    }

    pub fn set(&mut self, key: &str, value: Value) {
        self.data.insert(key.to_owned(), value);
    }

    /// Writes atomically (temp file + rename) so a crash never leaves half a file.
    pub fn save(&self) -> Result<(), String> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(&self.data).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("{}: {e}", self.path.display()))
    }
}

pub const DECK_FILE: &str = "store.json";
pub const DECK_KEY: &str = "deck";
pub const SETTINGS_FILE: &str = "settings.json";
pub const SETTINGS_KEY: &str = "settings";
pub const TEMPLATES_FILE: &str = "templates.json";
pub const TEMPLATES_KEY: &str = "templates";
/// Small per-user values the web build kept in localStorage (recent prompts, window geometry...).
pub const LOCAL_FILE: &str = "local.json";
/// Coalesce bursts (split drags, hook events) into one save.
pub const SAVE_DEBOUNCE_MS: u64 = 300;

/// Saved deck, version 2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub version: u32,
    pub workspaces: Vec<Workspace>,
    pub tabs: Vec<SavedSession>,
    pub active_tab_id: String,
    pub last_cwd: String,
    #[serde(default)]
    pub archived: Vec<SavedSession>,
}

fn sessions(v: &Value) -> Vec<SavedSession> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| serde_json::from_value(s.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Upgrades any saved shape to v2. None when there is nothing usable.
pub fn migrate(saved: &Value) -> Option<Saved> {
    match saved["version"].as_u64() {
        Some(2) => {
            let s: Saved = serde_json::from_value(saved.clone()).ok()?;
            (!s.workspaces.is_empty() && !s.tabs.is_empty()).then_some(s)
        }
        Some(1) => {
            // v1 had no workspaces: old sessions go into a "Default" workspace.
            let ws = Workspace {
                id: new_id(),
                name: DEFAULT_WORKSPACE_NAME.into(),
                path: String::new(),
                git: None,
            };
            let with_ws = |mut s: SavedSession| {
                s.workspace_id = ws.id.clone();
                s
            };
            let tabs: Vec<_> = sessions(&saved["tabs"]).into_iter().map(with_ws).collect();
            if tabs.is_empty() {
                return None;
            }
            let archived = sessions(&saved["archived"]).into_iter().map(with_ws).collect();
            Some(Saved {
                version: 2,
                tabs,
                archived,
                active_tab_id: saved["activeTabId"].as_str().unwrap_or("").into(),
                last_cwd: saved["lastCwd"].as_str().unwrap_or("").into(),
                workspaces: vec![ws],
            })
        }
        _ => None,
    }
}

/// Snapshot of the deck for saving.
pub fn snapshot(deck: &DeckState, now: i64) -> Saved {
    Saved {
        version: 2,
        workspaces: deck.workspaces.clone(),
        tabs: deck
            .tabs
            .iter()
            .map(|t| tab_to_session(t, &deck.panes, now))
            .collect(),
        active_tab_id: deck.active_tab_id.clone(),
        last_cwd: deck.last_cwd.clone(),
        archived: deck.archived.clone(),
    }
}

/// Applies a saved state to the deck (without spawning). Returns the sessions whose panes to respawn.
pub fn hydrate(deck: &mut DeckState, saved: Saved) -> Vec<SavedSession> {
    let known: HashSet<String> = saved.workspaces.iter().map(|w| w.id.clone()).collect();
    let first = saved.workspaces[0].id.clone();
    // A tab whose workspace vanished (hand-edited file) falls back to the first workspace.
    let fix = |mut s: SavedSession| {
        if !known.contains(&s.workspace_id) {
            s.workspace_id = first.clone();
        }
        s
    };
    let restored: Vec<SavedSession> = saved.tabs.into_iter().map(fix).collect();
    let mut tabs: Vec<Tab> = restored
        .iter()
        .map(|s| Tab {
            id: s.id.clone(),
            name: s.name.clone(),
            workspace_id: s.workspace_id.clone(),
            pane_ids: Vec::new(),
            layout: s.layout.clone(),
        })
        .collect();
    for ws in &saved.workspaces {
        if !tabs.iter().any(|t| t.workspace_id == ws.id) {
            tabs.push(new_tab("Session 1", &ws.id, None));
        }
    }
    deck.active_tab_id = if tabs.iter().any(|t| t.id == saved.active_tab_id) {
        saved.active_tab_id
    } else {
        tabs[0].id.clone()
    };
    deck.workspaces = saved.workspaces;
    deck.tabs = tabs;
    deck.last_cwd = saved.last_cwd;
    deck.archived = saved.archived.into_iter().map(fix).collect();
    restored
}

/// Live panes the restored sessions contain are adopted; others are orphans to kill.
pub fn plan_restore(saved: &[SavedSession], alive: &[String]) -> (HashSet<String>, Vec<String>) {
    let restored: HashSet<&str> = saved
        .iter()
        .flat_map(|s| s.panes.iter().map(|p| p.id.as_str()))
        .collect();
    let adopt = alive
        .iter()
        .filter(|id| restored.contains(id.as_str()))
        .cloned()
        .collect();
    let orphans = alive
        .iter()
        .filter(|id| !restored.contains(id.as_str()))
        .cloned()
        .collect();
    (adopt, orphans)
}

/// False when only runtime fields (status, lastAction...) changed: then no save is needed.
pub fn specs_changed(next: &HashMap<String, Pane>, prev: &HashMap<String, Pane>) -> bool {
    next.len() != prev.len()
        || next
            .iter()
            .any(|(id, p)| prev.get(id).is_none_or(|old| old.spec != p.spec))
}

/// Loads the saved deck into `deck`. Returns the sessions to respawn (empty when nothing was saved).
pub fn load_deck(dir: &Path, deck: &mut DeckState) -> Result<Vec<SavedSession>, String> {
    let store = JsonStore::load(dir, DECK_FILE)?;
    Ok(migrate(&store.get(DECK_KEY)).map_or_else(Vec::new, |saved| hydrate(deck, saved)))
}

/// Saves the deck snapshot (store.json, key "deck").
pub fn save_deck(dir: &Path, deck: &DeckState, now: i64) -> Result<(), String> {
    let mut store = JsonStore::load(dir, DECK_FILE)?;
    let value = serde_json::to_value(snapshot(deck, now)).map_err(|e| e.to_string())?;
    store.set(DECK_KEY, value);
    store.save()
}

pub fn load_settings(dir: &Path) -> Result<Settings, String> {
    Ok(merge_settings(&JsonStore::load(dir, SETTINGS_FILE)?.get(SETTINGS_KEY)))
}

pub fn save_settings(dir: &Path, settings: &Settings) -> Result<(), String> {
    let mut store = JsonStore::load(dir, SETTINGS_FILE)?;
    store.set(
        SETTINGS_KEY,
        serde_json::to_value(settings).map_err(|e| e.to_string())?,
    );
    store.save()
}

pub fn load_templates(dir: &Path) -> Result<Vec<SessionTemplate>, String> {
    Ok(parse_templates(&JsonStore::load(dir, TEMPLATES_FILE)?.get(TEMPLATES_KEY)))
}

pub fn save_templates(dir: &Path, list: &[SessionTemplate]) -> Result<(), String> {
    let mut store = JsonStore::load(dir, TEMPLATES_FILE)?;
    store.set(
        TEMPLATES_KEY,
        serde_json::to_value(list).map_err(|e| e.to_string())?,
    );
    store.save()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentKind;
    use crate::logic::deck::{PaneSpec, new_pane};
    use serde_json::json;

    fn session() -> Value {
        json!({ "id": "s1", "name": "Session 1", "savedAt": 1, "layout": null, "panes": [] })
    }

    #[test]
    fn migrates_v1() {
        let mut old = session();
        old["id"] = json!("old");
        let saved = migrate(&json!({
            "version": 1, "tabs": [session()], "activeTabId": "s1", "lastCwd": "D:/proj", "archived": [old],
        }))
        .unwrap();
        assert_eq!(saved.version, 2);
        assert_eq!(saved.workspaces[0].name, "Default");
        let ws = &saved.workspaces[0].id;
        assert_eq!(&saved.tabs[0].workspace_id, ws);
        assert_eq!(&saved.archived[0].workspace_id, ws);
        assert_eq!(saved.last_cwd, "D:/proj");
        let no_archive =
            migrate(&json!({ "version": 1, "tabs": [session()], "activeTabId": "s1", "lastCwd": "" }));
        assert!(no_archive.unwrap().archived.is_empty());
        assert!(migrate(&Value::Null).is_none());
        assert!(migrate(&json!({ "version": 1, "tabs": [], "activeTabId": "", "lastCwd": "" })).is_none());
        assert!(migrate(&json!({ "version": 99 })).is_none());
    }

    #[test]
    fn hydrate_repairs() {
        let mut deck = DeckState::default();
        let mut s = session();
        s["workspaceId"] = json!("gone");
        let saved: Saved = serde_json::from_value(json!({
            "version": 2,
            "workspaces": [{ "id": "w1", "name": "A", "path": "/a" }, { "id": "w2", "name": "B", "path": "/b" }],
            "tabs": [s], "activeTabId": "missing", "lastCwd": "/a", "archived": [],
        }))
        .unwrap();
        let to_spawn = hydrate(&mut deck, saved);
        assert_eq!(to_spawn[0].workspace_id, "w1");
        let mut ws: Vec<_> = deck.tabs.iter().map(|t| t.workspace_id.as_str()).collect();
        ws.sort();
        assert_eq!(ws, ["w1", "w2"]);
        assert_eq!(deck.active_tab_id, "s1");
    }

    #[test]
    fn restore_plan() {
        let pane = |id: &str| {
            let mut p = PaneSpec::new(AgentKind::Claude, "t", "/");
            p.id = id.into();
            p
        };
        let mk = |id: &str, panes| SavedSession {
            id: id.into(),
            name: "S".into(),
            workspace_id: "w".into(),
            saved_at: 1,
            layout: None,
            panes,
        };
        let tabs = vec![mk("s1", vec![pane("a"), pane("b")]), mk("s2", vec![pane("c")])];
        let alive = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (adopt, orphans) = plan_restore(&tabs, &alive(&["b", "zombie", "c"]));
        assert!(adopt.contains("b") && adopt.contains("c") && !adopt.contains("a"));
        assert_eq!(orphans, ["zombie"]);
        assert_eq!(plan_restore(&[], &alive(&["x"])).1, ["x"]);
    }

    #[test]
    fn roundtrip_on_disk() {
        let dir = std::env::temp_dir().join(format!("agentdeck-persist-{}", std::process::id()));
        let mut deck = DeckState::default();
        let tab = deck.tabs[0].id.clone();
        deck.add_pane(new_pane(PaneSpec::new(AgentKind::Codex, "x", "/p"), 0), &tab);
        save_deck(&dir, &deck, 5).unwrap();
        let mut back = DeckState::default();
        let restored = load_deck(&dir, &mut back).unwrap();
        assert_eq!(restored[0].panes[0].title, "x");
        assert_eq!(back.active_tab_id, deck.active_tab_id);
        assert!(!specs_changed(&deck.panes, &deck.panes.clone()));
        let mut s = Settings::default();
        s.font_size = 20.0;
        save_settings(&dir, &s).unwrap();
        assert_eq!(load_settings(&dir).unwrap().font_size, 20.0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
