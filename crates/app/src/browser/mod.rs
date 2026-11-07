//! Browser panel for a Claude pane: a native webview (gpui-webview, Wry/WebView2) laid over the panel body,
//! driven by the agent through chrome-devtools-mcp on a per-pane DevTools port. URL bar, back/forward,
//! reload, close. The only web view in the app.
//! Original: src/components/Browser/BrowserPanel.tsx, src-tauri/src/browser.rs
//! (browser_open -> Claude args, browser_bounds, browser_go, browser_url, browser_close).
//!
//! shortcut: the debugging port listens on 127.0.0.1 without auth, so a local process could drive this
//! webview (not the app UI). Fine for a dev tool on a personal machine.

use std::cell::Cell;
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use gpui_webview::WebView;
use serde_json::{Value, json};

use crate::state::Deck;
use crate::theme::{hsla, palette, ui};

/// Same start page as the Tauri build.
pub const START_URL: &str = "https://www.google.com";
/// wry's defaults, lost when browser args are set.
const WRY_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";
/// How often a shown panel checks it is still on screen (tab switches, layout changes).
const FOLLOW_MS: u64 = 150;
/// How often the address bar reads the page URL.
const URL_POLL_MS: u64 = 1000;

pub fn init(cx: &mut App) {
    cx.set_global(Browsers::default());
}

/// One pane's webview. The WebContext is kept alive with it (it owns the profile directory).
struct Browser {
    view: Entity<WebView>,
    port: u16,
    _context: wry::WebContext,
}

/// pane id -> its browser webview (children of the main window).
#[derive(Default)]
struct Browsers(HashMap<String, Browser>);

impl Global for Browsers {}

fn browser_view(pane_id: &str, cx: &App) -> Result<Entity<WebView>, String> {
    cx.try_global::<Browsers>()
        .and_then(|b| b.0.get(pane_id))
        .map(|b| b.view.clone())
        .ok_or_else(|| "no browser for this agent".into())
}

fn valid_id(pane_id: &str) -> Result<(), String> {
    let ok = (1..=64).contains(&pane_id.len())
        && pane_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    ok.then_some(()).ok_or_else(|| "invalid pane id".into())
}

/// Creates (or reuses) the pane's browser webview in `window`, hidden until its panel is drawn, writes the
/// MCP config and returns the Claude args that connect the agent to it (`--mcp-config <file>`).
pub fn open(pane_id: &str, data_dir: &Path, window: &mut Window, cx: &mut App) -> Result<Vec<String>, String> {
    valid_id(pane_id)?;
    let dir = data_dir.join("browser");
    let known = cx
        .try_global::<Browsers>()
        .and_then(|b| b.0.get(pane_id))
        .map(|b| b.port);
    let port = match known {
        Some(port) => port,
        None => {
            let port = free_port()?;
            // Different browser args need their own WebView2 environment, hence data directory.
            let mut context = wry::WebContext::new(Some(dir.join("profiles").join(pane_id)));
            let builder = wry::WebViewBuilder::new_with_web_context(&mut context).with_url(START_URL);
            #[cfg(windows)]
            let builder = {
                use wry::WebViewBuilderExtWindows as _;
                builder.with_additional_browser_args(format!("{WRY_ARGS} --remote-debugging-port={port}"))
            };
            let webview =
                WebView::build(builder, window, cx).map_err(|e| format!("cannot open the browser: {e}"))?;
            // Hidden natively only: the panel's prepaint shows it again at its bounds.
            let _ = webview.raw().set_visible(false);
            let view = cx.new(|_| webview);
            cx.default_global::<Browsers>().0.insert(
                pane_id.to_owned(),
                Browser { view, port, _context: context },
            );
            port
        }
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let config = dir.join(format!("{pane_id}.json"));
    let text = serde_json::to_string_pretty(&config_for(port)).map_err(|e| e.to_string())?;
    std::fs::write(&config, text).map_err(|e| format!("{}: {e}", config.display()))?;
    Ok(vec!["--mcp-config".into(), config.to_string_lossy().into_owned()])
}

/// Header controls: "back", "forward", "reload", or an http(s) URL to open.
pub fn go(pane_id: &str, to: &str, cx: &mut App) -> Result<(), String> {
    let view = browser_view(pane_id, cx)?;
    let raw = view.read(cx).raw();
    let result = match to {
        "back" => raw.evaluate_script("history.back()"),
        "forward" => raw.evaluate_script("history.forward()"),
        "reload" => raw.reload(),
        _ => raw.load_url(parse_url(to)?.as_str()),
    };
    result.map_err(|e| e.to_string())
}

/// Current URL of the pane's browser, for the header.
pub fn url(pane_id: &str, cx: &App) -> Result<String, String> {
    browser_view(pane_id, cx)?.read(cx).raw().url().map_err(|e| e.to_string())
}

/// Closes the pane's browser for good (pane closed or browser turned off).
pub fn close(pane_id: &str, cx: &mut App) {
    // Dropping the entity hides the native view; it is destroyed with its last handle.
    if cx.has_global::<Browsers>() {
        cx.global_mut::<Browsers>().0.remove(pane_id);
    }
}

/// "example.com" or "https://example.com/x"; only http(s).
fn parse_url(text: &str) -> Result<url::Url, String> {
    let text = text.trim();
    let full = if text.contains("://") {
        text.to_owned()
    } else {
        format!("https://{text}")
    };
    let parsed = url::Url::parse(&full).map_err(|_| format!("not a URL: {text}"))?;
    if matches!(parsed.scheme(), "http" | "https") {
        Ok(parsed)
    } else {
        Err("only http and https URLs".into())
    }
}

fn free_port() -> Result<u16, String> {
    // shortcut: the port is released before WebView2 binds it; a race is unlikely on 127.0.0.1.
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("no free port: {e}"))
}

