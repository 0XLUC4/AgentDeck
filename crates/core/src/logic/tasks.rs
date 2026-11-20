//! Per-pane task log built from hook events (`src/store/tasks.ts`). In memory only.

use crate::hooks_server::PaneStatus;

pub const MAX_ACTIONS: usize = 200;
pub const MAX_HISTORY: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct TaskAction {
    pub at: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub prompt: Option<String>,
    pub started_at: i64,
    pub actions: Vec<TaskAction>,
    pub summary: Option<String>,
    pub ended_at: Option<i64>,
    pub status: PaneStatus,
}

/// `current` is the running task; finished ones move to `history` (newest first).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskLog {
    pub current: Option<Task>,
    pub history: Vec<Task>,
}

/// The fields of an `agent-status` event the reducer needs.
#[derive(Debug, Clone, Default)]
pub struct TaskEvent<'a> {
    pub status: Option<PaneStatus>,
    pub event: &'a str,
    pub last_action: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub summary: Option<&'a str>,
}

impl<'a> From<&'a crate::hooks_server::AgentStatusEvent> for TaskEvent<'a> {
    fn from(e: &'a crate::hooks_server::AgentStatusEvent) -> Self {
        Self {
            status: Some(e.status),
            event: &e.event,
            last_action: e.last_action.as_deref(),
            prompt: e.prompt.as_deref(),
            summary: e.summary.as_deref(),
        }
    }
}

fn new_task(started_at: i64, prompt: Option<&str>) -> Task {
    Task {
        prompt: prompt.map(str::to_owned),
        started_at,
        actions: Vec::new(),
        summary: None,
        ended_at: None,
        status: PaneStatus::Working,
    }
}

fn archive(history: &mut Vec<Task>, task: Task) {
    history.insert(0, task);
    history.truncate(MAX_HISTORY);
}

/// Pure reducer. A task starts on UserPromptSubmit or on the first working event without a running
/// task, ends on done/error. `fallback_start` dates tasks seen only at their end (Codex notify).
pub fn apply_task_event(log: &TaskLog, e: &TaskEvent, now: i64, fallback_start: i64) -> TaskLog {
    let mut next = log.clone();
    next.apply(e, now, fallback_start);
    next
}

impl TaskLog {
    /// Applies a live hook without copying prior actions and finished tasks.
    pub fn apply(&mut self, e: &TaskEvent, now: i64, fallback_start: i64) {
        let status = e.status.unwrap_or(PaneStatus::Idle);
        let ended = matches!(status, PaneStatus::Done | PaneStatus::Error);
        if e.event == "UserPromptSubmit" || (status == PaneStatus::Working && self.current.is_none()) {
            if let Some(mut interrupted) = self.current.take() {
                interrupted.ended_at = Some(now);
                archive(&mut self.history, interrupted);
            }
            self.current = Some(new_task(now, e.prompt));
        }
        if self.current.is_none() && !ended {
            return;
        }
        let task = self.current.get_or_insert_with(|| new_task(fallback_start.min(now), None));
        if let Some(action) = e.last_action
            && status != PaneStatus::Error
            && task.actions.last().is_none_or(|a| a.text != action)
        {
            task.actions.push(TaskAction {
                at: now,
                text: action.to_owned(),
            });
            let extra = task.actions.len().saturating_sub(MAX_ACTIONS);
            task.actions.drain(..extra);
        }
        if task.prompt.is_none() {
            task.prompt = e.prompt.map(str::to_owned);
        }
        if !ended {
            return;
        }
        task.summary = e
            .summary
            .or(if status == PaneStatus::Error { e.last_action } else { None })
            .map(str::to_owned);
        task.ended_at = Some(now);
        task.status = status;
        if let Some(done) = self.current.take() {
            archive(&mut self.history, done);
        }
    }
}

/// The two instructions a handoff starts from (the user edits them in the draft).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffKind {
    Review,
    Continue,
}

impl HandoffKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Review => "Review the changes",
            Self::Continue => "Continue the work",
        }
    }

    pub fn instruction(self) -> &'static str {
        match self {
            Self::Review => {
                "Review the changes made in the source worktree above: run `git status` and `git diff` there and \
                 read the files they touch. Report real defects only (logic, edge cases, missing tests), ranked by \
                 severity, with file and line. Do not edit files."
            }
            Self::Continue => {
                "Continue this work in the source worktree above: pick up where it stopped, keep its approach, then \
                 tell me what you changed and what is left."
            }
        }
    }
}

