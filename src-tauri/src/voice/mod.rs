//! Voice input: global push-to-talk shortcut, mic capture (cpal), transcription,
//! and typing the transcript into the active pane.
//!
//! Events: `voice-level` (f32 0..1, ~30/s), `voice-state` ("idle" | "listening" |
//! "transcribing" | "error"), `voice-transcript` (String), `voice-error` (String).

mod audio;

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::AppState;
use audio::Capture;

// TODO(settings): make shortcut + mode configurable via tauri-plugin-store.
/// Ctrl+Space conflicts with IME switching on Windows.
const SHORTCUT: &str = "ctrl+shift+space";
/// true: hold to talk. false: press to start, press again to stop.
const PUSH_TO_TALK: bool = true;
/// Append `\r` after the transcript. Off: the user validates in the terminal.
const SUBMIT_ON_TRANSCRIPT: bool = false;
const MIN_CLIP_SECS: f32 = 0.3;
/// Auto-stop (and transcribe) if the key-up is lost.
const MAX_RECORDING: Duration = Duration::from_secs(60);
const HIDE_DELAY: Duration = Duration::from_millis(1500);
const ERROR_HIDE_DELAY: Duration = Duration::from_secs(4);
/// Distance between the pill and the bottom of the screen, in logical px.
const BOTTOM_MARGIN: f64 = 80.0;

const DEFAULT_STT_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
const DEFAULT_STT_MODEL: &str = "whisper-1";

/// Voice state managed by Tauri.
#[derive(Default)]
struct Voice {
    /// Running capture, tagged with its recording session id.
    capture: Mutex<Option<(u64, Capture)>>,
    /// Bumped on every state change so a pending hide is cancelled by newer activity.
    generation: AtomicU64,
    /// Bumped per recording so the max-duration timer only stops its own recording.
    session: AtomicU64,
    /// A transcription is in flight: new recordings are ignored until it settles.
    transcribing: AtomicBool,
}

/// Registers the global shortcut and voice state. Called from `setup`.
pub fn init(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    app.manage(Voice::default());
    if let Some(pill) = app.get_webview_window("voice") {
        pill.set_focusable(false)?; // never steal focus from the terminal
    }
    let registered = app
        .global_shortcut()
        .on_shortcut(SHORTCUT, |app, _, event| {
            let result = match (event.state(), PUSH_TO_TALK) {
                (ShortcutState::Pressed, true) => start(app),
                (ShortcutState::Released, true) => stop(app, None),
                (ShortcutState::Pressed, false) if is_recording(app) => stop(app, None),
                (ShortcutState::Pressed, false) => start(app),
                (ShortcutState::Released, false) => Ok(()),
            };
            if let Err(e) = result {
                fail(app, &e);
            }
        });
    // Another app may own the shortcut: keep the app usable, voice_start still works.
    if let Err(e) = registered {
        eprintln!("voice: cannot register shortcut {SHORTCUT}: {e}");
    }
    Ok(())
}

#[tauri::command]
pub fn voice_start(app: AppHandle) -> Result<(), String> {
    start(&app).inspect_err(|e| fail(&app, e))
}

#[tauri::command]
pub fn voice_stop(app: AppHandle) -> Result<(), String> {
    stop(&app, None).inspect_err(|e| fail(&app, e))
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

fn is_recording(app: &AppHandle) -> bool {
    app.state::<Voice>()
        .capture
        .lock()
        .is_ok_and(|c| c.is_some())
}

fn start(app: &AppHandle) -> Result<(), String> {
    let voice = app.state::<Voice>();
    let mut slot = voice.capture.lock().map_err(|e| e.to_string())?;
    // Key repeat, double start, or a transcription still in flight.
    if slot.is_some() || voice.transcribing.load(Ordering::SeqCst) {
        return Ok(());
    }
    let emitter = app.clone();
    let capture = Capture::start(move |rms| {
        // Speech RMS is ~0.01..0.2: sqrt spreads it over the visible 0..1 range.
        let _ = emitter.emit("voice-level", (rms * 10.0).sqrt().min(1.0));
    })?;
    let session = voice.session.fetch_add(1, Ordering::SeqCst) + 1;
    *slot = Some((session, capture));
    drop(slot);
    set_state(app, "listening");
    show_pill(app);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(MAX_RECORDING);
        if let Err(e) = stop(&app, Some(session)) {
            fail(&app, &e);
        }
    });
    Ok(())
}

/// Stops the running capture (only if it is `session`, when given) and transcribes it.
fn stop(app: &AppHandle, session: Option<u64>) -> Result<(), String> {
    let voice = app.state::<Voice>();
    let capture = {
        let mut slot = voice.capture.lock().map_err(|e| e.to_string())?;
        match slot.take() {
            Some((id, capture)) if session.is_none_or(|s| s == id) => {
                // Set under the slot lock so no new recording can start in between.
                voice.transcribing.store(true, Ordering::SeqCst);
                capture
            }
            other => {
                *slot = other;
                return Ok(());
            }
        }
    };
    let (samples, rate) = capture
        .stop()
        .inspect_err(|_| voice.transcribing.store(false, Ordering::SeqCst))?;
    if (samples.len() as f32) < rate as f32 * MIN_CLIP_SECS {
        set_state(app, "idle");
        voice.transcribing.store(false, Ordering::SeqCst);
        return Ok(());
    }
    set_state(app, "transcribing");
    let app = app.clone();
    std::thread::spawn(move || {
        match transcribe_and_type(&app, &samples, rate) {
            Ok(()) => set_state(&app, "idle"),
            Err(e) => fail(&app, &e),
        }
        // Cleared after the final state so it can't overwrite a newer "listening".
        app.state::<Voice>()
            .transcribing
            .store(false, Ordering::SeqCst);
    });
    Ok(())
}

