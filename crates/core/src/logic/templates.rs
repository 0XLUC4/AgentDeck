//! Session templates (`src/store/templates.ts`): agents, titles, args, layout and font sizes of a
//! session, reusable in any workspace. Saved in `templates.json` under key "templates".

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::deck::{Layout, Pane, PaneSpec, Tab, new_id};
use super::settings::Settings;
use super::text::split_args;
use crate::agents::AgentKind;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplatePane {
    /// Pane id inside `layout`; replaced by a fresh id on each use.
    pub key: String,
    pub agent: AgentKind,
    pub title: String,
    /// Relative to the workspace folder ("" = the folder itself); None = the workspace / last used folder.
    #[serde(default)]
    pub cwd: Option<String>,
    /// None: the agent's defaults from Settings > Agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTemplate {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub builtin: bool,
    /// None: panes side by side.
    #[serde(default)]
    pub layout: Layout,
    pub panes: Vec<TemplatePane>,
}

fn builtin_pane(key: &str, agent: AgentKind, title: &str) -> TemplatePane {
    TemplatePane {
        key: key.into(),
        agent,
        title: title.into(),
        cwd: None,
        args: None,
        worktree: None,
        font_size: None,
    }
}

pub fn builtin_templates() -> Vec<SessionTemplate> {
    let t = |id: &str, name: &str, panes| SessionTemplate {
        id: id.into(),
        name: name.into(),
        builtin: true,
        layout: None,
        panes,
    };
    use AgentKind::*;
    vec![
        t(
            "builtin:front-back-tests",
            "Front + Back + Tests",
            vec![
                builtin_pane("front", Claude, "Front"),
                builtin_pane("back", Claude, "Back"),
                builtin_pane("tests", Claude, "Tests"),
            ],
        ),
        t(
            "builtin:claude-vs-codex",
            "Claude vs Codex",
            vec![builtin_pane("claude", Claude, "Claude"), builtin_pane("codex", Codex, "Codex")],
        ),
        t("builtin:solo-claude", "Solo Claude", vec![builtin_pane("solo", Claude, "Claude")]),
    ]
}

/// Builtins first, then the user's.
pub fn all_templates(user: &[SessionTemplate]) -> Vec<SessionTemplate> {
    let mut all = builtin_templates();
    all.extend_from_slice(user);
    all
}

fn comparable(path: &str) -> String {
    let mut p = path.replace('\\', "/");
    while p.contains("//") {
        p = p.replace("//", "/");
    }
    p.trim_end_matches('/').to_lowercase()
}

/// `path` relative to `root` with "/" separators ("" for root itself), or None when outside.
pub fn relative_path(path: &str, root: &str) -> Option<String> {
    if root.is_empty() || path.is_empty() {
        return None;
    }
    let (p, r) = (comparable(path), comparable(root));
    if p == r {
        return Some(String::new());
    }
    if !p.starts_with(&format!("{r}/")) {
        return None;
    }
    // Keep the original case of the relative part.
    let mut original = path.replace('\\', "/");
    while original.contains("//") {
        original = original.replace("//", "/");
    }
    Some(original.trim_end_matches('/')[r.len() + 1..].to_owned())
}

/// Joins a relative "/" path to `root` with root's own separator.
pub fn join_path(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        return root.to_owned();
    }
    let sep = if root.contains('\\') { "\\" } else { "/" };
    format!(
        "{}{sep}{}",
        root.trim_end_matches(['\\', '/']),
        rel.split('/').collect::<Vec<_>>().join(sep)
    )
}

/// Template from a live session. Pane ids stay as layout keys.
pub fn session_to_template(
    name: &str,
    tab: &Tab,
    panes: &HashMap<String, Pane>,
    workspace_path: &str,
) -> SessionTemplate {
    SessionTemplate {
        id: new_id(),
        name: name.to_owned(),
        builtin: false,
        layout: tab.layout.clone(),
        panes: tab
            .pane_ids
            .iter()
            .filter_map(|id| panes.get(id))
            .map(|p| TemplatePane {
                key: p.id.clone(),
                agent: p.agent,
                title: p.title.clone(),
                cwd: relative_path(&p.cwd, workspace_path),
                args: Some(p.args.clone()),
                worktree: Some(p.worktree),
                font_size: Some(p.font_size),
            })
            .collect(),
    }
}

