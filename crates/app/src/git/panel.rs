//! Right slide-over with the Git state of one pane's working directory (its worktree when it has one).
//! Original: src/components/Git/GitPanel.tsx. Emits `DismissEvent` when closed.

use std::collections::BTreeSet;

use agentdeck_core::git::{
    Checkpoint, Comparison, FileChange, GitStatus, IndexStatus, PrStatus, Upstream, git_apply_patch, git_checkpoint,
    git_commit_staged, git_compare, git_create_pr, git_diff_index, git_diff_trees, git_index_status, git_list_checkpoints,
    git_merge_worktree, git_pane_dir, git_pull, git_push, git_remove_worktree, git_pr_status, git_restore_checkpoint,
    git_stage, git_status, git_unstage, git_upstream, hunk_changes, merge_conflict_prompt, select_patch,
};
use agentdeck_core::logic::deck::Pane;
use agentdeck_core::logic::text::agent_name;
use agentdeck_core::logic::theme::{FONT_MONO, FONT_SANS};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::checkpoints::{pane_git_dir, set_reviewer};
use super::chip::checks_label;
use super::diff::{Comment, DiffLineKind, classify_diff, diff_lines, line_numbers, review_prompt};
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};
use agentdeck_core::logic::theme::{RADIUS_MD};

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
    index: IndexStatus,
    checkpoints: Vec<Checkpoint>,
    upstream: Upstream,
}

