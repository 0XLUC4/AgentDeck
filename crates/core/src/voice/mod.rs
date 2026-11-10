//! Voice input: global push-to-talk shortcut, mic capture (cpal), transcription
//! (local whisper.cpp or an OpenAI-compatible API). The main window receives the
//! transcript, picks the target pane and types it via `write_to_pane`.
//! Settings live in `config` and apply without restart.
//!
//! The global shortcut itself is registered by the app (system module), which calls
//! [`on_shortcut`]; the floating pill window listens to `voice-pill` ("show" | "hide").
//!
//! Events: `voice-level` (f32 0..1, ~30/s), `voice-state` ("idle" | "listening" |
//! "downloading" | "transcribing" | "error"), `voice-download` ({model, received, total}),
//! `voice-transcript` (String), `voice-error` (String).

mod audio;
pub mod config;
mod local;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use crate::AppHandle;

use audio::Capture;
pub use config::{Engine, Shortcut, VoiceConfig, VoiceConfigView, WhisperModel};

const MIN_CLIP_SECS: f32 = 0.3;
/// Auto-stop (and transcribe) if the key-up is lost.
const MAX_RECORDING: Duration = Duration::from_secs(60);
const HIDE_DELAY: Duration = Duration::from_millis(1500);
const ERROR_HIDE_DELAY: Duration = Duration::from_secs(4);
/// Voice state, managed on the [`AppHandle`].
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
pub struct DownloadProgress {
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

/// Loads settings and voice state. Called from [`crate::start`]. The app registers
/// `voice_get_config().config.shortcut` as a global shortcut and forwards it to [`on_shortcut`].
pub fn init(app: &AppHandle) -> Result<(), String> {
    let config = VoiceConfig::load(&config_path(app)?);
    app.manage(Voice {
        config: Mutex::new(config),
        ..Voice::default()
    });
    preload(app);
    Ok(())
}

/// Loads the local model in the background if it's already downloaded (no surprise download).
fn preload(app: &AppHandle) {
    let Ok(config) = config_snapshot(app) else {
        return;
    };
    if config.engine != Engine::Local {
        return;
    }
    let Some((path, _)) = models_dir(app)
        .ok()
        .and_then(|dir| local::downloaded(&dir, config.model))
    else {
        return;
    };
    std::thread::spawn(move || {
        if let Err(e) = local::preload(&path) {
            eprintln!("voice: preload model: {e}");
        }
    });
}

/// Global shortcut key down (`pressed`) or up: push-to-talk or toggle, per the settings.
pub fn on_shortcut(app: &AppHandle, pressed: bool) {
    let push_to_talk = app
        .state::<Voice>()
        .config
        .lock()
        .map_or(true, |c| c.push_to_talk);
    let result = match (pressed, push_to_talk) {
        (true, true) => start(app),
        (false, true) => stop(app, None),
        (true, false) if is_recording(app) => stop(app, None),
        (true, false) => start(app),
        (false, false) => Ok(()),
    };
    if let Err(e) = result {
        fail(app, &e);
    }
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.app_config_dir()?.join("voice.json"))
}

fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.app_data_dir()?.join("models"))
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

pub fn voice_get_config(app: &AppHandle) -> Result<VoiceConfigView, String> {
    config_snapshot(app).map(view)
}

/// Validates, applies live and persists the settings. The app swaps the global shortcut
/// (register the new one first; on failure keep the old one and do not call this).
pub fn voice_set_config(app: &AppHandle, config: VoiceConfig) -> Result<VoiceConfigView, String> {
    config.validate()?;
    *app.state::<Voice>()
        .config
        .lock()
        .map_err(|e| e.to_string())? = config.clone();
    config.save(&config_path(app)?)?;
    preload(app);
    Ok(view(config))
}

/// Saves the API key in the OS credential store; `None` deletes it.
pub fn voice_set_api_key(key: Option<String>) -> Result<(), String> {
    config::set_api_key(key.as_deref())
}

pub fn voice_list_models(app: &AppHandle) -> Result<Vec<ModelStatus>, String> {
    let dir = models_dir(app)?;
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

/// Downloads `model` (no-op if present). Blocking: call off the UI thread. Progress via `voice-download`.
pub fn voice_download_model(app: &AppHandle, model: WhisperModel) -> Result<(), String> {
    let dir = models_dir(app)?;
    local::ensure_model(&dir, model, |received, total| {
        emit_download(app, model, received, total);
    })
    .map(drop)
}

pub fn voice_start(app: &AppHandle) -> Result<(), String> {
    start(app).inspect_err(|e| fail(app, e))
}

pub fn voice_stop(app: &AppHandle) -> Result<(), String> {
    stop(app, None).inspect_err(|e| fail(app, e))
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
    warm_up(app);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(MAX_RECORDING);
        if let Err(e) = stop(&app, Some(session)) {
            fail(&app, &e);
        }
    });
    Ok(())
}

/// Local engine with the model on disk: one silent pass on key-down ramps the CPU clocks and pages
/// the model back in while the user starts speaking. No live preview: it kept the CPU busy and
/// delayed the final transcription.
fn warm_up(app: &AppHandle) {
    let Ok(config) = config_snapshot(app) else {
        return;
    };
    if config.engine != Engine::Local || local::is_warm() {
        return;
    }
    let Some((model, _)) = models_dir(app)
        .ok()
        .and_then(|dir| local::downloaded(&dir, config.model))
    else {
        return; // first use downloads the model on release
    };
    local::allow_partials();
    // Result discarded; aborted by stop() so the real transcription never waits behind it.
    std::thread::spawn(move || drop(local::transcribe_partial(&model, &[], &config.language)));
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
        // Silence around the speech is cut first: a smaller upload and less audio for the server.
        Engine::Api => transcribe_api(&config, audio::wav_16bit_mono(local::speech(&pcm), audio::TARGET_RATE))?,
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
    // Reused so later dictations skip the TCP + TLS handshake.
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    let mut request = CLIENT
        .get_or_init(reqwest::blocking::Client::new)
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
        if app.state::<Voice>().generation.load(Ordering::SeqCst) == generation {
            let _ = app.emit("voice-pill", "hide");
        }
    });
}

/// Asks the app to show the pill bottom-center of the monitor under the cursor, without focusing it.
fn show_pill(app: &AppHandle) {
    let _ = app.emit("voice-pill", "show");
}