/// Where the finished task ran.
#[derive(Debug, Clone, Default)]
pub struct HandoffSource<'a> {
    /// Name of the agent pane that did the task.
    pub agent: &'a str,
    /// Project (workspace) name.
    pub project: &'a str,
    /// Folder the agent worked in (its worktree when it has one).
    pub dir: &'a str,
    pub branch: Option<&'a str>,
}

/// Editable handoff text for another agent: project, source worktree and branch (so an agent opened
/// in another worktree finds the work), initial request, result, then the instruction. None when the
/// task has neither a prompt nor a summary: there is nothing to hand off.
pub fn handoff_draft(task: &Task, source: &HandoffSource, kind: HandoffKind) -> Option<String> {
    let non_empty = |t: &Option<String>| t.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
    let (prompt, summary) = (non_empty(&task.prompt), non_empty(&task.summary));
    if prompt.is_none() && summary.is_none() {
        return None;
    }
    let mut text = format!("Handoff from {} (project {})\nSource worktree: {}\n", source.agent, source.project, source.dir);
    if let Some(branch) = source.branch.filter(|b| !b.is_empty()) {
        text.push_str(&format!("Branch: {branch}\n"));
    }
    text.push_str(&format!(
        "\nInitial request:\n{}\n\nResult from {}:\n{}\n\nInstruction:\n{}",
        prompt.as_deref().unwrap_or("(none received)"),
        source.agent,
        summary.as_deref().unwrap_or("(no summary received)"),
        kind.instruction()
    ));
    Some(text)
}

