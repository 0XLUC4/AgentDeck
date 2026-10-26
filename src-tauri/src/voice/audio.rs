//! Mic capture (cpal) on a dedicated thread, plus the pure DSP helpers:
//! RMS, linear resampling and in-memory 16-bit PCM WAV encoding.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

/// Sample rate expected by Whisper-style STT APIs.
pub const TARGET_RATE: u32 = 16_000;
/// ~30 level updates per second.
const LEVEL_INTERVAL: Duration = Duration::from_millis(33);

type Shared = Arc<Mutex<Vec<f32>>>;
/// Captured mono samples and their sample rate.
pub type Recording = (Vec<f32>, u32);

/// A running capture. cpal's `Stream` is `!Send`, so it lives on its own thread.
pub struct Capture {
    stop: Sender<()>,
    thread: JoinHandle<Result<Recording, String>>,
}

impl Capture {
    /// Opens the default input device and starts recording. `on_level` receives
    /// the RMS of the latest samples ~30 times per second.
    pub fn start(on_level: impl Fn(f32) + Send + 'static) -> Result<Self, String> {
        let (stop, stop_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || run(ready_tx, stop_rx, on_level));
        ready_rx
            .recv()
            .map_err(|_| "capture thread exited early".to_string())??;
        Ok(Self { stop, thread })
    }

    /// Stops recording and returns the mono samples with their sample rate.
    pub fn stop(self) -> Result<Recording, String> {
        let _ = self.stop.send(()); // thread may already be gone; join reports why
        self.thread
            .join()
            .map_err(|_| "capture thread panicked".to_string())?
    }
}

fn run(
    ready: SyncSender<Result<(), String>>,
    stop: Receiver<()>,
    on_level: impl Fn(f32),
) -> Result<Recording, String> {
    let buffer: Shared = Arc::default();
    let (stream, rate) = match open_stream(buffer.clone()) {
        Ok(opened) => opened,
        Err(e) => {
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    let _ = ready.send(Ok(()));

    let mut seen = 0;
    // Stop message or disconnected sender both end the capture.
    while let Err(RecvTimeoutError::Timeout) = stop.recv_timeout(LEVEL_INTERVAL) {
        let samples = buffer.lock().map_err(|e| e.to_string())?;
        on_level(rms(&samples[seen..]));
        seen = samples.len();
    }
    drop(stream);
    let samples = std::mem::take(&mut *buffer.lock().map_err(|e| e.to_string())?);
    Ok((samples, rate))
}

fn open_stream(buffer: Shared) -> Result<(cpal::Stream, u32), String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("no microphone found")?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("microphone config: {e}"))?;
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, buffer),
        SampleFormat::I16 => build::<i16>(&device, &config, buffer),
        SampleFormat::U16 => build::<u16>(&device, &config, buffer),
        SampleFormat::I32 => build::<i32>(&device, &config, buffer),
        SampleFormat::I8 => build::<i8>(&device, &config, buffer),
        SampleFormat::U8 => build::<u8>(&device, &config, buffer),
        other => return Err(format!("unsupported sample format {other}")),
    }
    .map_err(|e| format!("microphone stream: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("microphone start: {e}"))?;
    Ok((stream, config.sample_rate))
}

/// Builds an input stream that downmixes every frame to mono f32 into `buffer`.
fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Shared,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    device.build_input_stream(
        *config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if let Ok(mut out) = buffer.lock() {
                out.extend(data.chunks(channels).map(|frame| {
                    frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>() / frame.len() as f32
                }));
            }
        },
        |e| eprintln!("voice: stream error: {e}"),
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
// shortcut: no anti-alias low-pass before downsampling; fine for speech STT, upgrade if quality suffers.
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
