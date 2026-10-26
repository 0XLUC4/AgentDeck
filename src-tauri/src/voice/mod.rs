//! Voice input: global push-to-talk shortcut, mic capture (cpal), transcription,
//! and typing the transcript into the active pane.
//!
//! Events: `voice-level` (f32 0..1, ~30/s), `voice-state` ("idle" | "listening" |
//! "transcribing" | "error"), `voice-transcript` (String).

use tauri::{AppHandle, State};

use crate::AppState;

/// Registers the global shortcut and shows/hides the pill window. Called from `setup`.
pub fn init(_app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    // TODO(team-voice)
    Ok(())
}

#[tauri::command]
pub fn voice_start(_app: AppHandle) -> Result<(), String> {
    Err("voice not implemented yet".into()) // TODO(team-voice)
}

#[tauri::command]
pub fn voice_stop(_app: AppHandle) -> Result<(), String> {
    Err("voice not implemented yet".into()) // TODO(team-voice)
}

/// Types `text` into the active pane, followed by `\r` when `submit`.
#[tauri::command]
pub fn voice_send(state: State<'_, AppState>, text: String, submit: bool) -> Result<(), String> {
    let pane_id = state
        .active_pane
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or("no active pane")?;
    let data = if submit { format!("{text}\r") } else { text };
    state.panes.write(&pane_id, data.as_bytes())
}
