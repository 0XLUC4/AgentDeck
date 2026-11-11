//! App preferences (`src/store/settings.ts`), saved to `settings.json` under key "settings".
//! Voice config lives in [`crate::voice`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::deck::{FONT_SIZE_DEFAULT, clamp_font_size};
use super::text::AGENT_KINDS;
use crate::agents::AgentKind;

pub const DEFAULT_FONT_FAMILY: &str = "Cascadia Code, Consolas, monospace";
/// Interface zoom in percent: 80..=150 in steps of 10.
pub const UI_ZOOM_MIN: u32 = 80;
pub const UI_ZOOM_MAX: u32 = 150;
pub const UI_ZOOM_DEFAULT: u32 = 100;
pub const UI_ZOOM_STEP: u32 = 10;

/// Snaps a zoom percent to the nearest step inside the allowed range.
pub fn clamp_ui_zoom(percent: f64) -> u32 {
    let step = f64::from(UI_ZOOM_STEP);
    ((percent / step).round() * step).clamp(f64::from(UI_ZOOM_MIN), f64::from(UI_ZOOM_MAX)) as u32
}

/// Next zoom one step up (`delta` > 0) or down (`delta` < 0); `delta` 0 resets to 100%.
pub fn step_ui_zoom(current: u32, delta: i32) -> u32 {
    match delta.signum() {
        0 => UI_ZOOM_DEFAULT,
        sign => clamp_ui_zoom(f64::from(current) + f64::from(sign) * f64::from(UI_ZOOM_STEP)),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentDefaults {
    pub args: String,
    pub worktree: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifySettings {
    pub enabled: bool,
    pub sound: bool,
    pub done: bool,
    pub waiting: bool,
    pub error: bool,
    /// Only when the window is unfocused or the pane isn't the active one.
    pub only_unseen: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechSettings {
    pub enabled: bool,
    /// Voice id, "" = first voice of the language.
    pub voice: String,
    pub rate: f64,
}

/// Permission auto-approved by AgentDeck. `rule`: "Read", "Bash(pnpm test)" or "Bash(pnpm test *)".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoAllowRule {
    pub rule: String,
    /// Folder the rule applies to (and its subfolders); "" = every folder.
    pub folder: String,
}

/// Cost limits in USD; 0 = off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetSettings {
    pub daily_usd: f64,
    pub pane_usd: f64,
    /// Interrupt the working agents and pause the orchestrator once a limit is hit.
    pub interrupt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Dark,
    Light,
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    pub theme: ThemeMode,
    /// "#rrggbb", or "" for the theme's neutral accent.
    pub accent: String,
    /// "#rrggbb" tinting the app's surfaces, or "" for neutral grey.
    pub background: String,
}

/// Phone page on the local network (`core::remote`): off until the user turns it on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteSettings {
    pub enabled: bool,
    pub port: u16,
    /// Secret of the page's link; made when the access is first turned on.
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub font_size: f32,
    pub font_family: String,
    pub confirm_bulk: bool,
    pub sidebar_open: bool,
    pub activity_open: bool,
    /// Closing the main window hides it in the system tray instead of quitting.
    pub close_to_tray: bool,
    /// Re-arranges a session's panes in a grid whenever one is added or removed.
    pub auto_grid: bool,
    pub notify: NotifySettings,
    pub agents: BTreeMap<AgentKind, AgentDefaults>,
    pub trust_agent_folders: bool,
    pub resume_sessions: bool,
    pub stuck_minutes: u32,
    pub remind_minutes: u32,
    pub base_port: u16,
    /// Shell command run once in each new agent worktree (install deps, copy .env); "" = none.
    #[serde(default)]
    pub worktree_setup: String,
    pub retry_rate_limit_minutes: u32,
    pub speech: SpeechSettings,
    pub auto_allow: Vec<AutoAllowRule>,
    pub budget: BudgetSettings,
    /// Webhook (ntfy topic URL or similar) also receiving the notifications; "" = off.
    pub phone_url: String,
    #[serde(default)]
    pub remote: RemoteSettings,
    pub appearance: Appearance,
    /// Interface zoom in percent (whole UI scale, not the terminals).
    #[serde(default = "default_ui_zoom")]
    pub ui_zoom: u32,
}

fn default_ui_zoom() -> u32 {
    UI_ZOOM_DEFAULT
}

impl Settings {
    pub fn agent(&self, kind: AgentKind) -> AgentDefaults {
        self.agents.get(&kind).cloned().unwrap_or(AgentDefaults {
            args: String::new(),
            worktree: false,
        })
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: FONT_SIZE_DEFAULT,
            font_family: DEFAULT_FONT_FAMILY.into(),
            confirm_bulk: true,
            sidebar_open: true,
            activity_open: false,
            close_to_tray: false,
            auto_grid: true,
            notify: NotifySettings {
                enabled: true,
                sound: true,
                done: true,
                waiting: true,
                error: false,
                only_unseen: true,
            },
            speech: SpeechSettings {
                enabled: false,
                voice: String::new(),
                rate: 1.0,
            },
            trust_agent_folders: true,
            resume_sessions: true,
            stuck_minutes: 10,
            remind_minutes: 5,
            base_port: 0,
            worktree_setup: String::new(),
            retry_rate_limit_minutes: 0,
            auto_allow: Vec::new(),
            budget: BudgetSettings {
                daily_usd: 0.0,
                pane_usd: 0.0,
                interrupt: false,
            },
            phone_url: String::new(),
            remote: RemoteSettings::default(),
            appearance: Appearance {
                theme: ThemeMode::Dark,
                accent: String::new(),
                background: String::new(),
            },
            ui_zoom: UI_ZOOM_DEFAULT,
            agents: AGENT_KINDS
                .into_iter()
                .map(|k| {
                    (
                        k,
                        AgentDefaults {
                            args: String::new(),
                            worktree: false,
                        },
                    )
                })
                .collect(),
        }
    }
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn hex_color(v: &Value) -> String {
    v.as_str()
        .filter(|s| is_hex_color(s))
        .map(str::to_lowercase)
        .unwrap_or_default()
}

fn boolean(v: &Value, fallback: bool) -> bool {
    v.as_bool().unwrap_or(fallback)
}

fn minutes(v: &Value, fallback: u32) -> u32 {
    v.as_u64().filter(|m| *m <= 1440).map_or(fallback, |m| m as u32)
}

fn port(v: &Value) -> u16 {
    v.as_u64()
        .filter(|p| (1024..=65000).contains(p))
        .map_or(0, |p| p as u16)
}

fn usd(v: &Value) -> f64 {
    v.as_f64().filter(|x| x.is_finite() && *x > 0.0).unwrap_or(0.0)
}

fn string(v: &Value) -> String {
    v.as_str().unwrap_or("").to_owned()
}

/// Builds valid settings from anything read from disk: unknown or invalid fields fall back to defaults.
pub fn merge_settings(raw: &Value) -> Settings {
    let d = Settings::default();
    if !raw.is_object() {
        return d;
    }
    let n = &raw["notify"];
    let sp = &raw["speech"];
    let b = &raw["budget"];
    let ap = &raw["appearance"];
    let font_family = raw["fontFamily"].as_str().unwrap_or("").trim();
    Settings {
        font_size: raw["fontSize"]
            .as_f64()
            .map_or(d.font_size, |f| clamp_font_size(f as f32)),
        font_family: if font_family.is_empty() {
            d.font_family.clone()
        } else {
            font_family.to_owned()
        },
        confirm_bulk: boolean(&raw["confirmBulk"], d.confirm_bulk),
        sidebar_open: boolean(&raw["sidebarOpen"], d.sidebar_open),
        activity_open: boolean(&raw["activityOpen"], d.activity_open),
        close_to_tray: boolean(&raw["closeToTray"], d.close_to_tray),
        auto_grid: boolean(&raw["autoGrid"], d.auto_grid),
        trust_agent_folders: boolean(&raw["trustAgentFolders"], d.trust_agent_folders),
        resume_sessions: boolean(&raw["resumeSessions"], d.resume_sessions),
        stuck_minutes: minutes(&raw["stuckMinutes"], d.stuck_minutes),
        remind_minutes: minutes(&raw["remindMinutes"], d.remind_minutes),
        base_port: port(&raw["basePort"]),
        worktree_setup: raw["worktreeSetup"].as_str().unwrap_or("").trim().to_owned(),
        retry_rate_limit_minutes: minutes(&raw["retryRateLimitMinutes"], d.retry_rate_limit_minutes),
        notify: NotifySettings {
            enabled: boolean(&n["enabled"], d.notify.enabled),
            sound: boolean(&n["sound"], d.notify.sound),
            done: boolean(&n["done"], d.notify.done),
            waiting: boolean(&n["waiting"], d.notify.waiting),
            error: boolean(&n["error"], d.notify.error),
            only_unseen: boolean(&n["onlyUnseen"], d.notify.only_unseen),
        },
        speech: SpeechSettings {
            enabled: boolean(&sp["enabled"], d.speech.enabled),
            voice: sp["voice"].as_str().map_or(d.speech.voice.clone(), str::to_owned),
            rate: sp["rate"]
                .as_f64()
                .filter(|r| (0.5..=2.0).contains(r))
                .unwrap_or(d.speech.rate),
        },
        auto_allow: raw["autoAllow"]
            .as_array()
            .map(|rules| {
                rules
                    .iter()
                    .filter_map(|r| {
                        let rule = r["rule"].as_str()?.trim();
                        (!rule.is_empty()).then(|| AutoAllowRule {
                            rule: rule.to_owned(),
                            folder: string(&r["folder"]),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        budget: BudgetSettings {
            daily_usd: usd(&b["dailyUsd"]),
            pane_usd: usd(&b["paneUsd"]),
            interrupt: boolean(&b["interrupt"], d.budget.interrupt),
        },
        phone_url: raw["phoneUrl"].as_str().unwrap_or("").trim().to_owned(),
        remote: serde_json::from_value(raw["remote"].clone()).unwrap_or_default(),
        appearance: Appearance {
            theme: serde_json::from_value(ap["theme"].clone()).unwrap_or(d.appearance.theme),
            accent: hex_color(&ap["accent"]),
            background: hex_color(&ap["background"]),
        },
        ui_zoom: raw["uiZoom"].as_f64().filter(|z| z.is_finite()).map_or(d.ui_zoom, clamp_ui_zoom),
        agents: AGENT_KINDS
            .into_iter()
            .map(|k| {
                let key = serde_json::to_value(k).ok();
                let a = key
                    .as_ref()
                    .and_then(Value::as_str)
                    .map_or(&Value::Null, |name| &raw["agents"][name]);
                (
                    k,
                    AgentDefaults {
                        args: string(&a["args"]),
                        worktree: boolean(&a["worktree"], false),
                    },
                )
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_for_garbage() {
        assert_eq!(merge_settings(&Value::Null), Settings::default());
        assert_eq!(merge_settings(&json!("nope")), Settings::default());
        assert_eq!(merge_settings(&json!({ "notify": 3, "agents": [] })), Settings::default());
    }

    #[test]
    fn field_by_field() {
        let s = merge_settings(&json!({
            "fontSize": 40, "fontFamily": "  ", "confirmBulk": false, "sidebarOpen": "yes",
            "notify": { "sound": false, "error": true, "enabled": 1 },
            "agents": { "claude": { "args": "--model opus", "worktree": true }, "codex": { "args": 5 } },
        }));
        let d = Settings::default();
        assert_eq!(s.font_size, 32.0);
        assert_eq!(s.font_family, d.font_family);
        assert!(!s.confirm_bulk);
        assert_eq!(s.sidebar_open, d.sidebar_open);
        assert!(!s.notify.sound && s.notify.error && s.notify.enabled);
        assert_eq!(s.agent(AgentKind::Claude).args, "--model opus");
        assert!(s.agent(AgentKind::Claude).worktree);
        assert_eq!(s.agent(AgentKind::Codex).args, "");
        assert_eq!(s.agents.len(), 6);
    }

    #[test]
    fn rules_budget_phone_appearance() {
        let s = merge_settings(&json!({
            "autoAllow": [{ "rule": " Bash(ls) ", "folder": "D:\\p" }, { "rule": "" }, "Read", { "rule": "Edit", "folder": 3 }],
            "budget": { "dailyUsd": 5, "paneUsd": -1, "interrupt": "yes" },
            "phoneUrl": " https://ntfy.sh/t ", "worktreeSetup": " pnpm install ",
            "appearance": { "theme": "light", "accent": "#4F8FF7", "background": "#1E2A44" },
        }));
        assert_eq!(
            s.auto_allow,
            vec![
                AutoAllowRule { rule: "Bash(ls)".into(), folder: "D:\\p".into() },
                AutoAllowRule { rule: "Edit".into(), folder: String::new() },
            ]
        );
        assert_eq!(s.budget, BudgetSettings { daily_usd: 5.0, pane_usd: 0.0, interrupt: false });
        assert_eq!(s.phone_url, "https://ntfy.sh/t");
        assert_eq!(s.worktree_setup, "pnpm install");
        assert_eq!(s.appearance.theme, ThemeMode::Light);
        assert_eq!(s.appearance.accent, "#4f8ff7");
        let bad = merge_settings(&json!({ "appearance": { "theme": "neon", "accent": "red", "background": 3 } }));
        assert_eq!(bad.appearance, Settings::default().appearance);
    }

    #[test]
    fn font_size_rounding() {
        assert_eq!(merge_settings(&json!({ "fontSize": 2 })).font_size, 8.0);
        assert_eq!(merge_settings(&json!({ "fontSize": 14.6 })).font_size, 15.0);
    }

    #[test]
    fn ui_zoom_clamps_and_steps() {
        assert_eq!(merge_settings(&json!({})).ui_zoom, 100);
        assert_eq!(merge_settings(&json!({ "uiZoom": 500 })).ui_zoom, 150);
        assert_eq!(merge_settings(&json!({ "uiZoom": 114 })).ui_zoom, 110);
        assert_eq!(merge_settings(&json!({ "uiZoom": "big" })).ui_zoom, 100);
        assert_eq!(step_ui_zoom(100, 1), 110);
        assert_eq!(step_ui_zoom(150, 1), 150);
        assert_eq!(step_ui_zoom(80, -1), 80);
        assert_eq!(step_ui_zoom(130, 0), 100);
    }

    #[test]
    fn roundtrips_through_json() {
        let s = Settings::default();
        let back = merge_settings(&serde_json::to_value(&s).unwrap());
        assert_eq!(back, s);
    }
}
