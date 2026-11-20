//! Settings > Hosts: SSH machines agents can run on (add, edit, remove, test, browse files).
//! No password is ever stored: ssh uses the user's keys, ssh-agent and ~/.ssh/config.

use std::collections::{HashMap, HashSet};

use agentdeck_core::hosts::Host;
use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt;
use gpui_kit::component::input::InputState;
use gpui_kit::*;

use super::controls::{button, confirm, field, hint_text, icon_button, row, section};
use super::{SettingsDialog, input};
use crate::theme::{hsla, palette, ui};

pub(super) struct HostsUi {
    /// Host being edited: None = the form adds a new one.
    editing: Option<String>,
    name: Entity<InputState>,
    target: Entity<InputState>,
    port: Entity<InputState>,
    identity: Entity<InputState>,
    folder: Entity<InputState>,
    testing: HashSet<String>,
    /// Last test answer per host id ("Reachable" or the ssh error).
    results: HashMap<String, String>,
}

impl HostsUi {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsDialog>) -> Self {
        Self {
            editing: None,
            name: input(window, cx, "My server", ""),
            target: input(window, cx, "user@host", ""),
            port: input(window, cx, "22", ""),
            identity: input(window, cx, "Optional: C:\\Users\\me\\.ssh\\id_ed25519", ""),
            folder: input(window, cx, "Optional: ~/projects/app", ""),
            testing: HashSet::new(),
            results: HashMap::new(),
        }
    }
}

impl SettingsDialog {
    /// The form as a host (`id` kept when editing), validated.
    fn host_from_form(&self, cx: &App) -> Result<Host, String> {
        let h = &self.hosts;
        let text = |state: &Entity<InputState>| state.read(cx).value().trim().to_owned();
        let port = text(&h.port);
        let identity = text(&h.identity);
        let mut host = Host::new(&text(&h.name), &text(&h.target));
        if let Some(id) = &h.editing {
            host.id = id.clone();
        }
        host.port = if port.is_empty() { None } else { Some(port.parse().map_err(|_| "Port: 1 to 65535.".to_owned())?) };
        host.identity = (!identity.is_empty()).then_some(identity);
        host.folder = text(&h.folder);
        host.validate()?;
        Ok(host)
    }

    fn fill_form(&mut self, host: Option<&Host>, window: &mut Window, cx: &mut Context<Self>) {
        self.hosts.editing = host.map(|h| h.id.clone());
        let values = match host {
            Some(h) => [
                h.name.clone(),
                h.target.clone(),
                h.port.map(|p| p.to_string()).unwrap_or_default(),
                h.identity.clone().unwrap_or_default(),
                h.folder.clone(),
            ],
            None => Default::default(),
        };
        let h = &self.hosts;
        for (state, value) in [&h.name, &h.target, &h.port, &h.identity, &h.folder].into_iter().zip(values) {
            state.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        cx.notify();
    }

    fn save_host(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let host = match self.host_from_form(cx) {
            Ok(host) => host,
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                return;
            }
        };
        let mut list = crate::hosts::list(cx);
        match list.iter_mut().find(|h| h.id == host.id) {
            Some(slot) => *slot = host,
            None => list.push(host),
        }
        let result = crate::hosts::save(list, cx);
        self.report(result, cx);
        self.fill_form(None, window, cx);
    }

    fn delete_host(&mut self, host: &Host, window: &mut Window, cx: &mut Context<Self>) {
        if !confirm(&format!("Remove host \"{}\"? Panes on it keep running as plain ssh panes.", host.name)) {
            return;
        }
        let list = crate::hosts::list(cx).into_iter().filter(|h| h.id != host.id).collect();
        let result = crate::hosts::save(list, cx);
        self.report(result, cx);
        if self.hosts.editing.as_deref() == Some(host.id.as_str()) {
            self.fill_form(None, window, cx);
        }
    }

    /// `ssh -o BatchMode=yes -o ConnectTimeout=5 <target> echo ok`, off the UI thread.
    fn test_host(&mut self, host: Host, window: &Window, cx: &mut Context<Self>) {
        if !self.hosts.testing.insert(host.id.clone()) {
            return;
        }
        let id = host.id.clone();
        self.run(window, cx, move || Ok(host.test_connection()), move |this, result, _, cx| {
            let ok = matches!(result, Ok(Ok(())));
            let text = match result {
                Ok(Ok(())) => "Reachable".to_owned(),
                Ok(Err(e)) | Err(e) => e,
            };
            this.hosts.testing.remove(&id);
            this.hosts.results.insert(id.clone(), text);
            crate::hosts::set_reachable(&id, ok, cx);
            cx.notify();
        });
        cx.notify();
    }

