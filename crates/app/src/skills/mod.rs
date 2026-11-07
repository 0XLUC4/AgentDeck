//! Skills (list/install/remove per agent), notes, agent files dialog + editor (Ctrl+S saves),
//! notes graph, 3D assets browser (.glb/.gltf/.obj/.fbx/.stl/.blend via Blender export).
//! Original: src/components/{Skills,Graph,Assets}. Backend: agentdeck_core::{skills,notes_graph,assets}.
//!
//! Dialogs do not need gpui-component's `Root`: the open one is kept in a global and drawn by
//! [`overlay`], which the root view renders as its last child.

mod agent_dialog;
mod assets;
mod graph;
mod model3d;
mod viewer;

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

pub use agent_dialog::{AgentDialogKind, AgentFilesDialog};
pub use assets::AssetsDialog;
pub use graph::GraphDialog;

actions!(agentdeck, [SaveAgentFile, CloseSkillsDialog]);

/// Key context of the file editor (Ctrl+S saves).
const EDITOR_CONTEXT: &str = "AgentFileEditor";
/// Key context of every dialog of this module (Esc closes).
const DIALOG_CONTEXT: &str = "SkillsDialog";

/// Registers this module's key bindings and globals. Called once from `main`.
pub fn init(cx: &mut App) {
    cx.set_global(OpenDialog(None));
    cx.bind_keys([
        KeyBinding::new("ctrl-s", SaveAgentFile, Some(EDITOR_CONTEXT)),
        KeyBinding::new("escape", CloseSkillsDialog, Some(DIALOG_CONTEXT)),
    ]);
}

/// The dialog currently shown by [`overlay`].
struct OpenDialog(Option<AnyView>);

impl Global for OpenDialog {}

/// The open dialog of this module, drawn above everything. Render it as the root view's last child
/// (the root must be `relative`, it usually is with `size_full`).
pub fn overlay(cx: &App) -> Option<AnyElement> {
    let view = cx.try_global::<OpenDialog>()?.0.clone()?;
    Some(
        deferred(div().absolute().inset_0().child(view))
            .with_priority(2)
            .into_any_element(),
    )
}

fn show(view: AnyView, focus: FocusHandle, window: &mut Window, cx: &mut App) {
    cx.set_global(OpenDialog(Some(view)));
    focus.focus(window, cx);
    window.refresh();
}

/// Closes the open dialog of this module (if any).
pub fn close(window: &mut Window, cx: &mut App) {
    cx.set_global(OpenDialog(None));
    window.refresh();
}

/// Opens the skills manager of an agent pane.
pub fn open_skills(deck: Entity<Deck>, pane_id: &str, window: &mut Window, cx: &mut App) {
    open_agent_dialog(deck, pane_id, AgentDialogKind::Skills, window, cx);
}

/// Opens the notes (instruction and memory files) editor of an agent pane.
pub fn open_notes(deck: Entity<Deck>, pane_id: &str, window: &mut Window, cx: &mut App) {
    open_agent_dialog(deck, pane_id, AgentDialogKind::Notes, window, cx);
}

fn open_agent_dialog(
    deck: Entity<Deck>,
    pane_id: &str,
    kind: AgentDialogKind,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(pane) = deck.read(cx).state.panes.get(pane_id).map(|p| p.spec.clone()) else {
        return;
    };
    let view = cx.new(|cx| AgentFilesDialog::new(pane, kind, window, cx));
    let focus = view.read(cx).initial_focus(cx);
    show(view.into(), focus, window, cx);
}

/// Opens the 3D assets browser of a workspace (sidebar button).
pub fn open_assets(deck: Entity<Deck>, workspace_id: &str, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| AssetsDialog::new(deck, workspace_id.to_string(), window, cx));
    let focus = view.read(cx).focus.clone();
    show(view.into(), focus, window, cx);
}

/// Opens the notes graph of a workspace (sidebar button).
pub fn open_graph(deck: Entity<Deck>, workspace_id: &str, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| GraphDialog::new(deck, workspace_id.to_string(), cx));
    let focus = view.read(cx).focus.clone();
    show(view.into(), focus, window, cx);
}

