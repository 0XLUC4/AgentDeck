//! Voice input: global push-to-talk shortcut, mic capture (cpal), transcription
//! (local whisper.cpp or an OpenAI-compatible API). The main window receives the
//! transcript, picks the target pane and types it via `voice_send` / `write_to_pane`.
//! Settings live in `config` and apply without restart.
//!
//! Events: `voice-level` (f32 0..1, ~30/s), `voice-state` ("idle" | "listening" |
//! "downloading" | "transcribing" | "error"), `voice-download` ({model, received, total}),
//! `voice-transcript` (String), `voice-error` (String).

mod audio;
mod config;
mod local;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::AppState;
use audio::Capture;
use config::{Engine, VoiceConfig, VoiceConfigView, WhisperModel};

const MIN_CLIP_SECS: f32 = 0.3;
/// Auto-stop (and transcribe) if the key-up is lost.
const MAX_RECORDING: Duration = Duration::from_secs(60);
const HIDE_DELAY: Duration = Duration::from_millis(1500);
const ERROR_HIDE_DELAY: Duration = Duration::from_secs(4);
/// Distance between the pill and the bottom of the screen, in logical px.
const BOTTOM_MARGIN: f64 = 80.0;

/// Voice state managed by Tauri.
#[derive(Default)]
struct Voice {
    /// Live settings; snapshot (clone) before long work instead of holding the lock.
    config: Mutex<VoiceConfig>,
    /// Running capture, tagged with its recording session id.
    capture: Mutex<Option<(u64, Capture)>>,
    /// Bumped on every state change so a pending hide is cancelled by newer activity.
    generation: AtomicU64,
    /// Bumped per recording so the max-duration timer only stops its own recording.
    session: AtomicU64,
    /// A transcription is in flight: new recordings are ignored until it settles.
    transcribing: AtomicBool,
}

#[derive(Clone, Serialize)]
struct DownloadProgress {
    model: WhisperModel,
    received: u64,
    /// `None` when the server sends no Content-Length.
    total: Option<u64>,
}

fn emit_download(app: &AppHandle, model: WhisperModel, received: u64, total: u64) {
    let total = (total > 0).then_some(total);
    let _ = app.emit(
        "voice-download",
        DownloadProgress {
            model,
            received,
            total,
        },
    );
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    model: WhisperModel,
    downloaded: bool,
    size_bytes: Option<u64>,
}

/// Loads settings, registers the global shortcut and voice state. Called from `setup`.
pub fn init(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let config = VoiceConfig::load(&config_path(app)?);
    let shortcut = config.shortcut.clone();
    app.manage(Voice {
        config: Mutex::new(config),
        ..Voice::default()
    });
    if let Some(pill) = app.get_webview_window("voice") {
        pill.set_focusable(false)?; // never steal focus from the terminal
    }
    // Another app may own the shortcut: keep the app usable, voice_start still works.
    if let Err(e) = app
        .global_shortcut()
        .on_shortcut(shortcut.as_str(), on_shortcut)
    {
        eprintln!("voice: cannot register shortcut {shortcut}: {e}");
    }
    Ok(())
}

fn on_shortcut(app: &AppHandle, _: &Shortcut, event: ShortcutEvent) {
    let push_to_talk = app
        .state::<Voice>()
        .config
        .lock()
        .map_or(true, |c| c.push_to_talk);
    let result = match (event.state(), push_to_talk) {
        (ShortcutState::Pressed, true) => start(app),
        (ShortcutState::Released, true) => stop(app, None),
        (ShortcutState::Pressed, false) if is_recording(app) => stop(app, None),
        (ShortcutState::Pressed, false) => start(app),
        (ShortcutState::Released, false) => Ok(()),
    };
    if let Err(e) = result {
        fail(app, &e);
    }
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("voice.json"))
}

fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models"))
}

fn config_snapshot(app: &AppHandle) -> Result<VoiceConfig, String> {
    Ok(app
        .state::<Voice>()
        .config
        .lock()
        .map_err(|e| e.to_string())?
        .clone())
}

fn view(config: VoiceConfig) -> VoiceConfigView {
    VoiceConfigView {
        config,
        has_api_key: config::api_key().is_some(),
    }
}

#[tauri::command]
pub fn voice_get_config(app: AppHandle) -> Result<VoiceConfigView, String> {
    config_snapshot(&app).map(view)
}

