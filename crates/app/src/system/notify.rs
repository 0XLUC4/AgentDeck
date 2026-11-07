//! Desktop notification (Windows toast), short two-tone beep and phone push (ntfy topic or webhook).

use agentdeck_core::logic::settings::Settings;

/// Notifies per the user's settings: phone push when `phone_url` is set, beep when `notify.sound`,
/// toast when `notify.enabled`. Never blocks the caller; failures are logged.
pub fn notify(settings: &Settings, title: &str, body: &str) {
    let phone_url = settings.phone_url.clone();
    if !phone_url.is_empty() {
        let (title, body) = (title.to_owned(), body.to_owned());
        std::thread::spawn(move || {
            if let Err(e) = agentdeck_core::notify::push_notify(phone_url, title, body) {
                eprintln!("phone push failed: {e}");
            }
        });
    }
    if settings.notify.sound {
        std::thread::spawn(beep);
    }
    if settings.notify.enabled {
        let (title, body) = (title.to_owned(), body.to_owned());
        std::thread::spawn(move || {
            let mut toast = notify_rust::Notification::new();
            toast.summary(&title).body(&body).appname("AgentDeck");
            if let Some(id) = toast_app_id() {
                toast.app_id(id);
            }
            if let Err(e) = toast.show() {
                eprintln!("notification failed: {e}");
            }
        });
    }
}

/// Toast sender id with AgentDeck's name and icon. Without it Windows shows toasts as PowerShell's.
/// Registered once per run under HKCU (no installer step needed); None if that fails.
fn toast_app_id() -> Option<&'static str> {
    static ID: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
    *ID.get_or_init(|| match register_app_id() {
        Ok(()) => Some(agentdeck_core::APP_IDENTIFIER),
        Err(e) => {
            eprintln!("notification icon not registered: {e}");
            None
        }
    })
}

/// HKCU\Software\Classes\AppUserModelId\<id>: DisplayName + IconUri (a PNG next to the app data).
/// Ref: https://learn.microsoft.com/windows/apps/design/shell/tiles-and-notifications/send-local-toast-other-apps
#[cfg(windows)]
fn register_app_id() -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
    let dir = agentdeck_core::AppHandle::default_data_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let icon = dir.join("notification-icon.png");
    std::fs::write(&icon, include_bytes!("../../assets/logo.png")).map_err(|e| e.to_string())?;
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let key = wide(&format!(r"Software\Classes\AppUserModelId\{}", agentdeck_core::APP_IDENTIFIER));
    for (name, value) in [("DisplayName", "AgentDeck".to_owned()), ("IconUri", icon.display().to_string())] {
        let (name, value) = (wide(name), wide(&value));
        // SAFETY: NUL-terminated UTF-16 buffers that outlive the call; size in bytes includes the NUL.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        };
        if status != 0 {
            return Err(format!("RegSetKeyValueW: error {status}"));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn register_app_id() -> Result<(), String> {
    Err("Windows only".into())
}

const SAMPLE_RATE: u32 = 22_050;
/// Peak level (0..1): a quiet chime, not an alarm.
const VOLUME: f32 = 0.18;

/// Soft two-note chime (880 Hz then 660 Hz, like the original WebAudio beep) as a 16-bit mono WAV:
/// sine tones with a short attack and exponential decay, so there is no click and no harsh edge.
#[cfg_attr(not(windows), allow(dead_code))]
fn chime_wav() -> Vec<u8> {
    let notes = [(880.0f32, 0.0f32, 0.35f32), (660.0, 0.12, 0.5)]; // (Hz, start s, length s)
    let total = notes.iter().map(|&(_, start, len)| start + len).fold(0.0, f32::max);
    let count = (total * SAMPLE_RATE as f32) as usize;
    let mut samples = vec![0.0f32; count];
    for (freq, start, len) in notes {
        let first = (start * SAMPLE_RATE as f32) as usize;
        for n in 0..(len * SAMPLE_RATE as f32) as usize {
            let t = n as f32 / SAMPLE_RATE as f32;
            let envelope = (t / 0.01).min(1.0) * (-t * 9.0).exp();
            samples[first + n] += (std::f32::consts::TAU * freq * t).sin() * envelope;
        }
    }
    let data_len = (count * 2) as u32;
    let mut wav = Vec::with_capacity(44 + count * 2);
    for chunk in [&b"RIFF"[..], &(36 + data_len).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes()] {
        wav.extend_from_slice(chunk);
    }
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let value = (sample * VOLUME).clamp(-1.0, 1.0) * i16::MAX as f32;
        wav.extend_from_slice(&(value as i16).to_le_bytes());
    }
    wav
}

#[cfg(windows)]
fn beep() {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC};
    let wav = chime_wav();
    // SAFETY: SND_MEMORY reads the WAV from `wav`, which outlives the synchronous call.
    unsafe { PlaySoundW(wav.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_SYNC | SND_NODEFAULT) };
}

#[cfg(not(windows))]
fn beep() {}

#[cfg(test)]
mod tests {
    #[test]
    fn chime_wav_header_matches_data() {
        let wav = super::chime_wav();
        assert_eq!(&wav[..4], b"RIFF");
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(wav.len(), 44 + data_len);
    }
}
