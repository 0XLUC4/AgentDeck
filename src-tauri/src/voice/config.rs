//! Voice settings: persisted JSON (no secrets) + API key in the OS credential store.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri_plugin_global_shortcut::Shortcut;

pub const DEFAULT_STT_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
const KEYRING_SERVICE: &str = "agentdeck";
const KEYRING_USER: &str = "stt-api-key";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Local,
    Api,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WhisperModel {
    #[serde(rename = "base")]
    Base,
    #[serde(rename = "small")]
    Small,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "large-v3-turbo")]
    LargeV3Turbo,
}

impl WhisperModel {
    pub const ALL: [Self; 4] = [Self::Base, Self::Small, Self::Medium, Self::LargeV3Turbo];

    pub fn name(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::LargeV3Turbo => "large-v3-turbo",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    pub fn file_name(self) -> String {
        format!("ggml-{}.bin", self.name())
    }

    pub fn url(self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            self.file_name()
        )
    }
}

/// Persisted settings. Missing fields fall back to `Default` (env-derived).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VoiceConfig {
    pub shortcut: String,
    pub push_to_talk: bool,
    pub submit: bool,
    pub engine: Engine,
    pub model: WhisperModel,
    pub language: String,
    pub api_url: String,
    pub api_model: String,
}

/// First-run defaults; AGENTDECK_STT_* / OPENAI_API_KEY select the API engine.
impl Default for VoiceConfig {
    fn default() -> Self {
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let api = ["AGENTDECK_STT_URL", "AGENTDECK_STT_KEY", "OPENAI_API_KEY"]
            .iter()
            .any(|name| env(name).is_some());
        Self {
            shortcut: "ctrl+space".into(),
            push_to_talk: true,
            submit: false,
            engine: if api { Engine::Api } else { Engine::Local },
            model: env("AGENTDECK_WHISPER_MODEL")
                .and_then(|m| WhisperModel::parse(&m))
                .unwrap_or(WhisperModel::Small),
            language: env("AGENTDECK_STT_LANG").unwrap_or_else(|| "fr".into()),
            api_url: env("AGENTDECK_STT_URL").unwrap_or_else(|| DEFAULT_STT_URL.into()),
            api_model: env("AGENTDECK_STT_MODEL").unwrap_or_else(|| "whisper-1".into()),
        }
    }
}

/// What `voice_get_config` returns: the config plus whether a key is available.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceConfigView {
    #[serde(flatten)]
    pub config: VoiceConfig,
    pub has_api_key: bool,
}

impl VoiceConfig {
    /// Reads `path`; a missing or corrupt file yields the defaults.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!("voice: ignoring invalid {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("write {}: {e}", path.display()))
    }

    /// Validates user input; returns the parsed shortcut.
    pub fn validate(&self) -> Result<Shortcut, String> {
        let shortcut = self
            .shortcut
            .parse::<Shortcut>()
            .map_err(|e| format!("invalid shortcut \"{}\": {e}", self.shortcut))?;
        check_language(&self.language)?;
        check_stt_url(&self.api_url)?;
        if self.api_model.trim().is_empty() {
            return Err("API model must not be empty".into());
        }
        Ok(shortcut)
    }
}

/// "auto" or an ISO 639-1/3 code ("fr", "en", "yue"...).
fn check_language(language: &str) -> Result<(), String> {
    let code =
        (2..=3).contains(&language.len()) && language.bytes().all(|b| b.is_ascii_lowercase());
    if language == "auto" || code {
        Ok(())
    } else {
        Err(format!(
            "invalid language \"{language}\" (use \"auto\" or a code like \"fr\")"
        ))
    }
}

/// Audio and the API key must not travel in clear: https only, except loopback.
pub fn check_stt_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid API URL: {e}"))?;
    let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match parsed.scheme() {
        "https" => Ok(()),
        "http" if loopback => Ok(()),
        _ => Err("API URL must use https (http only for localhost)".into()),
    }
}

fn key_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("credential store unavailable: {e}"))
}

/// Saved key, else AGENTDECK_STT_KEY, else OPENAI_API_KEY.
pub fn api_key() -> Option<String> {
    key_entry()
        .ok()
        .and_then(|entry| entry.get_password().ok())
        .or_else(|| std::env::var("AGENTDECK_STT_KEY").ok())
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .filter(|key| !key.trim().is_empty())
}

/// Stores the key in the OS credential store; `None` (or blank) deletes it.
pub fn set_api_key(key: Option<&str>) -> Result<(), String> {
    let entry = key_entry()?;
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(key) => entry
            .set_password(key)
            .map_err(|e| format!("cannot save API key: {e}")),
        None => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("cannot delete API key: {e}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn language_codes() {
        for ok in ["auto", "fr", "en", "yue"] {
            assert!(check_language(ok).is_ok(), "{ok}");
        }
        for bad in ["", "f", "FR", "french", "f1", "fr-FR"] {
            assert!(check_language(bad).is_err(), "{bad}");
        }
    }

    fn valid() -> VoiceConfig {
        VoiceConfig {
            shortcut: "ctrl+space".into(),
            push_to_talk: true,
            submit: false,
            engine: Engine::Local,
            model: WhisperModel::Small,
            language: "fr".into(),
            api_url: DEFAULT_STT_URL.into(),
            api_model: "whisper-1".into(),
        }
    }

    #[test]
    fn validate_config() {
        assert!(valid().validate().is_ok());
        assert!(
            VoiceConfig {
                shortcut: "ctrl+shift+KeyD".into(),
                ..valid()
            }
            .validate()
            .is_ok()
        );
        assert!(
            VoiceConfig {
                shortcut: "ctrl+nope".into(),
                ..valid()
            }
            .validate()
            .is_err()
        );
        assert!(
            VoiceConfig {
                shortcut: String::new(),
                ..valid()
            }
            .validate()
            .is_err()
        );
        assert!(
            VoiceConfig {
                language: "xx-YY".into(),
                ..valid()
            }
            .validate()
            .is_err()
        );
        assert!(
            VoiceConfig {
                api_url: "http://example.com".into(),
                ..valid()
            }
            .validate()
            .is_err()
        );
        assert!(
            VoiceConfig {
                api_model: " ".into(),
                ..valid()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn json_roundtrip_uses_contract_names() {
        let json = serde_json::to_value(valid()).unwrap();
        assert_eq!(json["pushToTalk"], true);
        assert_eq!(json["engine"], "local");
        assert_eq!(json["model"], "small");
        assert_eq!(json["apiUrl"], DEFAULT_STT_URL);
        let partial: VoiceConfig =
            serde_json::from_str(r#"{"model":"large-v3-turbo","engine":"api"}"#).unwrap();
        assert_eq!(partial.model, WhisperModel::LargeV3Turbo);
        assert_eq!(partial.engine, Engine::Api);
    }

    #[test]
    fn model_files_and_urls() {
        assert_eq!(WhisperModel::Small.file_name(), "ggml-small.bin");
        assert_eq!(
            WhisperModel::LargeV3Turbo.file_name(),
            "ggml-large-v3-turbo.bin"
        );
        assert_eq!(
            WhisperModel::Base.url(),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"
        );
        assert_eq!(WhisperModel::parse("medium"), Some(WhisperModel::Medium));
        assert_eq!(WhisperModel::parse("large"), None);
    }
}
