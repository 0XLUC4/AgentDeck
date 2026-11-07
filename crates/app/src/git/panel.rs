//! Right slide-over with the Git state of one pane's working directory (its worktree when it has one).
//! Original: src/components/Git/GitPanel.tsx. Emits `DismissEvent` when closed.

use agentdeck_core::git::{
    Checkpoint, Comparison, GitStatus, Upstream, git_checkpoint, git_commit, git_compare, git_create_pr, git_diff,
    git_diff_trees, git_list_checkpoints, git_merge_worktree, git_pane_dir, git_pull, git_push, git_remove_worktree,
    git_restore_checkpoint, git_status, git_upstream,
};
use agentdeck_core::logic::deck::Pane;
use agentdeck_core::logic::text::agent_name;
use agentdeck_core::logic::theme::FONT_MONO;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::checkpoints::{pane_git_dir, set_reviewer};
use super::diff::{DiffLineKind, classify_diff, diff_lines};
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

const DIFF_PAGE: usize = 500;

/// Pane title, or its agent's name when untitled.
pub fn pane_name(pane: &Pane) -> String {
    if pane.title.is_empty() {
        agent_name(pane.agent).to_owned()
    } else {
        pane.title.clone()
    }
}

/// `new Date(ts).toLocaleString("en-US")`, in local time.
fn when(ts_ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ts_ms as i64)
        .single()
        .map_or_else(|| ts_ms.to_string(), |d| d.format("%-m/%-d/%Y, %-I:%M:%S %p").to_string())
}

struct Snapshot {
    status: GitStatus,
    checkpoints: Vec<Checkpoint>,
    upstream: Upstream,
}

