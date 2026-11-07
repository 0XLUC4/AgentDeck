//! Skills and Notes dialogs of one agent pane, with the Markdown file editor they share.
//! Original: components/Skills/{AgentDialog,SkillsButton,NotesButton}.tsx.

use agentdeck_core::logic::deck::PaneSpec;
use agentdeck_core::logic::text::agent_name;
use agentdeck_core::logic::theme::FONT_MONO;
use agentdeck_core::skills::{
    NoteFile, SkillInfo, agent_file_read, agent_file_write, notes_list, skills_install,
    skills_list, skills_remove,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{EDITOR_CONTEXT, SaveAgentFile, close_button, confirm, modal_frame, segmented};
use crate::theme::{hsla, palette, ui};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentDialogKind {
    Skills,
    Notes,
}

/// Where a skill is installed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    User,
    Project,
}

impl Scope {
    fn id(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Project => "project",
        }
    }
}

fn scope_label(scope: &str) -> &'static str {
    if scope == "project" { "This project" } else { "All projects" }
}

/// One listed file open in the editor; created on save when missing.
struct Editor {
    path: String,
    /// Content on disk; None while loading.
    saved: Option<String>,
    text: Entity<TextareaState>,
    _changes: Subscription,
}

impl Editor {
    fn dirty(&self, cx: &App) -> bool {
        self.saved
            .as_deref()
            .is_some_and(|saved| self.text.read(cx).value().as_ref() != saved)
    }
}

/// Modal for one pane's agent: skills manager or notes list, both opening the file editor.
pub struct AgentFilesDialog {
    pane: PaneSpec,
    kind: AgentDialogKind,
    pub focus: FocusHandle,
    skills: Option<Vec<SkillInfo>>,
    notes: Option<Vec<NoteFile>>,
    editing: Option<Editor>,
    url: Entity<InputState>,
    scope: Scope,
    busy: bool,
    /// Footer line; the default text shows when empty.
    status: String,
    _url_events: Subscription,
}

