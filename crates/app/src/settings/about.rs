//! Settings > About: version, updates (shared `system::Updater`), data folder and links.

use std::path::PathBuf;

use agentdeck_core::logic::theme::FONT_MONO;
use gpui_kit::assets::IconName;
use gpui_kit::*;

use super::SettingsDialog;
use super::controls::{button, error_text, row, section};
use crate::system::{UpdatePhase, updater};
use crate::theme::{hsla, palette};

const LINKS: [(&str, &str); 4] = [
    ("Claude Code", "https://docs.anthropic.com/en/docs/claude-code"),
    ("Codex CLI", "https://github.com/openai/codex"),
    ("Gemini CLI", "https://github.com/google-gemini/gemini-cli"),
    ("GPUI", "https://www.gpui.rs"),
];

pub(super) struct About {
    data_dir: PathBuf,
    /// "Check for updates" was pressed here: only then "up to date" is shown.
    checked_here: bool,
}

impl About {
    pub fn new(data_dir: PathBuf, cx: &mut Context<SettingsDialog>, subs: &mut Vec<Subscription>) -> Self {
        if let Some(u) = updater(cx) {
            subs.push(cx.observe(&u, |_, _, cx| cx.notify()));
        }
        Self {
            data_dir,
            checked_here: false,
        }
    }
}

impl SettingsDialog {
    /// Check -> show the new version -> "Install and restart".
    fn update_row(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = palette(cx);
        let Some(entity) = updater(cx) else {
            return vec![row("Updates", Some("Unavailable"), div(), &p)];
        };
        let u = entity.read(cx);
        let busy = u.busy() || u.phase == UpdatePhase::Checking;
        let status = match (u.phase, &u.update) {
            (UpdatePhase::Checking, _) => Some("Checking\u{2026}".to_owned()),
            (UpdatePhase::Downloading | UpdatePhase::Installing, _) => Some("Downloading and installing\u{2026}".to_owned()),
            (_, Some(up)) => Some(format!("Version {} available", up.version)),
            (UpdatePhase::Idle, None) if self.about.checked_here => Some("AgentDeck is up to date".to_owned()),
            _ => None,
        };
        let has_update = u.update.is_some();
        let error = u.error.clone();
        let (name, label) = if has_update {
            (IconName::Download, "Install and restart")
        } else {
            (IconName::RefreshCw, "Check for updates")
        };
        let mut action = button("update-action", Some(name), label, busy, &p);
        if !busy {
            action = action.on_click(cx.listener(move |this, _, _, cx| {
                this.about.checked_here = true;
                entity.update(cx, |u, cx| if has_update { u.install(cx) } else { u.check(false, cx) });
            }));
        }
        let mut rows = vec![row("Updates", status.as_deref(), action, &p)];
        if let Some(e) = error {
            rows.push(error_text(&format!("Update failed: {e}"), &p).into_any_element());
        }
        rows
    }

    pub(super) fn render_about(&mut self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let data_dir = self.about.data_dir.clone();
        let mut rows = vec![row(
            "Version",
            None,
            div()
                .font_family(FONT_MONO)
                .text_xs()
                .text_color(hsla(p.muted))
                .child(env!("CARGO_PKG_VERSION")),
            &p,
        )];
        rows.extend(self.update_row(cx));
        rows.push(row(
            "Data folder",
            Some(&data_dir.display().to_string()),
            button("data-open", Some(IconName::FolderOpen), "Open", false, &p)
                .on_click(move |_, _, cx| cx.reveal_path(&data_dir)),
            &p,
        ));
        rows.push(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .py_3()
                .children(LINKS.iter().map(|&(label, url)| {
                    button(label, None, label, false, &p)
                        .child(super::controls::icon(IconName::ExternalLink, 13.0, p.muted))
                        .on_click(move |_, _, cx| cx.open_url(url))
                }))
                .into_any_element(),
        );
        section("About", rows, &p)
    }
}
