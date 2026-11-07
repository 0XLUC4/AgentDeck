//! Auto-update from this repo's release feed: GitHub `latest.json` pointing at the portable exe,
//! signed with the minisign key from tauri.conf.json. The running exe is swapped in place. Checked at startup and every few hours; shared by the
//! title bar button ([`UpdateButton`]) and Settings > About ([`super::AppControls`]).

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use agentdeck_core::logic::text::now_ms;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use crate::theme::{hsla, palette, ui};

const ENDPOINT: &str = "https://github.com/0XLUC4/AgentDeck/releases/latest/download/latest.json";
/// `plugins.updater.pubkey` of tauri.conf.json (base64 of the minisign.pub file).
const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEQxRUJBQjlCQTcwRjAxRjAKUldUd0FRK25tNnZyMGVQN0ppbDhaZCt4TG5SVGwvZHMyWUdRVmZOdnQ3bWZtUnBRMHFPdGNKMjgK";
const RECHECK: Duration = Duration::from_secs(6 * 60 * 60);
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub version: String,
    /// Release notes.
    pub body: Option<String>,
    url: String,
    /// base64 of the minisign `.sig` file.
    signature: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePhase {
    Idle,
    Checking,
    Downloading,
    Installing,
    Error,
}

pub struct Updater {
    pub update: Option<Update>,
    pub phase: UpdatePhase,
    /// 0..1 while downloading, None when the size is unknown.
    pub progress: Option<f32>,
    pub error: Option<String>,
    pub checked_at: Option<i64>,
}

struct UpdaterGlobal(Entity<Updater>);

impl Global for UpdaterGlobal {}

/// The shared updater state, once the main window is up.
pub fn updater(cx: &App) -> Option<Entity<Updater>> {
    cx.try_global::<UpdaterGlobal>().map(|g| g.0.clone())
}

/// Startup check, then every few hours. Network or endpoint errors stay quiet.
pub(super) fn init(cx: &mut App) {
    // Leftover of the previous update (the old exe cannot be deleted while it runs).
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
    }
    let entity = cx.new(|_| Updater {
        update: None,
        phase: UpdatePhase::Idle,
        progress: None,
        error: None,
        checked_at: None,
    });
    cx.set_global(UpdaterGlobal(entity.clone()));
    let weak = entity.downgrade();
    cx.spawn(async move |cx| {
        loop {
            if weak.update(cx, |u, cx| u.check(true, cx)).is_err() {
                break;
            }
            cx.background_executor().timer(RECHECK).await;
        }
    })
    .detach();
}

impl Updater {
    pub fn busy(&self) -> bool {
        matches!(self.phase, UpdatePhase::Downloading | UpdatePhase::Installing)
    }

