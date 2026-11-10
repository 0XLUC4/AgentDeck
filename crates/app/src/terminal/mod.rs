//! Terminal emulator view for one pane (replaces xterm.js): alacritty_terminal parses the VT stream,
//! [`element`] paints the grid on the GPU. Scrollback 5000, selection + copy, paste (text, images
//! saved to a temp file, copied files), file drop as agent refs, zoom (Ctrl+= / Ctrl+- / Ctrl+0),
//! bracketed paste, mouse reporting, Ctrl+click on URLs, resize reported to the pty, find in the
//! scrollback (Ctrl+Shift+F, [`search`]).
//! Original: src/components/TerminalPane/TerminalPane.tsx, src/lib/{ringBuffer,wakeTerminal,fileRefs}.ts.

mod colors;
mod element;
mod glyphs;
mod input;
mod search;

use std::cell::{OnceCell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use agentdeck_core::agents::AgentKind;
use agentdeck_core::logic::text::file_refs;
use agentdeck_core::logic::layout::ReplayGate;
use agentdeck_core::logic::theme::RADIUS_XL;
use agentdeck_core::{pty, skills};
use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as AlacEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point as AlacPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, TermMode, viewport_to_point};
use alacritty_terminal::vte::ansi::Processor;
use gpui_kit::*;

use crate::panes::runtime;
use crate::state::Deck;
use crate::theme::{hsla, palette, ui};

/// Lines kept above the screen (xterm `scrollback: 5000`).
const SCROLLBACK: usize = 5000;
/// Cursor blink half-period.
const BLINK_MS: u64 = 530;
/// Delay before a new size is sent to the pty (TerminalPane.tsx `resizeTimer`).
const RESIZE_REPORT_MS: u64 = 100;
/// Sink owner label for `pty::attach_pane_output`.
const OWNER: &str = "main";
/// Padding of the terminal host (`py-1 pl-2`).
const PAD_LEFT: f32 = 8.0;
const PAD_Y: f32 = 4.0;

/// Replies the emulator wants to send (answers to terminal queries) or clipboard writes (OSC 52).
enum Reply {
    Pty(String),
    Clipboard(String),
    /// OSC 4/10/11/12 query, answered in `feed` with the current theme.
    Color(usize, ColorFormat),
    /// OSC 0/2 window title: state, not an answer, so a replay applies it too.
    Title(String),
}

type ColorFormat = std::sync::Arc<dyn Fn(alacritty_terminal::vte::ansi::Rgb) -> String + Sync + Send>;

#[derive(Clone, Default)]
struct Listener(Rc<RefCell<Vec<Reply>>>);

impl EventListener for Listener {
    fn send_event(&self, event: AlacEvent) {
        let reply = match event {
            AlacEvent::PtyWrite(text) => Reply::Pty(text),
            // OSC 4/10/11/12 queries: TUIs ask for the palette and background before choosing colors.
            AlacEvent::ColorRequest(index, format) => Reply::Color(index, format),
            AlacEvent::ClipboardStore(_, text) => Reply::Clipboard(text),
            AlacEvent::Title(title) => Reply::Title(title),
            _ => return,
        };
        self.0.borrow_mut().push(reply);
    }
}

/// Cell geometry of the last paint, used to map mouse positions to cells.
#[derive(Clone, Copy)]
struct Metrics {
    origin: Point<Pixels>,
    cell: Size<Pixels>,
}

fn new_term(cols: usize, rows: usize, listener: Listener) -> Term<Listener> {
    let config = Config { scrolling_history: SCROLLBACK, ..Config::default() };
    Term::new(config, &TermSize::new(cols, rows), listener)
}