/// Switches the draft's instruction from `from` to `to`, keeping the user's other edits; when the
/// user rewrote the old instruction, the new one is appended instead.
pub fn swap_instruction(text: &str, from: HandoffKind, to: HandoffKind) -> String {
    if from == to || text.contains(to.instruction()) {
        return text.to_owned();
    }
    if text.contains(from.instruction()) {
        return text.replacen(from.instruction(), to.instruction(), 1);
    }
    format!("{}\n\n{}", text.trim_end(), to.instruction())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(prompt: Option<&str>, summary: Option<&str>) -> Task {
        Task {
            summary: summary.map(str::to_owned),
            ended_at: Some(1),
            status: PaneStatus::Done,
            ..new_task(0, prompt)
        }
    }

    const SOURCE: HandoffSource<'static> = HandoffSource {
        agent: "Claude",
        project: "shop",
        dir: r"D:\code\shop-agentdeck-1a2b3c4d",
        branch: Some("agentdeck/1a2b3c4d"),
    };

    #[test]
    fn handoff_draft_names_the_source_worktree() {
        let draft = handoff_draft(&finished(Some("fix cart"), Some("Fixed the total.")), &SOURCE, HandoffKind::Review).unwrap();
        for part in [
            "Handoff from Claude (project shop)",
            r"Source worktree: D:\code\shop-agentdeck-1a2b3c4d",
            "Branch: agentdeck/1a2b3c4d",
            "Initial request:\nfix cart",
            "Result from Claude:\nFixed the total.",
            HandoffKind::Review.instruction(),
        ] {
            assert!(draft.contains(part), "{part} missing in {draft}");
        }
        // No branch (detached HEAD, not a repo): no Branch line.
        let plain = HandoffSource { branch: None, ..SOURCE };
        assert!(!handoff_draft(&finished(Some("a"), None), &plain, HandoffKind::Continue).unwrap().contains("Branch:"));
    }

    #[test]
    fn handoff_needs_a_prompt_or_a_summary() {
        // No summary but an initial request: still transferable.
        let draft = handoff_draft(&finished(Some("add tests"), None), &SOURCE, HandoffKind::Continue).unwrap();
        assert!(draft.contains("add tests") && draft.contains("(no summary received)"));
        assert!(handoff_draft(&finished(None, Some("Done.")), &SOURCE, HandoffKind::Review).is_some());
        assert!(handoff_draft(&finished(None, None), &SOURCE, HandoffKind::Review).is_none());
        assert!(handoff_draft(&finished(Some("  "), Some("")), &SOURCE, HandoffKind::Review).is_none());
    }

    #[test]
    fn swapping_the_instruction_keeps_edits() {
        let draft = handoff_draft(&finished(Some("a"), Some("b")), &SOURCE, HandoffKind::Review).unwrap();
        let edited = draft.replace("Initial request:", "Initial request (edited):");
        let swapped = swap_instruction(&edited, HandoffKind::Review, HandoffKind::Continue);
        assert!(swapped.contains("(edited)") && swapped.contains(HandoffKind::Continue.instruction()));
        assert!(!swapped.contains(HandoffKind::Review.instruction()));
        assert_eq!(swap_instruction(&swapped, HandoffKind::Continue, HandoffKind::Continue), swapped);
        // Old instruction rewritten by the user: the new one is added after it.
        let rewritten = "my own words";
        assert_eq!(
            swap_instruction(rewritten, HandoffKind::Review, HandoffKind::Continue),
            format!("my own words\n\n{}", HandoffKind::Continue.instruction())
        );
    }

    fn ev<'a>(status: PaneStatus, event: &'a str) -> TaskEvent<'a> {
        TaskEvent {
            status: Some(status),
            event,
            ..Default::default()
        }
    }
    fn prompt(text: &str) -> TaskEvent<'_> {
        TaskEvent {
            prompt: Some(text),
            ..ev(PaneStatus::Working, "UserPromptSubmit")
        }
    }
    fn tool(text: &str) -> TaskEvent<'_> {
        TaskEvent {
            last_action: Some(text),
            ..ev(PaneStatus::Working, "PreToolUse")
        }
    }
    fn stop(summary: Option<&str>) -> TaskEvent<'_> {
        TaskEvent {
            summary,
            ..ev(PaneStatus::Done, "Stop")
        }
    }
    fn run(events: &[TaskEvent]) -> TaskLog {
        events.iter().enumerate().fold(TaskLog::default(), |l, (i, e)| {
            apply_task_event(&l, e, i as i64 * 1000, i as i64 * 1000)
        })
    }

    #[test]
    fn records_and_archives() {
        let log = run(&[
            prompt("fix it"),
            tool("Read a.ts"),
            tool("Read a.ts"),
            tool("Edit a.ts"),
            stop(Some("Fixed.")),
        ]);
        assert!(log.current.is_none());
        let t = &log.history[0];
        assert_eq!(t.prompt.as_deref(), Some("fix it"));
        assert_eq!(t.summary.as_deref(), Some("Fixed."));
        assert_eq!((t.started_at, t.ended_at), (0, Some(4000)));
        let texts: Vec<_> = t.actions.iter().map(|a| a.text.as_str()).collect();
        assert_eq!(texts, ["Read a.ts", "Edit a.ts"]);
    }

    #[test]
    fn starts_after_done_and_ignores_idle() {
        let log = run(&[prompt("a"), stop(Some("A")), tool("Bash: ls")]);
        assert_eq!(log.current.as_ref().unwrap().actions[0].text, "Bash: ls");
        assert!(log.current.as_ref().unwrap().prompt.is_none());
        assert_eq!(run(&[ev(PaneStatus::Idle, "SessionStart")]), TaskLog::default());
    }

    #[test]
    fn lone_end_event_and_error() {
        let e = TaskEvent {
            prompt: Some("p"),
            summary: Some("s"),
            ..ev(PaneStatus::Done, "agent-turn-complete")
        };
        let log = apply_task_event(&TaskLog::default(), &e, 5000, 2000);
        assert_eq!(log.history[0].started_at, 2000);
        assert_eq!(log.history[0].ended_at, Some(5000));
        let err = TaskEvent {
            last_action: Some("rate_limit"),
            ..ev(PaneStatus::Error, "StopFailure")
        };
        let log = run(&[prompt("a"), err]);
        assert_eq!(log.history[0].status, PaneStatus::Error);
        assert_eq!(log.history[0].summary.as_deref(), Some("rate_limit"));
        assert!(log.history[0].actions.is_empty());
    }

    #[test]
    fn caps_and_interrupts() {
        let names: Vec<String> = (0..MAX_ACTIONS + 5).map(|i| format!("t{i}")).collect();
        let mut events = vec![prompt("a")];
        events.extend(names.iter().map(|n| tool(n)));
        let log = run(&events);
        let actions = &log.current.unwrap().actions;
        assert_eq!(actions.len(), MAX_ACTIONS);
        assert_eq!(actions[MAX_ACTIONS - 1].text, format!("t{}", MAX_ACTIONS + 4));
        let log = run(&[prompt("a"), tool("x"), prompt("b")]);
        assert_eq!(log.current.unwrap().prompt.as_deref(), Some("b"));
        assert_eq!(log.history[0].ended_at, Some(2000));
        assert_eq!(log.history[0].status, PaneStatus::Working);
    }
}
