//! Event bus replacing Tauri events: same names (`agent-status`, `voice-state`, `pane-exit:<id>`...)
//! and JSON payloads. Each subscriber gets its own channel; dead receivers are dropped on emit.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, PoisonError};

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Event {
    pub name: String,
    pub payload: Value,
}

#[derive(Default)]
pub struct Bus {
    subscribers: Mutex<Vec<Sender<Event>>>,
}

impl Bus {
    pub fn subscribe(&self) -> Receiver<Event> {
        let (tx, rx) = channel();
        self.lock().push(tx);
        rx
    }

    pub fn emit(&self, name: &str, payload: Value) {
        let event = Event {
            name: name.to_owned(),
            payload,
        };
        self.lock().retain(|tx| tx.send(event.clone()).is_ok());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Sender<Event>>> {
        self.subscribers.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
