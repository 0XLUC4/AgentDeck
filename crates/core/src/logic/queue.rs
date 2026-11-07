//! Per-pane prompt queue (`src/store/queue.ts`), code-then-review pairs (`src/store/review.ts`)
//! and how a prompt is typed into a terminal (`components/Broadcast/send.ts`). In memory only.

use std::collections::HashMap;

use super::deck::Pane;
use crate::agents::AgentKind;
use crate::hooks_server::PaneStatus;

/// Delay between a pane finishing and its next queued prompt (or review request).
pub const ADVANCE_DELAY_MS: u64 = 1500;
/// Agent TUIs read text + Enter arriving in one write as a paste: Enter goes separately after this.
pub const SUBMIT_DELAY_MS: u64 = 120;
/// Only agents with hooks report "done"; the others never advance a queue.
pub const QUEUE_AGENTS: [AgentKind; 2] = [AgentKind::Claude, AgentKind::Codex];

#[derive(Debug, Clone, PartialEq)]
pub struct QueueItem {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaneQueue {
    pub items: Vec<QueueItem>,
    pub paused: bool,
}

/// Text as the agent should receive it: multiline text goes as one bracketed paste.
pub fn prompt_text(text: &str) -> String {
    let body = text.replace("\r\n", "\n").replace('\r', "\n");
    if body.contains('\n') {
        format!("\x1b[200~{body}\x1b[201~")
    } else {
        body
    }
}

/// True when a pane just finished and its queue has something to send.
pub fn should_advance(prev: Option<PaneStatus>, next: PaneStatus, queue: Option<&PaneQueue>) -> bool {
    prev != Some(PaneStatus::Done)
        && next == PaneStatus::Done
        && queue.is_some_and(|q| !q.paused && !q.items.is_empty())
}

/// After the delay: still alive and not busy again.
pub fn ready_for_next(pane: Option<&Pane>) -> bool {
    pane.is_some_and(|p| !p.exited && matches!(p.status, PaneStatus::Done | PaneStatus::Idle))
}

/// Moves the item at `index` by `delta` places (clamped).
pub fn move_item<T>(items: &mut Vec<T>, index: usize, delta: isize) {
    if index >= items.len() {
        return;
    }
    let to = (index as isize + delta).clamp(0, items.len() as isize - 1) as usize;
    if to != index {
        let item = items.remove(index);
        items.insert(to, item);
    }
}

/// All queues, by pane id.
#[derive(Debug, Clone, Default)]
pub struct Queues(pub HashMap<String, PaneQueue>);

impl Queues {
    pub fn get(&self, pane_id: &str) -> Option<&PaneQueue> {
        self.0.get(pane_id)
    }

    pub fn add(&mut self, pane_id: &str, text: &str) {
        let text = text.trim();
        if !text.is_empty() {
            self.0.entry(pane_id.to_owned()).or_default().items.push(QueueItem {
                id: super::deck::new_id(),
                text: text.to_owned(),
            });
        }
    }

    pub fn remove(&mut self, pane_id: &str, item_id: &str) {
        if let Some(q) = self.0.get_mut(pane_id) {
            q.items.retain(|i| i.id != item_id);
        }
    }

    pub fn move_by(&mut self, pane_id: &str, item_id: &str, delta: isize) {
        if let Some(q) = self.0.get_mut(pane_id)
            && let Some(i) = q.items.iter().position(|i| i.id == item_id)
        {
            move_item(&mut q.items, i, delta);
        }
    }

    pub fn set_paused(&mut self, pane_id: &str, paused: bool) {
        self.0.entry(pane_id.to_owned()).or_default().paused = paused;
    }

    pub fn forget(&mut self, pane_id: &str) {
        self.0.remove(pane_id);
    }

    /// Takes the next item to send (put it back with [`Queues::requeue_front`] if the write fails).
    pub fn pop_next(&mut self, pane_id: &str) -> Option<QueueItem> {
        let q = self.0.get_mut(pane_id)?;
        (!q.items.is_empty()).then(|| q.items.remove(0))
    }

