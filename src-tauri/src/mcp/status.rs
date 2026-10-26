//! Latest hook status per pane, fed by the hook server, and `wait_for` built on it.

use std::collections::HashMap;
use std::sync::{Condvar, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::hooks_server::PaneStatus;

pub static STATUS: LazyLock<Statuses> = LazyLock::new(Statuses::default);

/// Condition of `wait_for`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    /// Every pane finished (done, error or exited).
    Done,
    /// At least one pane waits for an answer.
    Waiting,
    /// At least one pane finished or waits.
    Any,
}

impl Until {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "done" => Some(Self::Done),
            "waiting" => Some(Self::Waiting),
            "any" => Some(Self::Any),
            _ => None,
        }
    }

    fn met(self, labels: &[&str]) -> bool {
        let finished = |l: &&str| matches!(*l, "done" | "error" | "exited");
        match self {
            Self::Done => labels.iter().all(finished),
            Self::Waiting => labels.contains(&"waiting"),
            Self::Any => labels.iter().any(|l| finished(l) || *l == "waiting"),
        }
    }
}

pub fn label(status: PaneStatus) -> &'static str {
    match status {
        PaneStatus::Idle => "idle",
        PaneStatus::Working => "working",
        PaneStatus::Waiting => "waiting",
        PaneStatus::Done => "done",
        PaneStatus::Error => "error",
    }
}

pub fn parse(s: &str) -> Option<PaneStatus> {
    Some(match s {
        "idle" => PaneStatus::Idle,
        "working" => PaneStatus::Working,
        "waiting" => PaneStatus::Waiting,
        "done" => PaneStatus::Done,
        "error" => PaneStatus::Error,
        _ => return None,
    })
}

#[derive(Default)]
pub struct Statuses {
    map: Mutex<HashMap<String, PaneStatus>>,
    changed: Condvar,
}

impl Statuses {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, PaneStatus>> {
        self.map.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set(&self, pane_id: &str, status: PaneStatus) {
        self.lock().insert(pane_id.to_owned(), status);
        self.changed.notify_all();
    }

    /// Fills in a status the hooks never reported (agents without hooks: status derived by the UI).
    pub fn seed(&self, pane_id: &str, status: PaneStatus) {
        self.lock().entry(pane_id.to_owned()).or_insert(status);
    }

    /// Blocks until `until` holds for `ids` or `timeout` elapses. A pane that is no longer
    /// `alive` counts as "exited". Returns (condition met, status label per pane).
    pub fn wait(
        &self,
        ids: &[String],
        until: Until,
        timeout: Duration,
        alive: impl Fn(&str) -> bool,
    ) -> (bool, Map<String, Value>) {
        let deadline = Instant::now() + timeout;
        let mut map = self.lock();
        loop {
            let labels: Vec<&str> = ids
                .iter()
                .map(|id| match map.get(id) {
                    _ if !alive(id) => "exited",
                    Some(s) => label(*s),
                    None => "idle",
                })
                .collect();
            let met = until.met(&labels);
            let left = deadline.saturating_duration_since(Instant::now());
            if met || left.is_zero() {
                let out = ids
                    .iter()
                    .zip(labels)
                    .map(|(id, l)| (id.clone(), Value::from(l)))
                    .collect();
                return (met, out);
            }
            // Re-checks every second: a closed pane sends no status event.
            map = self
                .changed
                .wait_timeout(map, left.min(Duration::from_secs(1)))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn conditions() {
        assert!(Until::Done.met(&["done", "error", "exited"]));
        assert!(!Until::Done.met(&["done", "working"]));
        assert!(Until::Waiting.met(&["working", "waiting"]));
        assert!(!Until::Waiting.met(&["done"]));
        assert!(Until::Any.met(&["working", "done"]));
        assert!(!Until::Any.met(&["working", "idle"]));
        assert_eq!(Until::parse("any"), Some(Until::Any));
        assert_eq!(Until::parse("nope"), None);
    }

    #[test]
    fn returns_immediately_when_met_and_times_out_otherwise() {
        let s = Statuses::default();
        s.set("a", PaneStatus::Done);
        s.seed("a", PaneStatus::Working); // seed never overrides a hook status
        let (met, out) = s.wait(&ids(&["a"]), Until::Done, Duration::from_secs(5), |_| true);
        assert!(met);
        assert_eq!(out["a"], "done");

        s.set("b", PaneStatus::Working);
        let start = Instant::now();
        let (met, out) = s.wait(
            &ids(&["a", "b"]),
            Until::Done,
            Duration::from_millis(50),
            |_| true,
        );
        assert!(!met && start.elapsed() < Duration::from_secs(2));
        assert_eq!(out["b"], "working");

        let (met, out) = s.wait(&ids(&["b"]), Until::Done, Duration::from_secs(5), |_| false);
        assert!(met);
        assert_eq!(out["b"], "exited");
    }

    #[test]
    fn wakes_on_status_change() {
        let s = Arc::new(Statuses::default());
        s.set("p", PaneStatus::Working);
        let s2 = s.clone();
        let waiter = std::thread::spawn(move || {
            s2.wait(&ids(&["p"]), Until::Any, Duration::from_secs(10), |_| true)
        });
        std::thread::sleep(Duration::from_millis(50));
        s.set("p", PaneStatus::Waiting);
        let (met, out) = waiter.join().unwrap();
        assert!(met);
        assert_eq!(out["p"], "waiting");
    }
}
