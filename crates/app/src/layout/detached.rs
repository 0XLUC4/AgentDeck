//! Detached pane windows (DetachedPane.tsx + host.ts). In GPUI the window renders the pane's own
//! terminal entity, so there is no scrollback replay or input forwarding: exactly one window draws
//! the terminal at a time, and the main window shows "Detached in a window" meanwhile.
//! The window's size and position are remembered per pane in local.json (the old localStorage key).

use agentdeck_core::logic::deck::{FONT_SIZE_MAX, FONT_SIZE_MIN};
use agentdeck_core::logic::text::agent_name;
use agentdeck_core::persist::{JsonStore, LOCAL_FILE};
use gpui_kit::assets::IconName;
use gpui_kit::component::TitleBar;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::json;

use super::DeckView;
use super::chrome::icon_button;
use crate::panes::{status_color, status_label};
use crate::state::Deck;
use crate::terminal::TerminalView;
use crate::theme::{hsla, palette, ui};

/// localStorage key of a pane window's geometry in the Tauri build (logical pixels).
fn geometry_key(pane_id: &str) -> String {
    format!("agentdeck:pane-window:{pane_id}")
}

fn read_geometry(deck: &Deck, pane_id: &str) -> Option<Bounds<Pixels>> {
    let store = JsonStore::load(&deck.data_dir, LOCAL_FILE).ok()?;
    let raw = store.get(&geometry_key(pane_id));
    // The web build stored a JSON string; accept an object too.
    let value = match raw.as_str() {
        Some(text) => serde_json::from_str(text).ok()?,
        None => raw,
    };
    let field = |name: &str| value.get(name).and_then(serde_json::Value::as_f64).filter(|v| v.is_finite());
    let (x, y, width, height) = (field("x")?, field("y")?, field("width")?, field("height")?);
    if width < 200.0 || height < 120.0 {
        return None;
    }
    Some(Bounds::new(
        point(px(x as f32), px(y as f32)),
        size(px(width as f32), px(height as f32)),
    ))
}

fn save_geometry(data_dir: &std::path::Path, pane_id: &str, bounds: Bounds<Pixels>) {
    let saved = JsonStore::load(data_dir, LOCAL_FILE).and_then(|mut store| {
        let geometry = json!({
            "x": f32::from(bounds.origin.x),
            "y": f32::from(bounds.origin.y),
            "width": f32::from(bounds.size.width),
            "height": f32::from(bounds.size.height),
        });
        store.set(&geometry_key(pane_id), json!(geometry.to_string()));
        store.save()
    });
    if let Err(e) = saved {
        eprintln!("save window geometry failed: {e}");
    }
}

fn pane_title(deck: &Deck, pane_id: &str) -> String {
    deck.state
        .panes
        .get(pane_id)
        .map(|p| if p.title.is_empty() { agent_name(p.agent).to_owned() } else { p.title.clone() })
        .unwrap_or_default()
}

/// Opens the pane's window. Closing it (title bar X or Alt+F4) reattaches the pane.
pub(super) fn open(
    deck: Entity<Deck>,
    pane_id: &str,
    terminal: Entity<TerminalView>,
    shell: WeakEntity<DeckView>,
    cx: &mut App,
) -> Result<AnyWindowHandle, String> {
    let title = pane_title(deck.read(cx), pane_id);
    let bounds = read_geometry(deck.read(cx), pane_id)
        .unwrap_or_else(|| Bounds::centered(None, size(px(900.0), px(600.0)), cx));
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(title.clone().into()),
            ..TitleBar::title_bar_options()
        }),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(360.0), px(200.0))),
        ..TitleBar::window_options()
    };
    let id = pane_id.to_owned();
    let data_dir = deck.read(cx).data_dir.clone();
    let opened = gpui_kit::open_window(options, cx, move |window, cx| {
        let (close_id, close_shell) = (id.clone(), shell.clone());
        window.on_window_should_close(cx, move |window, cx| {
            save_geometry(&data_dir, &close_id, window.window_bounds().get_bounds());
            if let Err(e) = close_shell.update(cx, |shell, cx| shell.mark_attached(&close_id, cx)) {
                eprintln!("reattach on close failed: {e}");
            }
            true
        });
        cx.new(|cx| DetachedView::new(deck, id, terminal, shell, cx))
    });
    opened.map(|(handle, _)| handle).map_err(|e| e.to_string())
}