/// Terminal of one pane. It subscribes to the pane output when created, so nothing is lost
/// before its first paint.
pub struct TerminalView {
    pub pane_id: String,
    /// Deck to read the pane (agent, zoom, restarts) from. Found from the main window when unset.
    pub deck: Option<Entity<Deck>>,
    /// Only one terminal per pane answers queries, sends input and resizes (`forwards_input`).
    pub forwards_input: bool,
    term: Term<Listener>,
    parser: Processor,
    replies: Listener,
    focus: OnceCell<FocusHandle>,
    sink: u64,
    output: Option<smol::channel::Receiver<String>>,
    started: bool,
    // Pane state read from the deck at render.
    agent: AgentKind,
    font_size: f32,
    font_family: SharedString,
    generation: Option<u32>,
    focused: bool,
    blink_on: bool,
    /// IME composition not committed yet, drawn at the cursor (xterm's composition view).
    preedit: Option<String>,
    metrics: Option<Metrics>,
    sized: bool,
    scroll_px: f32,
    selecting: bool,
    last_mouse_cell: Option<(usize, usize)>,
    /// Find in terminal, while its bar is open.
    search: Option<search::Search>,
    resize_task: Option<Task<()>>,
    tasks: Vec<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl TerminalView {
    pub fn new(pane_id: impl Into<String>) -> Self {
        let pane_id = pane_id.into();
        let replies = Listener::default();
        // Bridge the pty sink (std channel) to an async channel through a small thread.
        let (sink_tx, sink_rx) = mpsc::channel::<String>();
        let (ui_tx, ui_rx) = smol::channel::unbounded();
        let sink = pty::attach_pane_output(OWNER, pane_id.clone(), sink_tx);
        let spawned = std::thread::Builder::new()
            .name(format!("term-out-{pane_id}"))
            .spawn(move || {
                // Ends when the sink is detached (sender dropped) or the view is gone.
                while let Ok(chunk) = sink_rx.recv() {
                    if ui_tx.send_blocking(chunk).is_err() {
                        break;
                    }
                }
            });
        if let Err(e) = spawned {
            eprintln!("terminal {pane_id}: output thread not started: {e}");
        }
        Self {
            pane_id,
            deck: None,
            forwards_input: true,
            term: new_term(80, 24, replies.clone()),
            parser: Processor::new(),
            replies,
            focus: OnceCell::new(),
            sink,
            output: Some(ui_rx),
            started: false,
            preedit: None,
            agent: AgentKind::Shell,
            font_size: agentdeck_core::logic::deck::FONT_SIZE_DEFAULT,
            font_family: agentdeck_core::logic::theme::FONT_MONO.into(),
            generation: None,
            focused: false,
            blink_on: true,
            metrics: None,
            sized: false,
            scroll_px: 0.0,
            selecting: false,
            last_mouse_cell: None,
            search: None,
            resize_task: None,
            tasks: Vec::new(),
            subscriptions: Vec::new(),
        }
    }

    /// Clears the screen (pane restarted: fresh output).
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.term = new_term(self.term.columns(), self.term.screen_lines(), self.replies.clone());
        self.parser = Processor::new();
        cx.notify();
    }