fn load(dir: &str, pane_id: &str) -> Result<Snapshot, String> {
    Ok(Snapshot {
        status: git_status(dir.to_owned())?,
        checkpoints: git_list_checkpoints(dir.to_owned(), pane_id.to_owned())?,
        upstream: git_upstream(dir.to_owned())?,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum SyncKind {
    Pull,
    Push,
}

/// Replaces the original's window.confirm / window.prompt.
enum Confirm {
    Merge,
    PullRequest { body: String },
    RemoveWorktree,
    Restore(Checkpoint),
}

pub struct GitPanel {
    deck: Entity<Deck>,
    pane_id: String,
    dir: Option<String>,
    status: Option<GitStatus>,
    upstream: Option<Upstream>,
    checkpoints: Vec<Checkpoint>,
    /// Active comparison and the other pane's id.
    comparison: Option<(Comparison, String)>,
    selected: Option<String>,
    diff: Option<Vec<(DiffLineKind, SharedString)>>,
    shown: usize,
    notice: Option<(bool, String)>,
    busy: bool,
    syncing: Option<SyncKind>,
    pending: Option<Confirm>,
    message: Entity<TextareaState>,
    pr_title: Entity<InputState>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for GitPanel {}

impl Focusable for GitPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl GitPanel {
    pub fn new(deck: Entity<Deck>, pane_id: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pane_id = pane_id.into();
        let message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("feat(scope): description")
                .rows(4)
        });
        let pr_title = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe(&message, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe_in(&pr_title, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.confirm(window, cx);
                }
            }),
            cx.observe(&deck, |_, _, cx| cx.notify()),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let dir = pane_git_dir(deck.read(cx), &pane_id);
        let mut this = Self {
            deck,
            pane_id,
            dir,
            status: None,
            upstream: None,
            checkpoints: Vec::new(),
            comparison: None,
            selected: None,
            diff: None,
            shown: DIFF_PAGE,
            notice: None,
            busy: false,
            syncing: None,
            pending: None,
            message,
            pr_title,
            focus,
            _subscriptions: subscriptions,
        };
        if this.dir.is_none() {
            this.notice = Some((false, "This pane no longer exists.".into()));
        }
        this.refresh(cx);
        this
    }

    fn pane(&self, cx: &App) -> Option<Pane> {
        self.deck.read(cx).state.panes.get(&self.pane_id).cloned()
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn apply(&mut self, snapshot: Snapshot) {
        self.status = Some(snapshot.status);
        self.upstream = Some(snapshot.upstream);
        self.checkpoints = snapshot.checkpoints;
    }

    /// Initial load: status, restore points and upstream.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.dir.clone() else { return };
        let pane_id = self.pane_id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { load(&dir, &pane_id) }).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(snapshot) => this.apply(snapshot),
                    Err(e) => this.notice = Some((false, e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Shows a file's diff (against HEAD, or between the compared panes).
    fn open_file(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        self.selected = path.clone();
        self.diff = None;
        self.shown = DIFF_PAGE;
        cx.notify();
        let (Some(path), Some(dir)) = (path, self.dir.clone()) else { return };
        let trees = self.comparison.as_ref().map(|(c, _)| (c.from.clone(), c.to.clone()));
        let file = path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    match trees {
                        Some((from, to)) => git_diff_trees(dir, from, to, file),
                        None => git_diff(dir, file),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(text) if this.selected.as_deref() == Some(path.as_str()) => {
                        let lines = diff_lines(&text);
                        let kinds = classify_diff(&lines);
                        this.diff = Some(
                            kinds
                                .into_iter()
                                .zip(lines)
                                .map(|(k, l)| (k, SharedString::from(l.to_owned())))
                                .collect(),
                        );
                    }
                    Ok(_) => {}
                    Err(e) => this.notice = Some((false, e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Runs one git job off the UI thread with a busy state, reports its result and refreshes.
    /// `then` applies the job's value and returns the success notice, if any.
    fn act<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(String) -> Result<T, String> + Send + 'static,
        then: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) -> Option<String> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = self.dir.clone() else { return };
        self.busy = true;
        self.notice = None;
        cx.notify();
        let pane_id = self.pane_id.clone();
        cx.spawn_in(window, async move |this, cx| {
            let (result, snapshot) = cx
                .background_spawn(async move {
                    match job(dir.clone()) {
                        Ok(value) => (Ok(value), Some(load(&dir, &pane_id))),
                        Err(e) => (Err(e), None),
                    }
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                this.syncing = None;
                match result {
                    Err(e) => this.notice = Some((false, e)),
                    Ok(value) => {
                        if let Some(text) = then(this, value, window, cx) {
                            this.notice = Some((true, text));
                        }
                        match snapshot {
                            Some(Ok(snapshot)) => this.apply(snapshot),
                            Some(Err(e)) => this.notice = Some((false, e)),
                            None => {}
                        }
                        if this.comparison.is_none() && this.selected.is_some() {
                            this.open_file(this.selected.clone(), cx);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.comparison = None;
        self.act(
            |_| Ok(()),
            |this, (), _, cx| {
                this.open_file(None, cx);
                None
            },
            window,
            cx,
        );
    }

    fn compare_with(&mut self, other_id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let other = other_id
            .as_deref()
            .and_then(|id| self.deck.read(cx).state.panes.get(id))
            .map(|p| (p.id.clone(), git_pane_dir(p.cwd.clone(), p.id.clone(), p.worktree)));
        let Some((other_id, other_dir)) = other.filter(|_| self.dir.is_some()) else {
            self.comparison = None;
            self.open_file(None, cx);
            return;
        };
        self.act(
            move |dir| git_compare(dir, other_dir),
            move |this, cmp, _, cx| {
                this.comparison = Some((cmp, other_id));
                this.open_file(None, cx);
                None
            },
            window,
            cx,
        );
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.message.read(cx).value().to_string();
        self.act(
            move |dir| git_commit(dir, message),
            |this, sha, window, cx| {
                this.message.update(cx, |m, cx| m.set_value("", window, cx));
                Some(format!("Created commit {sha}."))
            },
            window,
            cx,
        );
    }

    fn sync(&mut self, kind: SyncKind, window: &mut Window, cx: &mut Context<Self>) {
        self.act(
            move |dir| match kind {
                SyncKind::Pull => git_pull(dir),
                SyncKind::Push => git_push(dir),
            },
            |_, out, _, _| Some(out),
            window,
            cx,
        );
        self.syncing = Some(kind);
    }

    fn create_restore_point(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane_id = self.pane_id.clone();
        self.act(
            move |dir| git_checkpoint(dir, pane_id),
            |_, sha, _, _| sha.map(|_| "Restore point created.".to_owned()),
            window,
            cx,
        );
    }

    /// Asks before a destructive or remote action (the original used window.confirm / prompt).
    fn ask(&mut self, confirm: Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Confirm::PullRequest { .. } = confirm {
            let task = self.deck.read(cx).tasks.get(&self.pane_id).and_then(|t| t.history.first());
            let first_line = task
                .and_then(|t| t.prompt.as_deref())
                .and_then(|p| p.lines().next())
                .map(|l| l.chars().take(120).collect::<String>())
                .filter(|l| !l.is_empty());
            let suggested = first_line
                .or_else(|| self.status.as_ref().and_then(|s| s.branch.clone()))
                .unwrap_or_default();
            self.pr_title.update(cx, |input, cx| input.set_value(suggested, window, cx));
            self.pr_title.read(cx).focus_handle(cx).focus(window, cx);
        }
        self.pending = Some(confirm);
        cx.notify();
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else { return };
        self.focus.focus(window, cx);
        match pending {
            Confirm::Merge => self.act(git_merge_worktree, |_, out, _, _| Some(out), window, cx),
            Confirm::PullRequest { body } => {
                let title = self.pr_title.read(cx).value().trim().to_owned();
                if title.is_empty() {
                    return;
                }
                self.act(
                    move |dir| git_create_pr(dir, title, body),
                    |_, url, _, _| Some(format!("Pull request opened: {url}")),
                    window,
                    cx,
                );
            }
            Confirm::RemoveWorktree => self.remove_worktree(window, cx),
            Confirm::Restore(cp) => {
                let pane_id = self.pane_id.clone();
                let label = when(cp.ts);
                self.act(
                    move |dir| git_restore_checkpoint(dir, pane_id, cp.sha),
                    move |_, _, _, _| Some(format!("Files restored to {label}.")),
                    window,
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn remove_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = self.dir.clone() else { return };
        let branch = self.status.as_ref().and_then(|s| s.branch.clone()).unwrap_or_default();
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { git_remove_worktree(dir) }).await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.deck.update(cx, |deck, cx| deck.toast(format!("Deleted worktree {branch}."), cx));
                    this.close(cx);
                }
                Err(e) => {
                    this.notice = Some((false, e));
                    this.busy = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn pr_body(&self, cx: &App) -> String {
        let deck = self.deck.read(cx);
        let summary = deck
            .tasks
            .get(&self.pane_id)
            .and_then(|t| t.history.first())
            .and_then(|t| t.summary.clone())
            .filter(|s| !s.is_empty());
        let agent = deck.state.panes.get(&self.pane_id).map(|p| agent_name(p.agent)).unwrap_or("");
        summary.map_or_else(String::new, |s| format!("{s}\n\nMade with {agent} in AgentDeck."))
    }
}

// ---- rendering ----

fn icon(name: IconName, color: Hsla) -> Svg {
    svg().path(name.path()).size(ui(15.0)).flex_shrink_0().text_color(color)
}

fn section(title: &'static str, line: Hsla, subtle: Hsla) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .border_t_1()
        .border_color(line)
        .px_3()
        .py_3()
        .child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(subtle).child(title))
}

/// A dropdown that looks like the original `<select>`: (value, label) options, `None` is the first one.
fn select(
    id: &'static str,
    current: String,
    options: Vec<(Option<String>, String)>,
    disabled: bool,
    on_pick: impl Fn(Option<String>, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_pick = std::rc::Rc::new(on_pick);
    Button::new(id)
        .outline()
        .w_full()
        .label(current)
        .dropdown_caret(true)
        .disabled(disabled)
        .dropdown_menu(move |mut menu, _, _| {
            for (value, label) in options.clone() {
                let on_pick = on_pick.clone();
                menu = menu.item(
                    PopupMenuItem::new(label).on_click(move |_, window, cx| on_pick(value.clone(), window, cx)),
                );
            }
            menu
        })
}

impl Render for GitPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let (fg, muted, subtle, line) = (hsla(p.fg), hsla(p.muted), hsla(p.subtle), hsla(p.line));
        let deck = self.deck.read(cx);
        let pane = deck.state.panes.get(&self.pane_id).cloned();
        let name = pane.as_ref().map(pane_name).unwrap_or_default();
        let exited = pane.as_ref().is_none_or(|p| p.exited);
        let mut others: Vec<(Option<String>, String)> = deck
            .state
            .panes
            .values()
            .filter(|o| o.id != self.pane_id)
            .map(|o| (Some(o.id.clone()), format!("{} · {}", pane_name(o), agent_name(o.agent))))
            .collect();
        others.sort_by(|a, b| a.1.cmp(&b.1));
        let reviewer = deck.reviews.get(&self.pane_id).cloned();
        let compared_name = self
            .comparison
            .as_ref()
            .map(|(_, id)| deck.state.panes.get(id).map_or_else(|| "…".to_owned(), pane_name));
        let label_of = |id: &Option<String>, none: &str| {
            others
                .iter()
                .find(|(v, _)| v == id && id.is_some())
                .map_or_else(|| none.to_owned(), |(_, l)| l.clone())
        };
        let compare_label = label_of(
            &self.comparison.as_ref().map(|(_, id)| id.clone()),
            "Uncommitted changes (vs HEAD)",
        );
        let reviewer_label = label_of(&reviewer, "Nobody");
        let no_dir = self.dir.is_none();
        let busy = self.busy;
        let status = self.status.as_ref();
        let branch = status.and_then(|s| s.branch.clone());
        let files = self
            .comparison
            .as_ref()
            .map(|(c, _)| c.files.clone())
            .or_else(|| status.map(|s| s.files.clone()))
            .unwrap_or_default();
        let viewport = window.viewport_size();
        let width = (viewport.width * 0.92).min(ui(960.0).to_pixels(window.rem_size()));

        // Header.
        let header = div()
            .flex()
            .h(ui(40.0))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(line)
            .pl_3()
            .pr_2()
            .child(icon(IconName::GitBranch, subtle))
            .child(div().font_weight(FontWeight::MEDIUM).child("Git"))
            .child(div().min_w_0().flex_1().truncate().text_xs().text_color(subtle).child(name.clone()))
            .child(
                Button::new("git-refresh")
                    .ghost()
                    .small()
                    .icon(IconName::RefreshCw)
                    .tooltip("Refresh")
                    .disabled(busy || no_dir)
                    .on_click(cx.listener(|this, _, window, cx| this.refresh_all(window, cx))),
            )
            .child(
                Button::new("git-close")
                    .ghost()
                    .small()
                    .icon(IconName::X)
                    .tooltip("Close (Esc)")
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            );

        // Branch row.
        let ahead_behind = status
            .filter(|s| s.base.is_some() && s.branch != s.base)
            .map(|s| format!("↑{} ↓{} vs {}", s.ahead, s.behind, s.base.clone().unwrap_or_default()));
        let branch_row = div()
            .flex()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            .gap_x_3()
            .gap_y_1()
            .border_b_1()
            .border_color(line)
            .px_3()
            .py_2()
            .text_xs()
            .child(div().font_family(FONT_MONO).text_color(fg).child(match status {
                Some(s) => s.branch.clone().unwrap_or_else(|| "Detached HEAD".into()),
                None => "…".into(),
            }))
            .children(ahead_behind.map(|t| div().text_color(muted).child(t)))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_right()
                    .font_family(FONT_MONO)
                    .text_color(subtle)
                    .child(self.dir.clone().unwrap_or_default()),
            );

        let notice = self.notice.as_ref().map(|(ok, text)| {
            div()
                .flex_shrink_0()
                .border_b_1()
                .border_color(line)
                .px_3()
                .py_2()
                .text_xs()
                .when(*ok, |d| d.text_color(muted))
                .when(!*ok, |d| d.bg(hsla_a(p.bad, 0.1)).text_color(hsla(p.bad)))
                .child(text.clone())
        });

        let pending = self.pending.as_ref().map(|c| {
            let text = match c {
                Confirm::Merge => {
                    format!("Merge {} into the main repository (--no-ff)?", branch.clone().unwrap_or_default())
                }
                Confirm::PullRequest { .. } => format!(
                    "Pull request for {} (pushes the branch to origin):",
                    branch.clone().unwrap_or_default()
                ),
                Confirm::RemoveWorktree => format!(
                    "Delete worktree {} and its branch {}? Unmerged commits will be lost.",
                    self.dir.clone().unwrap_or_default(),
                    branch.clone().unwrap_or_default()
                ),
                Confirm::Restore(cp) => {
                    let running = if exited {
                        ""
                    } else {
                        "\n\nThe agent is still running: stop it first so it does not rewrite files."
                    };
                    format!(
                        "Restore files as of {}? Changes made since then (including commits) will be undone. The current state is saved to a new restore point first.{running}",
                        when(cp.ts)
                    )
                }
            };
            let is_pr = matches!(c, Confirm::PullRequest { .. });
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .gap_2()
                .border_b_1()
                .border_color(line)
                .bg(hsla(p.raised))
                .px_3()
                .py_2()
                .text_xs()
                .child(text)
                .when(is_pr, |d| d.child(Input::new(&self.pr_title)))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("git-confirm-cancel")
                                .outline()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pending = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("git-confirm-ok")
                                .primary()
                                .label("OK")
                                .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                        ),
                )
        });

        // Left column.
        let weak = cx.entity().downgrade();
        let compare = div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .text_xs()
            .text_color(subtle)
            .child("Compare with…")
            .child(select(
                "git-compare",
                compare_label,
                std::iter::once((None, "Uncommitted changes (vs HEAD)".to_owned()))
                    .chain(others.iter().cloned())
                    .collect(),
                busy || no_dir,
                {
                    let weak = weak.clone();
                    move |id, window, cx| {
                        weak.update(cx, |this, cx| this.compare_with(id, window, cx)).ok();
                    }
                },
            ));

        let files_title = div()
            .flex()
            .items_center()
            .px_3()
            .pt_1()
            .pb_1()
            .text_xs()
            .text_color(subtle)
            .child(match &compared_name {
                Some(n) => format!("Differences with {n}"),
                None => "Changed files".into(),
            })
            .child(div().ml_auto().child(files.len().to_string()));

        let file_list = if files.is_empty() {
            div()
                .px_3()
                .pb_3()
                .text_xs()
                .text_color(subtle)
                .child(if status.is_some() { "No changes." } else { "Loading…" })
        } else {
            div().pb_2().children(files.iter().enumerate().map(|(i, f)| {
                let selected = self.selected.as_deref() == Some(f.path.as_str());
                let path = f.path.clone();
                div()
                    .id(("git-file", i))
                    .flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .h(ui(28.0))
                    .px_3()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(hsla(p.raised)).text_color(fg))
                    .when(!selected, |d| d.text_color(muted).hover(|s| s.bg(hsla(p.hover))))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_file(Some(path.clone()), cx)))
                    .child(div().w(ui(14.0)).flex_shrink_0().font_family(FONT_MONO).text_color(subtle).child(f.status.clone()))
                    .child(div().min_w_0().flex_1().truncate().child(f.path.clone()))
                    .child(match (f.additions, f.deletions) {
                        (Some(a), d) => div()
                            .flex()
                            .flex_shrink_0()
                            .gap_1()
                            .font_family(FONT_MONO)
                            .child(div().text_color(hsla(p.ok)).child(format!("+{a}")))
                            .child(div().text_color(hsla(p.bad)).child(format!("−{}", d.unwrap_or(0)))),
                        (None, _) => div().flex_shrink_0().text_color(subtle).child("bin"),
                    })
            }))
        };

        let has_files = status.is_some_and(|s| !s.files.is_empty());
        let message_empty = self.message.read(cx).value().trim().is_empty();
        let commit_section = self.comparison.is_none().then(|| {
            section("Commit", line, subtle)
                .child(Textarea::new(&self.message))
                .child(div().text_xs().text_color(subtle).child(
                    "Conventional Commits: feat, fix, refactor, docs, test, chore. Everything is staged (git add -A).",
                ))
                .child(
                    Button::new("git-commit")
                        .outline()
                        .icon(IconName::GitCommitHorizontal)
                        .label("Commit")
                        .disabled(busy || message_empty || !has_files)
                        .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
                )
        });

        let upstream_text = match &self.upstream {
            None => "…".to_owned(),
            Some(u) => match &u.name {
                Some(n) => format!("↑{} ↓{} vs {n} (as of the last fetch)", u.ahead, u.behind),
                None => "No upstream: Push publishes this branch to origin.".to_owned(),
            },
        };
        let has_upstream = self.upstream.as_ref().is_some_and(|u| u.name.is_some());
        let remote_section = self.comparison.is_none().then(|| {
            section("Remote", line, subtle)
                .child(div().text_xs().text_color(subtle).child(upstream_text))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("git-pull")
                                .outline()
                                .icon(IconName::ArrowDownToLine)
                                .label(if self.syncing == Some(SyncKind::Pull) { "Pulling…" } else { "Pull" })
                                .disabled(busy || !has_upstream)
                                .on_click(cx.listener(|this, _, window, cx| this.sync(SyncKind::Pull, window, cx))),
                        )
                        .child(
                            Button::new("git-push")
                                .outline()
                                .icon(IconName::ArrowUpFromLine)
                                .label(if self.syncing == Some(SyncKind::Push) { "Pushing…" } else { "Push" })
                                .disabled(busy || branch.is_none())
                                .on_click(cx.listener(|this, _, window, cx| this.sync(SyncKind::Push, window, cx))),
                        ),
                )
        });

        let base = status.and_then(|s| s.base.clone()).unwrap_or_else(|| "the main branch".into());
        let worktree_section = status.filter(|s| s.agent_worktree).map(|_| {
            section("Worktree", line, subtle)
                .child(
                    Button::new("git-merge")
                        .outline()
                        .icon(IconName::GitMerge)
                        .label(format!("Merge into {base}"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| this.ask(Confirm::Merge, window, cx))),
                )
                .child(
                    Button::new("git-pr")
                        .outline()
                        .icon(IconName::GitPullRequest)
                        .label("Open pull request")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let body = this.pr_body(cx);
                            this.ask(Confirm::PullRequest { body }, window, cx);
                        })),
                )
                .child(
                    Button::new("git-remove-worktree")
                        .outline()
                        .text_color(hsla(p.bad))
                        .icon(IconName::Trash)
                        .label("Delete worktree")
                        .disabled(busy || !exited)
                        .when(!exited, |b| b.tooltip("Stop the agent before deleting its worktree"))
                        .on_click(cx.listener(|this, _, window, cx| this.ask(Confirm::RemoveWorktree, window, cx))),
                )
        });

        let review_section = section("Auto review", line, subtle).child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .text_xs()
                .text_color(subtle)
                .child("When this agent finishes a task, ask another one to review its changes:")
                .child(select(
                    "git-reviewer",
                    reviewer_label,
                    std::iter::once((None, "Nobody".to_owned())).chain(others.iter().cloned()).collect(),
                    false,
                    {
                        let deck = self.deck.clone();
                        let author = self.pane_id.clone();
                        move |id, _, cx| {
                            deck.update(cx, |deck, cx| set_reviewer(deck, &author, id.as_deref(), cx));
                        }
                    },
                )),
        );

        let restore_section = section("Restore points", line, subtle)
            .child(
                Button::new("git-checkpoint")
                    .outline()
                    .icon(IconName::Clock)
                    .label("Create restore point")
                    .disabled(busy || no_dir)
                    .on_click(cx.listener(|this, _, window, cx| this.create_restore_point(window, cx))),
            )
            .child(if self.checkpoints.is_empty() {
                div().text_xs().text_color(subtle).child("No restore points for this agent.")
            } else {
                div().flex().flex_col().children(self.checkpoints.iter().enumerate().map(|(i, cp)| {
                    let checkpoint = cp.clone();
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(ui(28.0))
                        .child(div().text_color(muted).child(when(cp.ts)))
                        .child(
                            div()
                                .font_family(FONT_MONO)
                                .text_color(subtle)
                                .child(cp.sha.chars().take(7).collect::<String>()),
                        )
                        .child(
                            div().ml_auto().child(
                                Button::new(("git-restore", i))
                                    .ghost()
                                    .small()
                                    .label("Restore")
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.ask(Confirm::Restore(checkpoint.clone()), window, cx)
                                    })),
                            ),
                        )
                }))
            });

        let left = div()
            .id("git-left")
            .flex()
            .flex_col()
            .w(ui(320.0))
            .flex_shrink_0()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(line)
            .child(compare)
            .child(files_title)
            .child(file_list)
            .children(commit_section)
            .children(remote_section)
            .children(worktree_section)
            .child(review_section)
            .child(restore_section);

        // Diff view.
        let placeholder = |text: &'static str| div().p_4().text_xs().text_color(subtle).child(text);
        let right = div()
            .id("git-diff")
            .min_w_0()
            .flex_1()
            .overflow_scroll()
            .bg(hsla(p.base))
            .child(match (&self.selected, &self.diff) {
                (None, _) => placeholder("Select a file to see its diff."),
                (Some(_), None) => placeholder("Loading diff…"),
                (Some(_), Some(lines)) if lines.is_empty() => placeholder("No differences."),
                (Some(_), Some(lines)) => {
                    let left_over = lines.len().saturating_sub(self.shown);
                    div()
                        .py_2()
                        .font_family(FONT_MONO)
                        .text_xs()
                        .line_height(ui(20.0))
                        .children(lines.iter().take(self.shown).map(|(kind, text)| {
                            let (bg, color) = match kind {
                                DiffLineKind::Add => (Some(hsla_a(p.ok, 0.1)), hsla(p.ok)),
                                DiffLineKind::Del => (Some(hsla_a(p.bad, 0.1)), hsla(p.bad)),
                                DiffLineKind::Hunk | DiffLineKind::Meta => (None, subtle),
                                DiffLineKind::Context => (None, muted),
                            };
                            div()
                                .px_3()
                                .whitespace_nowrap()
                                .text_color(color)
                                .when_some(bg, |d, bg| d.bg(bg))
                                .child(if text.is_empty() { SharedString::from(" ") } else { text.clone() })
                        }))
                        .when(left_over > 0, |d| {
                            d.child(
                                div().m_3().child(
                                    Button::new("git-more")
                                        .outline()
                                        .label(format!("Show more ({left_over} lines left)"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.shown += DIFF_PAGE * 4;
                                            cx.notify();
                                        })),
                                ),
                            )
                        })
                }
            });

        let aside = div()
            .id("git-panel")
            .track_focus(&self.focus)
            .occlude()
            .relative()
            .flex()
            .flex_col()
            .h_full()
            .w(width)
            .border_l_1()
            .border_color(hsla(p.line_strong))
            .bg(hsla(p.panel))
            .text_size(ui(13.0))
            .text_color(fg)
            .child(header)
            .child(branch_row)
            .children(notice)
            .children(pending)
            .child(div().flex().min_h_0().flex_1().child(left).child(right));

        deferred(
            anchored().position(point(px(0.0), px(0.0))).child(
                div()
                    .id("git-overlay")
                    .occlude()
                    .flex()
                    .justify_end()
                    .w(viewport.width)
                    .h(viewport.height)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        if event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            if this.pending.take().is_none() {
                                this.close(cx);
                            }
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .id("git-backdrop")
                            .absolute()
                            .inset_0()
                            .bg(hsla_a(0x000000, 0.4))
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close(cx))),
                    )
                    .child(aside),
            ),
        )
        .with_priority(1)
    }
}
