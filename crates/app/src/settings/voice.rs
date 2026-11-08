//! Settings > Voice: dictation shortcut, mode, language, engine, Whisper models, speech-to-text API.
//! Voice settings live in the backend: every change goes through `voice::voice_set_config`.

use agentdeck_core::AppHandle;
use agentdeck_core::logic::text::{KeyCombo, format_shortcut, to_shortcut};
use agentdeck_core::logic::theme::{FONT_MONO, Palette};
use agentdeck_core::voice::{
    Engine, VoiceConfig, WhisperModel, voice_download_model, voice_get_config, voice_list_models,
    voice_set_api_key, voice_set_config,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use super::SettingsDialog;
use super::controls::{Opt, button, error_text, field, hint_text, icon, row, section, toggle};
use crate::theme::{hsla, palette, ui};

const LANGUAGES: [(&str, &str); 6] = [
    ("fr", "Fran\u{e7}ais"),
    ("en", "English"),
    ("es", "Espa\u{f1}ol"),
    ("de", "Deutsch"),
    ("it", "Italiano"),
    ("auto", "Auto-detect"),
];

const MODES: [(&str, &str); 2] = [("hold", "Hold to talk"), ("toggle", "Press to start/stop")];

const ENGINES: [(&str, &str); 2] = [("local", "Local (offline Whisper)"), ("api", "API")];

/// (label, URL, model) of the speech-to-text API presets.
const PRESETS: [(&str, &str, &str); 2] = [
    ("OpenAI", "https://api.openai.com/v1/audio/transcriptions", "whisper-1"),
    ("Groq", "https://api.groq.com/openai/v1/audio/transcriptions", "whisper-large-v3-turbo"),
];

/// Download sizes of the ggml models, shown before download.
fn model_size(model: WhisperModel) -> &'static str {
    match model {
        WhisperModel::Base => "82 MB",
        WhisperModel::Small => "466 MB",
        WhisperModel::Medium => "823 MB",
        WhisperModel::LargeV3Turbo => "874 MB",
    }
}

type Choice = Entity<SelectState<Vec<Opt>>>;

struct Download {
    model: WhisperModel,
    received: u64,
    total: Option<u64>,
}

pub(super) struct VoiceUi {
    backend: AppHandle,
    config: Option<VoiceConfig>,
    has_key: bool,
    error: Option<String>,
    /// (model, downloaded); None while unavailable.
    models: Option<Vec<(WhisperModel, bool)>>,
    download: Option<Download>,
    model_error: Option<String>,
    pub recording: bool,
    shortcut_focus: FocusHandle,
    mode: Choice,
    language: Choice,
    engine: Choice,
    api_url: Entity<InputState>,
    api_model: Entity<InputState>,
    api_key: Entity<InputState>,
}

fn engine_value(engine: Engine) -> &'static str {
    match engine {
        Engine::Local => "local",
        Engine::Api => "api",
    }
}

fn choice(
    window: &mut Window,
    cx: &mut Context<SettingsDialog>,
    subs: &mut Vec<Subscription>,
    options: &[(&str, &str)],
    current: &str,
    apply: fn(&mut VoiceConfig, &str),
) -> Choice {
    let items: Vec<Opt> = options.iter().map(|(v, l)| Opt::new(v.to_string(), l.to_string())).collect();
    let current = SharedString::from(current.to_owned());
    let state = cx.new(|cx| {
        let mut state = SelectState::new(items, None, window, cx);
        state.set_selected_value(&current, window, cx);
        state
    });
    subs.push(cx.subscribe_in(&state, window, move |this, _, event: &SelectEvent<Vec<Opt>>, window, cx| {
        if let SelectEvent::Confirm(Some(value)) = event {
            let value = value.to_string();
            this.update_voice(window, cx, |c| apply(c, &value));
        }
    }));
    state
}

/// The models and whether each is downloaded (`ModelStatus` is only serializable).
fn list_models(backend: &AppHandle) -> Result<Vec<(WhisperModel, bool)>, String> {
    let list = serde_json::to_value(voice_list_models(backend)?).map_err(|e| e.to_string())?;
    Ok(list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let model = serde_json::from_value::<WhisperModel>(m["model"].clone()).ok()?;
            Some((model, m["downloaded"].as_bool().unwrap_or(false)))
        })
        .collect())
}