/// Closes a pane window from the main window (Reattach, pane closed). Saves its geometry first.
pub(super) fn close(pane_id: &str, handle: AnyWindowHandle, deck: &Entity<Deck>, cx: &mut App) {
    let data_dir = deck.read(cx).data_dir.clone();
    let closed = handle.update(cx, |_, window, _| {
        save_geometry(&data_dir, pane_id, window.window_bounds().get_bounds());
        window.remove_window();
    });
    if let Err(e) = closed {
        eprintln!("close pane window failed: {e}");
    }
}

/// Content of a pane window: a title bar with the pane's identity and zoom/reattach, then its terminal.
pub struct DetachedView {
    deck: Entity<Deck>,
    pane_id: String,
    terminal: Entity<TerminalView>,
    shell: WeakEntity<DeckView>,
    title: String,
    _observe: Subscription,
}

impl DetachedView {
    fn new(
        deck: Entity<Deck>,
        pane_id: String,
        terminal: Entity<TerminalView>,
        shell: WeakEntity<DeckView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&deck, |_, _, cx| cx.notify());
        Self {
            deck,
            pane_id,
            terminal,
            shell,
            title: String::new(),
            _observe: observe,
        }
    }

    fn zoom(&self, delta: f32, cx: &mut Context<Self>) {
        let id = self.pane_id.clone();
        self.deck.update(cx, |deck, cx| {
            deck.state.zoom(&id, Some(delta));
            deck.save(cx);
            cx.notify();
        });
    }

    /// Same as closing the window: the pane goes back into the main window's grid.
    fn reattach(&self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.pane_id.clone();
        save_geometry(&self.deck.read(cx).data_dir, &id, window.window_bounds().get_bounds());
        if let Err(e) = self.shell.update(cx, |shell, cx| shell.mark_attached(&id, cx)) {
            eprintln!("reattach failed: {e}");
        }
        window.remove_window();
    }
}

impl Render for DetachedView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let deck = self.deck.read(cx);
        let pane = deck.state.panes.get(&self.pane_id).cloned();
        let title = pane_title(deck, &self.pane_id);
        if !title.is_empty() && title != self.title {
            window.set_window_title(&title);
            self.title = title.clone();
        }
        let font_size = pane.as_ref().map_or(0.0, |pane| pane.font_size);
        let mut bar = div().flex().items_center().gap_2().flex_1().min_w_0().pl(ui(10.0));
        match &pane {
            Some(pane) => {
                bar = bar
                    .child(div().size(ui(10.0)).flex_shrink_0().rounded_full().bg(hsla(p.agent(pane.agent))))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_color(hsla(p.fg)).child(title))
                    .child(div().size(ui(6.0)).flex_shrink_0().rounded_full().bg(hsla(status_color(pane.status, &p))))
                    .child(div().text_color(hsla(p.muted)).child(status_label(pane.status)))
                    .when(pane.exited, |el| {
                        el.child(
                            div()
                                .text_size(ui(12.0))
                                .text_color(hsla(p.subtle))
                                .child("Process exited: press R to restart"),
                        )
                    });
            }
            None => bar = bar.child("Loading..."),
        }
        let controls = div()
            .flex()
            .items_center()
            .gap(ui(2.0))
            .pr_1()
            .child(
                icon_button("zoom-out", IconName::ZoomOut, "Zoom out (Ctrl+-)", &p)
                    .when(pane.is_none() || font_size <= FONT_SIZE_MIN, |el| el.opacity(0.4))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.zoom(-1.0, cx))),
            )
            .child(
                icon_button("zoom-in", IconName::ZoomIn, "Zoom in (Ctrl+=, Ctrl+0 to reset)", &p)
                    .when(pane.is_none() || font_size >= FONT_SIZE_MAX, |el| el.opacity(0.4))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.zoom(1.0, cx))),
            )
            .child(
                icon_button("reattach", IconName::SquareArrowDownLeft, "Reattach to the main window", &p)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.reattach(window, cx))),
            );
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(hsla(p.base))
            .text_color(hsla(p.fg))
            .font_family(agentdeck_core::logic::theme::FONT_SANS)
            .text_size(ui(13.0))
            .child(TitleBar::new().child(div().flex().items_center().size_full().child(bar).child(controls)))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .py_1()
                    .pl_2()
                    .child(self.terminal.clone()),
            )
    }
}