/// Fresh pane specs + layout for a new session. `fallback_cwd` is used when the workspace has no folder.
pub fn instantiate(
    template: &SessionTemplate,
    workspace_path: &str,
    fallback_cwd: &str,
    settings: &Settings,
) -> (Layout, Vec<PaneSpec>) {
    let mut layout_json = template.layout.as_ref().map(Value::to_string);
    let panes = template
        .panes
        .iter()
        .map(|p| {
            let id = new_id();
            // Keys are pane UUIDs (or short builtin keys with no layout): a plain replace remaps the layout.
            if let Some(json) = layout_json.as_mut() {
                *json = json.replace(&format!("\"{}\"", p.key), &format!("\"{id}\""));
            }
            let defaults = settings.agent(p.agent);
            let base = if workspace_path.is_empty() { fallback_cwd } else { workspace_path };
            let cwd = match (&p.cwd, workspace_path.is_empty()) {
                (Some(rel), false) => join_path(workspace_path, rel),
                _ => base.to_owned(),
            };
            let mut spec = PaneSpec::new(p.agent, p.title.clone(), cwd);
            spec.id = id;
            spec.args = p.args.clone().unwrap_or_else(|| split_args(&defaults.args));
            spec.worktree = p.worktree.unwrap_or(defaults.worktree);
            spec.font_size = p.font_size.unwrap_or(settings.font_size);
            spec
        })
        .collect();
    let layout = layout_json.and_then(|j| serde_json::from_str(&j).ok());
    (layout, panes)
}

/// Keeps only well-formed user templates read from disk.
pub fn parse_templates(raw: &Value) -> Vec<SessionTemplate> {
    raw.as_array()
        .map(|list| {
            list.iter()
                .filter_map(|t| serde_json::from_value(t.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::deck::new_pane;
    use serde_json::json;

    #[test]
    fn paths() {
        assert_eq!(relative_path("D:\\proj\\web\\app", "D:\\proj").as_deref(), Some("web/app"));
        assert_eq!(relative_path("d:/PROJ/", "D:\\proj").as_deref(), Some(""));
        assert_eq!(relative_path("D:\\project2", "D:\\proj"), None);
        assert_eq!(relative_path("C:\\other", "D:\\proj"), None);
        assert_eq!(relative_path("D:\\proj", ""), None);
        assert_eq!(join_path("D:\\ws\\", "web/app"), "D:\\ws\\web\\app");
        assert_eq!(join_path("/home/me/ws", "api"), "/home/me/ws/api");
        assert_eq!(join_path("D:\\ws", ""), "D:\\ws");
    }

    #[test]
    fn roundtrip_and_instantiate() {
        let mk = |id: &str, cwd: &str, agent| {
            let mut s = PaneSpec::new(agent, id, cwd);
            s.id = id.into();
            s.args = vec!["--x".into()];
            s.worktree = true;
            s.completed_count = 4;
            s.font_size = 15.0;
            new_pane(s, 0)
        };
        let panes: HashMap<_, _> = [
            ("a".to_string(), mk("a", "D:\\proj\\web", AgentKind::Claude)),
            ("b".to_string(), mk("b", "C:\\elsewhere", AgentKind::Codex)),
        ]
        .into();
        let tab = Tab {
            id: "t".into(),
            name: "S".into(),
            workspace_id: "w".into(),
            pane_ids: vec!["a".into(), "b".into(), "gone".into()],
            layout: Some(json!({ "views": ["a", "b"] })),
        };
        let template = session_to_template("My template", &tab, &panes, "D:\\proj");
        assert_eq!(template.panes[0].cwd.as_deref(), Some("web"));
        assert_eq!(template.panes[1].cwd, None);
        assert!(!serde_json::to_string(&template).unwrap().contains("elsewhere"));
        let raw = json!([serde_json::to_value(&template).unwrap(), { "bogus": 1 }]);
        assert_eq!(parse_templates(&raw), vec![template.clone()]);

        let (layout, specs) = instantiate(&template, "E:\\other", "", &Settings::default());
        assert_eq!(specs[0].cwd, "E:\\other\\web");
        assert_eq!(specs[1].cwd, "E:\\other");
        assert!(specs.iter().all(|s| s.id != "a" && s.id != "b" && s.completed_count == 0));
        let out = layout.unwrap().to_string();
        assert!(out.contains(&specs[0].id) && !out.contains("\"a\""));
    }

    #[test]
    fn builtins_use_defaults() {
        let mut settings = Settings::default();
        settings.agents.insert(
            AgentKind::Claude,
            crate::logic::settings::AgentDefaults {
                args: "--model opus".into(),
                worktree: true,
            },
        );
        let (layout, specs) = instantiate(&builtin_templates()[0], "", "D:\\last", &settings);
        assert!(layout.is_none());
        assert_eq!(specs[0].title, "Front");
        assert_eq!(specs[0].cwd, "D:\\last");
        assert_eq!(specs[0].args, ["--model", "opus"]);
        assert!(specs[0].worktree);
    }
}
