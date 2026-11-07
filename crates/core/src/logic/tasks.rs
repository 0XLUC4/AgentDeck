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
    let mut current = log.current.clone();
    let mut history = log.history.clone();
    let status = e.status.unwrap_or(PaneStatus::Idle);
    let ended = matches!(status, PaneStatus::Done | PaneStatus::Error);
    if e.event == "UserPromptSubmit" || (status == PaneStatus::Working && current.is_none()) {
        if let Some(mut interrupted) = current.take() {
            interrupted.ended_at = Some(now);
            archive(&mut history, interrupted);
        }
        current = Some(new_task(now, e.prompt));
    }
    if current.is_none() && !ended {
        return TaskLog { current, history };
    }
    let mut task = current.unwrap_or_else(|| new_task(fallback_start.min(now), None));
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
        return TaskLog {
            current: Some(task),
            history,
        };
    }
    task.summary = e
        .summary
        .or(if status == PaneStatus::Error { e.last_action } else { None })
        .map(str::to_owned);
    task.ended_at = Some(now);
    task.status = status;
    archive(&mut history, task);
    TaskLog {
        current: None,
        history,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
