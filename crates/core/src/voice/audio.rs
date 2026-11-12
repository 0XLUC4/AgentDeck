//! Mic capture (cpal) on one long-lived thread, plus the pure DSP helpers:
//! RMS, linear resampling and in-memory 16-bit PCM WAV encoding.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

/// Sample rate expected by Whisper-style STT APIs.
pub const TARGET_RATE: u32 = 16_000;
/// ~30 level updates per second.
const LEVEL_INTERVAL: Duration = Duration::from_millis(33);
/// The mic stays open this long after a dictation: under CPU load (busy agents) opening it takes
/// 1-3 s instead of ~30 ms, so back-to-back dictations start at once. Samples are dropped meanwhile.
const KEEP_OPEN: Duration = Duration::from_secs(120);

type Shared = Arc<Mutex<Vec<f32>>>;
/// Captured mono samples and their sample rate.
pub type Recording = (Vec<f32>, u32);
type Level = Box<dyn Fn(f32) + Send>;

enum Command {
    Start(Level, SyncSender<Result<u32, String>>),
    Stop(SyncSender<Result<Recording, String>>),
}

/// Sender to the mic thread, spawned on first use. cpal's `Stream` is `!Send`, so it lives there.
static MIC: Mutex<Option<Sender<Command>>> = Mutex::new(None);

fn send(command: Command) -> Result<(), String> {
    let mut mic = MIC.lock().map_err(|e| e.to_string())?;
    let tx = mic.get_or_insert_with(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || mic_thread(rx));
        tx
    });
    if tx.send(command).is_err() {
        *mic = None; // thread died (panic): the next call spawns a fresh one
        return Err("microphone thread exited".into());
    }
    Ok(())
}

/// A running capture (at most one at a time).
pub struct Capture(());

impl Capture {
    /// Starts recording from the default input device (opened unless still open from the last
    /// dictation). `on_level` receives the RMS of the latest samples ~30 times per second.
    pub fn start(on_level: impl Fn(f32) + Send + 'static) -> Result<Self, String> {
        let (reply, ready) = mpsc::sync_channel(1);
        send(Command::Start(Box::new(on_level), reply))?;
        ready.recv().map_err(|_| "microphone thread exited".to_string())??;
        Ok(Self(()))
    }

    /// Stops recording and returns the mono samples with their sample rate.
    pub fn stop(self) -> Result<Recording, String> {
        let (reply, done) = mpsc::sync_channel(1);
        send(Command::Stop(reply))?;
        done.recv().map_err(|_| "microphone thread exited".to_string())?
    }
}