    pub fn requeue_front(&mut self, pane_id: &str, item: QueueItem) {
        self.0.entry(pane_id.to_owned()).or_default().items.insert(0, item);
    }
}

/// True when making `reviewer` review `author` would close a loop (A reviews B reviews A...).
pub fn creates_loop(map: &HashMap<String, String>, author: &str, reviewer: &str) -> bool {
    let mut seen = vec![author.to_owned()];
    let mut id = Some(reviewer.to_owned());
    while let Some(current) = id {
        if seen.contains(&current) {
            return true;
        }
        id = map.get(&current).cloned();
        seen.push(current);
    }
    false
}

/// Prompt asking a reviewer to review `author`'s changes in `dir`.
pub fn review_prompt(author_title: &str, author_agent: AgentKind, dir: &str, task: Option<&str>) -> String {
    let name = format!("{author_title} ({})", super::text::agent_name(author_agent));
    let task = task.map(|t| {
        let flat = t.split_whitespace().collect::<Vec<_>>().join(" ");
        format!("Its task was: \"{}\".", flat.chars().take(500).collect::<String>())
    });
    [
        Some(format!("Review the work {name} just finished in {dir}.")),
        task,
        Some(format!(
            "See its changes with `git -C \"{dir}\" status` and `git -C \"{dir}\" diff HEAD` (untracked files too)."
        )),
        Some("Report bugs, risks and missing tests, most serious first. Do not edit those files.".into()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
}

/// Recent prompts (newest first, max 30, shorter than 2000 chars) for reuse.
pub const MAX_RECENT_PROMPTS: usize = 30;
const MAX_RECENT_LENGTH: usize = 2000;

pub fn remember_prompt(list: &mut Vec<String>, prompt: &str) -> bool {
    let text = prompt.trim();
    if text.is_empty() || text.chars().count() > MAX_RECENT_LENGTH {
        return false;
    }
    list.retain(|p| p != text);
    list.insert(0, text.to_owned());
    list.truncate(MAX_RECENT_PROMPTS);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::deck::{PaneSpec, new_pane};

    fn q(n: usize, paused: bool) -> PaneQueue {
        PaneQueue {
            items: (0..n)
                .map(|i| QueueItem {
                    id: i.to_string(),
                    text: format!("p{i}"),
                })
                .collect(),
            paused,
        }
    }

    #[test]
    fn advance_rules() {
        use PaneStatus::*;
        assert!(should_advance(Some(Working), Done, Some(&q(1, false))));
        assert!(should_advance(None, Done, Some(&q(1, false))));
        assert!(!should_advance(Some(Done), Done, Some(&q(1, false))));
        assert!(!should_advance(Some(Working), Waiting, Some(&q(1, false))));
        assert!(!should_advance(Some(Working), Done, Some(&q(0, false))));
        assert!(!should_advance(Some(Working), Done, Some(&q(2, true))));
        assert!(!should_advance(Some(Working), Done, None));
        let mut p = new_pane(PaneSpec::new(AgentKind::Claude, "", ""), 0);
        p.status = Done;
        assert!(ready_for_next(Some(&p)));
        p.status = Working;
        assert!(!ready_for_next(Some(&p)));
        p.status = Done;
        p.exited = true;
        assert!(!ready_for_next(Some(&p)));
        assert!(!ready_for_next(None));
    }

    #[test]
    fn move_items() {
        fn mv<'a>(v: &[&'a str], i: usize, d: isize) -> Vec<&'a str> {
            let mut v = v.to_vec();
            move_item(&mut v, i, d);
            v
        }
        assert_eq!(mv(&["a", "b", "c"], 0, 1), ["b", "a", "c"]);
        assert_eq!(mv(&["a", "b", "c"], 2, -5), ["c", "a", "b"]);
        assert_eq!(mv(&["a", "b"], 1, 1), ["a", "b"]);
        assert_eq!(mv(&["a"], 5, 1), ["a"]);
    }

    #[test]
    fn queue_ops_and_prompt_text() {
        let mut qs = Queues::default();
        qs.add("p", "first");
        qs.add("p", "  ");
        qs.add("p", "multi\nline");
        assert_eq!(qs.pop_next("p").unwrap().text, "first");
        assert_eq!(prompt_text(&qs.get("p").unwrap().items[0].text), "\x1b[200~multi\nline\x1b[201~");
        assert_eq!(prompt_text("one"), "one");
        qs.forget("p");
        assert!(qs.get("p").is_none());
    }

    #[test]
    fn review_loops_and_prompt() {
        let m = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect::<HashMap<_, _>>()
        };
        assert!(!creates_loop(&m(&[]), "a", "b"));
        assert!(creates_loop(&m(&[("b", "a")]), "a", "b"));
        assert!(creates_loop(&m(&[("b", "c"), ("c", "a")]), "a", "b"));
        assert!(!creates_loop(&m(&[("b", "c")]), "a", "b"));
        assert!(creates_loop(&m(&[]), "a", "a"));
        let text = review_prompt("Api", AgentKind::Codex, "D:\\repo", Some("add\nlogin"));
        assert!(text.contains("git -C \"D:\\repo\" diff HEAD"));
        assert!(text.contains("\"add login\""));
        assert!(!text.contains('\n'));
    }

    #[test]
    fn recent_prompts() {
        let mut list = vec!["b".to_string(), "a".to_string()];
        assert!(remember_prompt(&mut list, " a "));
        assert_eq!(list, ["a", "b"]);
        assert!(!remember_prompt(&mut list, "  "));
    }
}