    /// Feeds pty output to the emulator. A replay's answers to old terminal queries are dropped.
    fn feed(&mut self, data: &str, replay: bool, cx: &mut Context<Self>) {
        let history = self.term.history_size();
        self.parser.advance(&mut self.term, data.as_bytes());
        if let Some(search) = &mut self.search {
            search.scrolled(self.term.history_size().saturating_sub(history));
        }
        let replies = std::mem::take(&mut *self.replies.0.borrow_mut());
        for reply in replies {
            if let Reply::Title(title) = &reply {
                crate::panes::runtime::set_topic(&self.pane_id, title, cx);
            }
            if replay {
                continue;
            }
            match reply {
                Reply::Title(_) => {}
                Reply::Pty(text) => self.send_input(&text, cx),
                Reply::Clipboard(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                Reply::Color(index, format) => self.send_input(&format(colors::query(index, colors::theme(cx))), cx),
            }
        }
        cx.notify();
    }

    /// Writes to the pane process through the runtime (which restarts an exited pane on "r").
    /// Only the owner terminal forwards: a second one would double the answers to queries.
    fn send_input(&mut self, data: &str, cx: &mut Context<Self>) {
        if self.forwards_input && !data.is_empty() {
            runtime::send_input(&self.pane_id, data, cx);
        }
    }

    /// Focus reporting (DECSET 1004), when the program asked for it: Codex only sends its
    /// "approval requested" notification while its terminal is not focused.
    fn report_focus(&mut self, focused: bool, cx: &mut Context<Self>) {
        if self.term.mode().contains(TermMode::FOCUS_IN_OUT) {
            self.send_input(if focused { "[I" } else { "[O" }, cx);
        }
    }

    /// Input typed by the user: back to the bottom, selection cleared, cursor shown.
    fn user_input(&mut self, data: &str, cx: &mut Context<Self>) {
        self.term.scroll_display(Scroll::Bottom);
        self.term.selection = None;
        self.blink_on = true;
        self.send_input(data, cx);
        cx.notify();
    }

    /// Pastes like xterm `term.paste`: CR newlines, bracketed when the program asked for it.
    fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let bracketed = self.term.mode().contains(TermMode::BRACKETED_PASTE);
        self.user_input(&input::paste_text(text, bracketed), cx);
    }

    /// Files reach the agent as paths in its prompt (`fileRefs`).
    fn paste_paths(&mut self, paths: &[String], window: &mut Window, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        self.paste_text(&file_refs(self.agent, paths), cx);
        window.focus(&self.focus_handle(cx), cx);
    }