/// Validates, applies live (shortcut swap included) and persists the settings.
/// Sync command: runs on the main thread, like shortcut registration and events.
#[tauri::command]
pub fn voice_set_config(app: AppHandle, config: VoiceConfig) -> Result<VoiceConfigView, String> {
    let new_shortcut = config.validate()?;
    let old = config_snapshot(&app)?;
    let old_shortcut = old.shortcut.parse::<Shortcut>().ok();
    if old_shortcut != Some(new_shortcut) {
        let shortcuts = app.global_shortcut();
        if let Some(old_shortcut) = old_shortcut {
            let _ = shortcuts.unregister(old_shortcut); // may never have been registered
        }
        if let Err(e) = shortcuts.on_shortcut(new_shortcut, on_shortcut) {
            if let Some(old_shortcut) = old_shortcut
                && let Err(e) = shortcuts.on_shortcut(old_shortcut, on_shortcut)
            {
                eprintln!("voice: cannot restore shortcut {}: {e}", old.shortcut);
            }
            return Err(format!(
                "cannot register shortcut \"{}\": {e}",
                config.shortcut
            ));
        }
    }
    *app.state::<Voice>()
        .config
        .lock()
        .map_err(|e| e.to_string())? = config.clone();
    config.save(&config_path(&app)?)?;
    Ok(view(config))
}

/// Saves the API key in the OS credential store; `None` deletes it.
#[tauri::command]
pub fn voice_set_api_key(key: Option<String>) -> Result<(), String> {
    config::set_api_key(key.as_deref())
}

#[tauri::command]
pub fn voice_list_models(app: AppHandle) -> Result<Vec<ModelStatus>, String> {
    let dir = models_dir(&app)?;
    Ok(WhisperModel::ALL
        .into_iter()
        .map(|model| {
            let size = local::downloaded(&dir, model).map(|(_, size)| size);
            ModelStatus {
                model,
                downloaded: size.is_some(),
                size_bytes: size,
            }
        })
        .collect())
}

/// Downloads `model` (no-op if present). Progress via `voice-download`.
#[tauri::command]
pub async fn voice_download_model(app: AppHandle, model: WhisperModel) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = models_dir(&app)?;
        local::ensure_model(&dir, model, |received, total| {
            emit_download(&app, model, received, total);
        })
        .map(drop)
    })
    .await
    .map_err(|e| e.to_string())?
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
        match transcribe(&app, &samples, rate) {
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

fn transcribe(app: &AppHandle, samples: &[f32], rate: u32) -> Result<(), String> {
    let config = config_snapshot(app)?;
    let pcm = audio::resample(samples, rate, audio::TARGET_RATE);
    let text = match config.engine {
        Engine::Api => transcribe_api(&config, audio::wav_16bit_mono(&pcm, audio::TARGET_RATE))?,
        Engine::Local => transcribe_local(app, &config, &pcm)?,
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    // The main window routes it ("Codex, ..." -> that pane, else the active one) and types it.
    let _ = app.emit("voice-transcript", text);
    Ok(())
}

/// Downloads the model on first use (pill shows progress), then runs whisper.cpp.
fn transcribe_local(app: &AppHandle, config: &VoiceConfig, pcm: &[f32]) -> Result<String, String> {
    let model = config.model;
    let mut announced = false;
    let path = local::ensure_model(&models_dir(app)?, model, |received, total| {
        if !announced {
            announced = true;
            set_state(app, "downloading");
        }
        emit_download(app, model, received, total);
    })?;
    if announced {
        set_state(app, "transcribing");
    }
    local::transcribe(&path, pcm, &config.language)
}

/// POSTs a WAV to an OpenAI-compatible `/audio/transcriptions` endpoint.
fn transcribe_api(config: &VoiceConfig, wav: Vec<u8>) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Transcript {
        text: String,
    }
    config::check_stt_url(&config.api_url)?;
    let key = config::api_key();
    if key.is_none() && config.api_url == config::DEFAULT_STT_URL {
        return Err("no API key set (Settings > Voice)".into());
    }
    let file = reqwest::blocking::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let mut form = reqwest::blocking::multipart::Form::new()
        .text("model", config.api_model.clone())
        .part("file", file);
    if config.language != "auto" {
        form = form.text("language", config.language.clone());
    }
    let mut request = reqwest::blocking::Client::new()
        .post(&config.api_url)
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
