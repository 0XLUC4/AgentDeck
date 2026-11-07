//! Windows text-to-speech voices (System.Speech / SAPI) for Settings > Notifications.
//! shortcut: PowerShell instead of a SAPI COM binding; move to the voice module's TTS once it has one.

use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const LIST: &str = "Add-Type -AssemblyName System.Speech; \
    (New-Object System.Speech.Synthesis.SpeechSynthesizer).GetInstalledVoices() | \
    Where-Object { $_.Enabled } | ForEach-Object { $_.VoiceInfo.Name + '|' + $_.VoiceInfo.Culture.Name }";

const SPEAK: &str = "Add-Type -AssemblyName System.Speech; \
    $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
    if ($env:AGENTDECK_TTS_VOICE) { $s.SelectVoice($env:AGENTDECK_TTS_VOICE) }; \
    $s.Rate = [int]$env:AGENTDECK_TTS_RATE; $s.Speak($env:AGENTDECK_TTS_TEXT)";

fn powershell(script: &str) -> Command {
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}

/// Installed voices as (name, language). Blocking (about a second): call off the UI thread.
pub fn list_voices() -> Result<Vec<(String, String)>, String> {
    let out = powershell(LIST).output().map_err(|e| format!("powershell: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.trim().split_once('|'))
        .map(|(name, lang)| (name.to_owned(), lang.to_owned()))
        .collect())
}

/// Speech-synthesis rate 0.5..2 to the SAPI scale -10..10.
fn sapi_rate(rate: f64) -> i32 {
    (((rate - 1.0) * 10.0).round() as i32).clamp(-10, 10)
}

/// Speaks `text` with `voice` ("" = system default) in the background.
pub fn speak(text: &str, voice: &str, rate: f64) -> Result<(), String> {
    // Values go through the environment, never into the script text.
    powershell(SPEAK)
        .env("AGENTDECK_TTS_VOICE", voice)
        .env("AGENTDECK_TTS_RATE", sapi_rate(rate).to_string())
        .env("AGENTDECK_TTS_TEXT", text)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("powershell: {e}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn rate_scale() {
        assert_eq!(super::sapi_rate(1.0), 0);
        assert_eq!(super::sapi_rate(0.5), -5);
        assert_eq!(super::sapi_rate(2.0), 10);
    }
}