    /// Ctrl+V: images are saved to temp files and pasted as paths, copied files as paths, else text.
    fn paste_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let images: Vec<(Vec<u8>, &'static str)> = item
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                ClipboardEntry::Image(image) => image_ext(image.format).map(|ext| (image.bytes.clone(), ext)),
                _ => None,
            })
            .collect();
        if !images.is_empty() {
            let saving = cx.background_spawn(async move {
                images
                    .iter()
                    .map(|(bytes, ext)| skills::save_paste_image(bytes, ext))
                    .collect::<Result<Vec<String>, String>>()
            });
            cx.spawn_in(window, async move |this, cx| {
                let saved = saving.await;
                let updated = this.update_in(cx, |view, window, cx| match saved {
                    Ok(paths) => view.paste_paths(&paths, window, cx),
                    Err(e) => view.toast(format!("Could not paste the image: {e}"), cx),
                });
                if let Err(e) = updated {
                    eprintln!("terminal paste: {e}");
                }
            })
            .detach();
            return;
        }
        let files: Vec<String> = item
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                ClipboardEntry::ExternalPaths(paths) => Some(paths.paths()),
                _ => None,
            })
            .flatten()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        if !files.is_empty() {
            return self.paste_paths(&files, window, cx);
        }
        if let Some(text) = item.text() {
            self.paste_text(&text, cx);
        }
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        match self.term.selection_to_string().filter(|t| !t.is_empty()) {
            Some(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                true
            }
            None => false,
        }
    }

    fn toast(&mut self, message: String, cx: &mut Context<Self>) {
        match &self.deck {
            Some(deck) => deck.update(cx, |deck, cx| deck.toast(message, cx)),
            None => eprintln!("terminal {}: {message}", self.pane_id),
        }
    }

    /// Ctrl+= / Ctrl+- / Ctrl+0: per-pane font size, saved with the deck.
    fn zoom(&mut self, delta: Option<f32>, cx: &mut Context<Self>) {
        let Some(deck) = self.deck.clone() else { return };
        let id = self.pane_id.clone();
        deck.update(cx, |deck, cx| {
            deck.state.zoom(&id, delta);
            deck.save(cx);
            cx.notify();
        });
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Keys coming up from the search input are not for the pty.
        if !self.focus_handle(cx).is_focused(window) {
            return;
        }
        let key = &event.keystroke;
        let m = &key.modifiers;
        if m.control && !m.alt && !m.platform {
            let handled = match key.key.as_str() {
                "=" | "+" => {
                    self.zoom(Some(1.0), cx);
                    true
                }
                "-" => {
                    self.zoom(Some(-1.0), cx);
                    true
                }
                "0" => {
                    self.zoom(None, cx);
                    true
                }
                "v" => {
                    self.paste_clipboard(window, cx);
                    true
                }
                // Ctrl+Shift+C copies; Ctrl+C copies only when text is selected, else it is ^C.
                "c" if m.shift => {
                    self.copy_selection(cx);
                    true
                }
                "c" if self.copy_selection(cx) => {
                    self.term.selection = None;
                    cx.notify();
                    true
                }
                _ => false,
            };
            if handled {
                cx.stop_propagation();
                return;
            }
        }
        // Shift+PageUp/PageDown scroll the scrollback (xterm), except in full-screen programs.
        if m.shift && !m.control && !m.alt && !self.term.mode().contains(TermMode::ALT_SCREEN) {
            let page = match key.key.as_str() {
                "pageup" => Some(Scroll::PageUp),
                "pagedown" => Some(Scroll::PageDown),
                _ => None,
            };
            if let Some(scroll) = page {
                self.term.scroll_display(scroll);
                cx.notify();
                cx.stop_propagation();
                return;
            }
        }
        if let Some(seq) = input::key_to_esc(key, *self.term.mode()) {
            self.user_input(&seq, cx);
            cx.stop_propagation();
        }
    }

    /// Screen cell under a window position (clamped), with the side of the cell.
    fn cell_at(&self, position: Point<Pixels>) -> Option<(usize, usize, Side)> {
        let m = self.metrics?;
        let x = f32::from(position.x - m.origin.x) / f32::from(m.cell.width);
        let y = f32::from(position.y - m.origin.y) / f32::from(m.cell.height);
        let col = (x.max(0.0) as usize).min(self.term.columns().saturating_sub(1));
        let row = (y.max(0.0) as usize).min(self.term.screen_lines().saturating_sub(1));
        let side = if x - x.floor() < 0.5 { Side::Left } else { Side::Right };
        Some((col, row, side))
    }

    fn grid_point(&self, col: usize, row: usize) -> AlacPoint {
        let offset = self.term.grid().display_offset();
        viewport_to_point(offset, AlacPoint::new(row, Column(col)))
    }

    /// Mouse reporting is on and Shift is not held (Shift forces local selection, like xterm).
    fn reports_mouse(&self, m: &Modifiers) -> bool {
        self.term.mode().intersects(TermMode::MOUSE_MODE) && !m.shift
    }

    fn report_mouse(&mut self, button: u8, position: Point<Pixels>, pressed: bool, m: &Modifiers, cx: &mut Context<Self>) {
        let Some((col, row, _)) = self.cell_at(position) else { return };
        if let Some(seq) = input::mouse_report(button, col, row, pressed, m, *self.term.mode()) {
            self.send_input(&seq, cx);
        }
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle(cx), cx);
        runtime::focus_pane(&self.pane_id, cx);
        let Some(button) = button_code(event.button) else { return };
        if self.reports_mouse(&event.modifiers) {
            self.report_mouse(button, event.position, true, &event.modifiers, cx);
            return;
        }
        if button != input::MOUSE_LEFT {
            return;
        }
        let Some((col, row, side)) = self.cell_at(event.position) else { return };
        if event.modifiers.control
            && let Some(url) = self.url_at(col, row)
        {
            cx.open_url(&url);
            return;
        }
        let point = self.grid_point(col, row);
        match (&mut self.term.selection, event.modifiers.shift) {
            (Some(selection), true) => selection.update(point, side),
            _ => {
                let ty = match event.click_count {
                    2 => SelectionType::Semantic,
                    n if n >= 3 => SelectionType::Lines,
                    _ => SelectionType::Simple,
                };
                self.term.selection = Some(Selection::new(ty, point, side));
            }
        }
        self.selecting = true;
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some((col, row, side)) = self.cell_at(event.position) else { return };
        if self.selecting && event.dragging() {
            let point = self.grid_point(col, row);
            if let Some(selection) = self.term.selection.as_mut() {
                selection.update(point, side);
                cx.notify();
            }
            return;
        }
        if !self.reports_mouse(&event.modifiers) || self.last_mouse_cell == Some((col, row)) {
            return;
        }
        self.last_mouse_cell = Some((col, row));
        let mode = *self.term.mode();
        // Drag reports need MOUSE_DRAG or MOUSE_MOTION; plain motion (button 3) needs MOUSE_MOTION.
        let button = match event.pressed_button.and_then(button_code) {
            Some(b) if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) => b,
            None if mode.contains(TermMode::MOUSE_MOTION) => 3,
            _ => return,
        };
        self.report_mouse(button + input::MOUSE_MOTION, event.position, true, &event.modifiers, cx);
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.selecting = false;
            // A click without a drag selects nothing.
            if self.term.selection.as_ref().is_some_and(|s| s.is_empty()) {
                self.term.selection = None;
                cx.notify();
            }
            return;
        }
        let Some(button) = button_code(event.button) else { return };
        if self.reports_mouse(&event.modifiers) {
            self.report_mouse(button, event.position, false, &event.modifiers, cx);
        }
    }

    fn on_scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = self.metrics else { return };
        let line = f32::from(m.cell.height).max(1.0);
        self.scroll_px += f32::from(event.delta.pixel_delta(px(line)).y);
        let lines = (self.scroll_px / line) as i32;
        if lines == 0 {
            return;
        }
        self.scroll_px -= lines as f32 * line;
        let mode = *self.term.mode();
        if self.reports_mouse(&event.modifiers) {
            let button = if lines > 0 { input::MOUSE_WHEEL_UP } else { input::MOUSE_WHEEL_DOWN };
            // One report per line, sent in a single write.
            if let Some((col, row, _)) = self.cell_at(event.position)
                && let Some(seq) = input::mouse_report(button, col, row, true, &event.modifiers, mode)
            {
                self.send_input(&seq.repeat(lines.unsigned_abs() as usize), cx);
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            // Full-screen programs without mouse mode get arrow keys (xterm alternate scroll).
            let app = mode.contains(TermMode::APP_CURSOR);
            let arrow = match (lines > 0, app) {
                (true, true) => "\x1bOA",
                (true, false) => "\x1b[A",
                (false, true) => "\x1bOB",
                (false, false) => "\x1b[B",
            };
            self.send_input(&arrow.repeat(lines.unsigned_abs() as usize), cx);
        } else {
            self.term.scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
        cx.stop_propagation();
    }

    /// The URL under a screen cell (Ctrl+click opens it).
    fn url_at(&self, col: usize, row: usize) -> Option<String> {
        let line = self.grid_point(0, row).line;
        let grid = self.term.grid();
        let chars: Vec<char> = (0..self.term.columns()).map(|c| grid[line][Column(c)].c).collect();
        input::url_at(&chars, col)
    }
    /// Applies the grid size from the layout; the pty hears about it after a short delay.
    fn resize_to(&mut self, cols: usize, rows: usize, metrics: Metrics, cx: &mut Context<Self>) {
        self.metrics = Some(metrics);
        let changed = cols != self.term.columns() || rows != self.term.screen_lines();
        if changed {
            self.term.resize(TermSize::new(cols, rows));
        }
        if changed || !self.sized {
            self.sized = true;
            self.resize_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(RESIZE_REPORT_MS)).await;
                let reported = this.update(cx, |view, cx| {
                    if view.forwards_input {
                        let (cols, rows) = (view.term.columns() as u16, view.term.screen_lines() as u16);
                        runtime::report_size(&view.pane_id, cols, rows, cx);
                    }
                });
                if let Err(e) = reported {
                    eprintln!("terminal resize: {e}");
                }
            }));
        }
    }

    /// Reads the pane state the terminal shows. A new generation (restart) clears the screen.
    /// Returns whether the font changed (a repaint is needed; `reset` asks for its own).
    fn read_pane(&mut self, deck: &Entity<Deck>, cx: &mut Context<Self>) -> bool {
        let (generation, font_changed) = {
            let deck = deck.read(cx);
            let pane = deck.state.panes.get(&self.pane_id);
            let family = mono_family(&deck.settings.font_family);
            let mut font_changed = self.font_family.as_ref() != family;
            if font_changed {
                self.font_family = family.into();
            }
            if let Some(pane) = pane {
                self.agent = pane.agent;
                font_changed |= self.font_size != pane.font_size;
                self.font_size = pane.font_size;
            }
            (pane.map(|p| p.generation), font_changed)
        };
        if generation.is_some() {
            if self.generation.is_some() && generation != self.generation {
                self.reset(cx);
            }
            self.generation = generation;
        }
        font_changed
    }

    /// Finds the deck (main window root when unset) and starts the terminal on first render.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.deck.is_none() {
            self.deck = window
                .root::<crate::layout::DeckView>()
                .flatten()
                .map(|root| root.read(cx).deck().clone());
        }
        let Some(deck) = self.deck.clone() else { return };
        self.read_pane(&deck, cx);
        self.focused = self.focus_handle(cx).is_focused(window);
        if !self.started {
            self.started = true;
            self.start(&deck, window, cx);
        }
    }

    /// Replays the output buffered before this view existed, then pumps live output into the grid.
    /// It needs no window: called when the view is created, a pane that is never shown parses its
    /// output as it comes instead of queueing it all. Later calls do nothing.
    pub fn start_output(&mut self, cx: &mut Context<Self>) {
        let Some(output) = self.output.take() else { return };
        // Live chunks already queued may also be in the runtime's buffer: the gate drops the overlap.
        let mut gate = ReplayGate::new();
        while let Ok(chunk) = output.try_recv() {
            gate.output(&chunk);
        }
        let replay = runtime::pane_output_text(&self.pane_id, cx);
        if let Some((old, held)) = gate.replay(&replay) {
            self.feed(&old, true, cx);
            for chunk in held {
                self.feed(&chunk, false, cx);
            }
        }
        self.tasks.push(cx.spawn(async move |this, cx| {
            while let Ok(mut chunk) = output.recv().await {
                while let Ok(more) = output.try_recv() {
                    chunk.push_str(&more);
                }
                if this.update(cx, |view, cx| view.feed(&chunk, false, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// Starts the output (unless the owner already did), blinks the cursor and follows the deck.
    fn start(&mut self, deck: &Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) {
        self.start_output(cx);
        search::bind_keys(cx);
        self.tasks.push(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(BLINK_MS)).await;
                let alive = this.update(cx, |view, cx| {
                    if view.focused {
                        view.blink_on = !view.blink_on;
                        cx.notify();
                    } else {
                        view.blink_on = true;
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
        let focus = self.focus_handle(cx);
        self.subscriptions.push(cx.on_focus(&focus, window, |view, _, cx| {
            view.focused = true;
            view.blink_on = true;
            view.report_focus(true, cx);
            cx.notify();
        }));
        self.subscriptions.push(cx.on_blur(&focus, window, |view, _, cx| {
            view.focused = false;
            view.report_focus(false, cx);
            cx.notify();
        }));
        // Switching to another app keeps the GPUI focus: report it too, or Codex thinks you are watching.
        self.subscriptions.push(cx.observe_window_activation(window, |view, window, cx| {
            if view.focused {
                view.report_focus(window.is_window_active(), cx);
            }
        }));
        // Theme switch: repaint with the new terminal colors.
        self.subscriptions.push(cx.observe_global::<crate::theme::AppTheme>(|_, cx| cx.notify()));
        // Checked on every deck change, before the restarted process prints.
        self.subscriptions.push(cx.observe(deck, |view, deck, cx| {
            if view.read_pane(&deck, cx) {
                cx.notify();
            }
        }));
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        pty::detach_pane_output(self.pane_id.clone(), self.sink);
    }
}

fn button_code(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(input::MOUSE_LEFT),
        MouseButton::Middle => Some(input::MOUSE_MIDDLE),
        MouseButton::Right => Some(input::MOUSE_RIGHT),
        _ => None,
    }
}

/// First family of a CSS font list ("Cascadia Code, Consolas, monospace" -> "Cascadia Code").
fn mono_family(css: &str) -> String {
    css.split(',')
        .map(|f| f.trim().trim_matches(['"', '\'']))
        .find(|f| !f.is_empty() && *f != "monospace")
        .unwrap_or(agentdeck_core::logic::theme::FONT_MONO)
        .to_owned()
}

/// Extension for a clipboard image format we save (`imageExt`).
fn image_ext(format: ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Png => Some("png"),
        ImageFormat::Jpeg => Some("jpg"),
        ImageFormat::Gif => Some("gif"),
        ImageFormat::Webp => Some("webp"),
        ImageFormat::Bmp => Some("bmp"),
        _ => None,
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.focus.get_or_init(|| cx.focus_handle()).clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(window, cx);
        let p = palette(cx);
        let focus = self.focus_handle(cx);
        div()
            .id(SharedString::from(format!("terminal-{}", self.pane_id)))
            .key_context("Terminal")
            .track_focus(&focus)
            .relative()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .rounded_b(ui(RADIUS_XL - 1.0))
            .bg(hsla(p.base))
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(Self::open_search))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .on_scroll(cx.listener(Self::on_scroll))
            // Native drag & drop gives real file paths; the pane under the cursor is outlined.
            .drag_over::<ExternalPaths>(move |style, _, _, _| {
                style.border_2().border_dashed().border_color(hsla(p.accent))
            })
            .on_drop(cx.listener(|view, paths: &ExternalPaths, window, cx| {
                let paths: Vec<String> = paths.paths().iter().map(|p| p.to_string_lossy().into_owned()).collect();
                view.paste_paths(&paths, window, cx);
            }))
            .child(element::terminal_canvas(cx.entity(), PAD_LEFT, PAD_Y))
            .children(self.render_search(cx))
    }
}

/// IME and plain text input: typed characters arrive here, keys with escape codes in `on_key_down`.
impl EntityInputHandler for TerminalView {
    fn text_for_range(&mut self, _: Range<usize>, _: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        None
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: 0..0, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.preedit.as_ref().map(|t| 0..t.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.preedit.take().is_some() {
            cx.notify();
        }
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        if self.preedit.take().is_some() {
            cx.notify();
        }
        self.user_input(text, cx);
    }

    /// IME composition (dead keys, CJK): shown at the cursor until committed, never sent to the pty.
    fn replace_and_mark_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        self.preedit = (!text.is_empty()).then(|| text.to_owned());
        cx.notify();
    }

    /// The cursor cell, so the IME window opens next to it.
    fn bounds_for_range(&mut self, _: Range<usize>, _: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let m = self.metrics?;
        let cursor = self.term.grid().cursor.point;
        let origin = point(
            m.origin.x + m.cell.width * cursor.column.0 as f32,
            m.origin.y + m.cell.height * cursor.line.0.max(0) as f32,
        );
        Some(Bounds::new(origin, m.cell))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}