/// Global-shortcut code ("KeyA", "Digit1", "F9", "Space") of a GPUI key name.
fn key_code(key: &str) -> String {
    match key {
        "up" | "down" | "left" | "right" => format!("Arrow{}{}", key[..1].to_uppercase(), &key[1..]),
        "control" | "ctrl" => "Control".into(),
        "platform" | "cmd" | "super" | "win" => "Meta".into(),
        _ if key.len() == 1 && key.chars().all(|c| c.is_ascii_alphabetic()) => format!("Key{}", key.to_uppercase()),
        _ if key.len() == 1 && key.chars().all(|c| c.is_ascii_digit()) => format!("Digit{key}"),
        _ => {
            let mut chars = key.chars();
            chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        }
    }
}

impl VoiceUi {
    pub fn new(backend: AppHandle, window: &mut Window, cx: &mut Context<SettingsDialog>, subs: &mut Vec<Subscription>) -> Self {
        let (config, has_key, error) = match voice_get_config(&backend) {
            Ok(view) => (Some(view.config), view.has_api_key, None),
            Err(e) => (None, false, Some(format!("Voice settings unavailable: {e}"))),
        };
        let c = config.clone().unwrap_or_default();
        let (models, model_error) = match list_models(&backend) {
            Ok(list) => (Some(list), None),
            Err(e) => (None, Some(format!("Model list unavailable: {e}"))),
        };
        let shortcut_focus = cx.focus_handle();
        subs.push(cx.on_blur(&shortcut_focus, window, |this, _, cx| {
            this.voice.recording = false;
            cx.notify();
        }));
        let mode = choice(window, cx, subs, &MODES, if c.push_to_talk { "hold" } else { "toggle" }, |c, v| {
            c.push_to_talk = v == "hold"
        });
        let language = choice(window, cx, subs, &LANGUAGES, &c.language, |c, v| c.language = v.to_owned());
        let engine = choice(window, cx, subs, &ENGINES, engine_value(c.engine), |c, v| {
            c.engine = if v == "api" { Engine::Api } else { Engine::Local }
        });

        // URL and model commit on blur / Enter: the backend validates the URL, half-typed values must not be sent.
        let api_url = super::input(window, cx, "", &c.api_url);
        let api_model = super::input(window, cx, "", &c.api_model);
        for state in [&api_url, &api_model] {
            subs.push(cx.subscribe_in(state, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.commit_api(window, cx);
                }
            }));
        }
        let key_placeholder = if has_key { "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}" } else { "sk-\u{2026}" };
        let api_key = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(key_placeholder));
        subs.push(cx.subscribe_in(&api_key, window, |this, state, event: &InputEvent, window, cx| match event {
            InputEvent::Change => cx.notify(), // enables "Save"
            InputEvent::PressEnter { .. } => {
                let key = state.read(cx).value().trim().to_owned();
                if !key.is_empty() {
                    this.store_key(Some(key), window, cx);
                }
            }
            _ => {}
        }));
        Self {
            backend,
            config,
            has_key,
            error,
            models,
            download: None,
            model_error,
            recording: false,
            shortcut_focus,
            mode,
            language,
            engine,
            api_url,
            api_model,
            api_key,
        }
    }

    /// `voice-download` payload `{ model, received, total }`.
    pub fn on_download_progress(&mut self, payload: &Value) {
        let Some(download) = &mut self.download else {
            return;
        };
        if serde_json::from_value::<WhisperModel>(payload["model"].clone()).ok() != Some(download.model) {
            return;
        }
        download.received = payload["received"].as_u64().unwrap_or(0);
        download.total = payload["total"].as_u64();
    }
}

