//! "+12 -3 · 4" in the pane header: uncommitted changes in the agent's folder, hidden when clean or
//! not a repo. Clicking it opens the Git panel, which this entity also hosts.
//! Original: src/components/Git/DiffChip.tsx.

use std::time::Duration;

use agentdeck_core::git::{PrStatus, git_pane_dir, git_pr_status, git_status, git_upstream};
use agentdeck_core::logic::theme::Palette;
use agentdeck_core::hooks_server::PaneStatus;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

use super::panel::GitPanel;
use crate::state::Deck;
use crate::theme::{hsla, palette, ui};

/// One git status costs about ten git processes: refresh once things settle, not on every tool call.
const REFRESH_DELAY: Duration = Duration::from_millis(2500);
/// While a pull request's checks run, its state is read again this often.
const PR_POLL: Duration = Duration::from_secs(60);

/// "2 checks failed" and its color: red when one failed, blue while some run, green when all passed.
pub(super) fn checks_label(pr: &PrStatus, p: &Palette) -> (String, u32) {
    let count = |n: u32, what: &str| format!("{n} check{} {what}", if n == 1 { "" } else { "s" });
    if pr.state != "OPEN" {
        (pr.state.to_lowercase(), p.subtle)
    } else if pr.failed > 0 {
        (count(pr.failed, "failed"), p.bad)
    } else if pr.pending > 0 {
        (count(pr.pending, "running"), p.busy)
    } else if pr.passed > 0 {
        (count(pr.passed, "passed"), p.ok)
    } else {
        ("no checks".to_owned(), p.subtle)
    }
}

#[derive(Clone, Copy)]
struct DiffCount {
    files: usize,
    additions: u32,
    deletions: u32,
}

/// What triggers a refresh: (cwd, worktree, exited, status, completed count).
type Key = (String, bool, bool, PaneStatus, u32);

pub struct DiffChip {
    deck: Entity<Deck>,
    pane_id: String,
    diff: Option<DiffCount>,
    /// Big folders not in `.gitignore` (see `GitStatus::huge_untracked`).
    huge_untracked: Vec<String>,
    /// Pull request of the agent's worktree branch, if it has one.
    pr: Option<PrStatus>,
    key: Option<Key>,
    panel: Option<Entity<GitPanel>>,
    _refresh: Option<Task<()>>,
    _panel_closed: Option<Subscription>,
    _observe: Subscription,
}

impl DiffChip {
    pub fn new(deck: Entity<Deck>, pane_id: impl Into<String>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&deck, |this, _, cx| this.schedule(cx));
        let mut this = Self {
            deck,
            pane_id: pane_id.into(),
            diff: None,
            huge_untracked: Vec::new(),
            pr: None,
            key: None,
            panel: None,
            _refresh: None,
            _panel_closed: None,
            _observe: observe,
        };
        this.schedule(cx);
        this
    }

    /// Opens the Git panel of this pane (also used by the pane menu).
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = cx.new(|cx| GitPanel::new(self.deck.clone(), self.pane_id.clone(), window, cx));
        self._panel_closed = Some(cx.subscribe(&panel, |this, _, _: &DismissEvent, cx| {
            this.panel = None;
            this._panel_closed = None;
            cx.notify();
        }));
        self.panel = Some(panel);
        cx.notify();
    }

    pub fn is_open(&self) -> bool {
        self.panel.is_some()
    }

    /// Refreshes the counts `REFRESH_DELAY` after the pane's status or folder changed.
    fn schedule(&mut self, cx: &mut Context<Self>) {
        let Some(pane) = self.deck.read(cx).state.panes.get(&self.pane_id) else {
            return;
        };
        let key: Key = (pane.cwd.clone(), pane.worktree, pane.exited, pane.status, pane.completed_count);
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.key = Some(key);
        if pane.exited {
            self._refresh = None;
            return;
        }
        let worktree = pane.worktree;
        let dir = git_pane_dir(pane.cwd.clone(), pane.id.clone(), worktree);
        self._refresh = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFRESH_DELAY).await;
            let status_dir = dir.clone();
            let (diff, huge, pushed) = cx
                .background_spawn(async move {
                    // Err: not a git repo.
                    let status = git_status(status_dir.clone()).ok();
                    let diff = status.as_ref().map(|s| DiffCount {
                        files: s.files.len(),
                        additions: s.files.iter().filter_map(|f| f.additions).sum(),
                        deletions: s.files.iter().filter_map(|f| f.deletions).sum(),
                    });
                    let huge = status.map(|s| s.huge_untracked).unwrap_or_default();
                    // A branch never pushed has no pull request: skip the gh call.
                    let pushed = worktree && git_upstream(status_dir).is_ok_and(|u| u.name.is_some());
                    (diff, huge, pushed)
                })
                .await;
            this.update(cx, |this, cx| {
                this.diff = diff;
                this.huge_untracked = huge;
                cx.notify();
            })
            .ok();
            // Pull requests are opened from agent worktrees: the others are not asked about.
            while pushed {
                let pr_dir = dir.clone();
                let pr = cx.background_spawn(async move { git_pr_status(pr_dir) }).await;
                let running = pr.as_ref().is_some_and(|pr| pr.state == "OPEN" && pr.pending > 0);
                let alive = this.update(cx, |this, cx| {
                    this.pr = pr;
                    cx.notify();
                });
                if alive.is_err() || !running {
                    break;
                }
                cx.background_executor().timer(PR_POLL).await;
            }
        }));
    }
}

impl Render for DiffChip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let chip = self.diff.filter(|d| d.files > 0).map(|d| {
            let plural = if d.files > 1 { "s" } else { "" };
            let label = format!(
                "{} changed file{plural}, {} added and {} removed lines",
                d.files, d.additions, d.deletions
            );
            let tip = SharedString::from(format!("{label}. Open the Git panel"));
            div()
                .id("diff-chip")
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_1()
                .rounded(ui(4.0))
                .px_1()
                .font_family(agentdeck_core::logic::theme::FONT_MONO)
                .text_size(ui(11.0))
                .text_color(hsla(p.subtle))
                .cursor_pointer()
                .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.open(window, cx);
                }))
                .child(div().text_color(hsla(p.ok)).child(format!("+{}", d.additions)))
                .child(div().text_color(hsla(p.bad)).child(format!("-{}", d.deletions)))
                .child(format!("· {}", d.files))
        });
        let pr_chip = self.pr.clone().map(|pr| {
            let (label, color) = checks_label(&pr, &p);
            let tip = SharedString::from(format!("Pull request #{}: {label}. Open it in the browser", pr.number));
            div()
                .id("pr-chip")
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_1()
                .rounded(ui(4.0))
                .px_1()
                .text_size(ui(11.0))
                .text_color(hsla(p.subtle))
                .cursor_pointer()
                .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().size(ui(6.0)).rounded_full().bg(hsla(color)))
                .child(format!("PR #{}", pr.number))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    cx.open_url(&pr.url);
                })
        });
        let huge_chip = (!self.huge_untracked.is_empty()).then(|| {
            let tip = SharedString::from(format!(
                "{} is not in .gitignore (slow): left out of the changes",
                self.huge_untracked.join(", ")
            ));
            div()
                .id("huge-untracked-chip")
                .flex_shrink_0()
                .px_1()
                .text_size(ui(11.0))
                .text_color(hsla(p.wait))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .child("⚠")
        });
        div()
            .flex()
            .children(huge_chip)
            .children(chip)
            .children(pr_chip)
            .children(self.panel.clone())
    }
}
