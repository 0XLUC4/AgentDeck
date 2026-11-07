//! Spoken task summaries with the Windows speech engine (System.Speech through PowerShell, the same
//! SAPI voices the WebView's speechSynthesis offered). Any key press silences it.
//! shortcut: one PowerShell process per utterance (~0.5 s start); move to direct SAPI COM calls if that lag matters.

use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use agentdeck_core::AppHandle;
use agentdeck_core::logic::settings::SpeechSettings;

static SPEAKING: Mutex<Option<Child>> = Mutex::new(None);

/// Picks the chosen voice, else the first voice of the language (the OS UI language when empty).
const SPEAK_SCRIPT: &str = r#"
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$voices = $s.GetInstalledVoices() | Where-Object Enabled | ForEach-Object { $_.VoiceInfo }
$lang = $env:AGENTDECK_TTS_LANG
if (-not $lang) { $lang = [Globalization.CultureInfo]::CurrentUICulture.Name }
$primary = $lang.Split('-')[0].ToLower()
$pick = $voices | Where-Object { $_.Name -eq $env:AGENTDECK_TTS_VOICE } | Select-Object -First 1
if (-not $pick) { $pick = $voices | Where-Object { $_.Culture.Name.ToLower().StartsWith($primary) } | Select-Object -First 1 }
if ($pick) { $s.SelectVoice($pick.Name) }
$s.Rate = [int]$env:AGENTDECK_TTS_RATE
$s.Speak($env:AGENTDECK_TTS_TEXT)
"#;

const VOICES_SCRIPT: &str = r#"
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$s.GetInstalledVoices() | Where-Object Enabled | ForEach-Object { $_.VoiceInfo.Name + "`t" + $_.VoiceInfo.Culture.Name }
"#;

/// An installed voice: `name` is what `settings.speech.voice` stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechVoice {
    pub name: String,
    pub lang: String,
}

fn powershell(script: &str) -> Command {
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Web Speech rate (0.1..10, 1 = normal) to SAPI rate (-10..10, about 3x per 10 steps).
fn sapi_rate(rate: f64) -> i32 {
    if !rate.is_finite() || rate <= 0.0 {
        return 0;
    }
    (10.0 * rate.ln() / 3f64.ln()).round().clamp(-10.0, 10.0) as i32
}

/// Speaks `text` (stops what was being said). The dictation language picks the voice when none is set.
pub fn speak(backend: &AppHandle, speech: &SpeechSettings, text: String) {
    let lang = agentdeck_core::voice::voice_get_config(backend)
        .map(|v| v.config.language)
        .ok()
        .filter(|l| !l.is_empty() && l != "auto")
        .unwrap_or_default();
    stop();
    let spawned = powershell(SPEAK_SCRIPT)
        .stdout(Stdio::null())
        .env("AGENTDECK_TTS_TEXT", text)
        .env("AGENTDECK_TTS_VOICE", &speech.voice)
        .env("AGENTDECK_TTS_LANG", lang)
        .env("AGENTDECK_TTS_RATE", sapi_rate(speech.rate).to_string())
        .spawn();
    match spawned {
        Ok(child) => {
            if let Ok(mut slot) = SPEAKING.lock() {
                *slot = Some(child);
            }
        }
        Err(e) => eprintln!("tts: {e}"),
    }
}

/// Silences the current utterance, if any.
pub fn stop() {
    let Ok(mut slot) = SPEAKING.lock() else {
        return;
    };
    if let Some(mut child) = slot.take()
        && matches!(child.try_wait(), Ok(None))
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Installed voices. Blocking (starts PowerShell): call from `cx.background_spawn`.
pub fn voices() -> Result<Vec<SpeechVoice>, String> {
    let out = powershell(VOICES_SCRIPT).output().map_err(|e| format!("speech voices: {e}"))?;
    if !out.status.success() {
        return Err("speech voices: the speech engine is unavailable".into());
    }
    Ok(parse_voices(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_voices(text: &str) -> Vec<SpeechVoice> {
    text.lines()
        .filter_map(|line| {
            let (name, lang) = line.trim_end().split_once('\t')?;
            Some(SpeechVoice { name: name.to_owned(), lang: lang.to_owned() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn rate_mapping() {
        assert_eq!(sapi_rate(1.0), 0);
        assert_eq!(sapi_rate(3.0), 10);
        assert_eq!(sapi_rate(10.0), 10);
        assert_eq!(sapi_rate(0.5), -6);
        assert_eq!(sapi_rate(f64::NAN), 0);
    }

    #[test]
    fn voice_lines() {
        let v = parse_voices("Microsoft David Desktop\ten-US\r\nbad line\nMicrosoft Hortense Desktop\tfr-FR\n");
        assert_eq!(v.len(), 2);
        assert_eq!(v[1], SpeechVoice { name: "Microsoft Hortense Desktop".into(), lang: "fr-FR".into() });
    }
}