fn config_for(port: u16) -> Value {
    let mut args = vec![
        "-y".to_string(),
        "chrome-devtools-mcp@latest".into(),
        "--browserUrl".into(),
        format!("http://127.0.0.1:{port}"),
    ];
    // npx is a .cmd script on Windows: Claude Code can only start it through cmd.
    let command = if cfg!(windows) {
        args.splice(0..0, ["/c".to_string(), "npx".into()]);
        "cmd"
    } else {
        "npx"
    };
    json!({ "mcpServers": { "browser": { "command": command, "args": args } } })
}

/// Emitted when the user closed the browser for good: the pane is saved with `browser: false` and its
/// webview is gone; the owner restarts the agent without its browser tools.
#[derive(Debug, Clone)]
pub struct BrowserRemoved {
    pub pane_id: String,
}

/// Dock panel of a pane's browser: address bar header, and a body the native webview is laid over.
pub struct BrowserPanel {
    pub deck: Entity<Deck>,
    pub pane_id: String,
    address: Entity<InputState>,
    editing: bool,
    /// Set by the body's prepaint: the panel was drawn in the last frame.
    seen: Rc<Cell<bool>>,
    /// The webview was hidden natively because the panel left the screen.
    hidden: Rc<Cell<bool>>,
    _subscriptions: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<BrowserRemoved> for BrowserPanel {}

impl BrowserPanel {
    pub fn new(deck: Entity<Deck>, pane_id: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pane_id = pane_id.into();
        let address = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&address, window, |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Focus => this.editing = true,
                InputEvent::Blur => this.editing = false,
                InputEvent::PressEnter { .. } => {
                    this.editing = false;
                    let to = input.read(cx).value().to_string();
                    this.go(&to, cx);
                    window.blur(cx);
                }
                InputEvent::Change => {}
            }),
            cx.observe(&deck, |_, _, cx| cx.notify()),
            // Leaving the tree (panel closed) must not leave the native view over the window.
            cx.on_release(|this, cx| this.hide_native(cx)),
        ];
        let follow = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(FOLLOW_MS)).await;
                if this.update_in(cx, |this, window, cx| this.follow(window, cx)).is_err() {
                    break;
                }
            }
        });
        let read_url = cx.spawn_in(window, async move |this, cx| {
            loop {
                if this.update_in(cx, |this, window, cx| this.read_url(window, cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(URL_POLL_MS)).await;
            }
        });
        Self {
            deck,
            pane_id,
            address,
            editing: false,
            seen: Rc::new(Cell::new(false)),
            hidden: Rc::new(Cell::new(true)),
            _subscriptions: subscriptions,
            _tasks: vec![follow, read_url],
        }
    }

    fn go(&self, to: &str, cx: &mut Context<Self>) {
        if let Err(e) = go(&self.pane_id, to, cx) {
            self.deck.update(cx, |d, cx| d.toast(format!("Browser: {e}"), cx));
        }
    }

    fn read_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing || self.hidden.get() {
            return;
        }
        let Ok(current) = url(&self.pane_id, cx) else { return };
        if self.address.read(cx).value() != current {
            self.address.update(cx, |input, cx| input.set_value(current, window, cx));
        }
    }

    fn hide_native(&self, cx: &App) {
        if let Ok(view) = browser_view(&self.pane_id, cx) {
            let _ = view.read(cx).raw().set_visible(false);
        }
        self.hidden.set(true);
    }

    /// While shown, redraws and checks after that frame that the body was drawn; hides the webview when not.
    fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hidden.get() {
            return; // a redraw that includes the body shows it again
        }
        self.seen.set(false);
        cx.notify();
        let this = cx.entity().downgrade();
        let seen = self.seen.clone();
        // Next-frame callbacks run before that frame draws: the second one runs after the redraw.
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |_, cx| {
                if !seen.get()
                    && let Some(this) = this.upgrade()
                {
                    this.read(cx).hide_native(cx);
                }
            });
        });
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self
            .deck
            .read(cx)
            .state
            .panes
            .get(&self.pane_id)
            .map(|p| p.spec.title.clone())
            .unwrap_or_else(|| "Agent".into());
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Close {title}'s browser? The agent restarts."),
            None,
            &["Close browser", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let pane_id = this.pane_id.clone();
                this.deck.update(cx, |d, cx| {
                    d.state.update_pane(&pane_id, |p| p.spec.browser = false);
                    d.save(cx);
                    cx.notify();
                });
                close(&pane_id, cx);
                cx.emit(BrowserRemoved { pane_id });
            });
        })
        .detach();
    }

    fn header(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let title = self.deck.read(cx).state.panes.get(&self.pane_id).map(|p| p.spec.title.clone());
        let label = match &title {
            Some(t) => format!("{t} · Browser"),
            None => "Browser".into(),
        };
        let nav = |id: &'static str, icon: IconName, tip: &'static str, to: &'static str| {
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(icon)
                .tooltip(tip)
                .on_click(cx.listener(move |this, _, _, cx| this.go(to, cx)))
        };
        let back = nav("browser-back", IconName::ArrowLeft, "Back", "back");
        let forward = nav("browser-forward", IconName::ArrowRight, "Forward", "forward");
        let reload = nav("browser-reload", IconName::RotateCw, "Reload", "reload");
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(ui(6.0))
            .h(ui(32.0))
            .px(ui(10.0))
            .min_w_0()
            .bg(hsla(p.panel))
            .border_b_1()
            .border_color(hsla(p.line))
            .text_size(ui(13.0))
            .text_color(hsla(p.fg))
            .child(Icon::new(IconName::Globe).size(ui(14.0)))
            .child(div().flex_none().font_weight(FontWeight::MEDIUM).child(label))
            .child(back)
            .child(forward)
            .child(reload)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(ui(12.0))
                    .child(Input::new(&self.address).xsmall()),
            )
            .when(title.is_some(), |el| {
                el.child(
                    Button::new("browser-close")
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip("Close browser")
                        .on_click(cx.listener(|this, _, window, cx| this.remove(window, cx))),
                )
            })
    }
}