/// Pane header button (agent panes) opening the skills manager.
pub fn skills_button(deck: Entity<Deck>, pane_id: String) -> Button {
    Button::new(SharedString::from(format!("skills-{pane_id}")))
        .ghost()
        .xsmall()
        .icon(IconName::Puzzle)
        .tooltip("Skills: see, edit, install (GitHub link), uninstall")
        .on_click(move |_, window, cx| open_skills(deck.clone(), &pane_id, window, cx))
}

/// Pane header button (agent panes) opening the notes editor.
pub fn notes_button(deck: Entity<Deck>, pane_id: String) -> Button {
    Button::new(SharedString::from(format!("notes-{pane_id}")))
        .ghost()
        .xsmall()
        .icon(IconName::NotebookPen)
        .tooltip("Notes: the agent's instructions and memory files")
        .on_click(move |_, window, cx| open_notes(deck.clone(), &pane_id, window, cx))
}

/// Full-window backdrop with a centered panel (the `<dialog>` of the original). As with
/// `showModal`, clicks on the backdrop do nothing; Esc and the close button close.
fn modal_frame(focus: &FocusHandle, width: f32, panel: Div, cx: &App) -> Stateful<Div> {
    let p = palette(cx);
    div()
        .id("skills-modal")
        .track_focus(focus)
        .key_context(DIALOG_CONTEXT)
        .on_action(|_: &CloseSkillsDialog, window, cx| close(window, cx))
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla_a(p.gutter, 0.7))
        .occlude()
        .child(
            panel
                .w(ui(width))
                .max_w(relative(0.96))
                .max_h(relative(0.86))
                .flex()
                .flex_col()
                .rounded(ui(agentdeck_core::logic::theme::RADIUS_LG))
                .border_1()
                .border_color(hsla(p.line))
                .bg(hsla(p.panel))
                .text_color(hsla(p.fg))
                .text_size(ui(13.0))
                .font_family(agentdeck_core::logic::theme::FONT_SANS)
                .overflow_hidden(),
        )
}

/// Header of the `Modal` shell: title, extra actions, close button.
fn modal_header(title: String, actions: Option<AnyElement>, cx: &App) -> Div {
    let p = palette(cx);
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_color(hsla(p.line))
        .px_5()
        .py_3()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(ui(15.0))
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
        .children(actions)
        .child(close_button())
}

fn close_button() -> Button {
    Button::new("skills-dialog-close")
        .ghost()
        .small()
        .icon(IconName::X)
        .tooltip("Close (Esc)")
        .on_click(|_, window, cx| close(window, cx))
}

/// Segmented control (one pressed button) used by the dialogs.
fn segmented<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    value: T,
    on_change: impl Fn(T, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    let p = palette(cx);
    let on_change = Rc::new(on_change);
    div()
        .flex()
        .flex_shrink_0()
        .gap_1()
        .rounded(ui(8.0))
        .bg(hsla(p.base))
        .p_1()
        .children(options.iter().enumerate().map(|(i, &(option, label))| {
            let on_change = on_change.clone();
            let pressed = option == value;
            div()
                .id((id, i))
                .h(ui(28.0))
                .px_3()
                .flex()
                .items_center()
                .rounded(ui(6.0))
                .text_size(ui(12.0))
                .cursor_pointer()
                .when(pressed, |d| {
                    d.bg(hsla(p.raised))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(hsla(p.fg))
                })
                .when(!pressed, |d| {
                    d.text_color(hsla(p.muted))
                        .hover(|s| s.text_color(hsla(p.fg)))
                })
                .child(label)
                .on_click(move |_, window, cx| on_change(option, window, cx))
        }))
}

/// Asks a yes/no question with a native dialog; true when the first answer is picked.
async fn confirm(
    message: String,
    yes: &'static str,
    cx: &mut AsyncWindowContext,
) -> bool {
    let answer = cx.update(|window, cx| {
        window.prompt(PromptLevel::Warning, &message, None, &[yes, "Cancel"], cx)
    });
    match answer {
        Ok(receiver) => matches!(receiver.await, Ok(0)),
        Err(_) => false,
    }
}
