//! Settings rows backed by the OS rather than settings.json: autostart and updates
//! (original: components/Settings/AppControls.tsx). Embed with `cx.new(AppControls::new)`.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::updater::{UpdatePhase, updater};
use super::{autostart_enabled, set_autostart};
use crate::theme::{hsla, palette, ui};

pub struct AppControls {
    autostart: bool,
    autostart_error: Option<String>,
    /// The user pressed "Check for updates" here (the status line only shows after that).
    checked_here: bool,
}

impl AppControls {
    pub fn new(cx: &mut Context<Self>) -> Self {
        if let Some(u) = updater(cx) {
            cx.observe(&u, |_, _, cx| cx.notify()).detach();
        }
        let read = cx.background_spawn(async { autostart_enabled() });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(on) => this.autostart = on,
                    Err(e) => this.autostart_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        Self { autostart: false, autostart_error: None, checked_here: false }
    }

    fn toggle_autostart(&mut self, cx: &mut Context<Self>) {
        let next = !self.autostart;
        let job = cx.background_spawn(async move { set_autostart(next) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.autostart = next;
                        this.autostart_error = None;
                    }
                    Err(e) => this.autostart_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for AppControls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let on = self.autostart;
        let toggle = div()
            .id("set-autostart")
            .w(ui(32.0))
            .h(ui(18.0))
            .p(ui(2.0))
            .flex()
            .rounded_full()
            .bg(hsla(if on { p.accent } else { p.line_strong }))
            .when(on, |d| d.justify_end())
            .child(div().size(ui(14.0)).rounded_full().bg(hsla(if on { p.on_accent } else { p.fg })))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_autostart(cx)));
        let error_text = |text: String| div().pb_2().text_xs().text_color(hsla(p.bad)).child(text);

        let mut root = div()
            .flex()
            .flex_col()
            .text_size(ui(13.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_6()
                    .py_3()
                    .child(div().text_color(hsla(p.fg)).child("Launch AgentDeck when Windows starts"))
                    .child(toggle),
            );
        if let Some(e) = &self.autostart_error {
            root = root.child(error_text(format!("Launch at startup unavailable: {e}")));
        }

        let Some(entity) = updater(cx) else {
            return root;
        };
        let u = entity.read(cx);
        let busy = u.busy() || u.phase == UpdatePhase::Checking;
        let status = match u.phase {
            UpdatePhase::Checking => Some("Checking…".to_owned()),
            UpdatePhase::Downloading | UpdatePhase::Installing => Some("Downloading and installing…".to_owned()),
            _ => match &u.update {
                Some(up) => Some(format!("Version {} available", up.version)),
                None if self.checked_here && u.phase == UpdatePhase::Idle => Some("AgentDeck is up to date".into()),
                None => None,
            },
        };
        let (icon, label) = if u.update.is_some() {
            (IconName::Download, "Install and restart")
        } else {
            (IconName::RefreshCw, "Check for updates")
        };
        let has_update = u.update.is_some();
        let error = u.error.clone();
        let action = entity.clone();
        let button = div()
            .id("update-action")
            .flex()
            .items_center()
            .gap_1p5()
            .h(ui(32.0))
            .px_3()
            .rounded(ui(6.0))
            .border_1()
            .border_color(hsla(p.line_strong))
            .text_color(hsla(p.fg))
            .when(busy, |d| d.opacity(0.5))
            .when(!busy, |d| d.hover(|s| s.bg(hsla(p.hover))))
            .child(icon)
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                if busy {
                    return;
                }
                this.checked_here = true;
                action.update(cx, |u, cx| if has_update { u.install(cx) } else { u.check(false, cx) });
            }));
        root = root.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_6()
                .py_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .min_w_0()
                        .child(div().text_color(hsla(p.fg)).child("Updates"))
                        .children(status.map(|s| div().text_xs().text_color(hsla(p.subtle)).child(s))),
                )
                .child(button),
        );
        if let Some(e) = error {
            root = root.child(error_text(format!("Update failed: {e}")));
        }
        root
    }
}
