//! Settings > MCP hub: shared MCP servers, one process run by AgentDeck and used by every agent pane.

use std::collections::HashMap;

use agentdeck_core::logic::text::split_args;
use agentdeck_core::logic::theme::FONT_MONO;
use agentdeck_core::mcp::hub::{HubServer, hub_claude_servers, hub_list, hub_save, hub_test};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use super::SettingsDialog;
use super::controls::{button, error_text, field, hint_text, section};
use crate::theme::{hsla, palette, ui};

pub(super) struct Hub {
    servers: Vec<HubServer>,
    error: Option<String>,
    /// Last test result per server name.
    tests: HashMap<String, String>,
    name: Entity<InputState>,
    line: Entity<InputState>,
}

/// What a row shows: the URL, or the command line.
fn target(s: &HubServer) -> String {
    if s.url.is_empty() {
        std::iter::once(s.command.as_str()).chain(s.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ")
    } else {
        s.url.clone()
    }
}

/// A new server from the form: a URL (streamable HTTP) or a command line (stdio).
fn server_from(name: &str, line: &str) -> HubServer {
    let text = line.trim();
    let is_url = text.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://"))
        || text.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://"));
    let mut server = HubServer {
        name: name.trim().to_owned(),
        ..HubServer::default()
    };
    if is_url {
        server.url = text.to_owned();
    } else {
        let mut parts = split_args(text).into_iter();
        server.command = parts.next().unwrap_or_default();
        server.args = parts.collect();
    }
    server
}

impl Hub {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsDialog>, subs: &mut Vec<Subscription>) -> Self {
        let name = super::input(window, cx, "Name (e.g. roblox-studio-plus)", "");
        let line = super::input(window, cx, "node D:/server/index.js  or  https://\u{2026}/mcp", "");
        for state in [&name, &line] {
            subs.push(cx.subscribe_in(state, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => cx.notify(), // enables "Add"
                InputEvent::PressEnter { .. } => this.add_server(window, cx),
                _ => {}
            }));
        }
        Self {
            servers: hub_list(),
            error: None,
            tests: HashMap::new(),
            name,
            line,
        }
    }
}

impl SettingsDialog {
    /// Saves the list (restarts the hub servers in the backend), then shows it.
    fn persist_hub(&mut self, next: Vec<HubServer>, window: &mut Window, cx: &mut Context<Self>) {
        let saved = next.clone();
        self.run(window, cx, move || hub_save(saved), move |this, result, _, cx| {
            match &result {
                Ok(()) => {
                    this.hub.error = None;
                    this.hub.servers = next;
                }
                Err(e) => this.hub.error = Some(e.clone()),
            }
            this.report(result, cx);
        });
    }

    fn add_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.hub.name.read(cx).value().to_string();
        let line = self.hub.line.read(cx).value().to_string();
        if name.trim().is_empty() || line.trim().is_empty() {
            return;
        }
        let mut next = self.hub.servers.clone();
        next.push(server_from(&name, &line));
        self.persist_hub(next, window, cx);
        self.hub.name.update(cx, |s, cx| s.set_value("", window, cx));
        self.hub.line.update(cx, |s, cx| s.set_value("", window, cx));
    }

    fn import_claude(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(window, cx, || Ok(hub_claude_servers()), |this, found: Result<Vec<HubServer>, String>, window, cx| {
            let found = found.unwrap_or_default();
            let fresh: Vec<HubServer> = found
                .into_iter()
                .filter(|f| !this.hub.servers.iter().any(|s| s.name == f.name))
                .collect();
            if fresh.is_empty() {
                this.hub.error = Some("No new server in ~/.claude.json.".into());
                cx.notify();
                return;
            }
            let mut next = this.hub.servers.clone();
            next.extend(fresh);
            this.persist_hub(next, window, cx);
        });
    }

    fn test_server(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.hub.tests.insert(name.clone(), "Starting\u{2026}".into());
        cx.notify();
        let server = name.clone();
        self.run(window, cx, move || hub_test(server), move |this, result, _, cx| {
            let text = match result {
                Ok(count) => format!("Works: {count} tools"),
                Err(e) => format!("Failed: {e}"),
            };
            this.hub.tests.insert(name, text);
            cx.notify();
        });
    }

    pub(super) fn render_mcp(&mut self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let h = &self.hub;
        let mut rows = vec![
            hint_text(
                "AgentDeck runs one copy of each server below and every agent (Claude, Codex, Grok) goes through it, \
                 under the same name. Use it for servers that accept a single client, like Roblox Studio: agents no \
                 longer disconnect each other, and tool calls run one at a time. Applies to agents started after a change.",
                &p,
            )
            .py_3()
            .into_any_element(),
        ];
        if let Some(e) = &h.error {
            rows.push(error_text(e, &p).into_any_element());
        }
        for (i, s) in h.servers.iter().enumerate() {
            let test = h.tests.get(&s.name).cloned();
            let (test_name, remove_name) = (s.name.clone(), s.name.clone());
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(ui(2.0))
                            .child(div().truncate().text_color(hsla(p.fg)).child(s.name.clone()))
                            .child(
                                div()
                                    .truncate()
                                    .font_family(FONT_MONO)
                                    .text_xs()
                                    .text_color(hsla(p.subtle))
                                    .child(target(s)),
                            )
                            .children(test.map(|t| {
                                let failed = t.starts_with("Failed");
                                div().text_xs().text_color(hsla(if failed { p.bad } else { p.muted })).child(t)
                            })),
                    )
                    .child(button(("hub-test", i), Some(IconName::Play), "Test", false, &p).on_click(cx.listener(
                        move |this, _, window, cx| this.test_server(test_name.clone(), window, cx),
                    )))
                    .child(button(("hub-remove", i), Some(IconName::Trash), "", false, &p).on_click(cx.listener(
                        move |this, _, window, cx| {
                            let next = this.hub.servers.iter().filter(|o| o.name != remove_name).cloned().collect();
                            this.persist_hub(next, window, cx);
                        },
                    )))
                    .into_any_element(),
            );
        }
        let can_add = !h.name.read(cx).value().trim().is_empty() && !h.line.read(cx).value().trim().is_empty();
        let mut add = button("hub-add", Some(IconName::Plus), "Add", !can_add, &p);
        if can_add {
            add = add.on_click(cx.listener(|this, _, window, cx| this.add_server(window, cx)));
        }
        rows.push(
            div()
                .flex()
                .items_center()
                .gap_2()
                .py_3()
                .child(field(&h.name, Some(192.0), false))
                .child(field(&h.line, None, true))
                .child(add)
                .into_any_element(),
        );
        rows.push(
            div()
                .py_3()
                .child(
                    button("hub-import", Some(IconName::Download), "Import from Claude Code", false, &p)
                        .on_click(cx.listener(|this, _, window, cx| this.import_claude(window, cx))),
                )
                .into_any_element(),
        );
        section("MCP hub", rows, &p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn form_line_to_server() {
        let s = server_from(" roblox ", "node D:/server/index.js --port 3");
        assert_eq!(s.name, "roblox");
        assert_eq!(s.command, "node");
        assert_eq!(s.args, ["D:/server/index.js", "--port", "3"]);
        assert_eq!(target(&s), "node D:/server/index.js --port 3");
        let u = server_from("web", "HTTPS://example.com/mcp");
        assert_eq!((u.url.as_str(), u.command.as_str()), ("HTTPS://example.com/mcp", ""));
        assert_eq!(target(&u), "HTTPS://example.com/mcp");
    }
}