fn load(dir: &str, pane_id: &str) -> Result<Snapshot, String> {
    Ok(Snapshot {
        status: git_status(dir.to_owned())?,
        index: git_index_status(dir.to_owned())?,
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
    /// Changes split into staged and unstaged (the "Staged" and "Changes" groups).
    index: Option<IndexStatus>,
    upstream: Option<Upstream>,
    checkpoints: Vec<Checkpoint>,
    /// Active comparison and the other pane's id.
    comparison: Option<(Comparison, String)>,
    selected: Option<String>,
    /// The selected file is shown from the Staged group (HEAD vs index), else from Changes.
    staged_view: bool,
    /// Change lines (indices in `diff`) picked for line staging.
    picked: BTreeSet<usize>,
    /// Last picked line: a shift-click picks the range from it.
    anchor: Option<usize>,
    /// Changed files list collapsed (long lists push the commit section out of view).
    files_collapsed: bool,
    /// Kind, text and file line number of each diff line.
    diff: Option<Vec<(DiffLineKind, SharedString, Option<u32>)>>,
    shown: usize,
    /// Review remarks on diff lines, sent to the agent in one prompt.
    comments: Vec<Comment>,
    /// Diff line being remarked on (index in `diff`).
    commenting: Option<usize>,
    comment: Entity<InputState>,
    /// Pull request of the branch and its checks; None: none, or no GitHub CLI.
    pr: Option<PrStatus>,
    notice: Option<(bool, String)>,
    busy: bool,
    /// A diff line's file is open in the editor: the panel hides until that page closes.
    editing: bool,
    syncing: Option<SyncKind>,
    pending: Option<Confirm>,
    message: Entity<TextareaState>,
    pr_title: Entity<InputState>,
    focus: FocusHandle,
    /// Last `deck_key`: the deck parts this panel renders.
    deck_key: DeckKey,
    _subscriptions: Vec<Subscription>,
}

/// Own pane exited, its reviewer, and every pane's id, name and agent (sorted).
type DeckKey = (bool, Option<String>, Vec<String>);

fn deck_key(deck: &Deck, pane_id: &str) -> DeckKey {
    let panes = &deck.state.panes;
    let mut names: Vec<String> = panes
        .values()
        .map(|p| format!("{}\0{}\0{}", p.id, pane_name(p), agent_name(p.agent)))
        .collect();
    names.sort();
    let exited = panes.get(pane_id).is_none_or(|p| p.exited);
    (exited, deck.reviews.get(pane_id).cloned(), names)
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
        let comment = cx.new(|cx| InputState::new(window, cx).placeholder("Remark for the agent (Enter to add)"));
        let subscriptions = vec![
            cx.subscribe(&message, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe_in(&pr_title, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.confirm(window, cx);
                }
            }),
            cx.subscribe_in(&comment, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.add_comment(window, cx);
                }
            }),
            // The deck changes every second while agents run: re-render only when what we show changed.
            cx.observe(&deck, |this: &mut Self, deck, cx| {
                let key = deck_key(deck.read(cx), &this.pane_id);
                if key != this.deck_key {
                    this.deck_key = key;
                    cx.notify();
                }
            }),
        ];
        let deck_key = deck_key(deck.read(cx), &pane_id);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let dir = pane_git_dir(deck.read(cx), &pane_id);
        let mut this = Self {
            deck,
            pane_id,
            dir,
            status: None,
            index: None,
            upstream: None,
            checkpoints: Vec::new(),
            comparison: None,
            selected: None,
            staged_view: false,
            picked: BTreeSet::new(),
            anchor: None,
            files_collapsed: false,
            diff: None,
            shown: DIFF_PAGE,
            comments: Vec::new(),
            commenting: None,
            comment,
            pr: None,
            notice: None,
            busy: false,
            editing: false,
            syncing: None,
            pending: None,
            message,
            pr_title,
            focus,
            deck_key,
            _subscriptions: subscriptions,
        };
        if this.dir.is_none() {
            this.notice = Some((false, "This pane no longer exists.".into()));
        }
        this.refresh(cx);
        this.load_pr(cx);
        this
    }

    fn pane(&self, cx: &App) -> Option<Pane> {
        self.deck.read(cx).state.panes.get(&self.pane_id).cloned()
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn apply(&mut self, snapshot: Snapshot) {
        // The selected file left its group (fully staged or unstaged): follow it to the other one.
        if let Some(path) = &self.selected {
            let has = |files: &[FileChange]| files.iter().any(|f| &f.path == path);
            let index = &snapshot.index;
            let (here, there) =
                if self.staged_view { (&index.staged, &index.unstaged) } else { (&index.unstaged, &index.staged) };
            if !has(here) && has(there) {
                self.staged_view = !self.staged_view;
            }
        }
        self.status = Some(snapshot.status);
        self.index = Some(snapshot.index);
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

    /// Shows a file's diff: staged (HEAD vs index) or unstaged (index vs working tree) per
    /// `staged_view`, or between the compared panes.
    fn open_file(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        self.selected = path.clone();
        self.commenting = None;
        self.diff = None;
        self.picked.clear();
        self.anchor = None;
        self.shown = DIFF_PAGE;
        cx.notify();
        let (Some(path), Some(dir)) = (path, self.dir.clone()) else { return };
        let trees = self.comparison.as_ref().map(|(c, _)| (c.from.clone(), c.to.clone()));
        let staged = self.staged_view;
        let file = path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let text = match trees {
                        Some((from, to)) => git_diff_trees(dir, from, to, file),
                        None => git_diff_index(dir, file, staged),
                    }?;
                    // Split and classify here: a 100k-line lockfile diff would freeze the window.
                    let lines = diff_lines(&text);
                    let kinds = classify_diff(&lines);
                    let numbers = line_numbers(&lines, &kinds);
                    Ok::<Vec<_>, String>(
                        kinds
                            .into_iter()
                            .zip(lines)
                            .zip(numbers)
                            .map(|((k, l), n)| (k, SharedString::from(l.to_owned()), n))
                            .collect(),
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(diff) if this.selected.as_deref() == Some(path.as_str()) => {
                        this.diff = Some(diff);
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
        self.load_pr(cx);
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

    /// Commits the index; with nothing staged the button reads "Stage all and commit".
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.message.read(cx).value().to_string();
        let stage_all = self.index.as_ref().is_some_and(|i| i.staged.is_empty());
        self.act(
            move |dir| {
                if stage_all {
                    git_stage(dir.clone(), Vec::new())?;
                }
                git_commit_staged(dir, message)
            },
            |this, sha, window, cx| {
                this.message.update(cx, |m, cx| m.set_value("", window, cx));
                Some(format!("Created commit {sha}."))
            },
            window,
            cx,
        );
    }

    /// Stages or unstages whole files; every change when `paths` is empty.
    fn stage_files(&mut self, paths: Vec<String>, stage: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.act(
            move |dir| if stage { git_stage(dir, paths) } else { git_unstage(dir, paths) },
            |_, (), _, _| None,
            window,
            cx,
        );
    }

    /// Stages (Changes view) or unstages (Staged view) these change lines of the shown diff.
    fn stage_lines(&mut self, lines: &BTreeSet<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let reverse = self.staged_view;
        let patch = self.diff.as_ref().and_then(|diff| {
            let text: Vec<&str> = diff.iter().map(|(_, t, _)| t.as_ref()).collect();
            select_patch(&text, lines, reverse)
        });
        let Some(patch) = patch.filter(|_| !self.busy) else { return };
        self.act(move |dir| git_apply_patch(dir, patch, reverse), |_, (), _, _| None, window, cx);
    }

    /// Stages or unstages the hunk whose header is diff line `header`.
    fn stage_hunk(&mut self, header: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(diff) = &self.diff else { return };
        let text: Vec<&str> = diff.iter().map(|(_, t, _)| t.as_ref()).collect();
        let lines: BTreeSet<usize> = hunk_changes(&text, header).into_iter().collect();
        self.stage_lines(&lines, window, cx);
    }

    /// Picks or unpicks a change line; with shift, picks every change line from the last pick.
    fn pick_line(&mut self, index: usize, range: bool, cx: &mut Context<Self>) {
        let Some(diff) = &self.diff else { return };
        match self.anchor.filter(|_| range) {
            Some(anchor) => {
                let changes = (anchor.min(index)..=anchor.max(index))
                    .filter(|&i| matches!(diff.get(i), Some((DiffLineKind::Add | DiffLineKind::Del, _, _))));
                self.picked.extend(changes);
            }
            None => {
                if !self.picked.remove(&index) {
                    self.picked.insert(index);
                }
            }
        }
        self.anchor = Some(index);
        cx.notify();
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
        // Two git jobs at once in one repo (restore during a pull) race on the index.
        if self.busy {
            return;
        }
        let Some(pending) = self.pending.take() else { return };
        self.focus.focus(window, cx);
        match pending {
            Confirm::Merge => {
                let check = self.deck.read(cx).settings.merge_check.clone();
                self.act(move |dir| git_merge_worktree(dir, check), |_, out, _, _| Some(out), window, cx)
            }
            Confirm::PullRequest { body } => {
                let title = self.pr_title.read(cx).value().trim().to_owned();
                if title.is_empty() {
                    return;
                }
                self.act(
                    move |dir| git_create_pr(dir, title, body),
                    |this, url, _, cx| {
                        this.load_pr(cx);
                        Some(format!("Pull request opened: {url}"))
                    },
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

    /// Loads the branch's pull request and the state of its checks.
    fn load_pr(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.dir.clone() else { return };
        cx.spawn(async move |this, cx| {
            let pr = cx.background_spawn(async move { git_pr_status(dir) }).await;
            this.update(cx, |this, cx| {
                this.pr = pr;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Opens the file of a diff line in the code editor, cursor on that line; the review comes
    /// back when the editor closes (see `render`).
    fn edit_at(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(dir), Some(path)) = (self.dir.clone(), self.selected.clone()) else { return };
        let Some(number) = self.diff.as_ref().and_then(|diff| diff.get(index)).and_then(|(_, _, n)| *n) else {
            return;
        };
        let root = std::path::PathBuf::from(dir);
        let file = root.join(&path);
        self.editing = true;
        self.commenting = None;
        crate::skills::open_file_at(self.deck.clone(), root, file, number.saturating_sub(1), window, cx);
        cx.notify();
    }

    /// Opens the remark field under a diff line.
    fn start_comment(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.commenting = Some(index);
        self.comment.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn add_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.comment.read(cx).value().trim().to_owned();
        let (Some(index), Some(path)) = (self.commenting.take(), self.selected.clone()) else { return };
        let line = self.diff.as_ref().and_then(|diff| diff.get(index)).filter(|_| !text.is_empty());
        if let Some((_, code, number)) = line {
            self.comments.push(Comment { path, index, line: *number, code: code.to_string(), text });
        }
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Sends every remark to the agent as one prompt.
    fn send_comments(&mut self, cx: &mut Context<Self>) {
        let count = self.comments.len();
        crate::panes::runtime::send_prompt(&self.pane_id, &review_prompt(&self.comments), cx);
        self.comments.clear();
        let plural = if count == 1 { "" } else { "s" };
        self.notice = Some((true, format!("Sent {count} remark{plural} to the agent.")));
        cx.notify();
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
        if self.editing {
            if crate::skills::review_file_open(cx) {
                return div().into_any_element();
            }
            // The editor closed: back to the review, with the diff of what may have been saved.
            self.editing = false;
            self.focus.focus(window, cx);
            cx.defer_in(window, |this, _, cx| {
                this.refresh(cx);
                this.open_file(this.selected.clone(), cx);
            });
        }
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
                crate::panes::close_button("git-close", "Close (Esc)").on_click({
                    let this = cx.entity().downgrade();
                    move |_, cx| {
                        let _ = this.update(cx, |this, cx| this.close(cx));
                    }
                }),
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

        // Merge refused for conflicts: the worktree's agent can merge the target in and resolve them.
        let fix_prompt = self.notice.as_ref().filter(|(ok, _)| !ok).and_then(|(_, text)| merge_conflict_prompt(text));
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
                .when_some(fix_prompt.clone(), |d, prompt| {
                    d.child(
                        Button::new("git-fix-conflicts")
                            .mt_2()
                            .primary()
                            .label("Ask the agent to resolve")
                            .tooltip("Sends the agent a prompt: merge the target into its worktree, fix the conflicts, commit")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                crate::panes::runtime::send_prompt(&this.pane_id, &prompt, cx);
                                this.notice = Some((true, "Sent to the agent. Merge again once it is done.".into()));
                                cx.notify();
                            })),
                    )
                })
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
                                .disabled(self.busy)
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

        let collapsed = self.files_collapsed;
        let files_title = div()
            .id("git-files-toggle")
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .pt_1()
            .pb_1()
            .text_xs()
            .text_color(subtle)
            .cursor_pointer()
            .hover(|s| s.text_color(muted))
            .on_click(cx.listener(|this, _, _, cx| {
                this.files_collapsed = !this.files_collapsed;
                cx.notify();
            }))
            .child(icon(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown }, subtle).size(ui(12.0)))
            .child(match &compared_name {
                Some(n) => format!("Differences with {n}"),
                None => "Changed files".into(),
            })
            .child(div().ml_auto().child(files.len().to_string()));

        // One changed file; `group`: Some(staged) in the Staged / Changes groups, None in a comparison.
        let file_row = |key: &'static str, i: usize, f: &FileChange, group: Option<bool>| {
            let selected = self.selected.as_deref() == Some(f.path.as_str())
                && group.is_none_or(|staged| staged == self.staged_view);
            let path = f.path.clone();
            div()
                .id((key, i))
                .flex()
                .w_full()
                .items_center()
                .gap_2()
                .h(ui(28.0))
                .px_3()
                .cursor_pointer()
                .when(selected, |d| d.bg(hsla(p.raised)).text_color(fg))
                .when(!selected, |d| d.text_color(muted).hover(|s| s.bg(hsla(p.hover))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(staged) = group {
                        this.staged_view = staged;
                    }
                    this.open_file(Some(path.clone()), cx)
                }))
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
                .when_some(group, |d, staged| {
                    let path = f.path.clone();
                    d.child(
                        Button::new((if staged { "git-unstage" } else { "git-stage" }, i))
                            .ghost()
                            .xsmall()
                            .icon(if staged { IconName::Minus } else { IconName::Plus })
                            .tooltip(if staged { "Unstage" } else { "Stage" })
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.stage_files(vec![path.clone()], !staged, window, cx);
                            })),
                    )
                })
        };
        let empty = |text: &'static str| div().px_3().pb_2().text_xs().text_color(subtle).child(text);
        let group_title = |title: &'static str, count: usize, staged: bool| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(ui(28.0))
                .px_3()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(subtle)
                .child(format!("{title} ({count})"))
                .child(
                    div().ml_auto().child(
                        Button::new(if staged { "git-unstage-all" } else { "git-stage-all" })
                            .ghost()
                            .small()
                            .label(if staged { "Unstage all" } else { "Stage all" })
                            .disabled(busy || count == 0)
                            .on_click(cx.listener(move |this, _, window, cx| this.stage_files(Vec::new(), !staged, window, cx))),
                    ),
                )
        };
        let file_list = match (&self.comparison, &self.index) {
            (Some(_), _) if files.is_empty() => empty("No changes."),
            (Some(_), _) => div().pb_2().children(files.iter().enumerate().map(|(i, f)| file_row("git-file", i, f, None))),
            (None, None) => empty("Loading…"),
            (None, Some(index)) => div()
                .pb_2()
                .child(group_title("Staged", index.staged.len(), true))
                .children(index.staged.iter().enumerate().map(|(i, f)| file_row("git-staged", i, f, Some(true))))
                .when(index.staged.is_empty(), |d| d.child(empty("Nothing staged.")))
                .child(group_title("Changes", index.unstaged.len(), false))
                .children(index.unstaged.iter().enumerate().map(|(i, f)| file_row("git-unstaged", i, f, Some(false))))
                .when(index.unstaged.is_empty(), |d| d.child(empty("No changes."))),
        };

        let has_files = status.is_some_and(|s| !s.files.is_empty());
        let message_empty = self.message.read(cx).value().trim().is_empty();
        // What the commit will contain.
        let staged = self.index.as_ref().map(|i| i.staged.as_slice()).unwrap_or_default();
        let (adds, dels) = staged
            .iter()
            .fold((0, 0), |(a, d), f| (a + f.additions.unwrap_or(0), d + f.deletions.unwrap_or(0)));
        let staged_summary = match staged.len() {
            0 => "Nothing staged: commit stages every change first.".to_owned(),
            n => format!("{n} file{}, +{adds} −{dels} staged", if n == 1 { "" } else { "s" }),
        };
        let stage_all = staged.is_empty();
        let commit_section = self.comparison.is_none().then(|| {
            section("Commit", line, subtle)
                .child(Textarea::new(&self.message))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if stage_all { subtle } else { fg })
                        .child(staged_summary),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(subtle)
                        .child("Conventional Commits: feat, fix, refactor, docs, test, chore. Only staged changes are committed."),
                )
                .child(
                    Button::new("git-commit")
                        .outline()
                        .icon(IconName::GitCommitHorizontal)
                        .label(if stage_all { "Stage all and commit" } else { "Commit" })
                        .disabled(busy || message_empty || self.index.is_none() || (stage_all && !has_files))
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
        let pr_row = self.pr.clone().map(|pr| {
            let (label, color) = checks_label(&pr, &p);
            let url = pr.url.clone();
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .child(
                    Button::new("git-pr-open")
                        .ghost()
                        .icon(IconName::GitPullRequest)
                        .label(format!("PR #{}", pr.number))
                        .tooltip("Open the pull request in the browser")
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
                .child(div().size(ui(6.0)).flex_shrink_0().rounded_full().bg(hsla(color)))
                .child(div().text_color(hsla(color)).child(label))
        });
        let remote_section = self.comparison.is_none().then(|| {
            section("Remote", line, subtle)
                .child(div().text_xs().text_color(subtle).child(upstream_text))
                .children(pr_row)
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
            .when(!collapsed, |d| d.child(file_list))
            .children(commit_section)
            .children(remote_section)
            .children(worktree_section)
            .child(review_section)
            .child(restore_section);

        // Diff view.
        let placeholder = |text: &'static str| div().p_4().text_xs().text_color(subtle).child(text);
        let remarks = self.comments.len();
        // Against HEAD only: a comparison's lines belong to the other pane's tree.
        let can_edit = self.comparison.is_none() && !no_dir;
        let can_stage = can_edit && self.index.is_some();
        let staged_view = self.staged_view;
        let accent = hsla(p.accent);
        let picked = self.picked.len();
        let picked_bar = (can_stage && picked > 0).then(|| {
            div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(line)
                .bg(hsla(p.raised))
                .px_3()
                .py_2()
                .text_xs()
                .child(div().flex_1().child(format!(
                    "{picked} line{} selected (shift-click a box to select a range)",
                    if picked == 1 { "" } else { "s" }
                )))
                .child(Button::new("git-picked-clear").ghost().label("Clear").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.picked.clear();
                        this.anchor = None;
                        cx.notify();
                    },
                )))
                .child(
                    Button::new("git-picked-stage")
                        .primary()
                        .label(if staged_view { "Unstage lines" } else { "Stage lines" })
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let lines = this.picked.clone();
                            this.stage_lines(&lines, window, cx);
                        })),
                )
        });
        let remarks_bar = (remarks > 0).then(|| {
            div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(line)
                .bg(hsla(p.raised))
                .px_3()
                .py_2()
                .text_xs()
                .child(div().flex_1().child(format!("{remarks} remark{} on the diff", if remarks == 1 { "" } else { "s" })))
                .child(Button::new("git-remarks-clear").ghost().label("Clear").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.comments.clear();
                        cx.notify();
                    },
                )))
                .child(
                    Button::new("git-remarks-send")
                        .primary()
                        .label("Send to the agent")
                        .disabled(exited)
                        .on_click(cx.listener(|this, _, _, cx| this.send_comments(cx))),
                )
        });
        let right = div()
            .id("git-diff")
            .min_w_0()
            .min_h_0()
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
                        .children(lines.iter().take(self.shown).enumerate().flat_map(|(i, (kind, text, number))| {
                            let (bg, color) = match kind {
                                DiffLineKind::Add => (Some(hsla_a(p.ok, 0.1)), hsla(p.ok)),
                                DiffLineKind::Del => (Some(hsla_a(p.bad, 0.1)), hsla(p.bad)),
                                DiffLineKind::Hunk | DiffLineKind::Meta => (None, subtle),
                                DiffLineKind::Context => (None, muted),
                            };
                            // A click on a line of code opens a remark for the agent under it.
                            let code = matches!(kind, DiffLineKind::Add | DiffLineKind::Del | DiffLineKind::Context);
                            let change = matches!(kind, DiffLineKind::Add | DiffLineKind::Del);
                            let is_picked = self.picked.contains(&i);
                            // Box to pick a change line for line staging (shift: a range).
                            let pick = can_stage.then(|| {
                                div()
                                    .id(("diff-pick", i))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(ui(20.0))
                                    .flex_shrink_0()
                                    .when(change, |d| {
                                        d.cursor_pointer()
                                            .tooltip(|window, cx| {
                                                Tooltip::new("Select this line (shift-click: range)").build(window, cx)
                                            })
                                            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                                                cx.stop_propagation();
                                                this.pick_line(i, event.modifiers().shift, cx);
                                            }))
                                            .child(if is_picked {
                                                icon(IconName::SquareCheck, accent).size(ui(12.0))
                                            } else {
                                                icon(IconName::Square, subtle).size(ui(12.0))
                                            })
                                    })
                            });
                            let hunk_button = (can_stage && *kind == DiffLineKind::Hunk).then(|| {
                                Button::new(("git-hunk", i))
                                    .ghost()
                                    .small()
                                    .ml_3()
                                    .label(if staged_view { "Unstage hunk" } else { "Stage hunk" })
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.stage_hunk(i, window, cx);
                                    }))
                            });
                            let row = div()
                                .id(("diff-line", i))
                                .group("diff-row")
                                .flex()
                                .pr_3()
                                .whitespace_nowrap()
                                .text_color(color)
                                .when_some(bg, |d, bg| d.bg(bg))
                                .when(code, |d| {
                                    d.cursor_pointer()
                                        .hover(|s| s.bg(hsla(p.hover)))
                                        .on_click(cx.listener(move |this, _, window, cx| this.start_comment(i, window, cx)))
                                })
                                .when(is_picked, |d| d.bg(hsla_a(p.accent, 0.15)))
                                .children(pick)
                                // File line number (the old file's for a deleted line); a click on
                                // it opens the file in the editor at that line.
                                .child(
                                    div()
                                        .id(("diff-goto", i))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .w(ui(48.0))
                                        .flex_shrink_0()
                                        .pl_1()
                                        .pr_2()
                                        .text_color(subtle)
                                        .when(can_edit && number.is_some(), |d| {
                                            d.cursor_pointer()
                                                .hover(|s| s.text_color(accent))
                                                .tooltip(|window, cx| {
                                                    Tooltip::new("Open in the editor at this line").build(window, cx)
                                                })
                                                .on_click(cx.listener(move |this, _, window, cx| {
                                                    cx.stop_propagation();
                                                    this.edit_at(i, window, cx);
                                                }))
                                                .child(
                                                    div()
                                                        .opacity(0.0)
                                                        .group_hover("diff-row", |s| s.opacity(1.0))
                                                        .child(icon(IconName::Pencil, accent).size(ui(11.0))),
                                                )
                                        })
                                        .child(div().ml_auto().child(number.map_or_else(String::new, |n| n.to_string()))),
                                )
                                .child(if text.is_empty() { SharedString::from(" ") } else { text.clone() })
                                .children(hunk_button)
                                .into_any_element();
                            let here = |c: &&Comment| c.index == i && Some(&c.path) == self.selected.as_ref();
                            let remarks = self.comments.iter().enumerate().filter(|(_, c)| here(c)).map(|(n, c)| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .mx_3()
                                    .my_1()
                                    .rounded(ui(RADIUS_MD))
                                    .border_l_2()
                                    .border_color(hsla(p.accent))
                                    .bg(hsla(p.raised))
                                    .px_2()
                                    .py_1()
                                    .font_family(FONT_SANS)
                                    .text_color(fg)
                                    .whitespace_normal()
                                    .child(div().min_w_0().flex_1().child(c.text.clone()))
                                    .child(
                                        Button::new(("git-remark-remove", n))
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::X)
                                            .tooltip("Remove this remark")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if n < this.comments.len() {
                                                    this.comments.remove(n);
                                                }
                                                cx.notify();
                                            })),
                                    )
                                    .into_any_element()
                            });
                            let field = (self.commenting == Some(i)).then(|| {
                                div().mx_3().my_1().font_family(FONT_SANS).child(Input::new(&self.comment)).into_any_element()
                            });
                            std::iter::once(row).chain(remarks).chain(field).collect::<Vec<_>>()
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
            .child(
                div()
                    .flex()
                    .min_h_0()
                    .flex_1()
                    .child(left)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .flex_1()
                            .children(remarks_bar)
                            .children(picked_bar)
                            .child(right),
                    ),
            );

        crate::browser::popup_shown();
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
                            // Esc backs out of the innermost thing: a remark, a confirmation, the panel.
                            if this.commenting.take().is_none() && this.pending.take().is_none() {
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
        .into_any_element()
    }
}
