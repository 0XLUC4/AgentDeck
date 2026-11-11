//! "New session from template" + "Save session as template" (components/Templates/TemplatePicker.tsx),
//! plus the template store actions used by Settings > Templates (store/templates.ts).

use agentdeck_core::logic::deck::new_tab;
use agentdeck_core::logic::templates::{all_templates, instantiate, session_to_template};
use agentdeck_core::persist;
use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::ui;
use crate::state::Deck;
use crate::theme::{hsla, palette};

actions!(agentdeck_palette, [SaveTemplate]);

const CONTEXT: &str = "TemplatePicker";

/// Enter in the name field saves (and must not reach the dialog's own Enter, which closes it).
pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("enter", SaveTemplate, Some(CONTEXT))]);
}

/// Writes the user templates; the in-memory list is updated even if the write fails.
fn persist_templates(deck: &mut Deck, list: Vec<agentdeck_core::logic::templates::SessionTemplate>) -> Result<(), String> {
    deck.templates = list;
    persist::save_templates(&deck.data_dir, &deck.templates)
}

/// Saves a session (the active one when `tab_id` is None) as a template. Errors if the session is
/// empty or the write fails.
pub fn save_session_as_template(
    deck: &Entity<Deck>,
    name: &str,
    tab_id: Option<&str>,
    cx: &mut App,
) -> Result<(), String> {
    deck.update(cx, |d, cx| {
        let tab_id = tab_id.map_or_else(|| d.state.active_tab_id.clone(), str::to_owned);
        let tab = d
            .state
            .tabs
            .iter()
            .find(|t| t.id == tab_id && !t.pane_ids.is_empty())
            .ok_or("the session has no agents")?;
        let root = d
            .state
            .workspaces
            .iter()
            .find(|w| w.id == tab.workspace_id)
            .map(|w| w.path.clone())
            .unwrap_or_default();
        let label = if name.trim().is_empty() { tab.name.clone() } else { name.trim().to_owned() };
        let template = session_to_template(&label, tab, &d.state.panes, &root);
        let mut list = d.templates.clone();
        list.push(template);
        cx.notify();
        persist_templates(d, list)
    })
}

pub fn rename_template(deck: &Entity<Deck>, id: &str, name: &str, cx: &mut App) -> Result<(), String> {
    deck.update(cx, |d, cx| {
        let list = d
            .templates
            .iter()
            .cloned()
            .map(|mut t| {
                if t.id == id {
                    t.name = name.to_owned();
                }
                t
            })
            .collect();
        cx.notify();
        persist_templates(d, list)
    })
}

pub fn delete_template(deck: &Entity<Deck>, id: &str, cx: &mut App) -> Result<(), String> {
    deck.update(cx, |d, cx| {
        let list = d.templates.iter().filter(|t| t.id != id).cloned().collect();
        cx.notify();
        persist_templates(d, list)
    })
}

/// Opens a new session in the active workspace from a template and spawns its agents.
pub fn new_session_from_template(deck: &Entity<Deck>, template_id: &str, cx: &mut App) {
    let planned = deck.update(cx, |d, cx| {
        let template = all_templates(&d.templates).into_iter().find(|t| t.id == template_id)?;
        let workspace = d.state.active_workspace().clone();
        if workspace.path.is_empty() && d.state.last_cwd.is_empty() {
            d.toast("No working folder: link a project or add an agent first.", cx);
            return None;
        }
        let (layout, specs) = instantiate(&template, &workspace.path, &d.state.last_cwd, &d.settings);
        let tab_id = d.state.add_tab(Some(new_tab(&template.name, &workspace.id, layout)));
        cx.notify();
        Some((tab_id, specs))
    });
    let Some((tab_id, specs)) = planned else { return };
    for spec in specs {
        crate::panes::runtime::create_pane(spec, &tab_id, false, cx);
    }
}

pub struct TemplatePicker {
    deck: Entity<Deck>,
    name: Entity<InputState>,
    message: Option<(bool, String)>,
    focus: FocusHandle,
}

/// Opens the template picker for the active workspace (sidebar, command palette).
pub fn open_template_picker(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    let global = cx.global::<super::PaletteGlobal>();
    if global.picker.as_ref().and_then(WeakEntity::upgrade).is_some() {
        return;
    }
    let view = cx.new(|cx| TemplatePicker::new(deck, window, cx));
    cx.global_mut::<super::PaletteGlobal>().picker = Some(view.downgrade());
    let focus = view.read(cx).focus.clone();
    super::open_modal(view, 512.0, None, focus, window, cx);
}

