//! Settings > Templates: rename or delete saved session templates (builtins are read-only).

use agentdeck_core::logic::templates::{SessionTemplate, builtin_templates};
use agentdeck_core::persist;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use super::SettingsDialog;
use super::controls::{confirm, field, hint_text, icon_button, section};
use crate::theme::{hsla, palette, ui};

pub(super) struct Templates {
    /// Id of the template being renamed (Esc cancels it, see `SettingsDialog::intercept_key`).
    pub renaming: Option<String>,
    name: Entity<InputState>,
}

impl Templates {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsDialog>, subs: &mut Vec<Subscription>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx));
        subs.push(cx.subscribe(&name, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                this.commit_rename(cx);
            }
        }));
        Self { renaming: None, name }
    }
}

impl SettingsDialog {
    /// Edits the saved templates and writes templates.json.
    fn edit_templates(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut Vec<SessionTemplate>)) {
        let result = self.deck.update(cx, |deck, cx| {
            edit(&mut deck.templates);
            cx.notify();
            persist::save_templates(&deck.data_dir, &deck.templates)
        });
        self.report(result, cx);
    }

    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.templates.renaming.take() else {
            return;
        };
        let name = self.templates.name.read(cx).value().trim().to_owned();
        let unchanged = self.deck.read(cx).templates.iter().any(|t| t.id == id && t.name == name);
        if !name.is_empty() && !unchanged {
            self.edit_templates(cx, |list| {
                if let Some(t) = list.iter_mut().find(|t| t.id == id) {
                    t.name = name;
                }
            });
        }
        cx.notify();
    }

    fn template_row(&self, t: &SessionTemplate, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let renaming = self.templates.renaming.as_deref() == Some(t.id.as_str());
        let name: AnyElement = if renaming {
            div().flex_1().flex().child(field(&self.templates.name, None, false)).into_any_element()
        } else {
            div().flex_1().min_w_0().truncate().text_color(hsla(p.fg)).child(t.name.clone()).into_any_element()
        };
        let agents = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            // shortcut: color dots instead of the panes module's AgentLogo.
            .children(t.panes.iter().map(|pane| div().size(ui(9.0)).rounded_full().bg(hsla(p.agent(pane.agent)))));
        let actions: AnyElement = if t.builtin {
            div()
                .w(ui(52.0))
                .flex_shrink_0()
                .text_right()
                .text_xs()
                .text_color(hsla(p.subtle))
                .child("built-in")
                .into_any_element()
        } else {
            let (rename_id, rename_name) = (t.id.clone(), t.name.clone());
            let (delete_id, delete_name) = (t.id.clone(), t.name.clone());
            div()
                .flex()
                .flex_shrink_0()
                .child(icon_button(("template-rename", i), IconName::Pencil, false, &p).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.templates.renaming = Some(rename_id.clone());
                        let name = rename_name.clone();
                        this.templates.name.update(cx, |s, cx| {
                            s.set_value(name, window, cx);
                            let focus = s.focus_handle(cx);
                            window.focus(&focus, cx);
                        });
                        cx.notify();
                    },
                )))
                .child(icon_button(("template-delete", i), IconName::Trash, true, &p).on_click(cx.listener(
                    move |this, _, _, cx| {
                        if confirm(&format!("Delete template \"{delete_name}\"?")) {
                            let id = delete_id.clone();
                            this.edit_templates(cx, |list| list.retain(|t| t.id != id));
                        }
                    },
                )))
                .into_any_element()
        };
        div().flex().h_12().items_center().gap_3().child(name).child(agents).child(actions).into_any_element()
    }

    pub(super) fn render_templates(&mut self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let list: Vec<SessionTemplate> = builtin_templates().into_iter().chain(self.deck.read(cx).templates.clone()).collect();
        let mut rows = vec![
            hint_text(
                "Save a session from \"New session from template\" (sidebar). Folders are stored relative to the workspace.",
                &p,
            )
            .pb_2()
            .into_any_element(),
        ];
        rows.extend(list.iter().enumerate().map(|(i, t)| self.template_row(t, i, cx)));
        section("Session templates", rows, &p)
    }
}
