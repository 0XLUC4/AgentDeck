//! Global app state as one GPUI entity ([`Deck`]): the core deck model, settings, queues, task logs,
//! permissions and orchestrator log, plus the bridge from the core event bus to the UI thread.
//!
//! Views hold an `Entity<Deck>`, read it in `render`, and `cx.subscribe` to [`DeckEvent`] for
//! backend events (pane output goes through `pty::attach_pane_output`, not this bus).

use std::collections::HashMap;
use std::path::PathBuf;

use agentdeck_core::hooks_server::AgentStatusEvent;
use agentdeck_core::hooks_server::permissions::{PermissionRequest, answer_permission};
use agentdeck_core::logic::permissions::auto_allowed;
use agentdeck_core::logic::deck::{DeckState, SavedSession};
use agentdeck_core::logic::orchestrator::{OrchestratorState, handle_read_tool};
use agentdeck_core::logic::queue::Queues;
use agentdeck_core::logic::settings::Settings;
use agentdeck_core::logic::tasks::{TaskEvent, TaskLog, apply_task_event};
use agentdeck_core::logic::templates::SessionTemplate;
use agentdeck_core::logic::text::now_ms;
use agentdeck_core::{AppHandle, Event, persist};
use gpui_kit::*;

/// Emitted for every backend bus event, after [`Deck`] applied what it owns.
#[derive(Debug, Clone)]
pub enum DeckEvent {
    Backend(Event),
}

pub struct Deck {
    pub backend: AppHandle,
    pub data_dir: PathBuf,
    pub state: DeckState,
    pub settings: Settings,
    pub queues: Queues,
    pub tasks: HashMap<String, TaskLog>,
    /// Code-then-review pairs: author pane id -> reviewer pane id.
    pub reviews: HashMap<String, String>,
    pub orchestrator: OrchestratorState,
    pub templates: Vec<SessionTemplate>,
    /// Pending permission prompts (`agent-permission`), oldest first.
    pub permissions: Vec<PermissionRequest>,
    pub recent_prompts: Vec<String>,
    /// Saved sessions whose panes the terminal layer must (re)spawn at startup.
    pub pending_restore: Vec<SavedSession>,
    _pump: Task<()>,
}

impl EventEmitter<DeckEvent> for Deck {}

impl Deck {
    /// Loads settings, templates and the saved deck, and starts the backend event pump.
    pub fn new(backend: AppHandle, data_dir: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut state = DeckState::default();
        let mut startup_errors = Vec::new();
        let pending_restore = persist::load_deck(&data_dir, &mut state).unwrap_or_else(|e| {
            startup_errors.push(format!("Could not restore the session: {e}"));
            Vec::new()
        });
        let settings = persist::load_settings(&data_dir).unwrap_or_else(|e| {
            startup_errors.push(format!("Settings not loaded: {e}"));
            Settings::default()
        });
        let queues = persist::load_queues(&data_dir);
        let templates = persist::load_templates(&data_dir).unwrap_or_else(|e| {
            startup_errors.push(format!("Templates not loaded: {e}"));
            Vec::new()
        });
        for message in startup_errors {
            state.toast(message, now_ms());
        }
        Self {
            _pump: pump(&backend, cx),
            backend,
            data_dir,
            state,
            settings,
            queues,
            tasks: HashMap::new(),
            reviews: HashMap::new(),
            orchestrator: OrchestratorState::default(),
            templates,
            permissions: Vec::new(),
            recent_prompts: Vec::new(),
            pending_restore,
        }
    }