/// Opens the picker for a given workspace (selects it first).
pub fn open_template_picker_for(deck: Entity<Deck>, workspace_id: &str, window: &mut Window, cx: &mut App) {
    deck.update(cx, |d, cx| {
        d.state.select_workspace(workspace_id);
        d.save(cx);
        cx.notify();
    });
    open_template_picker(deck, window, cx);
}

impl TemplatePicker {
    fn new(deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = deck
            .read(cx)
            .state
            .active_tab()
            .map_or_else(|| "Template name".to_owned(), |t| t.name.clone());
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        Self { deck, name, message: None, focus: cx.focus_handle() }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.name.read(cx).value().trim().to_owned();
        let label = if typed.is_empty() {
            self.deck.read(cx).state.active_tab().map_or_else(|| "Template".to_owned(), |t| t.name.clone())
        } else {
            typed
        };
        self.message = Some(match save_session_as_template(&self.deck, &label, None, cx) {
            Ok(()) => {
                self.name.update(cx, |n, cx| n.set_value("", window, cx));
                (true, format!("Template \"{label}\" saved."))
            }
            Err(e) => (false, format!("Could not save: {e}")),
        });
        cx.notify();
    }
}

impl Render for TemplatePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let (workspace, templates, can_save) = {
            let d = self.deck.read(cx);
            (
                d.state.active_workspace().name.clone(),
                all_templates(&d.templates),
                d.state.active_tab().is_some_and(|t| !t.pane_ids.is_empty()),
            )
        };
        let hover = hsla(p.hover);
        let fg = hsla(p.fg);
        let rows = templates.into_iter().map(|t| {
            let titles = t.panes.iter().map(|p| p.title.clone()).collect::<Vec<_>>().join(", ");
            let id = t.id.clone();
            div()
                .id(SharedString::from(format!("template-{}", t.id)))
                .flex()
                .items_center()
                .gap_3()
                .h(crate::theme::ui(40.0))
                .px_2()
                .rounded(crate::theme::ui(6.0))
                .text_color(hsla(p.muted))
                .cursor_pointer()
                .hover(move |s| s.bg(hover).text_color(fg))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.global_mut::<super::PaletteGlobal>().picker = None;
                    window.close_dialog(cx);
                    new_session_from_template(&this.deck, &id, cx);
                }))
                .child(div().text_size(crate::theme::ui(15.0)).text_color(hsla(p.subtle)).child(IconName::LayoutTemplate))
                .child(div().truncate().text_color(hsla(p.fg)).child(t.name.clone()))
                .when(t.builtin, |s| s.child(div().text_xs().text_color(hsla(p.subtle)).child("built-in")))
                .child(
                    div()
                        .id(SharedString::from(format!("agents-{}", t.id)))
                        .ml_auto()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_1()
                        .children(t.panes.iter().map(|tp| ui::agent_mark(tp.agent, 13.0, &p)))
                        .tooltip(move |window, cx| Tooltip::new(titles.clone()).build(window, cx)),
                )
        });
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SaveTemplate, window, cx| {
                if this.name.focus_handle(cx).is_focused(window) {
                    this.save(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .flex()
            .flex_col()
            .w_full()
            .text_size(crate::theme::ui(14.0))
            .child(ui::header("New session from template", format!("In {workspace}"), &p))
            .child(
                div()
                    .id("templates")
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .max_h(crate::theme::ui(320.0))
                    .overflow_y_scroll()
                    .p_3()
                    .children(rows),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .border_t_1()
                    .border_color(hsla(p.line))
                    .px_5()
                    .py_4()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(hsla(p.muted))
                            .child("Save current session as template"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(Input::new(&self.name)))
                            .child(
                                ui::secondary_button("save-template", IconName::Save, "Save", !can_save, &p).on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        if can_save {
                                            this.save(window, cx);
                                        }
                                    }),
                                ),
                            ),
                    )
                    .children(self.message.clone().map(|(ok, text)| {
                        div().text_xs().text_color(hsla(if ok { p.ok } else { p.bad })).child(text)
                    })),
            )
    }
}