    fn host_row(&self, host: &Host, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let color = match crate::hosts::reachable(&host.id, cx) {
            Some(true) => p.ok,
            Some(false) => p.bad,
            None => p.subtle,
        };
        let testing = self.hosts.testing.contains(&host.id);
        let mut detail = host.target.clone();
        if let Some(port) = host.port {
            detail.push_str(&format!(" -p {port}"));
        }
        if !host.folder.is_empty() {
            detail.push_str(&format!(" · {}", host.folder));
        }
        let result = self.hosts.results.get(&host.id).cloned();
        let (test, files, edit, delete) = (host.clone(), host.clone(), host.clone(), host.clone());
        div()
            .flex()
            .items_center()
            .gap_3()
            .py_2()
            .child(div().size(ui(8.0)).flex_shrink_0().rounded_full().bg(hsla(color)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().text_color(hsla(p.fg)).child(host.name.clone()))
                    .child(hint_text(&detail, &p).truncate())
                    .children(result.map(|r| hint_text(&r, &p).truncate())),
            )
            .child({
                let b = button(("host-test", i), Some(IconName::PlugZap), if testing { "Testing…" } else { "Test" }, testing, &p);
                if testing { b } else { b.on_click(cx.listener(move |this, _, window, cx| this.test_host(test.clone(), window, cx))) }
            })
            .child(button(("host-files", i), Some(IconName::FolderOpen), "Files", false, &p).on_click(move |_, window, cx| {
                // One dialog at a time: Settings closes, the files dialog opens on the next tick.
                window.close_dialog(cx);
                let (host, handle) = (files.clone(), window.window_handle());
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| crate::hosts::open_files(host, window, cx));
                });
            }))
            .child(icon_button(("host-edit", i), IconName::Pencil, false, &p).on_click(cx.listener(
                move |this, _, window, cx| this.fill_form(Some(&edit), window, cx),
            )))
            .child(icon_button(("host-delete", i), IconName::Trash, true, &p).on_click(cx.listener(
                move |this, _, window, cx| this.delete_host(&delete, window, cx),
            )))
            .into_any_element()
    }

    pub(super) fn render_hosts(&mut self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let list = crate::hosts::list(cx);
        let mut rows = vec![
            hint_text(
                "Run agents on other machines with your system ssh (OpenSSH). Keys come from ssh-agent or the identity \
                 file; passwords are never stored. Agents run in tmux there: they keep working while this computer \
                 sleeps, and restarting the pane reattaches.",
                &p,
            )
            .pb_2()
            .into_any_element(),
        ];
        rows.extend(list.iter().enumerate().map(|(i, h)| self.host_row(h, i, cx)));
        if list.is_empty() {
            rows.push(hint_text("No host yet.", &p).py_3().into_any_element());
        }
        let h = &self.hosts;
        let adding = h.editing.is_none();
        let form = vec![
            row("Name", None, field(&h.name, Some(320.0), false), &p),
            row("Target", Some("user@host, or a Host alias of ~/.ssh/config"), field(&h.target, Some(320.0), true), &p),
            row("Port", Some("Empty: 22 or the ssh config"), field(&h.port, Some(120.0), true), &p),
            row("Identity file", Some("Private key (ssh -i)"), field(&h.identity, Some(320.0), true), &p),
            row("Default folder", Some("Where agents start; empty: the remote home"), field(&h.folder, Some(320.0), true), &p),
            div()
                .flex()
                .justify_end()
                .gap_2()
                .py_3()
                .children((!adding).then(|| {
                    button("host-cancel", None, "Cancel", false, &p)
                        .on_click(cx.listener(|this, _, window, cx| this.fill_form(None, window, cx)))
                }))
                .child(
                    button("host-save", Some(if adding { IconName::Plus } else { IconName::Check }), if adding { "Add host" } else { "Save" }, false, &p)
                        .on_click(cx.listener(|this, _, window, cx| this.save_host(window, cx))),
                )
                .into_any_element(),
        ];
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(section("SSH hosts", rows, &p))
            .child(section(if adding { "Add a host" } else { "Edit host" }, form, &p))
    }
}