    /// Asks for a save: the runtime's debounced autosave writes store.json once the burst ends
    /// (holding Ctrl+= used to rewrite the file on every key repeat, on the UI thread).
    pub fn save(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// Writes workspaces, tabs and archive to store.json now (autosave, quit). Errors become a toast.
    pub fn save_now(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = persist::save_deck(&self.data_dir, &self.state, now_ms()) {
            self.toast(format!("Save failed: {e}"), cx);
        }
    }

    /// Applies a settings change and writes settings.json.
    pub fn update_settings(&mut self, edit: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        edit(&mut self.settings);
        if let Err(e) = persist::save_settings(&self.data_dir, &self.settings) {
            self.toast(format!("Settings not saved: {e}"), cx);
        }
        cx.notify();
    }

    pub fn toast(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.state.toast(message, now_ms());
        cx.notify();
    }

    /// Applies the backend events the deck owns, then re-emits them for the feature views.
    fn on_backend(&mut self, event: Event, cx: &mut Context<Self>) {
        let now = now_ms();
        match event.name.as_str() {
            "agent-status" => {
                if let Ok(e) = serde_json::from_value::<AgentStatusEvent>(event.payload.clone()) {
                    let log = self.tasks.remove(&e.pane_id).unwrap_or_default();
                    let started = self.state.panes.get(&e.pane_id).and_then(|p| p.task_started_at);
                    let next = apply_task_event(&log, &TaskEvent::from(&e), now, started.unwrap_or(now));
                    self.tasks.insert(e.pane_id.clone(), next);
                    // A terminal notification is no hook: output keeps driving the status of such panes.
                    if e.event != agentdeck_core::logic::text::TERMINAL_NOTICE {
                        self.state.update_pane(&e.pane_id, |p| p.hooked = true);
                    }
                    self.state.set_status(&e.pane_id, e.status, e.last_action.clone(), now);
                }
            }
            "agent-permission" => {
                if let Ok(req) = serde_json::from_value::<PermissionRequest>(event.payload.clone()) {
                    self.on_permission(req, now);
                }
            }
            "agent-permission-resolved" => {
                let id = event.payload["requestId"].as_str().unwrap_or("");
                self.permissions.retain(|r| r.request_id != id);
            }
            "mcp-activity" => self.orchestrator.record(&event.payload, &self.state.panes, now),
            "mcp-request" => self.answer_read_tool(&event.payload, now),
            name => {
                if let Some(pane_id) = name.strip_prefix("pane-exit:") {
                    self.state.update_pane(pane_id, |p| {
                        p.exited = true;
                        p.status_since = now;
                    });
                }
            }
        }
        // These change nothing in the deck (about 30 levels a second while dictating): subscribers only.
        let silent = matches!(event.name.as_str(), "voice-level" | "voice-partial" | "voice-download");
        cx.emit(DeckEvent::Backend(event));
        if !silent {
            cx.notify();
        }
    }

    /// Auto-allow rules answer at once (with a toast); other requests wait for a permission card.
    fn on_permission(&mut self, req: PermissionRequest, now: i64) {
        let Some(cwd) = self.state.panes.get(&req.pane_id).map(|p| p.cwd.clone()) else {
            return;
        };
        let rule = auto_allowed(&req.tool_name, &req.detail, &cwd, &self.settings.auto_allow).cloned();
        let Some(rule) = rule else {
            self.permissions.push(req);
            return;
        };
        match answer_permission(req.request_id.clone(), "allow".into(), None) {
            Ok(()) => {
                self.state.toast(format!("Auto-allowed {} ({})", req.summary, rule.rule), now);
            }
            Err(e) => {
                self.state.toast(format!("Auto-allow failed ({e}): reply in the terminal."), now);
                self.permissions.push(req);
            }
        }
    }

    /// Read-only orchestrator tools are answered here; the others are left to the orchestrator module.
    fn answer_read_tool(&self, req: &serde_json::Value, now: i64) {
        let tool = req["tool"].as_str().unwrap_or("");
        let orchestrator_id = req["orchestratorId"].as_str().unwrap_or("");
        let Some(result) = handle_read_tool(&self.state, &self.tasks, orchestrator_id, tool, &req["args"], now)
        else {
            return;
        };
        let request_id = req["requestId"].as_str().unwrap_or("").to_owned();
        let (ok, err) = match result {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };
        if let Err(e) = agentdeck_core::mcp::mcp_respond(request_id, ok, err) {
            eprintln!("mcp_respond: {e}");
        }
    }
}

/// Bridges the core bus (std channel, any thread) to the UI thread.
fn pump(backend: &AppHandle, cx: &mut Context<Deck>) -> Task<()> {
    let rx = backend.subscribe();
    let (tx, events) = smol::channel::unbounded::<Event>();
    std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            if tx.send_blocking(event).is_err() {
                break;
            }
        }
    });
    cx.spawn(async move |this, cx| {
        while let Ok(event) = events.recv().await {
            if this.update(cx, |deck, cx| deck.on_backend(event, cx)).is_err() {
                break;
            }
        }
    })
}