impl AgentFilesDialog {
    pub fn new(
        pane: PaneSpec,
        kind: AgentDialogKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://github.com/owner/repo (or …/tree/main/skills/name)")
        });
        let url_events = cx.subscribe_in(&url, window, |this, _, event: &InputEvent, window, cx| {
            match event {
                InputEvent::PressEnter { .. } => this.install(window, cx),
                _ => cx.notify(),
            }
        });
        let mut this = Self {
            pane,
            kind,
            focus: cx.focus_handle(),
            skills: None,
            notes: None,
            editing: None,
            url,
            scope: Scope::User,
            busy: false,
            status: String::new(),
            _url_events: url_events,
        };
        this.refresh(cx);
        this
    }

    /// What gets the focus on open: the GitHub link field for skills (the dialog opens to install one).
    pub fn initial_focus(&self, cx: &App) -> FocusHandle {
        match self.kind {
            AgentDialogKind::Skills => self.url.read(cx).focus_handle(cx),
            AgentDialogKind::Notes => self.focus.clone(),
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let agent = self.pane.agent;
        let cwd = self.pane.cwd.clone();
        let kind = self.kind;
        cx.spawn(async move |this, cx| {
            match kind {
                AgentDialogKind::Skills => {
                    let result = cx
                        .background_executor()
                        .spawn(async move { skills_list(agent, cwd) })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        match result {
                            Ok(list) => this.skills = Some(list),
                            Err(e) => this.status = e,
                        }
                        cx.notify();
                    });
                }
                AgentDialogKind::Notes => {
                    let result = cx
                        .background_executor()
                        .spawn(async move { notes_list(agent, cwd) })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        match result {
                            Ok(list) => this.notes = Some(list),
                            Err(e) => this.status = e,
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn edit(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let text = cx.new(|cx| TextareaState::new(window, cx));
        let changes = cx.subscribe(&text, |_, _, _: &InputEvent, cx| cx.notify());
        self.editing = Some(Editor {
            path: path.clone(),
            saved: None,
            text: text.clone(),
            _changes: changes,
        });
        let agent = self.pane.agent;
        let cwd = self.pane.cwd.clone();
        cx.spawn_in(window, async move |this, cx| {
            let read_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { agent_file_read(agent, cwd, read_path) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(editor) = this.editing.as_mut().filter(|e| e.path == path) else {
                    return;
                };
                match result {
                    Ok(content) => {
                        editor
                            .text
                            .update(cx, |t, cx| t.set_value(content.clone(), window, cx));
                        editor.saved = Some(content);
                        editor.text.read(cx).focus_handle(cx).focus(window, cx);
                    }
                    Err(e) => this.status = e,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editing.as_ref().filter(|e| e.dirty(cx)) else {
            return;
        };
        let text = editor.text.read(cx).value().to_string();
        let path = editor.path.clone();
        let agent = self.pane.agent;
        let cwd = self.pane.cwd.clone();
        cx.spawn(async move |this, cx| {
            let (write_path, write_text) = (path.clone(), text.clone());
            let result = cx
                .background_executor()
                .spawn(async move { agent_file_write(agent, cwd, write_path, write_text) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        if let Some(editor) = this.editing.as_mut().filter(|e| e.path == path) {
                            editor.saved = Some(text);
                        }
                        this.status = format!("Saved {path}");
                    }
                    Err(e) => this.status = e,
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = self.editing.as_ref().is_some_and(|e| e.dirty(cx));
        cx.spawn_in(window, async move |this, cx| {
            if dirty && !confirm("Discard your changes?".into(), "Discard", cx).await {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.editing = None;
                this.refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.url.read(cx).value().trim().to_string();
        if self.busy || url.is_empty() {
            return;
        }
        self.busy = true;
        self.status = "Downloading…".into();
        cx.notify();
        let agent = self.pane.agent;
        let cwd = self.pane.cwd.clone();
        let scope = self.scope.id().to_string();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { skills_install(agent, cwd, url, scope) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(names) => {
                        this.status = format!(
                            "Installed: {}. Restart the agent if it does not see them yet.",
                            names.join(", ")
                        );
                        this.url.update(cx, |u, cx| u.set_value("", window, cx));
                        this.refresh(cx);
                    }
                    Err(e) => this.status = e,
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove(&mut self, name: String, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let agent = self.pane.agent;
        let cwd = self.pane.cwd.clone();
        cx.spawn_in(window, async move |this, cx| {
            let question = format!("Uninstall the skill \"{name}\"? Its folder is deleted.");
            if !confirm(question, "Uninstall", cx).await {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move { skills_remove(agent, cwd, path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.status = format!("Removed {name}.");
                        this.refresh(cx);
                    }
                    Err(e) => this.status = e,
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn default_status(&self) -> String {
        match self.kind {
            AgentDialogKind::Skills => format!(
                "{} skill(s). Plugin and bundled skills are managed in the agent itself.",
                self.skills.as_ref().map_or(0, Vec::len)
            ),
            AgentDialogKind::Notes => "Instructions the agent reads at start, and its memory files. \
                                       Changes apply to its next session."
                .into(),
        }
    }

    fn render_editor(&self, editor: &Editor, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let dirty = editor.dirty(cx);
        let height = window.viewport_size().height * 0.55;
        div()
            .flex()
            .flex_col()
            .gap_3()
            .key_context(EDITOR_CONTEXT)
            .on_action(cx.listener(|this, _: &SaveAgentFile, _, cx| this.save(cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("agent-file-back")
                            .ghost()
                            .small()
                            .icon(IconName::ArrowLeft)
                            .tooltip("Back")
                            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                    )
                    .child(
                        div()
                            .id("agent-file-path")
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(FONT_MONO)
                            .text_size(ui(12.0))
                            .text_color(hsla(p.muted))
                            .child(editor.path.clone()),
                    )
                    .child(
                        Button::new("agent-file-save")
                            .primary()
                            .small()
                            .icon(IconName::Save)
                            .label("Save")
                            .disabled(!dirty)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    ),
            )
            .child(
                div()
                    .font_family(FONT_MONO)
                    .text_size(ui(12.0))
                    .child(
                        Textarea::new(&editor.text)
                            .h(DefiniteLength::from(height))
                            .disabled(editor.saved.is_none()),
                    ),
            )
            .into_any_element()
    }

    fn render_skills(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let can_install = !self.busy && !self.url.read(cx).value().trim().is_empty();
        let scope = self.scope;
        let picker = segmented(
            "skill-scope",
            &[(Scope::User, "All projects"), (Scope::Project, "This project")],
            scope,
            {
                let this = cx.entity().downgrade();
                move |scope, _, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.scope = scope;
                        cx.notify();
                    });
                }
            },
            cx,
        );
        let form = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().min_w_0().child(Input::new(&self.url).small()))
            .child(picker)
            .child(
                Button::new("skill-install")
                    .primary()
                    .small()
                    .icon(IconName::Download)
                    .label("Install")
                    .disabled(!can_install)
                    .on_click(cx.listener(|this, _, window, cx| this.install(window, cx))),
            );
        let list = match &self.skills {
            None => centered_note("Loading…", p.muted).into_any_element(),
            Some(skills) if skills.is_empty() => {
                centered_note("No skills installed.", p.muted).into_any_element()
            }
            Some(skills) => list_box("skills-list", &p).children(skills.iter().enumerate().map(
                |(i, skill)| {
                    let (edit_path, name, path) =
                        (skill.path.clone(), skill.name.clone(), skill.path.clone());
                    row(&p)
                        .items_start()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .pt(ui(2.0))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .truncate()
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(skill.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .rounded(ui(4.0))
                                                .bg(hsla(p.base))
                                                .px(ui(6.0))
                                                .text_size(ui(11.0))
                                                .text_color(hsla(p.subtle))
                                                .child(scope_label(skill.scope)),
                                        ),
                                )
                                .when(!skill.description.is_empty(), |d| {
                                    d.child(
                                        div()
                                            .text_size(ui(12.0))
                                            .text_color(hsla(p.muted))
                                            .line_clamp(2)
                                            .child(skill.description.clone()),
                                    )
                                }),
                        )
                        .child(
                            Button::new(("skill-edit", i))
                                .ghost()
                                .small()
                                .icon(IconName::Pencil)
                                .tooltip(format!("Edit {}", skill.name))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit(edit_path.clone(), window, cx)
                                })),
                        )
                        .child(
                            Button::new(("skill-remove", i))
                                .ghost()
                                .small()
                                .icon(IconName::Trash)
                                .tooltip(format!("Uninstall {}", skill.name))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.remove(name.clone(), path.clone(), window, cx)
                                })),
                        )
                },
            ))
            .into_any_element(),
        };
        div().flex().flex_col().gap_3().child(form).child(list).into_any_element()
    }

    fn render_notes(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(notes) = &self.notes else {
            return centered_note("Loading…", p.muted).into_any_element();
        };
        list_box("notes-list", &p).children(notes.iter().enumerate().map(|(i, note)| {
            let path = note.path.clone();
            let label = if note.exists {
                format!("Edit {}", note.label)
            } else {
                format!("Create {}", note.label)
            };
            row(&p)
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_color(hsla(if note.exists { p.fg } else { p.subtle }))
                                .child(note.label.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .font_family(FONT_MONO)
                                .text_size(ui(11.0))
                                .text_color(hsla(p.subtle))
                                .child(note.path.clone()),
                        ),
                )
                .child(
                    Button::new(("note-edit", i))
                        .ghost()
                        .small()
                        .icon(if note.exists { IconName::Pencil } else { IconName::FilePlus })
                        .tooltip(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit(path.clone(), window, cx)
                        })),
                )
        }))
        .into_any_element()
    }
}

impl Render for AgentFilesDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let title = match self.kind {
            AgentDialogKind::Skills => "Skills",
            AgentDialogKind::Notes => "Notes",
        };
        let body = match (&self.editing, self.kind) {
            (Some(editor), _) => self.render_editor(editor, window, cx),
            (None, AgentDialogKind::Skills) => self.render_skills(cx),
            (None, AgentDialogKind::Notes) => self.render_notes(cx),
        };
        let status = if self.status.is_empty() {
            self.default_status()
        } else {
            self.status.clone()
        };
        let panel = div()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt_4()
                    .pr_3()
                    .pl_5()
                    .child(
                        div()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(ui(15.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(ui(12.0))
                                    .text_color(hsla(p.subtle))
                                    .child(format!(
                                        "{} · {}",
                                        agent_name(self.pane.agent),
                                        self.pane.cwd
                                    )),
                            ),
                    )
                    .child(close_button()),
            )
            .child(div().flex().flex_col().gap_3().p_5().min_h_0().child(body))
            .child(
                div()
                    .border_t_1()
                    .border_color(hsla(p.line))
                    .px_5()
                    .py_3()
                    .text_size(ui(12.0))
                    .text_color(hsla(p.subtle))
                    .child(status),
            );
        modal_frame(&self.focus, 768.0, panel, cx)
    }
}

/// Bordered scrolling list (max 24rem) of the dialogs.
fn list_box(id: &'static str, p: &agentdeck_core::logic::theme::Palette) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .max_h(ui(384.0))
        .overflow_y_scroll()
        .rounded(ui(6.0))
        .border_1()
        .border_color(hsla(p.line))
}

fn row(p: &agentdeck_core::logic::theme::Palette) -> Div {
    div()
        .flex()
        .gap_2()
        .border_b_1()
        .border_color(hsla(p.line))
        .py(ui(6.0))
        .pr_1()
        .pl_3()
}

fn centered_note(text: &'static str, color: u32) -> Div {
    div()
        .py_3()
        .flex()
        .justify_center()
        .text_color(hsla(color))
        .child(text)
}