fn transcribe_and_type(app: &AppHandle, samples: &[f32], rate: u32) -> Result<(), String> {
    let pcm = audio::resample(samples, rate, audio::TARGET_RATE);
    let text = transcribe(audio::wav_16bit_mono(&pcm, audio::TARGET_RATE))?;
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    let _ = app.emit("voice-transcript", text);
    // TODO(team-voice): route "Codex, ..." / "Claude, ..." to the matching pane once
    // pane metadata is available backend-side.
    voice_send(
        app.state::<AppState>(),
        text.to_string(),
        SUBMIT_ON_TRANSCRIPT,
    )
}

/// POSTs a WAV to an OpenAI-compatible `/audio/transcriptions` endpoint.
/// Env: AGENTDECK_STT_URL, AGENTDECK_STT_MODEL, AGENTDECK_STT_KEY (else OPENAI_API_KEY).
fn transcribe(wav: Vec<u8>) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Transcript {
        text: String,
    }
    let url = std::env::var("AGENTDECK_STT_URL").ok();
    if let Some(url) = &url {
        check_stt_url(url)?;
    }
    let key = std::env::var("AGENTDECK_STT_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .ok();
    if url.is_none() && key.is_none() {
        return Err("OPENAI_API_KEY is not set".into());
    }
    let model = std::env::var("AGENTDECK_STT_MODEL").unwrap_or_else(|_| DEFAULT_STT_MODEL.into());
    let file = reqwest::blocking::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let form = reqwest::blocking::multipart::Form::new()
        .text("model", model)
        .part("file", file);
    let mut request = reqwest::blocking::Client::new()
        .post(url.as_deref().unwrap_or(DEFAULT_STT_URL))
        .timeout(Duration::from_secs(60))
        .multipart(form);
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    let response = request
        .send()
        .map_err(|e| format!("transcription request failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let body: String = response
            .text()
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect();
        return Err(format!("transcription failed ({status}): {body}"));
    }
    response
        .json::<Transcript>()
        .map(|t| t.text)
        .map_err(|e| format!("bad transcription response: {e}"))
}

/// Audio and the API key must not travel in clear: https only, except loopback.
fn check_stt_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid AGENTDECK_STT_URL: {e}"))?;
    let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match parsed.scheme() {
        "https" => Ok(()),
        "http" if loopback => Ok(()),
        _ => Err("AGENTDECK_STT_URL must use https (http only for localhost)".into()),
    }
}

/// Reports an error. Never touches the capture slot: every failing path has
/// already consumed its own capture, and a newer recording must survive.
fn fail(app: &AppHandle, message: &str) {
    eprintln!("voice: {message}");
    let _ = app.emit("voice-error", message);
    set_state(app, "error");
    show_pill(app);
}

/// Emits `voice-state`; idle/error schedule a delayed hide unless newer activity happens.
fn set_state(app: &AppHandle, state: &'static str) {
    let _ = app.emit("voice-state", state);
    let generation = app
        .state::<Voice>()
        .generation
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    let delay = match state {
        "idle" => HIDE_DELAY,
        "error" => ERROR_HIDE_DELAY,
        _ => return,
    };
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        if app.state::<Voice>().generation.load(Ordering::SeqCst) == generation
            && let Some(pill) = app.get_webview_window("voice")
            && let Err(e) = pill.hide()
        {
            eprintln!("voice: hide pill: {e}");
        }
    });
}

/// Shows the pill bottom-center of the monitor under the cursor, without focusing it.
fn show_pill(app: &AppHandle) {
    let Some(pill) = app.get_webview_window("voice") else {
        return;
    };
    let placed = (|| -> tauri::Result<()> {
        let cursor = app.cursor_position()?;
        let monitor = match app.monitor_from_point(cursor.x, cursor.y)? {
            Some(m) => Some(m),
            None => app.primary_monitor()?,
        };
        if let Some(monitor) = monitor {
            let area = monitor.work_area();
            let size = pill.outer_size()?;
            let margin = (BOTTOM_MARGIN * monitor.scale_factor()) as i32;
            pill.set_position(PhysicalPosition::new(
                area.position.x + (area.size.width as i32 - size.width as i32) / 2,
                area.position.y + area.size.height as i32 - size.height as i32 - margin,
            ))?;
        }
        Ok(())
    })();
    if let Err(e) = placed {
        eprintln!("voice: place pill: {e}");
    }
    if let Err(e) = pill.show() {
        eprintln!("voice: show pill: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::check_stt_url;

    #[test]
    fn stt_url_requires_https_unless_loopback() {
        assert!(check_stt_url("https://api.openai.com/v1/audio/transcriptions").is_ok());
        assert!(check_stt_url("http://localhost:8080/v1/audio/transcriptions").is_ok());
        assert!(check_stt_url("http://127.0.0.1:9000/x").is_ok());
        assert!(check_stt_url("http://[::1]:9000/x").is_ok());
        assert!(check_stt_url("http://example.com/x").is_err());
        assert!(check_stt_url("http://localhost.evil.com/x").is_err());
        assert!(check_stt_url("ftp://localhost/x").is_err());
        assert!(check_stt_url("not a url").is_err());
    }
}
