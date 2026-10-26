//! Offline transcription with whisper.cpp (whisper-rs): model download, cached
//! model state, and a filter for Whisper's classic silence hallucinations.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

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
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result.map(|()| path)
}

// shortcut: checks byte count against Content-Length, not the SHA-256; add it if corrupt models show up.
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

/// Transcribes 16 kHz mono samples with the model at `model_path`.
/// `language` is a code like "fr", or "auto" to detect it.
pub fn transcribe(model_path: &Path, samples: &[f32], language: &str) -> Result<String, String> {
    let mut loaded = LOADED.lock().map_err(|e| e.to_string())?;
    if loaded.as_ref().is_none_or(|(path, _)| path != model_path) {
        *loaded = None; // free the previous model before loading another
        let path = model_path.to_str().ok_or("model path is not valid UTF-8")?;
        let context = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|e| format!("cannot load {}: {e}", model_path.display()))?;
        let state = context
            .create_state()
            .map_err(|e| format!("cannot init whisper: {e}"))?;
        *loaded = Some((model_path.to_path_buf(), state));
    }
    let (_, state) = loaded.as_mut().ok_or("model not loaded")?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    params.set_n_threads(i32::try_from(threads).unwrap_or(4));
    params.set_language(Some(language));
    params.set_no_timestamps(true);
    params.set_no_context(true);
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    state
        .full(params, samples)
        .map_err(|e| format!("transcription failed: {e}"))?;
    let text: String = state
        .as_iter()
        .filter_map(|segment| segment.to_str_lossy().ok().map(|s| s.into_owned()))
        .collect();
    Ok(clean(&text, samples.len() as f32 / 16_000.0))
}

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
    let caption = ["sous-titr", "amara.org", "abonnez-vous"]
        .iter()
        .any(|pattern| normalized.contains(pattern));
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
    }
}