    /// Fetches latest.json. `silent`: an error leaves the phase idle and shows nothing.
    pub fn check(&mut self, silent: bool, cx: &mut Context<Self>) {
        if self.phase == UpdatePhase::Checking || self.busy() {
            return;
        }
        self.phase = UpdatePhase::Checking;
        self.error = None;
        cx.notify();
        let job = cx.background_spawn(async move { fetch_latest() });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |u, cx| {
                // An install started meanwhile owns the phase now.
                if u.phase != UpdatePhase::Checking {
                    return;
                }
                match result {
                    Ok(update) => {
                        u.update = update;
                        u.phase = UpdatePhase::Idle;
                        u.checked_at = Some(now_ms());
                    }
                    Err(_) if silent => u.phase = UpdatePhase::Idle,
                    Err(e) => {
                        u.phase = UpdatePhase::Error;
                        u.error = Some(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Downloads with progress, verifies the signature, swaps the exe, restarts and quits.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        let Some(update) = self.update.clone() else {
            return;
        };
        if self.busy() {
            return;
        }
        self.phase = UpdatePhase::Downloading;
        self.progress = None;
        self.error = None;
        cx.notify();
        let (tx, rx) = smol::channel::unbounded::<Option<f32>>();
        let job = cx.background_spawn(async move {
            download(&update, |p| {
                let _ = tx.send_blocking(p);
            })
        });
        cx.spawn(async move |this, cx| {
            while let Ok(progress) = rx.recv().await {
                let _ = this.update(cx, |u, cx| {
                    u.progress = progress;
                    cx.notify();
                });
            }
            let result = job.await;
            let _ = this.update(cx, |u, cx| {
                match result.and_then(|path| {
                    u.phase = UpdatePhase::Installing;
                    u.progress = Some(1.0);
                    replace_and_restart(&path)
                }) {
                    Ok(()) => super::quit(cx),
                    Err(e) => {
                        u.phase = UpdatePhase::Error;
                        u.error = Some(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(format!("AgentDeck/{VERSION}"))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())
}

fn fetch_latest() -> Result<Option<Update>, String> {
    let text = client()?
        .get(ENDPOINT)
        .timeout(Duration::from_secs(30))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::text)
        .map_err(|e| format!("update check failed: {e}"))?;
    let manifest: Value = serde_json::from_str(&text).map_err(|e| format!("bad update manifest: {e}"))?;
    parse_manifest(&manifest, VERSION, &platform_keys())
}

/// latest.json platform keys, most specific first (Tauri v2: `{os}-{arch}-{installer}`, then `{os}-{arch}`).
fn platform_keys() -> Vec<String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let base = format!("{os}-{}", std::env::consts::ARCH);
    vec![format!("{base}-nsis"), format!("{base}-msi"), base]
}

/// The update described by `manifest`, or None when it is not newer than `current`.
fn parse_manifest(manifest: &Value, current: &str, keys: &[String]) -> Result<Option<Update>, String> {
    let version = manifest["version"]
        .as_str()
        .ok_or("update manifest has no version")?
        .trim_start_matches('v')
        .to_owned();
    if !is_newer(&version, current) {
        return Ok(None);
    }
    let platform = keys
        .iter()
        .find_map(|k| manifest["platforms"].get(k))
        .ok_or("no update for this platform")?;
    let field = |name: &str| {
        platform[name]
            .as_str()
            .map(str::to_owned)
            .ok_or(format!("update manifest has no {name}"))
    };
    Ok(Some(Update {
        body: manifest["notes"].as_str().filter(|s| !s.is_empty()).map(str::to_owned),
        url: field("url")?,
        signature: field("signature")?,
        version,
    }))
}

/// Semver core comparison ("0.1.20" > "0.1.9"); pre-release and build tags are ignored.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|n| n.parse().unwrap_or(0))
            .collect()
    };
    let (mut a, mut b) = (parts(candidate), parts(current));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}

fn download(update: &Update, progress: impl Fn(Option<f32>)) -> Result<PathBuf, String> {
    let mut response = client()?
        .get(&update.url)
        .timeout(Duration::from_secs(30 * 60)) // the client default (30 s) also covers the body: slow links failed
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| format!("download failed: {e}"))?;
    let total = response.content_length().filter(|t| *t > 0);
    let mut bytes = Vec::with_capacity(total.unwrap_or(0) as usize);
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = response.read(&mut chunk).map_err(|e| format!("download failed: {e}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        progress(total.map(|t| (bytes.len() as f32 / t as f32).min(1.0)));
    }
    verify(&bytes, &update.signature)?;
    let name = update
        .url
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty() && !n.contains(['\\', ':']))
        .unwrap_or("AgentDeck-setup.exe");
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

fn verify(bytes: &[u8], signature_b64: &str) -> Result<(), String> {
    let text = |b64: &str| {
        base64_decode(b64)
            .and_then(|v| String::from_utf8(v).ok())
            .ok_or("bad base64 in the update signature or key")
    };
    let key = minisign_verify::PublicKey::decode(&text(PUBKEY)?).map_err(|e| format!("update key: {e}"))?;
    let signature = minisign_verify::Signature::decode(&text(signature_b64)?)
        .map_err(|e| format!("update signature: {e}"))?;
    key.verify(bytes, &signature, true)
        .map_err(|e| format!("the update signature does not match: {e}"))
}

/// Standard base64 (padding optional, whitespace ignored).
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in input.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = (acc << 6) | u32::from(value(c)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Replaces the running exe with `new` (Windows lets a running exe be renamed, not overwritten),
/// then starts it. On failure the old exe is put back.
fn replace_and_restart(new: &std::path::Path) -> Result<(), String> {
    let current = std::env::current_exe().map_err(|e| format!("cannot find AgentDeck's exe: {e}"))?;
    let old = current.with_extension("exe.old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&current, &old).map_err(|e| format!("{}: {e}", current.display()))?;
    if let Err(e) = std::fs::copy(new, &current) {
        let _ = std::fs::rename(&old, &current);
        return Err(format!("cannot install the update: {e}"));
    }
    let _ = std::fs::remove_file(new);
    std::process::Command::new(&current)
        .arg(super::instance::AFTER_UPDATE)
        .spawn()
        .map(drop)
        .map_err(|e| format!("cannot restart AgentDeck: {e}"))
}

/// Title bar button: a dot when a new release is out, a small panel to install it and restart.
pub struct UpdateButton {
    open: bool,
}

impl UpdateButton {
    pub fn new(cx: &mut Context<Self>) -> Self {
        if let Some(u) = updater(cx) {
            cx.observe(&u, |_, _, cx| cx.notify()).detach();
        }
        Self { open: false }
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        if self.open
            && let Some(u) = updater(cx)
        {
            u.update(cx, |u, cx| {
                if u.update.is_none() {
                    u.check(false, cx);
                }
            });
        }
        cx.notify();
    }
}

impl Render for UpdateButton {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let Some(entity) = updater(cx) else {
            return div().id("update-button");
        };
        let u = entity.read(cx);
        let (update, phase, progress, error, busy) =
            (u.update.clone(), u.phase, u.progress, u.error.clone(), u.busy());
        let hint = match &update {
            Some(up) => format!("AgentDeck {} is available", up.version),
            None => "Check for updates".to_owned(),
        };
        let button = div()
            .id("update-toggle")
            .size(ui(28.0))
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .rounded(ui(6.0))
            .text_size(ui(16.0)) // same icon size as the other title bar buttons (icon_button)
            .text_color(hsla(if self.open { p.fg } else { p.muted }))
            .when(self.open, |d| d.bg(hsla(p.raised)))
            .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
            .child(IconName::CircleArrowUp)
            .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(hint.clone()).build(window, cx))
            .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)));

        let mut root = div().id("update-button").relative().flex().child(button);
        if update.is_some() {
            root = root.child(
                div()
                    .absolute()
                    .top(ui(4.0))
                    .right(ui(4.0))
                    .size(ui(6.0))
                    .rounded_full()
                    .bg(hsla(p.busy)),
            );
        }
        if !self.open {
            return root;
        }

        let small = |text: String, color: u32| div().mt(ui(2.0)).text_xs().text_color(hsla(color)).child(text);
        let mut panel = div()
            .id("update-panel")
            .occlude()
            .mt(ui(34.0))
            .w(ui(288.0))
            .p_3()
            .rounded(ui(8.0))
            .border_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.panel))
            .shadow_lg()
            .text_size(ui(13.0))
            .text_color(hsla(p.fg))
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                if !busy {
                    this.open = false;
                    cx.notify();
                }
            }));
        match &update {
            Some(up) => {
                panel = panel
                    .child(div().font_weight(FontWeight::MEDIUM).child(format!("AgentDeck {} is available", up.version)))
                    .child(small(format!("You have {VERSION}."), p.subtle));
                if let Some(body) = &up.body {
                    panel = panel.child(
                        div()
                            .id("update-notes")
                            .mt_2()
                            .max_h(ui(128.0))
                            .overflow_y_scroll()
                            .text_xs()
                            .text_color(hsla(p.muted))
                            .child(body.clone()),
                    );
                }
                panel = if busy {
                    let bar = div().h_full().bg(hsla(p.busy));
                    let bar = match progress {
                        Some(f) => bar.w(relative(f)),
                        None => bar.w(relative(1.0 / 3.0)),
                    };
                    panel.child(
                        div()
                            .mt_3()
                            .child(div().h(ui(4.0)).overflow_hidden().rounded_full().bg(hsla(p.raised)).child(bar))
                            .child(small(
                                if phase == UpdatePhase::Installing {
                                    "Installing, AgentDeck will restart…".into()
                                } else {
                                    "Downloading…".into()
                                },
                                p.subtle,
                            )),
                    )
                } else {
                    let install = entity.clone();
                    panel.child(
                        div()
                            .id("update-install")
                            .mt_3()
                            .h(ui(32.0))
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(ui(6.0))
                            .bg(hsla(p.busy))
                            .text_color(hsla(p.on_accent))
                            .font_weight(FontWeight::MEDIUM)
                            .hover(|s| s.opacity(0.9))
                            .child("Update and restart")
                            .on_click(move |_, _, cx| install.update(cx, |u, cx| u.install(cx))),
                    )
                };
                panel = panel.child(small("Running agents are stopped when the app restarts.".into(), p.subtle));
            }
            None => {
                let checking = phase == UpdatePhase::Checking;
                let again = entity.clone();
                panel = panel.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .child(div().font_weight(FontWeight::MEDIUM).child(if checking {
                                    "Checking for updates…"
                                } else {
                                    "You're up to date"
                                }))
                                .child(small(format!("AgentDeck {VERSION}"), p.subtle)),
                        )
                        .child(
                            div()
                                .id("update-again")
                                .size(ui(28.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(ui(6.0))
                                .text_color(hsla(p.muted))
                                .when(checking, |d| d.opacity(0.5))
                                .when(!checking, |d| d.hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg))))
                                .child(IconName::RefreshCw)
                                .on_click(move |_, _, cx| again.update(cx, |u, cx| u.check(false, cx))),
                        ),
                );
            }
        }
        if let Some(e) = error {
            panel = panel.child(div().mt_2().text_xs().text_color(hsla(p.bad)).child(e));
        }
        root.child(deferred(anchored().anchor(Anchor::TopRight).snap_to_window_with_margin(px(8.0)).child(panel)))
    }
}

