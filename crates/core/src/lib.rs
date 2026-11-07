//! AgentDeck core: everything that is not UI. Ported from the Tauri backend (`src-tauri/src`)
//! and the pure TypeScript logic (`src/lib`, `src/store`).
//!
//! Tauri pieces are replaced by [`AppHandle`]: app data dir, a typed state map (`manage` / `state`)
//! and an event bus (`emit` / `subscribe`) carrying the same event names and JSON payloads.

pub mod agents;
pub mod assets;
pub mod bus;
pub mod git;
pub mod history;
pub mod hooks_server;
pub mod logic;
pub mod mcp;
pub mod notes_graph;
pub mod notify;
pub mod persist;
pub mod pty;
pub mod skills;
pub mod trust;
pub mod usage;
pub mod voice;
pub mod workspace;

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

pub use bus::{Bus, Event};

/// Same folder as the Tauri build (`com.agentdeck.app`), so existing data is picked up.
pub const APP_IDENTIFIER: &str = "com.agentdeck.app";

/// Shared backend state.
pub struct AppState {
    pub panes: pty::Panes,
    /// Pane that receives voice transcripts.
    pub active_pane: Mutex<Option<String>>,
    pub hooks: hooks_server::HookServer,
}

type StateMap = HashMap<TypeId, Arc<dyn Any + Send + Sync>>;

struct Inner {
    data_dir: PathBuf,
    bus: Bus,
    states: Mutex<StateMap>,
}

/// Cheap clonable handle to the backend: replaces `tauri::AppHandle`.
#[derive(Clone)]
pub struct AppHandle(Arc<Inner>);

impl AppHandle {
    /// A handle on `data_dir` with an empty state map. Use [`start`] for the full backend.
    pub fn new(data_dir: PathBuf) -> Self {
        Self(Arc::new(Inner {
            data_dir,
            bus: Bus::default(),
            states: Mutex::default(),
        }))
    }

    /// `%APPDATA%/com.agentdeck.app` (or the platform equivalent).
    pub fn default_data_dir() -> Result<PathBuf, String> {
        dirs::data_dir()
            .map(|d| d.join(APP_IDENTIFIER))
            .ok_or_else(|| "no app data directory".to_string())
    }

    /// App data dir, created if missing.
    pub fn app_data_dir(&self) -> Result<PathBuf, String> {
        std::fs::create_dir_all(&self.0.data_dir)
            .map_err(|e| format!("{}: {e}", self.0.data_dir.display()))?;
        Ok(self.0.data_dir.clone())
    }

    /// Tauri had a separate config dir; it is the same folder on Windows.
    pub fn app_config_dir(&self) -> Result<PathBuf, String> {
        self.app_data_dir()
    }

    pub fn bus(&self) -> &Bus {
        &self.0.bus
    }

    /// Broadcasts `name` with a JSON payload to every subscriber.
    pub fn emit<S: serde::Serialize>(&self, name: &str, payload: S) -> Result<(), String> {
        let payload = serde_json::to_value(payload).map_err(|e| e.to_string())?;
        self.0.bus.emit(name, payload);
        Ok(())
    }

    /// Tauri targeted a window label; there is one bus here, the target is informational.
    pub fn emit_to<S: serde::Serialize>(
        &self,
        _target: &str,
        name: &str,
        payload: S,
    ) -> Result<(), String> {
        self.emit(name, payload)
    }

    pub fn subscribe(&self) -> std::sync::mpsc::Receiver<Event> {
        self.0.bus.subscribe()
    }

    /// Registers a singleton state value (replaces `Manager::manage`).
    pub fn manage<T: Any + Send + Sync>(&self, value: T) {
        self.states().insert(TypeId::of::<T>(), Arc::new(value));
    }

    pub fn try_state<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.states()
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|s| s.downcast::<T>().ok())
    }

    /// Like Tauri: asking for a state that was never managed is a programming error.
    pub fn state<T: Any + Send + Sync>(&self) -> Arc<T> {
        self.try_state::<T>()
            .unwrap_or_else(|| panic!("state not managed: {}", std::any::type_name::<T>()))
    }

    fn states(&self) -> std::sync::MutexGuard<'_, StateMap> {
        self.0.states.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Starts the backend like Tauri's `setup`: hook server, MCP hub, pane state, voice, history.
/// The UI then subscribes to the bus and calls the module functions.
pub fn start(data_dir: PathBuf) -> AppHandle {
    let app = AppHandle::new(data_dir);
    let hooks = hooks_server::start(app.clone());
    mcp::hub::init(&app, hooks.port);
    app.manage(AppState {
        panes: pty::Panes::default(),
        active_pane: Mutex::new(None),
        hooks,
    });
    if let Err(e) = voice::init(&app) {
        eprintln!("voice: {e}");
    }
    history::init(&app);
    app
}

/// App exit: stops shared MCP servers (they would keep their ports otherwise).
pub fn shutdown() {
    mcp::hub::stop_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_map_and_bus() {
        let app = AppHandle::new(std::env::temp_dir().join("agentdeck-core-test"));
        app.manage(5u32);
        assert_eq!(*app.state::<u32>(), 5);
        assert!(app.try_state::<String>().is_none());
        let rx = app.subscribe();
        app.emit("x", serde_json::json!({ "a": 1 })).unwrap();
        let event = rx.recv().unwrap();
        assert_eq!(event.name, "x");
        assert_eq!(event.payload["a"], 1);
    }
}