impl SettingsDialog {
    /// Optimistic: applied at once; on failure the backend's real config is reloaded and the error shown.
    fn update_voice(&mut self, window: &mut Window, cx: &mut Context<Self>, edit: impl FnOnce(&mut VoiceConfig)) {
        let Some(current) = self.voice.config.clone() else {
            return;
        };
        let mut next = current.clone();
        edit(&mut next);
        if next == current {
            return;
        }
        // The global shortcut is swapped first; if it cannot be registered the config is not saved.
        let shortcut_changed = next.shortcut != current.shortcut;
        if shortcut_changed
            && let Err(e) = crate::system::set_voice_shortcut(&next.shortcut, cx)
        {
            self.report(Err(e), cx);
            return;
        }
        match voice_set_config(&self.voice.backend, next) {
            Ok(view) => {
                self.voice.config = Some(view.config);
                self.voice.has_key = view.has_api_key;
                self.report(Ok(()), cx);
            }
            Err(e) => {
                if shortcut_changed
                    && let Err(e) = crate::system::set_voice_shortcut(&current.shortcut, cx)
                {
                    eprintln!("voice: cannot restore shortcut: {e}");
                }
                self.reload_voice(window, cx);
                self.report(Err(e), cx);
            }
        }
    }

    fn reload_voice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match voice_get_config(&self.voice.backend) {
            Ok(view) => {
                let c = view.config;
                let values = [
                    (&self.voice.mode, if c.push_to_talk { "hold" } else { "toggle" }),
                    (&self.voice.language, c.language.as_str()),
                    (&self.voice.engine, engine_value(c.engine)),
                ];
                for (select, value) in values {
                    let value = SharedString::from(value.to_owned());
                    select.update(cx, |s, cx| s.set_selected_value(&value, window, cx));
                }
                self.voice.api_url.update(cx, |s, cx| s.set_value(c.api_url.clone(), window, cx));
                self.voice.api_model.update(cx, |s, cx| s.set_value(c.api_model.clone(), window, cx));
                self.voice.has_key = view.has_api_key;
                self.voice.config = Some(c);
            }
            Err(e) => self.voice.error = Some(format!("Voice settings unavailable: {e}")),
        }
        cx.notify();
    }

    fn commit_api(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.voice.api_url.read(cx).value().to_string();
        let model = self.voice.api_model.read(cx).value().to_string();
        self.update_voice(window, cx, |c| {
            c.api_url = url;
            c.api_model = model;
        });
    }

    /// Saves (Some) or deletes (None) the API key in the OS credential store.
    fn store_key(&mut self, key: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.run(window, cx, move || voice_set_api_key(key), |this, result, window, cx| {
            if result.is_ok() {
                this.voice.api_key.update(cx, |s, cx| s.set_value("", window, cx));
                this.reload_voice(window, cx);
            }
            this.report(result, cx);
        });
    }

    fn download_model(&mut self, model: WhisperModel, window: &mut Window, cx: &mut Context<Self>) {
        self.voice.model_error = None;
        self.voice.download = Some(Download {
            model,
            received: 0,
            total: None,
        });
        cx.notify();
        let backend = self.voice.backend.clone();
        self.run(window, cx, move || voice_download_model(&backend, model), |this, result, _, cx| {
            if let Err(e) = result {
                this.voice.model_error = Some(format!("Download failed: {e}"));
            }
            this.voice.download = None;
            match list_models(&this.voice.backend) {
                Ok(list) => this.voice.models = Some(list),
                Err(e) => this.voice.model_error = Some(format!("Model list unavailable: {e}")),
            }
            cx.notify();
        });
    }

    /// Called by the dialog's keystroke interceptor while recording, before any key binding runs.
    pub(super) fn on_shortcut_key(&mut self, k: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation(); // also keeps Esc from closing the dialog
        let m = k.modifiers;
        if k.key == "escape" && !m.control && !m.alt && !m.shift && !m.platform {
            self.voice.recording = false;
            cx.notify();
            return;
        }
        let combo = KeyCombo {
            ctrl: m.control,
            shift: m.shift,
            alt: m.alt,
            meta: m.platform,
            code: key_code(&k.key),
        };
        let Some(shortcut) = to_shortcut(&combo) else {
            return;
        };
        self.voice.recording = false;
        self.update_voice(window, cx, |c| c.shortcut = shortcut);
    }

    fn shortcut_recorder(&self, shortcut: &str, p: &Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        let recording = self.voice.recording;
        let label = if recording { "Press a key combination\u{2026} (Esc)".to_owned() } else { format_shortcut(shortcut) };
        div()
            .id("set-voice-shortcut")
            .track_focus(&self.voice.shortcut_focus)
            .flex()
            .items_center()
            .justify_center()
            .w(ui(224.0))
            .h_8()
            .rounded_md()
            .border_1()
            .border_color(hsla(if recording { p.accent } else { p.line }))
            .bg(hsla(p.base))
            .cursor_pointer()
            .font_family(FONT_MONO)
            .text_xs()
            .text_color(hsla(if recording { p.muted } else { p.fg }))
            .on_click(cx.listener(|this, _, window, cx| {
                this.voice.recording = true;
                window.focus(&this.voice.shortcut_focus, cx);
                cx.notify();
            }))
            .child(label)
    }

    fn model_list(&self, config: &VoiceConfig, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(models) = &self.voice.models else {
            return match &self.voice.model_error {
                Some(e) => error_text(e, p).into_any_element(),
                None => hint_text("Loading\u{2026}", p).py_3().into_any_element(),
            };
        };
        let busy = self.voice.download.is_some();
        div()
            .flex()
            .flex_col()
            .gap(ui(6.0))
            .py_3()
            .child(div().mb_2().text_color(hsla(p.fg)).child("Whisper model"))
            .children(models.iter().map(|&(model, downloaded)| {
                let selected = config.model == model;
                let progress = self.voice.download.as_ref().filter(|d| d.model == model);
                let status: AnyElement = if downloaded {
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .text_color(hsla(p.ok))
                        .child(icon(IconName::Check, 12.0, p.ok))
                        .child("Downloaded")
                        .into_any_element()
                } else if let Some(d) = progress {
                    let pct = d.total.filter(|t| *t > 0).map(|t| (d.received as f64 / t as f64 * 100.0).round() as u32);
                    div()
                        .flex()
                        .w(ui(160.0))
                        .items_center()
                        .gap_2()
                        .child(
                            div().flex_1().h(ui(6.0)).rounded_full().bg(hsla(p.line)).overflow_hidden().child(
                                div()
                                    .h_full()
                                    .bg(hsla(p.accent))
                                    .w(relative(pct.unwrap_or(0) as f32 / 100.0)),
                            ),
                        )
                        .child(
                            div()
                                .w(ui(36.0))
                                .text_right()
                                .text_xs()
                                .text_color(hsla(p.muted))
                                .child(pct.map_or("\u{2026}".to_owned(), |v| format!("{v}%"))),
                        )
                        .into_any_element()
                } else {
                    let b = button(("model-download", model as usize), Some(IconName::Download), "Download", busy, p);
                    if busy {
                        b.into_any_element()
                    } else {
                        b.on_click(cx.listener(move |this, _, window, cx| this.download_model(model, window, cx)))
                            .into_any_element()
                    }
                };
                let radio = div()
                    .id(("model", model as usize))
                    .flex()
                    .flex_1()
                    .items_center()
                    .gap_3()
                    .when(!downloaded, |d| d.opacity(0.7))
                    .when(downloaded, |d| {
                        d.cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.update_voice(window, cx, |c| c.model = model)
                            }))
                    })
                    .child(
                        div()
                            .size_4()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .border_1()
                            .border_color(hsla(if selected { p.accent } else { p.line_strong }))
                            .when(selected, |d| d.child(div().size(ui(8.0)).rounded_full().bg(hsla(p.accent)))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .font_family(FONT_MONO)
                            .text_xs()
                            .text_color(hsla(p.fg))
                            .child(model.name())
                            .child(div().text_color(hsla(p.subtle)).child(format!("({})", model_size(model)))),
                    );
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(hsla(if selected { p.line_strong } else { p.line }))
                    .when(selected, |d| d.bg(hsla(p.raised)))
                    .child(radio)
                    .child(status)
            }))
            .children(self.voice.model_error.as_deref().map(|e| error_text(e, p)))
            .into_any_element()
    }

    fn api_rows(&self, p: &Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let v = &self.voice;
        let presets = div().flex().gap_2().children(PRESETS.iter().map(|&(label, url, model)| {
            button(label, None, label, false, p).on_click(cx.listener(move |this, _, window, cx| {
                this.voice.api_url.update(cx, |s, cx| s.set_value(url, window, cx));
                this.voice.api_model.update(cx, |s, cx| s.set_value(model, window, cx));
                this.update_voice(window, cx, |c| {
                    c.api_url = url.to_owned();
                    c.api_model = model.to_owned();
                });
            }))
        }));
        let key_empty = v.api_key.read(cx).value().trim().is_empty();
        let mut save = button("set-api-key-save", Some(IconName::KeyRound), "Save", key_empty, p);
        if !key_empty {
            save = save.on_click(cx.listener(|this, _, window, cx| {
                let key = this.voice.api_key.read(cx).value().trim().to_owned();
                this.store_key(Some(key), window, cx);
            }));
        }
        let mut delete = button("set-api-key-delete", Some(IconName::Trash), "Delete", !v.has_key, p);
        if v.has_key {
            delete = delete.on_click(cx.listener(|this, _, window, cx| this.store_key(None, window, cx)));
        }
        vec![
            row("Presets", None, presets, p),
            row("URL", None, field(&v.api_url, Some(320.0), true), p),
            row("Model", None, field(&v.api_model, Some(320.0), true), p),
            row(
                "API key",
                Some(if v.has_key { "Key saved" } else { "No key saved" }),
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(field(&v.api_key, Some(192.0), true))
                    .child(save)
                    .child(delete),
                p,
            ),
        ]
    }

    pub(super) fn render_voice(&mut self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let Some(config) = self.voice.config.clone() else {
            return match &self.voice.error {
                Some(e) => div().child(error_text(e, &p)),
                None => hint_text("Loading\u{2026}", &p),
            };
        };
        let v = &self.voice;
        let mut rows = vec![
            row(
                "Shortcut",
                Some("Global, works even outside the window"),
                self.shortcut_recorder(&config.shortcut, &p, cx),
                &p,
            ),
            row("Mode", None, div().w(ui(224.0)).child(Select::new(&v.mode)), &p),
            row(
                "Send automatically",
                Some("Presses Enter after transcription"),
                toggle("set-voice-submit", config.submit, &p).on_click(cx.listener(|this, _, window, cx| {
                    this.update_voice(window, cx, |c| c.submit = !c.submit)
                })),
                &p,
            ),
            row("Language", None, div().w(ui(224.0)).child(Select::new(&v.language)), &p),
            row("Engine", None, div().w(ui(224.0)).child(Select::new(&v.engine)), &p),
        ];
        if config.engine == Engine::Local {
            rows.push(self.model_list(&config, &p, cx));
        } else {
            rows.extend(self.api_rows(&p, cx));
        }
        if let Some(e) = &self.voice.error {
            rows.push(error_text(e, &p).into_any_element());
        }
        section("Voice", rows, &p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn key_codes_feed_to_shortcut() {
        let combo = |code: &str| KeyCombo {
            ctrl: true,
            shift: true,
            code: key_code(code),
            ..KeyCombo::default()
        };
        assert_eq!(to_shortcut(&combo("a")).as_deref(), Some("ctrl+shift+a"));
        assert_eq!(to_shortcut(&combo("1")).as_deref(), Some("ctrl+shift+1"));
        assert_eq!(to_shortcut(&combo("space")).as_deref(), Some("ctrl+shift+space"));
        assert_eq!(to_shortcut(&combo("f9")).as_deref(), Some("ctrl+shift+f9"));
        assert_eq!(to_shortcut(&combo("up")).as_deref(), Some("ctrl+shift+arrowup"));
        assert_eq!(to_shortcut(&combo("shift")), None);
        assert_eq!(to_shortcut(&combo("control")), None);
    }
}