/// Owns the stream: opens it on start, keeps it [`KEEP_OPEN`] after stop, reopens it after an error.
fn mic_thread(commands: Receiver<Command>) {
    boost_priority();
    let buffer = Shared::default();
    let recording = Arc::new(AtomicBool::new(false));
    let broken = Arc::new(AtomicBool::new(false));
    let mut stream: Option<(cpal::Stream, u32)> = None;
    let mut on_level: Option<Level> = None;
    let mut seen = 0;
    let mut idle_since = Instant::now();
    loop {
        let command = match (&on_level, &stream) {
            (Some(_), _) => commands.recv_timeout(LEVEL_INTERVAL),
            (None, Some(_)) => commands.recv_timeout(KEEP_OPEN.saturating_sub(idle_since.elapsed())),
            (None, None) => commands.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match command {
            Ok(Command::Start(level, reply)) => {
                if broken.swap(false, Ordering::SeqCst) {
                    stream = None; // device lost or reset: reopen the current default one
                }
                if stream.is_none() {
                    // Windows often refuses the mic for a moment right after another app released it: retry once.
                    let opened = open_stream(&buffer, &recording, &broken).or_else(|_| {
                        std::thread::sleep(Duration::from_millis(250));
                        open_stream(&buffer, &recording, &broken)
                    });
                    match opened {
                        Ok(opened) => stream = Some(opened),
                        Err(e) => {
                            let _ = reply.send(Err(e));
                            continue;
                        }
                    }
                }
                if let Ok(mut samples) = buffer.lock() {
                    samples.clear();
                }
                seen = 0;
                recording.store(true, Ordering::SeqCst);
                on_level = Some(level);
                let _ = reply.send(Ok(stream.as_ref().map_or(0, |(_, rate)| *rate)));
            }
            Ok(Command::Stop(reply)) => {
                recording.store(false, Ordering::SeqCst);
                on_level = None;
                idle_since = Instant::now();
                let rate = stream.as_ref().map_or(0, |(_, rate)| *rate);
                let samples = buffer.lock().map(|mut s| std::mem::take(&mut *s)).map_err(|e| e.to_string());
                let _ = reply.send(samples.map(|s| (s, rate)));
            }
            Err(RecvTimeoutError::Timeout) => match &on_level {
                Some(level) => {
                    if let Ok(samples) = buffer.lock() {
                        level(rms(&samples[seen.min(samples.len())..]));
                        seen = samples.len();
                    }
                }
                None => stream = None, // idle long enough: release the mic
            },
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Runs the mic thread above normal priority: with busy agents saturating the CPU, opening the mic
/// at normal priority took ~2x longer (cpal boosts its own callback thread already).
#[cfg(windows)]
fn boost_priority() {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST};
    // SAFETY: pseudo-handle of the calling thread, always valid.
    if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) } == 0 {
        eprintln!("voice: cannot raise capture priority: {}", std::io::Error::last_os_error());
    }
}

#[cfg(not(windows))]
fn boost_priority() {}

fn open_stream(buffer: &Shared, recording: &Arc<AtomicBool>, broken: &Arc<AtomicBool>) -> Result<(cpal::Stream, u32), String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("no microphone found")?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("microphone config: {e}"))?;
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, buffer.clone(), recording.clone(), broken.clone()),
        SampleFormat::I16 => build::<i16>(&device, &config, buffer.clone(), recording.clone(), broken.clone()),
        SampleFormat::U16 => build::<u16>(&device, &config, buffer.clone(), recording.clone(), broken.clone()),
        SampleFormat::I32 => build::<i32>(&device, &config, buffer.clone(), recording.clone(), broken.clone()),
        SampleFormat::I8 => build::<i8>(&device, &config, buffer.clone(), recording.clone(), broken.clone()),
        SampleFormat::U8 => build::<u8>(&device, &config, buffer.clone(), recording.clone(), broken.clone()),
        other => return Err(format!("unsupported sample format {other}")),
    }
    .map_err(|e| format!("microphone stream: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("microphone start: {e}"))?;
    Ok((stream, config.sample_rate))
}

/// Builds an input stream that downmixes every frame to mono f32 into `buffer` while `recording`,
/// and sets `broken` on a stream error (device unplugged or reset).
fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Shared,
    recording: Arc<AtomicBool>,
    broken: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    device.build_input_stream(
        *config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if !recording.load(Ordering::Relaxed) {
                return;
            }
            if let Ok(mut out) = buffer.lock() {
                out.extend(data.chunks(channels).map(|frame| {
                    frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>() / frame.len() as f32
                }));
            }
        },
        move |e| {
            eprintln!("voice: stream error: {e}");
            broken.store(true, Ordering::SeqCst);
        },
        None,
    )
}

/// Root mean square of `samples` (0 for an empty slice).
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Linear-interpolation resampler.
// Note: no anti-alias low-pass before downsampling; fine for speech STT, upgrade if quality suffers.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || from == 0 || input.is_empty() {
        return input.to_vec();
    }
    let out_len = (input.len() as u64 * u64::from(to) / u64::from(from)) as usize;
    let step = f64::from(from) / f64::from(to);
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * step;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx];
            let b = input.get(idx + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}

/// Encodes mono samples as a 16-bit PCM WAV file (44-byte header + data).
pub fn wav_16bit_mono(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_values() {
        assert_eq!(rms(&[]), 0.0);
        assert!((rms(&[0.5, -0.5, 0.5, -0.5]) - 0.5).abs() < 1e-6);
        assert!((rms(&[1.0, 0.0]) - (0.5f32).sqrt()).abs() < 1e-6);
    }

    #[test]
    fn resample_length_and_values() {
        let input: Vec<f32> = (0..48_000).map(|i| i as f32).collect();
        let out = resample(&input, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        assert_eq!(out[1], 3.0); // every 3rd input sample
        let up = resample(&[0.0, 1.0], 1, 2);
        assert_eq!(up, vec![0.0, 0.5, 1.0, 1.0]);
        assert_eq!(resample(&[0.1, 0.2], 16_000, 16_000), vec![0.1, 0.2]);
    }

    #[test]
    fn wav_header_bytes() {
        let wav = wav_16bit_mono(&[0.0, 1.0, -1.0], 16_000);
        assert_eq!(wav.len(), 44 + 6);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[4..8], &42u32.to_le_bytes());
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(&wav[16..20], &16u32.to_le_bytes());
        assert_eq!(&wav[20..24], &[1, 0, 1, 0]);
        assert_eq!(&wav[24..28], &16_000u32.to_le_bytes());
        assert_eq!(&wav[28..32], &32_000u32.to_le_bytes());
        assert_eq!(&wav[32..36], &[2, 0, 16, 0]);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(&wav[40..44], &6u32.to_le_bytes());
        assert_eq!(&wav[44..], &[0, 0, 0xFF, 0x7F, 0x01, 0x80]);
    }
}