impl Render for BrowserPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let body = match browser_view(&self.pane_id, cx) {
            Ok(view) => {
                let (seen, hidden) = (self.seen.clone(), self.hidden.clone());
                let webview = view.clone();
                // Runs before the webview's own prepaint, which positions and shows the native view.
                let guard = canvas(
                    move |_, window, cx| {
                        seen.set(true);
                        hidden.set(false);
                        // A native webview sits above every GPUI layer: hide it while a dialog is open.
                        let dialog = window.has_active_dialog(cx);
                        webview.update(cx, |w, _| {
                            if w.visible() == dialog {
                                let _ = w.set_visible(!dialog);
                            }
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full();
                div().relative().flex_1().min_h_0().child(guard).child(view)
            }
            Err(_) => div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_size(ui(12.0))
                .text_color(hsla(p.subtle))
                .child("Starting the browser…"),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(hsla(p.base))
            .child(self.header(cx))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::{Value, config_for, parse_url, valid_id};

    #[test]
    fn config_connects_to_the_webview_port() {
        let c = config_for(9333);
        let args: Vec<&str> = c["mcpServers"]["browser"]["args"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        assert!(args.ends_with(&["chrome-devtools-mcp@latest", "--browserUrl", "http://127.0.0.1:9333"]));
        assert!(valid_id("a-1").is_ok());
        assert!(valid_id("a b").is_err());
    }

    #[test]
    fn urls_default_to_https_and_refuse_other_schemes() {
        assert_eq!(parse_url("example.com").map(|u| u.to_string()), Ok("https://example.com/".into()));
        assert!(parse_url("http://localhost:3000").is_ok());
        assert!(parse_url("file:///c:/x").is_err());
        assert!(parse_url("javascript:alert(1)").is_err());
    }
}