#[cfg(test)]
mod tests {
    use super::{PUBKEY, Value, base64_decode, is_newer, parse_manifest};

    #[test]
    fn newer_versions() {
        assert!(is_newer("0.1.20", "0.1.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.20", "0.1.20"));
        assert!(!is_newer("0.1.20-beta", "0.2.0"));
        assert!(is_newer("0.2.1", "0.2"));
    }

    #[test]
    fn decodes_the_tauri_pubkey() {
        let text = String::from_utf8(base64_decode(PUBKEY).unwrap()).unwrap();
        assert!(text.starts_with("untrusted comment: minisign public key: D1EBAB9BA70F01F0\n"));
        assert!(minisign_verify::PublicKey::decode(&text).is_ok());
    }

    #[test]
    fn manifest_picks_platform() {
        let m: Value = serde_json::from_str(
            r#"{ "version": "0.3.0", "notes": "Fixes", "platforms": {
                "windows-x86_64": { "url": "https://x/a.msi", "signature": "s1" },
                "windows-x86_64-nsis": { "url": "https://x/a.exe", "signature": "s2" } } }"#,
        )
        .unwrap();
        let keys = vec!["windows-x86_64-nsis".to_owned(), "windows-x86_64".to_owned()];
        let up = parse_manifest(&m, "0.2.0", &keys).unwrap().unwrap();
        assert_eq!((up.url.as_str(), up.body.as_deref()), ("https://x/a.exe", Some("Fixes")));
        assert_eq!(parse_manifest(&m, "0.3.0", &keys).unwrap(), None);
        assert!(parse_manifest(&m, "0.1.0", &["linux-x86_64".to_owned()]).is_err());
    }
}
