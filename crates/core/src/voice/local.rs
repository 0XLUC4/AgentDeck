//! Offline transcription with whisper.cpp (whisper-rs): model download, cached
//! model state, and a filter for Whisper's classic silence hallucinations.

use std::ffi::c_void;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use super::config::WhisperModel;

/// Loaded model + its decoding state, reused across transcriptions.
static LOADED: Mutex<Option<(PathBuf, WhisperState)>> = Mutex::new(None);
/// Serializes downloads so two callers never write the same `.part` file.
static DOWNLOAD: Mutex<()> = Mutex::new(());

/// Path of `model` in `dir` if it is fully downloaded.
pub fn downloaded(dir: &Path, model: WhisperModel) -> Option<(PathBuf, u64)> {
    let path = dir.join(model.file_name());
    let size = path.metadata().ok()?.len();
    (size > 0).then_some((path, size))
}

/// Returns the model path, downloading it first if needed. `on_progress(received, total)`
/// is called at most once per percent (total = 0 when unknown).
pub fn ensure_model(
    dir: &Path,
    model: WhisperModel,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<PathBuf, String> {
    let _guard = DOWNLOAD.lock().map_err(|e| e.to_string())?;
    if let Some((path, _)) = downloaded(dir, model) {
        return Ok(path);
    }
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = dir.join(model.file_name());
    let part = dir.join(format!("{}.part", model.file_name()));
    let result = download(&model.url(), &part, &mut on_progress)
        .and_then(|()| fs::rename(&part, &path).map_err(|e| format!("save model: {e}")));
    match result {
        Ok(()) => {
            let _ = fs::remove_file(dir.join(model.legacy_file_name()));
            Ok(path)
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

// Note: checks byte count against Content-Length, not the SHA-256; add it if corrupt models show up.
fn download(url: &str, part: &Path, on_progress: &mut impl FnMut(u64, u64)) -> Result<(), String> {
    let mut response = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(None) // a ~500 MB file: the default 30 s total timeout would kill it
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| format!("model download failed: {e}"))?;
    let total = response.content_length().unwrap_or(0);
    let mut file = fs::File::create(part).map_err(|e| format!("create {}: {e}", part.display()))?;
    let mut buffer = vec![0; 1 << 16];
    let (mut received, mut last_percent) = (0u64, None);
    on_progress(0, total);
    loop {
        let n = response
            .read(&mut buffer)
            .map_err(|e| format!("model download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n])
            .map_err(|e| format!("write model: {e}"))?;
        received += n as u64;
        let percent = (total > 0).then(|| received * 100 / total);
        if percent != last_percent || total == 0 {
            last_percent = percent;
            on_progress(received, total);
        }
    }
    file.sync_all().map_err(|e| format!("write model: {e}"))?;
    if received == 0 || (total > 0 && received != total) {
        return Err(format!(
            "model download incomplete ({received}/{total} bytes)"
        ));
    }
    Ok(())
}

/// End of the last inference: a model used moments ago is still paged in and the CPU still clocked up.
static LAST_RUN: Mutex<Option<Instant>> = Mutex::new(None);
const WARM_FOR: Duration = Duration::from_secs(45);

/// True when a warm-up pass would only burn CPU (and battery) for nothing.
pub fn is_warm() -> bool {
    LAST_RUN.lock().is_ok_and(|last| last.is_some_and(|at| at.elapsed() < WARM_FOR))
}

/// Loads the model ahead of the first dictation so the first press isn't slow.
pub fn preload(model_path: &Path) -> Result<(), String> {
    load(&mut *LOADED.lock().map_err(|e| e.to_string())?, model_path)?;
    // One pass on silence: the first inference allocates buffers (and compiles GPU kernels with
    // the `gpu` feature), which would otherwise land on the user's first dictation.
    // Best effort: a dictation started meanwhile cancels it.
    let _ = run(model_path, &[], "en", true);
    Ok(())
}

fn load(loaded: &mut Option<(PathBuf, WhisperState)>, model_path: &Path) -> Result<(), String> {
    if loaded.as_ref().is_none_or(|(path, _)| path != model_path) {
        *loaded = None; // free the previous model before loading another
        let path = model_path.to_str().ok_or("model path is not valid UTF-8")?;
        let mut params = WhisperContextParameters::default();
        params.flash_attn(true); // faster attention, also on CPU
        let context = WhisperContext::new_with_params(path, params)
            .map_err(|e| format!("cannot load {}: {e}", model_path.display()))?;
        let state = context
            .create_state()
            .map_err(|e| format!("cannot init whisper: {e}"))?;
        *loaded = Some((model_path.to_path_buf(), state));
    }
    Ok(())
}

/// Set to stop an in-flight partial transcription: the final one must not wait behind it.
static CANCEL_PARTIAL: AtomicBool = AtomicBool::new(false);

// Raw callback on purpose: whisper-rs 0.16's `set_abort_callback_safe` reads its boxed closure back as the wrong type.
unsafe extern "C" fn partial_cancelled(_: *mut c_void) -> bool {
    CANCEL_PARTIAL.load(Ordering::Relaxed)
}

/// A new recording starts: partial transcripts may run again.
pub fn allow_partials() {
    CANCEL_PARTIAL.store(false, Ordering::Relaxed);
}

/// Live preview while the user speaks. Gives up (Err) as soon as the final transcription starts.
pub fn transcribe_partial(
    model_path: &Path,
    samples: &[f32],
    language: &str,
) -> Result<String, String> {
    run(model_path, samples, language, true)
}

/// Transcribes 16 kHz mono samples with the model at `model_path`.
/// `language` is a code like "fr", or "auto" to detect it.
pub fn transcribe(model_path: &Path, samples: &[f32], language: &str) -> Result<String, String> {
    CANCEL_PARTIAL.store(true, Ordering::Relaxed);
    run(model_path, samples, language, false)
}

fn run(
    model_path: &Path,
    samples: &[f32],
    language: &str,
    partial: bool,
) -> Result<String, String> {
    static HIGH_QOS: std::sync::Once = std::sync::Once::new();
    HIGH_QOS.call_once(disable_power_throttling);
    let mut loaded = LOADED.lock().map_err(|e| e.to_string())?;
    if partial && CANCEL_PARTIAL.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    load(&mut loaded, model_path)?;
    let (_, state) = loaded.as_mut().ok_or("model not loaded")?;

    // Hallucination filter judges the clip as recorded; whisper only sees the speech part.
    let recorded_secs = samples.len().max(MIN_SAMPLES) as f32 / 16_000.0;
    // whisper.cpp silently returns nothing for clips under 1 s: pad with silence.
    let mut samples = speech(samples).to_vec();
    samples.resize(samples.len().max(MIN_SAMPLES), 0.0);

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    // Hyperthreads slow whisper.cpp down: use about the physical cores, capped at 8.
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get() / 2);
    params.set_n_threads(threads.clamp(2, 8) as i32);
    params.set_language(Some(language));
    // Full 30 s encoder context on purpose: stock models garble or repeat words with a shorter one.
    params.set_single_segment(true);
    // No temperature fallback: retries can re-decode a clip up to 5 times.
    params.set_temperature_inc(0.0);
    if partial {
        // SAFETY: a plain fn reading a static; no user data pointer is involved.
        unsafe { params.set_abort_callback(Some(partial_cancelled)) };
    }
    params.set_no_timestamps(true);
    params.set_no_context(true);
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    state
        .full(params, &samples)
        .map_err(|e| format!("transcription failed: {e}"))?;
    if let Ok(mut last) = LAST_RUN.lock() {
        *last = Some(Instant::now());
    }
    let text: String = state
        .as_iter()
        .filter_map(|segment| segment.to_str_lossy().ok().map(|s| s.into_owned()))
        .collect();
    Ok(clean(&text, recorded_secs))
}

/// Opts the process out of Windows power throttling (EcoQoS): otherwise Windows may run
/// whisper's threads at low clocks or on efficiency cores while the app is in the background.
/// Process-wide on purpose: whisper.cpp spawns its own worker threads, which inherit it.
#[cfg(windows)]
fn disable_power_throttling() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
        ProcessPowerThrottling, SetProcessInformation,
    };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0, // control bit set, state bit clear: throttling off (HighQoS)
    };
    // SAFETY: valid pseudo-handle and a correctly sized struct that outlives the call.
    let ok = unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&raw const state).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    };
    if ok == 0 {
        eprintln!(
            "voice: cannot disable power throttling: {}",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(not(windows))]
fn disable_power_throttling() {}

/// 20 ms frames at 16 kHz.
const FRAME: usize = 320;
/// Kept around the detected speech so soft word starts and ends survive.
const SPEECH_MARGIN: usize = 16_000 * 3 / 10;

/// Cuts the silence before and after speech (key held before talking, released late): a shorter
/// clip means a smaller encoder context, the main cost on CPU. Returns everything if unsure.
pub fn speech(samples: &[f32]) -> &[f32] {
    let rms: Vec<f32> = samples
        .chunks(FRAME)
        .map(|f| (f.iter().map(|s| s * s).sum::<f32>() / f.len() as f32).sqrt())
        .collect();
    if rms.len() < 10 {
        return samples;
    }
    let mut sorted = rms.clone();
    sorted.sort_by(f32::total_cmp);
    let noise = sorted[sorted.len() / 10];
    let threshold = (noise * 3.0).max(0.003);
    let (Some(first), Some(last)) = (
        rms.iter().position(|&r| r > threshold),
        rms.iter().rposition(|&r| r > threshold),
    ) else {
        return samples; // quiet mic or no clear speech: let whisper decide
    };
    let start = (first * FRAME).saturating_sub(SPEECH_MARGIN);
    let end = ((last + 1) * FRAME + SPEECH_MARGIN).min(samples.len());
    &samples[start..end]
}

/// 1.2 s at 16 kHz: just over whisper.cpp's 1 s minimum.
const MIN_SAMPLES: usize = 19_200;

/// Drops non-speech tags ("[Musique]", "(rires)") and the captions/politeness
/// lines Whisper invents on silence; trims the rest.
pub fn clean(text: &str, secs: f32) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut closing = None;
    for c in text.chars() {
        match (closing, c) {
            (None, '[') => closing = Some(']'),
            (None, '(') => closing = Some(')'),
            (Some(end), c) if c == end => closing = None,
            (None, c) => kept.push(c),
            _ => {}
        }
    }
    let kept = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized = kept
        .to_lowercase()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string();
    // French patterns on purpose: Whisper's typical silence hallucinations in French ("subtitles by", "subscribe").
    let caption = ["sous-titr", "amara.org", "abonnez-vous"]
        .iter()
        .any(|pattern| normalized.contains(pattern));
    // Short politeness fillers ("thanks", "goodbye") in French and English.
    let filler = secs < 2.0
        && [
            "merci",
            "merci beaucoup",
            "au revoir",
            "thank you",
            "thanks for watching",
            "bye",
        ]
        .contains(&normalized.as_str());
    if caption || filler || normalized.is_empty() {
        String::new()
    } else {
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_keeps_speech() {
        assert_eq!(
            clean("  Bonjour, lance les tests.  ", 3.0),
            "Bonjour, lance les tests."
        );
        assert_eq!(clean("Merci.", 3.0), "Merci.");
        assert_eq!(
            clean("Ouvre le fichier [inaudible] main.rs", 3.0),
            "Ouvre le fichier main.rs"
        );
    }

    #[test]
    fn clean_drops_hallucinations() {
        for text in [
            "[Musique]",
            " (musique) ",
            "[BLANK_AUDIO]",
            "Sous-titres réalisés par la communauté d'Amara.org",
            "Sous-titrage ST' 501",
            "...",
            "",
        ] {
            assert_eq!(clean(text, 5.0), "", "{text}");
        }
        assert_eq!(clean(" Merci. ", 1.0), "");
        assert_eq!(clean("Thank you.", 0.8), "");
    }

    #[test]
    fn speech_trims_silence_around_voice() {
        let mut clip = vec![0.0005_f32; 16_000 * 3];
        for (i, s) in clip[16_000..24_000].iter_mut().enumerate() {
            *s = 0.2 * (i as f32 * 0.05).sin();
        }
        let kept = speech(&clip);
        assert!(kept.len() < 16_000 * 2 && kept.len() >= 8_000 + 2 * SPEECH_MARGIN - FRAME);
        assert_eq!(speech(&vec![0.0; 16_000]).len(), 16_000);
    }

    #[test]
    fn power_throttling_opt_out_is_harmless() {
        disable_power_throttling();
        disable_power_throttling();
    }

    #[test]
    fn missing_model_is_not_downloaded() {
        let dir = std::env::temp_dir().join("agentdeck-no-models");
        assert!(downloaded(&dir, WhisperModel::Small).is_none());
    }

    /// Downloads a model (AGENTDECK_WHISPER_MODEL, default base) and transcribes 1 s of silence.
    /// Run: cargo test --lib local_silence -- --ignored --nocapture
    #[test]
    #[ignore = "downloads a model (~150 MB+)"]
    fn local_silence() {
        let model = std::env::var("AGENTDECK_WHISPER_MODEL")
            .ok()
            .and_then(|m| WhisperModel::parse(&m))
            .unwrap_or(WhisperModel::Base);
        let dir = std::env::temp_dir().join("agentdeck-models");
        let started = std::time::Instant::now();
        let path = ensure_model(&dir, model, |_, _| {}).expect("download");
        println!("model ready {:?} in {:?}", path, started.elapsed());
        let silence = vec![0.0f32; 16_000];
        let started = std::time::Instant::now();
        let first = transcribe(&path, &silence, "fr").expect("transcribe");
        println!("load + 1 s silence: {:?} -> {first:?}", started.elapsed());
        let started = std::time::Instant::now();
        let second = transcribe(&path, &silence, "fr").expect("transcribe");
        println!("warm 1 s silence: {:?} -> {second:?}", started.elapsed());
        assert_eq!(first, "");
        assert_eq!(second, "");
        // Optional real speech: a 16 kHz 16-bit mono WAV (plain 44-byte header).
        if let Ok(wav) = std::env::var("AGENTDECK_TEST_WAV") {
            let bytes = fs::read(&wav).expect("read wav");
            let pcm: Vec<f32> = bytes[44..]
                .chunks_exact(2)
                .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0)
                .collect();
            let started = std::time::Instant::now();
            let text = transcribe(&path, &pcm, "fr").expect("transcribe");
            println!("{:.1} s of speech in {:?} -> {text:?}", pcm.len() as f32 / 16_000.0, started.elapsed());
        }
    }
}
