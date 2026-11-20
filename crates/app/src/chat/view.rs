//! The chat pane body: turns ("Worked for 4s · 2 files +12 −3" folds the steps of a finished
//! turn and lists its changed files with their patches), user bubbles (copy, "Show more"),
//! markdown answers (copyable code blocks, "Fork from here" on the last one), tool rows with
//! icons, plan checklists, worker cards, Ctrl+F find, and the composer: plan panel, follow-up
//! queue, `/` and `@` pickers, prompt history, attach images, permission and model menus,
//! dictation, Send / Stop. Repaints when its transcript changes, and every second while a turn
//! runs (elapsed time).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agentdeck_core::agents::AgentKind;
use agentdeck_core::chat::{FileEdit, Item, Mode, PlanStep, StepStatus, Tool, Transcript, latest_plan, trigger_at, turn_changes, with_model};
use agentdeck_core::logic::deck::Pane;
use agentdeck_core::logic::text::agent_name;
use agentdeck_core::logic::theme::{FONT_MONO, Palette, RADIUS_LG, RADIUS_MD};
use agentdeck_core::skills::save_paste_image;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    Enter, Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp, Search, Textarea, TextareaState,
};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::TextView;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::flow::{self, Queued};
use crate::palette::fuzzy::fuzzy_score;
use crate::state::{Deck, DeckEvent};
use crate::theme::{hsla, hsla_a, palette, ui};

/// Corner radius of the message box (rounder than the panes, like a chat composer).
const RADIUS_COMPOSER: f32 = 14.0;
/// Side of an image thumbnail in a bubble / in the composer.
const THUMB: f32 = 64.0;
const THUMB_SMALL: f32 = 44.0;
const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];
/// A user message longer than this folds behind "Show more".
const FOLD_LINES: usize = 12;
const FOLD_CHARS: usize = 1200;
/// Files listed for `@` (walk cap) and rows shown in a picker.
const MAX_FILES: usize = 5000;
const MAX_CHOICES: usize = 50;
const CLAUDE_MODELS: [&str; 3] = ["opus", "sonnet", "haiku"];

/// Views by pane id, and the task that repaints them on transcript changes.
#[derive(Default)]
struct Views {
    map: HashMap<String, Entity<ChatView>>,
    _repaint: Option<Task<()>>,
}

impl Global for Views {}

/// The chat view of a pane, created on first use.
pub fn view(pane_id: &str, deck: &Entity<Deck>, window: &mut Window, cx: &mut App) -> Entity<ChatView> {
    if cx.default_global::<Views>()._repaint.is_none() {
        let task = cx.spawn(async move |cx| {
            while let Ok(id) = super::CHANGED.1.recv().await {
                cx.update(|cx| {
                    let view = cx.global::<Views>().map.get(&id).cloned();
                    if let Some(view) = view {
                        view.update(cx, |_, cx| cx.notify());
                    }
                });
            }
        });
        cx.global_mut::<Views>()._repaint = Some(task);
    }
    if let Some(view) = cx.global::<Views>().map.get(pane_id) {
        return view.clone();
    }
    let view = cx.new(|cx| ChatView::new(pane_id, deck.clone(), window, cx));
    // Views of closed panes go when a new one comes.
    let alive: HashSet<String> = deck.read(cx).state.panes.keys().cloned().collect();
    let views = &mut cx.global_mut::<Views>().map;
    views.retain(|id, _| alive.contains(id));
    views.insert(pane_id.to_owned(), view.clone());
    view
}

/// A dictation for a chat pane: typed into its message box at the next render. False when the
/// pane has no chat view yet (the caller sends it as a prompt instead).
pub fn dictate(pane_id: &str, text: &str, cx: &mut App) -> bool {
    let Some(view) = cx.try_global::<Views>().and_then(|v| v.map.get(pane_id).cloned()) else { return false };
    view.update(cx, |this, cx| {
        this.dictated.push_str(text);
        cx.notify();
    });
    true
}

/// The open `/` or `@` list: where its token starts and the highlighted row.
struct Picker {
    sigil: char,
    start: usize,
    query: String,
    selected: usize,
}

/// A row of a picker.
#[derive(Clone)]
struct Choice {
    group: &'static str,
    label: String,
    detail: String,
    pick: Pick,
}

#[derive(Clone)]
enum Pick {
    Clear,
    PlanMode,
    Insert(String),
}

pub struct ChatView {
    pane_id: String,
    deck: Entity<Deck>,
    input: Entity<TextareaState>,
    scroll: ScrollHandle,
    /// Tool rows unfolded to show their output.
    open: HashSet<String>,
    /// Finished turns unfolded to show their steps (index of their user item).
    unfolded: HashSet<usize>,
    /// Finished turns listing their changed files, and the patches shown ("turn:path").
    files_open: HashSet<usize>,
    patches_open: HashSet<String>,
    /// Long user messages shown whole.
    users_open: HashSet<usize>,
    plan_open: bool,
    /// Images attached to the next message.
    images: Vec<String>,
    /// Dictated text waiting for the next render (inserting needs the window).
    dictated: String,
    /// Dictation state ("listening", "transcribing"...), from `voice-state`.
    voice: String,
    /// Item count at the last render: new items scroll to the bottom.
    seen: usize,
    /// The queued card being edited, in `edit_input`.
    editing: Option<u64>,
    edit_input: Entity<TextareaState>,
    picker: Option<Picker>,
    /// Token start of a picker closed with Esc: it stays closed for that token.
    dismissed: Option<usize>,
    picker_scroll: ScrollHandle,
    skills: Option<Vec<(String, String)>>,
    files: Option<Arc<Vec<String>>>,
    /// Up / Down in an empty composer: how far back in the sent prompts (0 = last).
    recall: Option<usize>,
    find: Option<Entity<InputState>>,
    custom_model: Option<Entity<InputState>>,
    /// Repaints once a second while a turn runs (elapsed time).
    _tick: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ChatView {
    fn new(pane_id: &str, deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask anything, / for commands, @ for files")
                .auto_grow(1, 10)
                .submit_on_enter(true)
        });
        let on_input = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { shift: false, secondary } => this.submit(*secondary, window, cx),
            InputEvent::Change => {
                this.update_picker(cx);
                cx.notify(); // greys or lights up the send button
            }
            _ => {}
        });
        let edit_input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 6).submit_on_enter(true));
        let on_edit = cx.subscribe_in(&edit_input, window, |this, state, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { shift: false, .. } = event
                && let Some(id) = this.editing.take()
            {
                let text = state.read(cx).value().to_string();
                flow::edit_queued(&this.pane_id, id, &text);
                cx.notify();
            }
        });
        let on_deck = cx.subscribe(&deck, |this, _, event: &DeckEvent, cx| {
            let DeckEvent::Backend(event) = event;
            if event.name == "voice-state" {
                this.voice = event.payload.as_str().unwrap_or("").to_owned();
                cx.notify();
            }
        });
        Self {
            pane_id: pane_id.to_owned(),
            deck,
            input,
            scroll: ScrollHandle::new(),
            open: HashSet::new(),
            unfolded: HashSet::new(),
            files_open: HashSet::new(),
            patches_open: HashSet::new(),
            users_open: HashSet::new(),
            plan_open: false,
            images: Vec::new(),
            dictated: String::new(),
            voice: String::new(),
            seen: 0,
            editing: None,
            edit_input,
            picker: None,
            dismissed: None,
            picker_scroll: ScrollHandle::new(),
            skills: None,
            files: None,
            recall: None,
            find: None,
            custom_model: None,
            _tick: None,
            _subscriptions: vec![on_input, on_edit, on_deck],
        }
    }

    fn pane(&self, cx: &App) -> Option<Pane> {
        self.deck.read(cx).state.panes.get(&self.pane_id).cloned()
    }

    fn busy(&self) -> bool {
        super::chats().map.get(&self.pane_id).is_some_and(|c| c.transcript.busy)
    }

    fn toast(&self, message: &str, cx: &mut App) {
        self.deck.update(cx, |d, cx| d.toast(message, cx));
    }

    /// Enter: send, or queue while a turn runs. Ctrl+Enter (`steer`): into the running turn now;
    /// with an empty composer, the first queued card goes.
    fn submit(&mut self, steer: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        let Some(pane) = self.pane(cx) else { return };
        self.recall = None;
        match text.trim() {
            "/clear" => {
                self.input.update(cx, |s, cx| s.set_value("", window, cx));
                return self.run_clear(&pane, cx);
            }
            "/plan" if pane.agent == AgentKind::Claude => {
                self.input.update(cx, |s, cx| s.set_value("", window, cx));
                return self.toggle_plan(&pane, cx);
            }
            _ => {}
        }
        if text.trim().is_empty() && self.images.is_empty() {
            if steer && let Some(first) = flow::queued(&self.pane_id).0.first() {
                if let Some(prompt) = flow::unqueue(&self.pane_id, first.id) {
                    flow::steer(&pane, &prompt.text, &prompt.images, cx);
                }
            }
            return;
        }
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        self.picker = None;
        let images = std::mem::take(&mut self.images);
        if steer {
            flow::steer(&pane, &text, &images, cx);
        } else if self.busy() {
            flow::enqueue(&self.pane_id, &text, &images);
        } else {
            super::send_with_images(&pane, &text, &images, cx);
        }
        cx.notify();
    }

    fn run_clear(&mut self, pane: &Pane, cx: &mut Context<Self>) {
        if self.busy() {
            return self.toast("Stop the running turn first", cx);
        }
        flow::clear(pane, &self.deck, cx);
        self.unfolded.clear();
        self.files_open.clear();
        self.patches_open.clear();
        self.users_open.clear();
        self.open.clear();
        self.seen = 0;
        cx.notify();
    }

    fn toggle_plan(&mut self, pane: &Pane, cx: &mut Context<Self>) {
        let mode = if Mode::of(pane.agent, &pane.args) == Mode::Plan { Mode::AutoEdit } else { Mode::Plan };
        self.set_mode(mode, cx);
    }

    /// Model / permission change: saved in the pane's arguments (next start) and applied to the
    /// running agent (Codex next turn, Claude restarted on the same conversation once idle).
    fn edit_args(&mut self, edit: impl Fn(&[String]) -> Vec<String> + Clone + 'static, cx: &mut Context<Self>) {
        let id = self.pane_id.clone();
        let saved = edit.clone();
        self.deck.update(cx, |d, cx| {
            d.state.update_pane(&id, |p| p.spec.args = saved(&p.spec.args));
            d.save(cx);
            cx.notify();
        });
        flow::edit_args(&self.pane_id, edit, cx);
        cx.notify();
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        let Some(agent) = self.pane(cx).map(|p| p.agent) else { return };
        self.edit_args(move |args| mode.apply(agent, args), cx);
    }

    fn set_model(&mut self, model: Option<String>, cx: &mut Context<Self>) {
        self.custom_model = None;
        self.edit_args(move |args| with_model(args, model.as_deref()), cx);
    }

    fn open_custom_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = cx.new(|cx| InputState::new(window, cx).placeholder("Model name, Enter to use it"));
        let sub = cx.subscribe(&state, |this, state, event: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let model = state.read(cx).value().trim().to_owned();
                this.set_model((!model.is_empty()).then_some(model), cx);
            }
        });
        state.update(cx, |s, cx| s.focus(window, cx));
        self._subscriptions.push(sub);
        self.custom_model = Some(state);
        cx.notify();
    }

    fn fork(&mut self, cx: &mut Context<Self>) {
        let Some(pane) = self.pane(cx) else { return };
        if let Err(e) = flow::fork(&pane, &self.deck, cx) {
            self.toast(&e, cx);
        }
    }

    // ---- pickers -------------------------------------------------------------------------

    /// Opens, follows or closes the `/` / `@` list from the token before the cursor.
    fn update_picker(&mut self, cx: &mut Context<Self>) {
        let (text, cursor) = {
            let state = self.input.read(cx);
            (state.value(), state.cursor())
        };
        let trigger = trigger_at(&text, cursor);
        if trigger.is_none() {
            self.dismissed = None;
        }
        let Some(trigger) = trigger.filter(|t| self.dismissed != Some(t.start)) else {
            self.picker = None;
            return;
        };
        let fresh = self.picker.as_ref().is_none_or(|p| p.sigil != trigger.sigil || p.start != trigger.start);
        let selected = match &self.picker {
            Some(p) if !fresh && p.query == trigger.query => p.selected,
            _ => 0,
        };
        if fresh {
            self.load_picker_data(trigger.sigil, cx);
        }
        self.picker = Some(Picker { sigil: trigger.sigil, start: trigger.start, query: trigger.query, selected });
    }

    /// Skills once per view; the file list each time `@` opens (the tree changes).
    fn load_picker_data(&mut self, sigil: char, cx: &mut Context<Self>) {
        let Some(pane) = self.pane(cx) else { return };
        let cwd = super::chats()
            .map
            .get(&self.pane_id)
            .and_then(|c| c.launch.as_ref().map(|l| l.cwd.clone()))
            .unwrap_or_else(|| pane.cwd.clone());
        if sigil == '/' && self.skills.is_none() {
            self.skills = Some(Vec::new());
            let agent = pane.agent;
            let task = cx.background_spawn(async move { agentdeck_core::skills::skills_list(agent, cwd) });
            cx.spawn(async move |this, cx| {
                let skills = task.await.unwrap_or_default();
                let _ = this.update(cx, |this, cx| {
                    this.skills = Some(skills.into_iter().map(|s| (s.name, s.description)).collect());
                    cx.notify();
                });
            })
            .detach();
        } else if sigil == '@' {
            let task = cx.background_spawn(async move { agentdeck_core::chat::project_files(Path::new(&cwd), MAX_FILES) });
            cx.spawn(async move |this, cx| {
                let files = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.files = Some(Arc::new(files));
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn choices(&self, agent: AgentKind) -> Vec<Choice> {
        let Some(picker) = &self.picker else { return Vec::new() };
        let q = picker.query.as_str();
        if picker.sigil == '@' {
            let files = self.files.clone().unwrap_or_default();
            let mut scored: Vec<(f64, &String)> = files.iter().filter_map(|f| Some((fuzzy_score(q, f)?, f))).collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            return scored
                .into_iter()
                .take(MAX_CHOICES)
                .map(|(_, f)| Choice { group: "Files", label: f.clone(), detail: String::new(), pick: Pick::Insert(format!("{f} ")) })
                .collect();
        }
        let mut out: Vec<Choice> = commands(agent)
            .into_iter()
            .filter(|(name, _, _)| fuzzy_score(q, name).is_some())
            .map(|(name, detail, pick)| Choice { group: "Commands", label: format!("/{name}"), detail: detail.into(), pick })
            .collect();
        let prefix = if agent == AgentKind::Codex { '$' } else { '/' };
        let mut skills: Vec<(f64, &(String, String))> =
            self.skills.iter().flatten().filter_map(|s| Some((fuzzy_score(q, &s.0)?, s))).collect();
        skills.sort_by(|a, b| b.0.total_cmp(&a.0));
        out.extend(skills.into_iter().take(MAX_CHOICES).map(|(_, (name, detail))| Choice {
            group: "Skills",
            label: format!("{prefix}{name}"),
            detail: detail.clone(),
            pick: Pick::Insert(format!("{prefix}{name} ")),
        }));
        out
    }

    /// Replaces the picker's token (start..cursor) with `text`.
    fn replace_token(&mut self, start: usize, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.input.read(cx).cursor();
        let text = text.to_owned();
        self.input.update(cx, |s, cx| {
            s.set_selected_range(start.min(cursor)..cursor, cx);
            s.replace(text, window, cx);
            s.focus(window, cx);
        });
    }

    /// The highlighted row of an open picker that has rows (Enter / Tab pick it, else they type).
    fn picked_row(&self, cx: &App) -> Option<usize> {
        let selected = self.picker.as_ref()?.selected;
        let agent = self.pane(cx)?.agent;
        (selected < self.choices(agent).len()).then_some(selected)
    }

    fn pick(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.pane(cx) else { return };
        let Some(choice) = self.choices(pane.agent).into_iter().nth(index) else { return };
        let Some(picker) = self.picker.take() else { return };
        match choice.pick {
            Pick::Insert(text) => self.replace_token(picker.start, &text, window, cx),
            Pick::Clear => {
                self.replace_token(picker.start, "", window, cx);
                self.run_clear(&pane, cx);
            }
            Pick::PlanMode => {
                self.replace_token(picker.start, "", window, cx);
                self.toggle_plan(&pane, cx);
            }
        }
        self.picker = None;
        cx.notify();
    }

    /// Up / Down: the picker's row, else the previously sent prompts (empty or recalled composer).
    fn arrow(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.picker.is_some() {
            let choices = self.choices(self.pane(cx).map_or(AgentKind::Claude, |p| p.agent));
            if let Some(picker) = self.picker.as_mut()
                && !choices.is_empty()
            {
                picker.selected = if up { picker.selected.saturating_sub(1) } else { (picker.selected + 1).min(choices.len() - 1) };
                // Rows are preceded by their group titles in the list.
                let mut titles = 0;
                let mut group = "";
                for choice in &choices[..=picker.selected] {
                    if choice.group != group {
                        titles += 1;
                        group = choice.group;
                    }
                }
                self.picker_scroll.scroll_to_item(picker.selected + titles);
                cx.notify();
                return true;
            }
        }
        let sent: Vec<String> = super::chats()
            .map
            .get(&self.pane_id)
            .map(|c| {
                c.transcript
                    .items
                    .iter()
                    .filter_map(|it| match it {
                        Item::User { text, .. } if !text.is_empty() => Some(text.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let current = self.input.read(cx).value().to_string();
        let recalling = self.recall.is_some_and(|i| sent.len() > i && sent[sent.len() - 1 - i] == current);
        if sent.is_empty() || !(current.is_empty() || recalling) {
            self.recall = None;
            return false;
        }
        let next = match (self.recall.filter(|_| recalling), up) {
            (None, true) => Some(0),
            (None, false) => return false,
            (Some(i), true) => Some((i + 1).min(sent.len() - 1)),
            (Some(0), false) => None,
            (Some(i), false) => Some(i - 1),
        };
        let text = next.map_or(String::new(), |i| sent[sent.len() - 1 - i].clone());
        self.recall = next;
        self.input.update(cx, |s, cx| s.set_value(text, window, cx));
        self.picker = None;
        cx.notify();
        true
    }

    // ---- composer actions ----------------------------------------------------------------

    fn attach(&mut self, paths: impl IntoIterator<Item = PathBuf>, cx: &mut Context<Self>) {
        let before = self.images.len();
        for path in paths.into_iter().filter(|p| is_image(p)) {
            let path = path.to_string_lossy().into_owned();
            if !self.images.contains(&path) {
                self.images.push(path);
            }
        }
        if self.images.len() == before {
            self.toast("Only images can be attached (png, jpg, gif, webp, bmp)", cx);
        }
        cx.notify();
    }

    /// "+" button: the system file picker, images only.
    fn pick_images(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn(async move |this, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(e)) => {
                    let _ = this.update(cx, |this, cx| this.toast(&format!("File picker: {e}"), cx));
                    return;
                }
            };
            let _ = this.update(cx, |this, cx| this.attach(paths, cx));
        })
        .detach();
    }

    /// Mic button: same as the dictation shortcut, aimed at this pane.
    fn toggle_dictation(&mut self, cx: &mut Context<Self>) {
        crate::panes::runtime::focus_pane(&self.pane_id, cx);
        let backend = self.deck.read(cx).backend.clone();
        let recording = self.voice == "listening";
        // Off the UI thread: opening the microphone can take a moment.
        cx.background_spawn(async move {
            let result = if recording {
                agentdeck_core::voice::voice_stop(&backend)
            } else {
                agentdeck_core::voice::voice_start(&backend)
            };
            // Failures already reach the user as `voice-error` toasts.
            if let Err(e) = result {
                eprintln!("chat dictation: {e}");
            }
        })
        .detach();
    }

    fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = match &self.find {
            Some(state) => state.clone(),
            None => {
                let state = cx.new(|cx| InputState::new(window, cx).placeholder("Find in chat"));
                let sub = cx.subscribe(&state, |_, _, event: &InputEvent, cx| {
                    if let InputEvent::Change = event {
                        cx.notify();
                    }
                });
                self._subscriptions.push(sub);
                self.find = Some(state.clone());
                state
            }
        };
        state.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    fn find_query(&self, cx: &App) -> String {
        self.find.as_ref().map(|f| f.read(cx).value().trim().to_lowercase()).unwrap_or_default()
    }

    // ---- transcript rows -----------------------------------------------------------------

    fn tool_row(&self, tool: &Tool, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        if let Some(plan) = &tool.plan {
            return plan_card(plan, p).into_any_element();
        }
        let open = self.open.contains(&tool.id);
        let id = tool.id.clone();
        let label = ToolLabel::of(tool);
        let status: AnyElement = match (&tool.output, tool.error) {
            (None, _) => Spinner::new().icon(gpui_kit::assets::IconName::LoaderCircle).xsmall().color(hsla(p.muted)).into_any_element(),
            (Some(_), true) => Icon::new(IconName::X).size(ui(12.0)).text_color(hsla(p.bad)).into_any_element(),
            (Some(_), false) => div().into_any_element(),
        };
        let toggle = cx.listener(move |this, _: &ClickEvent, _, cx| {
            if !this.open.remove(&id) {
                this.open.insert(id.clone());
            }
            cx.notify();
        });
        let output = open.then(|| {
            let output = tool.output.clone().unwrap_or_else(|| "Running…".into());
            div()
                .id(SharedString::from(format!("tool-out-{}", tool.id)))
                .max_h(ui(240.0))
                .overflow_y_scroll()
                .px_3()
                .py_2()
                .rounded(ui(RADIUS_MD))
                .bg(hsla(p.panel))
                .border_1()
                .border_color(hsla(p.line))
                .font_family(FONT_MONO)
                .text_xs()
                .text_color(hsla(if tool.error { p.bad } else { p.muted }))
                .whitespace_normal()
                .child(if output.is_empty() { "(no output)".to_owned() } else { output })
        });
        // One line height for the verb (sans) and the target (mono, smaller): their text lines up.
        let header = div()
            .id(SharedString::from(format!("tool-{}", tool.id)))
            .flex()
            .items_center()
            .gap_2()
            .min_h(ui(24.0))
            .line_height(ui(18.0))
            .min_w_0()
            .cursor_pointer()
            .on_click(toggle);
        if label.worker {
            // Orchestrator calls: a card, like a message from a teammate.
            return div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    header
                        .px_3()
                        .py_2()
                        .rounded(ui(RADIUS_LG))
                        .border_1()
                        .border_color(hsla(p.line))
                        .hover(|s| s.bg(hsla(p.hover)))
                        .child(Icon::new(IconName::Bot).size(ui(16.0)).text_color(hsla(p.accent)))
                        .child(div().flex_shrink_0().text_color(hsla(p.muted)).child(label.verb))
                        .child(div().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(label.target))
                        .child(Icon::new(IconName::ArrowUpRight).size(ui(12.0)).text_color(hsla(p.muted)))
                        .child(div().flex_1())
                        .child(status)
                        .child(chevron(open, p)),
                )
                .children(output)
                .into_any_element();
        }
        let edit_counts = (!tool.edits.is_empty()).then(|| {
            let (added, removed) = tool.edits.iter().fold((0, 0), |(a, r), e| (a + e.added, r + e.removed));
            counts(added, removed, p)
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                header
                    .text_color(hsla(p.muted))
                    .hover(|s| s.text_color(hsla(p.fg)))
                    .child(Icon::new(label.icon).size(ui(14.0)).flex_shrink_0())
                    .child(div().flex_shrink_0().child(label.verb))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(FONT_MONO)
                            .text_size(ui(12.0))
                            .line_height(ui(18.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(hsla(p.fg))
                            .child(label.target),
                    )
                    .children(edit_counts)
                    .child(div().flex_1())
                    .child(status),
            )
            .children(output)
            .into_any_element()
    }

    fn user_bubble(&self, index: usize, text: &str, images: &[String], p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let group = SharedString::from(format!("user-{}-{index}", self.pane_id));
        let open = self.users_open.contains(&index);
        let folded = fold_text(text).filter(|_| !open);
        let long = fold_text(text).is_some();
        let shown = folded.clone().unwrap_or_else(|| text.to_owned());
        let copy_text = text.to_owned();
        div()
            .group(group.clone())
            .flex()
            .justify_end()
            .items_start()
            .gap_1()
            .when(!text.is_empty(), |el| {
                el.child(
                    small_icon_button(SharedString::from(format!("copy-user-{index}")), IconName::Copy, "Copy", p)
                        .opacity(0.0)
                        .group_hover(group.clone(), |s| s.opacity(1.0))
                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()))),
                )
            })
            .child(
                div()
                    .max_w(relative(0.85))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(ui(RADIUS_COMPOSER))
                    .border_1()
                    .border_color(hsla(p.line))
                    .bg(hsla(p.panel))
                    .when(!images.is_empty(), |el| {
                        el.child(div().flex().flex_wrap().gap_1().children(images.iter().map(|path| thumb(path, THUMB))))
                    })
                    .when(!text.is_empty(), |el| el.child(div().whitespace_normal().child(shown)))
                    .when(long, |el| {
                        el.child(
                            div()
                                .id(SharedString::from(format!("user-more-{index}")))
                                .text_xs()
                                .text_color(hsla(p.accent))
                                .cursor_pointer()
                                .hover(|s| s.text_color(hsla(p.accent_hover)))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if !this.users_open.remove(&index) {
                                        this.users_open.insert(index);
                                    }
                                    cx.notify();
                                }))
                                .child(if open { "Show less" } else { "Show more" }),
                        )
                    }),
            )
            .into_any_element()
    }

    fn answer(&self, index: usize, text: &str, p: &Palette) -> AnyElement {
        let muted = hsla(p.muted);
        let hover = hsla(p.hover);
        div()
            .child(
                TextView::markdown(SharedString::from(format!("chat-{}-{index}", self.pane_id)), text.to_owned())
                    .selectable(true)
                    .code_block_actions(move |block, _, _| {
                        let code = block.code();
                        div()
                            .id("copy-code")
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_1()
                            .rounded(ui(RADIUS_MD))
                            .text_xs()
                            .text_color(muted)
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(code.to_string())))
                            .child(Icon::new(IconName::Copy).size(ui(12.0)))
                            .child("Copy")
                    }),
            )
            .into_any_element()
    }

    fn item(&self, index: usize, item: &Item, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        match item {
            Item::User { text, images } => self.user_bubble(index, text, images, p, cx),
            Item::Assistant(text) => self.answer(index, text, p),
            Item::Tool(tool) => self.tool_row(tool, p, cx),
            Item::Notice(text) => div().text_xs().text_color(hsla(p.subtle)).child(text.clone()).into_any_element(),
            Item::Error(text) => div()
                .flex()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(ui(RADIUS_MD))
                .bg(hsla_a(p.bad, 0.12))
                .text_color(hsla(p.bad))
                .child(Icon::new(IconName::CircleAlert).size(ui(14.0)).mt(ui(2.0)))
                .child(div().flex_1().min_w_0().whitespace_normal().child(text.clone()))
                .into_any_element(),
            Item::Done { .. } => div().into_any_element(),
        }
    }

    /// The last answer of a finished conversation, with "Fork from here" on hover.
    fn last_answer(&self, index: usize, text: &str, agent: AgentKind, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let group = SharedString::from(format!("answer-{}", self.pane_id));
        let claude = agent == AgentKind::Claude;
        let tip = if claude { "New chat pane continuing a copy of this conversation" } else { "Fork needs Claude" };
        div()
            .group(group.clone())
            .flex()
            .flex_col()
            .gap_1()
            .child(self.answer(index, text, p))
            .child(
                div().flex().child(
                    div()
                        .id("fork-answer")
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py(ui(2.0))
                        .rounded(ui(RADIUS_MD))
                        .text_xs()
                        .text_color(hsla(p.muted))
                        .opacity(0.0)
                        .group_hover(group, |s| s.opacity(1.0))
                        .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                        .when(claude, |el| {
                            el.cursor_pointer()
                                .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.fork(cx)))
                        })
                        .child(Icon::new(IconName::GitFork).size(ui(12.0)))
                        .child("Fork from here"),
                ),
            )
            .into_any_element()
    }

    /// "Worked for 4s ›" / "Working for 5s", then "· 3 files +42 −7" (click: the file list).
    #[allow(clippy::too_many_arguments)]
    fn turn_header(
        &self,
        user_index: usize,
        label: String,
        foldable: bool,
        open: bool,
        changes: &[FileEdit],
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let files_open = self.files_open.contains(&user_index);
        let (added, removed) = changes.iter().fold((0, 0), |(a, r), e| (a + e.added, r + e.removed));
        let files = (!changes.is_empty()).then(|| {
            div()
                .id(SharedString::from(format!("turn-files-{}-{user_index}", self.pane_id)))
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .hover(|s| s.text_color(hsla(p.fg)))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if !this.files_open.remove(&user_index) {
                        this.files_open.insert(user_index);
                    }
                    cx.notify();
                }))
                .child("·")
                .child(format!("{} file{}", changes.len(), if changes.len() == 1 { "" } else { "s" }))
                .child(counts(added, removed, p))
                .child(chevron(files_open, p))
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pb_1()
                    .border_b_1()
                    .border_color(hsla(p.line))
                    .text_color(hsla(p.muted))
                    .child(
                        div()
                            .id(SharedString::from(format!("turn-{}-{user_index}", self.pane_id)))
                            .flex()
                            .items_center()
                            .gap_1()
                            .when(foldable, |el| {
                                el.cursor_pointer()
                                    .hover(|s| s.text_color(hsla(p.fg)))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        if !this.unfolded.remove(&user_index) {
                                            this.unfolded.insert(user_index);
                                        }
                                        cx.notify();
                                    }))
                            })
                            .child(label)
                            .when(foldable, |el| el.child(chevron(open, p))),
                    )
                    .children(files),
            )
            .when(files_open, |el| el.child(self.changed_files(user_index, changes, p, cx)))
            .into_any_element()
    }

    /// A finished turn's changed files; a click shows a file's patch.
    fn changed_files(&self, user_index: usize, changes: &[FileEdit], p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let rows = changes.iter().enumerate().map(|(n, edit)| {
            let key = format!("{user_index}:{}", edit.path);
            let open = self.patches_open.contains(&key);
            let toggle = key.clone();
            let patch = open.then(|| {
                let lines: Vec<AnyElement> = edit
                    .diff
                    .lines()
                    .map(|line| {
                        let (color, bg) = match line.chars().next() {
                            Some('+') => (p.ok, Some(hsla_a(p.ok, 0.12))),
                            Some('-') => (p.bad, Some(hsla_a(p.bad, 0.12))),
                            Some('@') | Some('…') => (p.subtle, None),
                            _ => (p.muted, None),
                        };
                        div()
                            .px_2()
                            .whitespace_nowrap()
                            .text_color(hsla(color))
                            .when_some(bg, |el, bg| el.bg(bg))
                            .child(if line.is_empty() { " ".to_owned() } else { line.to_owned() })
                            .into_any_element()
                    })
                    .collect();
                div()
                    .id(SharedString::from(format!("patch-{}-{user_index}-{n}", self.pane_id)))
                    .max_h(ui(320.0))
                    .overflow_scroll()
                    .py_1()
                    .rounded(ui(RADIUS_MD))
                    .bg(hsla(p.panel))
                    .border_1()
                    .border_color(hsla(p.line))
                    .font_family(FONT_MONO)
                    .text_xs()
                    .when(lines.is_empty(), |el| el.child(div().px_2().text_color(hsla(p.subtle)).child("No patch to show")))
                    .children(lines)
            });
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .id(SharedString::from(format!("file-{}-{user_index}-{n}", self.pane_id)))
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w_0()
                        .cursor_pointer()
                        .text_color(hsla(p.muted))
                        .hover(|s| s.text_color(hsla(p.fg)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if !this.patches_open.remove(&toggle) {
                                this.patches_open.insert(toggle.clone());
                            }
                            cx.notify();
                        }))
                        .child(Icon::new(IconName::FilePen).size(ui(13.0)))
                        .child(div().min_w_0().truncate().font_family(FONT_MONO).text_xs().text_color(hsla(p.fg)).child(edit.path.clone()))
                        .child(counts(edit.added, edit.removed, p))
                        .child(div().flex_1())
                        .child(chevron(open, p)),
                )
                .children(patch)
        });
        div().flex().flex_col().gap_1().pl_2().children(rows).into_any_element()
    }

    /// The message list: items grouped by turn. A finished turn folds its steps under its header and
    /// keeps its answer (last assistant text) and errors in view. With a find query: matching rows only.
    fn list(&self, transcript: &Transcript, agent: AgentKind, query: &str, p: &Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let items = &transcript.items;
        if !query.is_empty() {
            return items
                .iter()
                .enumerate()
                .filter(|(_, it)| matches_query(it, query))
                .map(|(n, it)| self.item(n, it, p, cx))
                .collect();
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i < items.len() {
            if !matches!(items[i], Item::User { .. }) {
                out.push(self.item(i, &items[i], p, cx));
                i += 1;
                continue;
            }
            let user = i;
            out.push(self.item(i, &items[i], p, cx));
            let end = items[i + 1..].iter().position(|it| matches!(it, Item::User { .. })).map_or(items.len(), |n| i + 1 + n);
            let turn = &items[user + 1..end];
            let done = turn.iter().find_map(|it| match it {
                Item::Done { secs } => Some(*secs),
                _ => None,
            });
            let has_steps = turn.iter().any(|it| matches!(it, Item::Tool(_)));
            match done {
                Some(secs) => {
                    let answer = turn.iter().rposition(|it| matches!(it, Item::Assistant(_)));
                    let open = self.unfolded.contains(&user);
                    let foldable = has_steps || turn.iter().filter(|it| matches!(it, Item::Assistant(_))).count() > 1;
                    let changes = turn_changes(turn);
                    if foldable || !changes.is_empty() {
                        out.push(self.turn_header(user, format!("Worked for {}", duration(secs)), foldable, open, &changes, p, cx));
                    }
                    let last_turn = end == items.len() && !transcript.busy;
                    for (n, it) in turn.iter().enumerate() {
                        let keep = open || !foldable || Some(n) == answer || matches!(it, Item::Error(_));
                        match it {
                            Item::Assistant(text) if last_turn && Some(n) == answer => {
                                out.push(self.last_answer(user + 1 + n, text, agent, p, cx));
                            }
                            _ if keep => out.push(self.item(user + 1 + n, it, p, cx)),
                            _ => {}
                        }
                    }
                }
                None => {
                    let running = transcript.busy && end == items.len();
                    if running && let Some(started) = transcript.started {
                        let secs = started.elapsed().as_secs();
                        out.push(self.turn_header(user, format!("Working for {}", duration(secs)), false, true, &[], p, cx));
                    }
                    out.extend(turn.iter().enumerate().map(|(n, it)| self.item(user + 1 + n, it, p, cx)));
                }
            }
            i = end;
        }
        out
    }

    // ---- above the composer --------------------------------------------------------------

    /// The latest plan, folded to its progress ("Plan · 3/5 done") and current step.
    fn plan_panel(&self, transcript: &Transcript, p: &Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let plan = latest_plan(&transcript.items)?;
        let done = plan.iter().filter(|s| s.status == StepStatus::Completed).count();
        if plan.is_empty() || (done == plan.len() && !transcript.busy) {
            return None;
        }
        let current = plan.iter().find(|s| s.status == StepStatus::InProgress).or_else(|| plan.iter().find(|s| s.status == StepStatus::Pending));
        let open = self.plan_open;
        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_2()
                .rounded(ui(RADIUS_LG))
                .border_1()
                .border_color(hsla(p.line))
                .bg(hsla(p.panel))
                .child(
                    div()
                        .id(SharedString::from(format!("plan-{}", self.pane_id)))
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w_0()
                        .cursor_pointer()
                        .text_color(hsla(p.muted))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.plan_open = !this.plan_open;
                            cx.notify();
                        }))
                        .child(Icon::new(IconName::ListTodo).size(ui(14.0)))
                        .child(div().flex_shrink_0().child(format!("Plan · {done}/{} done", plan.len())))
                        .when_some(current.filter(|_| !open), |el, step| {
                            el.child(div().min_w_0().truncate().text_color(hsla(p.fg)).child(step.text.clone()))
                        })
                        .child(div().flex_1())
                        .child(chevron(open, p)),
                )
                .when(open, |el| el.children(plan.iter().map(|s| plan_step(s, p))))
                .into_any_element(),
        )
    }

    /// Follow-ups waiting for the turn to end: edit, raise, steer now, delete; "Resume" when paused.
    fn queue_panel(&self, queue: &[Queued], paused: bool, p: &Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        if queue.is_empty() {
            return None;
        }
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(hsla(p.muted))
            .child(format!("Queued · {}", queue.len()))
            .when(paused, |el| {
                el.child(div().text_color(hsla(p.wait)).child("Paused")).child(div().flex_1()).child(
                    div()
                        .id("queue-resume")
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py(ui(2.0))
                        .rounded(ui(RADIUS_MD))
                        .bg(hsla(p.fg))
                        .text_color(hsla(p.base))
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.85))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            if let Some(pane) = this.pane(cx) {
                                flow::resume_queue(&pane, cx);
                            }
                        }))
                        .child(Icon::new(IconName::Play).size(ui(11.0)))
                        .child("Resume"),
                )
            });
        let cards = queue.iter().enumerate().map(|(n, q)| {
            let id = q.id;
            let editing = self.editing == Some(id);
            let mut card = div()
                .flex()
                .items_start()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(ui(RADIUS_LG))
                .border_1()
                .border_color(hsla(if editing { p.accent } else { p.line }))
                .bg(hsla(p.panel));
            if editing {
                card = card.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                            this.editing = None;
                            cx.stop_propagation();
                            cx.notify();
                        }))
                        .child(Textarea::new(&self.edit_input).appearance(false).aria_label("Edit queued message")),
                );
            } else {
                let first = q.prompt.text.lines().next().unwrap_or("").to_owned();
                let images = q.prompt.images.len();
                card = card
                    .child(div().flex_1().min_w_0().truncate().child(if first.is_empty() { "(images)".to_owned() } else { first }))
                    .when(images > 0, |el| el.child(div().text_xs().text_color(hsla(p.muted)).child(format!("+{images} image(s)"))));
            }
            let text = q.prompt.text.clone();
            card.child(
                small_icon_button(SharedString::from(format!("q-edit-{id}")), IconName::Pencil, "Edit", p).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.editing = Some(id);
                        let text = text.clone();
                        this.edit_input.update(cx, |s, cx| {
                            s.set_value(text, window, cx);
                            s.focus(window, cx);
                        });
                        cx.notify();
                    },
                )),
            )
            .when(n > 0, |el| {
                el.child(
                    small_icon_button(SharedString::from(format!("q-up-{id}")), IconName::ChevronUp, "Send sooner", p)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, _| flow::raise_queued(&this.pane_id, id))),
                )
            })
            .child(
                small_icon_button(SharedString::from(format!("q-steer-{id}")), IconName::CornerDownLeft, "Steer: send into the running turn now", p)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(pane) = this.pane(cx)
                            && let Some(prompt) = flow::unqueue(&this.pane_id, id)
                        {
                            flow::steer(&pane, &prompt.text, &prompt.images, cx);
                        }
                    })),
            )
            .child(
                small_icon_button(SharedString::from(format!("q-del-{id}")), IconName::X, "Remove", p).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        flow::unqueue(&this.pane_id, id);
                        if this.editing == Some(id) {
                            this.editing = None;
                        }
                        cx.notify();
                    },
                )),
            )
        });
        Some(div().flex().flex_col().gap_1().child(header).children(cards).into_any_element())
    }

    /// The `/` or `@` list, grouped, above the message box.
    fn picker_list(&self, agent: AgentKind, p: &Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let picker = self.picker.as_ref()?;
        let choices = self.choices(agent);
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut group = "";
        for (n, choice) in choices.iter().enumerate() {
            if choice.group != group {
                group = choice.group;
                rows.push(div().px_3().pt_2().pb_1().text_xs().text_color(hsla(p.subtle)).child(group).into_any_element());
            }
            let selected = n == picker.selected;
            rows.push(
                div()
                    .id(SharedString::from(format!("pick-{n}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .when(selected, |el| el.bg(hsla(p.hover)))
                    .hover(|s| s.bg(hsla(p.hover)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.pick(n, window, cx)))
                    .child(div().flex_shrink_0().font_family(FONT_MONO).text_xs().child(choice.label.clone()))
                    .child(div().min_w_0().truncate().text_xs().text_color(hsla(p.muted)).child(choice.detail.clone()))
                    .into_any_element(),
            );
        }
        if rows.is_empty() {
            let empty = match picker.sigil {
                '@' if self.files.is_none() => "Listing files…",
                '@' => "No matching file",
                _ => "No matching command or skill",
            };
            rows.push(div().px_3().py_2().text_xs().text_color(hsla(p.subtle)).child(empty).into_any_element());
        }
        Some(
            div()
                .id(SharedString::from(format!("picker-{}", self.pane_id)))
                .max_h(ui(240.0))
                .overflow_y_scroll()
                .track_scroll(&self.picker_scroll)
                .py_1()
                .rounded(ui(RADIUS_LG))
                .border_1()
                .border_color(hsla(p.line))
                .bg(hsla(p.raised))
                .shadow_lg()
                .children(rows)
                .into_any_element(),
        )
    }

    fn mode_pill(&self, pane: &Pane, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let agent = pane.agent;
        let current = Mode::of(agent, &pane.args);
        let weak = cx.entity().downgrade();
        Button::new("chat-mode")
            .ghost()
            .small()
            .icon(IconName::Shield)
            .label(current.label(agent))
            .text_color(hsla(if current == Mode::Full { p.wait } else { p.muted }))
            .tooltip("What the agent may do without asking (applies to the next turn)")
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, _| {
                for &mode in Mode::all(agent) {
                    let weak = weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(mode.label(agent))
                            .checked(mode == current)
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| this.set_mode(mode, cx));
                            }),
                    );
                }
                menu
            })
    }

    fn model_pill(&self, pane: &Pane, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let agent = pane.agent;
        let current = model_arg(&pane.args);
        let mut options: Vec<String> = match agent {
            AgentKind::Claude => CLAUDE_MODELS.map(String::from).to_vec(),
            _ => codex_config_model().into_iter().collect(),
        };
        if let Some(model) = current.as_ref().filter(|m| !options.contains(m)) {
            options.push(model.clone());
        }
        let label = current.clone().unwrap_or_else(|| "Default model".into());
        let weak = cx.entity().downgrade();
        Button::new("chat-model")
            .ghost()
            .small()
            .icon(IconName::Cpu)
            .label(label)
            .tooltip(format!("{} model (applies to the next turn)", agent_name(agent)))
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, _| {
                let set = |model: Option<String>| {
                    let weak = weak.clone();
                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        let _ = weak.update(cx, |this, cx| this.set_model(model.clone(), cx));
                    }
                };
                menu = menu.item(PopupMenuItem::new("Default").checked(current.is_none()).on_click(set(None)));
                for model in &options {
                    menu = menu.item(
                        PopupMenuItem::new(model.clone())
                            .checked(current.as_ref() == Some(model))
                            .on_click(set(Some(model.clone()))),
                    );
                }
                let weak = weak.clone();
                menu.separator().item(PopupMenuItem::new("Other model…").on_click(move |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| this.open_custom_model(window, cx));
                }))
            })
    }

    fn composer(&self, transcript: &Transcript, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let busy = transcript.busy;
        let pane = self.pane(cx);
        let agent = pane.as_ref().map_or(AgentKind::Claude, |p| p.agent);
        let typed = !self.input.read(cx).value().trim().is_empty() || !self.images.is_empty();
        let pane_id = self.pane_id.clone();
        let weak = cx.entity().downgrade();
        let (queue, paused) = flow::queued(&self.pane_id);
        let action = if busy && !typed {
            round_button("chat-stop", "Stop", p)
                .on_click(move |_, _, _| super::interrupt(&pane_id))
                .child(Icon::new(IconName::Square).size(ui(11.0)))
        } else {
            let tip = if busy { "Queue (Enter) · Steer now (Ctrl+Enter)" } else { "Send (Enter)" };
            round_button("chat-send", tip, p)
                .when(!typed, |el| el.opacity(0.35))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.submit(false, window, cx)))
                .child(Icon::new(IconName::ArrowUp).size(ui(15.0)))
        };
        let listening = self.voice == "listening";
        let transcribing = self.voice == "transcribing";
        let mic = ghost_button("chat-mic", if listening { "Stop dictation (Ctrl+Space)" } else { "Dictate (Ctrl+Space)" }, p)
            .when(listening, |el| el.bg(hsla_a(p.bad, 0.18)).text_color(hsla(p.bad)))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_dictation(cx)))
            .child(if transcribing {
                Spinner::new().icon(gpui_kit::assets::IconName::LoaderCircle).xsmall().color(hsla(p.muted)).into_any_element()
            } else {
                Icon::new(IconName::Mic).size(ui(15.0)).into_any_element()
            });
        let pills = pane.as_ref().map(|pane| (self.mode_pill(pane, p, cx), self.model_pill(pane, cx)));
        let custom_model = self.custom_model.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_3()
            .pb_3()
            .pt_1()
            .children(self.plan_panel(transcript, p, cx))
            .children(self.queue_panel(&queue, paused, p, cx))
            .children(self.picker_list(agent, p, cx))
            .child(
                div()
                    .id(SharedString::from(format!("composer-{}", self.pane_id)))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_3()
                    .pt_2()
                    .pb_2()
                    .rounded(ui(RADIUS_COMPOSER))
                    .border_1()
                    .border_color(hsla(p.line))
                    .bg(hsla(p.panel))
                    .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                        if this.arrow(true, window, cx) {
                            cx.stop_propagation();
                        }
                    }))
                    .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                        if this.arrow(false, window, cx) {
                            cx.stop_propagation();
                        }
                    }))
                    .capture_action(cx.listener(|this, action: &Enter, window, cx| {
                        if !action.secondary
                            && !action.shift
                            && let Some(selected) = this.picked_row(cx)
                        {
                            this.pick(selected, window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                        if let Some(selected) = this.picked_row(cx) {
                            this.pick(selected, window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                        if let Some(picker) = this.picker.take() {
                            this.dismissed = Some(picker.start);
                            cx.stop_propagation();
                        } else if this.custom_model.take().is_some() {
                            cx.stop_propagation();
                        }
                        cx.notify();
                    }))
                    .drag_over::<ExternalPaths>({
                        let accent = hsla(p.accent);
                        move |s, _, _, _| s.border_color(accent)
                    })
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.attach(paths.paths().iter().cloned(), cx)))
                    .when(!self.images.is_empty(), |el| {
                        el.child(div().flex().flex_wrap().gap_2().pb_1().children(self.images.iter().enumerate().map(|(n, path)| {
                            div()
                                .relative()
                                .child(thumb(path, THUMB_SMALL))
                                .child(
                                    div()
                                        .id(SharedString::from(format!("unattach-{n}")))
                                        .absolute()
                                        .top(ui(-4.0))
                                        .right(ui(-4.0))
                                        .size(ui(16.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(hsla(p.fg))
                                        .text_color(hsla(p.base))
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            if n < this.images.len() {
                                                this.images.remove(n);
                                            }
                                            cx.notify();
                                        }))
                                        .child(Icon::new(IconName::X).size(ui(10.0))),
                                )
                        })))
                    })
                    .when_some(custom_model, |el, state| {
                        el.child(div().flex().items_center().gap_2().text_xs().child(Icon::new(IconName::Cpu).size(ui(13.0))).child(div().flex_1().child(Input::new(&state))))
                    })
                    .child(Textarea::new(&self.input).appearance(false).aria_label("Message").on_paste(move |item, _, cx| {
                        let images: Vec<String> = item
                            .entries()
                            .iter()
                            .filter_map(|entry| match entry {
                                ClipboardEntry::Image(image) => clip_ext(image.format).map(|ext| save_paste_image(&image.bytes, ext)),
                                _ => None,
                            })
                            .filter_map(|saved| saved.inspect_err(|e| eprintln!("chat paste: {e}")).ok())
                            .collect();
                        let files: Vec<PathBuf> = item
                            .entries()
                            .iter()
                            .filter_map(|entry| match entry {
                                ClipboardEntry::ExternalPaths(paths) => Some(paths.paths().to_vec()),
                                _ => None,
                            })
                            .flatten()
                            .filter(|p| is_image(p))
                            .collect();
                        if images.is_empty() && files.is_empty() {
                            return false; // plain text: the textarea inserts it
                        }
                        let _ = weak.update(cx, |this, cx| this.attach(images.into_iter().map(PathBuf::from).chain(files), cx));
                        true
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                ghost_button("chat-attach", "Attach images", p)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.pick_images(cx)))
                                    .child(Icon::new(IconName::Plus).size(ui(16.0))),
                            )
                            .when_some(pills, |el, (mode, model)| el.child(mode).child(model))
                            .child(div().flex_1())
                            .child(mic)
                            .child(action),
                    ),
            )
            .into_any_element()
    }

    fn find_bar(&self, matches: Option<usize>, p: &Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.find.clone()?;
        Some(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .border_b_1()
                .border_color(hsla(p.line))
                .bg(hsla(p.panel))
                .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                    this.find = None;
                    cx.stop_propagation();
                    cx.notify();
                }))
                .child(Icon::new(IconName::Search).size(ui(14.0)).text_color(hsla(p.muted)))
                .child(div().flex_1().child(Input::new(&state)))
                .when_some(matches, |el, n| {
                    el.child(div().text_xs().text_color(hsla(p.muted)).child(format!("{n} match{}", if n == 1 { "" } else { "es" })))
                })
                .child(
                    small_icon_button("find-close", IconName::X, "Close (Esc)", p).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.find = None;
                        cx.notify();
                    })),
                )
                .into_any_element(),
        )
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        if !self.dictated.is_empty() {
            let text = std::mem::take(&mut self.dictated);
            self.input.update(cx, |s, cx| {
                s.insert(text, window, cx);
                s.focus(window, cx);
            });
        }
        let transcript = super::transcript(&self.pane_id);
        let pane = self.pane(cx);
        let exited = pane.as_ref().is_none_or(|p| p.exited);
        let agent = pane.as_ref().map_or(AgentKind::Claude, |p| p.agent);
        let query = self.find_query(cx);
        // New items, or streamed text growing the last one: follow the bottom (not while finding).
        if query.is_empty() && (transcript.items.len() != self.seen || transcript.busy) {
            self.seen = transcript.items.len();
            self.scroll.scroll_to_bottom();
        }
        // The elapsed time ticks while a turn runs.
        self._tick = transcript.busy.then(|| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
        });
        let items = self.list(&transcript, agent, &query, &p, cx);
        let empty = items.is_empty();
        let find_bar = self.find_bar((!query.is_empty()).then_some(items.len()), &p, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(hsla(p.base))
            .text_color(hsla(p.fg))
            .text_size(ui(13.0))
            .capture_action(cx.listener(|this, _: &Search, window, cx| {
                this.open_find(window, cx);
                cx.stop_propagation();
            }))
            .children(find_bar)
            .child(
                div()
                    .id(SharedString::from(format!("chat-list-{}", self.pane_id)))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .p_4()
                            .when(empty && query.is_empty(), |el| {
                                el.child(div().text_color(hsla(p.subtle)).child(if exited {
                                    "The agent is stopped. Send a message to start it again."
                                } else {
                                    "Ask the agent anything."
                                }))
                            })
                            .children(items)
                            .when(transcript.busy && query.is_empty(), |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .text_color(hsla(p.muted))
                                        .child(Spinner::new().icon(gpui_kit::assets::IconName::LoaderCircle).small().color(hsla(p.muted)))
                                        .child("Working…"),
                                )
                            }),
                    ),
            )
            .child(self.composer(&transcript, &p, cx))
    }
}

/// The chat commands of the `/` list.
fn commands(agent: AgentKind) -> Vec<(&'static str, &'static str, Pick)> {
    let mut out = vec![("clear", "Fresh conversation in this pane", Pick::Clear)];
    if agent == AgentKind::Claude {
        out.push(("compact", "Summarize the conversation to free context", Pick::Insert("/compact ".into())));
        out.push(("plan", "Toggle plan mode (read and plan, no edits)", Pick::PlanMode));
    }
    out
}

/// Codex's configured model, read once.
fn codex_config_model() -> Option<String> {
    static MODEL: OnceLock<Option<String>> = OnceLock::new();
    MODEL.get_or_init(agentdeck_core::chat::codex_default_model).clone()
}

fn matches_query(item: &Item, query: &str) -> bool {
    let has = |text: &str| text.to_lowercase().contains(query);
    match item {
        Item::User { text, .. } | Item::Assistant(text) | Item::Notice(text) | Item::Error(text) => has(text),
        Item::Tool(t) => has(&t.name) || has(&t.input) || t.output.as_deref().is_some_and(has),
        Item::Done { .. } => false,
    }
}

/// The start of a long user message (None when it is short enough to show whole).
fn fold_text(text: &str) -> Option<String> {
    let lines = text.lines().count();
    if lines <= FOLD_LINES && text.chars().count() <= FOLD_CHARS {
        return None;
    }
    let head: String = text.lines().take(FOLD_LINES).collect::<Vec<_>>().join("\n");
    let head: String = head.chars().take(FOLD_CHARS).collect();
    Some(format!("{}…", head.trim_end()))
}

fn plan_card(plan: &[PlanStep], p: &Palette) -> impl IntoElement {
    let done = plan.iter().filter(|s| s.status == StepStatus::Completed).count();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_color(hsla(p.muted))
                .child(Icon::new(IconName::ListTodo).size(ui(14.0)))
                .child(format!("Updated the plan · {done}/{} done", plan.len())),
        )
        .child(div().flex().flex_col().gap_1().pl_1().children(plan.iter().map(|s| plan_step(s, p))))
}

fn plan_step(step: &PlanStep, p: &Palette) -> impl IntoElement {
    let (icon, color, text) = match step.status {
        StepStatus::Completed => (IconName::CircleCheck, p.ok, p.muted),
        StepStatus::InProgress => (IconName::CircleDot, p.accent, p.fg),
        StepStatus::Pending => (IconName::Circle, p.subtle, p.muted),
    };
    div()
        .flex()
        .items_start()
        .gap_2()
        .child(Icon::new(icon).size(ui(13.0)).mt(ui(2.0)).text_color(hsla(color)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_normal()
                .text_color(hsla(text))
                .when(step.status == StepStatus::InProgress, |el| el.font_weight(FontWeight::SEMIBOLD))
                .child(step.text.clone()),
        )
}

/// "+12 −3" in green and red.
fn counts(added: u32, removed: u32, p: &Palette) -> impl IntoElement {
    div()
        .flex()
        .flex_shrink_0()
        .gap_1()
        .font_family(FONT_MONO)
        .text_xs()
        .child(div().text_color(hsla(p.ok)).child(format!("+{added}")))
        .child(div().text_color(hsla(p.bad)).child(format!("−{removed}")))
}

/// How a tool call reads in the list: "Read  main.rs", "Ran  cargo test", worker cards.
struct ToolLabel {
    icon: IconName,
    verb: String,
    target: String,
    /// An orchestrator call about a worker: drawn as a card.
    worker: bool,
}

impl ToolLabel {
    fn of(tool: &Tool) -> Self {
        // MCP tools: `mcp__server__tool` (Claude) or `server.tool` (Codex).
        let short = tool.name.rsplit("__").next().unwrap_or(&tool.name);
        let short = short.rsplit('.').next().unwrap_or(short);
        let file = || file_name(&tool.input);
        let input = || tool.input.clone();
        let (icon, verb, target, worker) = match short {
            "Read" | "read_file" => (IconName::FileText, "Read", file(), false),
            "Edit" | "MultiEdit" | "Write" | "NotebookEdit" | "edit" | "apply_patch" => (IconName::FilePen, "Edited", file(), false),
            "Bash" | "shell" | "PowerShell" | "BashOutput" => (IconName::Terminal, "Ran", input(), false),
            "Grep" | "Glob" | "search" => (IconName::Search, "Searched", input(), false),
            "WebFetch" => (IconName::Globe, "Fetched", input(), false),
            "WebSearch" | "web_search" => (IconName::Globe, "Searched the web", input(), false),
            "Task" | "Agent" => (IconName::Bot, "Agent", input(), false),
            "TodoWrite" | "update_plan" => (IconName::ListTodo, "Updated the plan", String::new(), false),
            "spawn_agent" => (IconName::Bot, "Started worker", input(), true),
            "send_prompt" | "queue_prompt" => (IconName::Bot, "Message to worker", input(), true),
            "wait_for" => (IconName::Clock, "Waited for workers", String::new(), false),
            _ if short != tool.name => (IconName::Plug, short, input(), false),
            _ => (IconName::Wrench, short, input(), false),
        };
        Self { icon, verb: verb.to_owned(), target, worker }
    }
}

/// Last component of a path ("src/a.rs" → "a.rs"); the input as is when it is not a path.
fn file_name(input: &str) -> String {
    Path::new(input).file_name().map_or_else(|| input.to_owned(), |n| n.to_string_lossy().into_owned())
}

fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

fn clip_ext(format: ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Png => Some("png"),
        ImageFormat::Jpeg => Some("jpg"),
        ImageFormat::Gif => Some("gif"),
        ImageFormat::Webp => Some("webp"),
        ImageFormat::Bmp => Some("bmp"),
        _ => None,
    }
}

/// "4s", "2m 05s", "1h 03m".
fn duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

fn thumb(path: &str, side: f32) -> impl IntoElement {
    img(PathBuf::from(path)).size(ui(side)).rounded(ui(RADIUS_MD)).object_fit(ObjectFit::Cover)
}

fn chevron(open: bool, p: &Palette) -> impl IntoElement {
    Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(ui(12.0)).text_color(hsla(p.muted))
}

/// Round button in the text color (white on dark themes), its icon in the background color.
fn round_button(id: &'static str, label: &'static str, p: &Palette) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(ui(28.0))
        .rounded_full()
        .bg(hsla(p.fg))
        .text_color(hsla(p.base))
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
}

/// Icon-only button without a background until hovered.
fn ghost_button(id: &'static str, label: &'static str, p: &Palette) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(ui(28.0))
        .rounded(ui(RADIUS_MD))
        .text_color(hsla(p.muted))
        .cursor_pointer()
        .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
}

/// Small icon button for row actions (copy, queue card actions).
fn small_icon_button(id: impl Into<ElementId>, icon: IconName, label: &'static str, p: &Palette) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(ui(22.0))
        .rounded(ui(RADIUS_MD))
        .text_color(hsla(p.muted))
        .cursor_pointer()
        .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
        .child(Icon::new(icon).size(ui(13.0)))
}

/// `--model <name>` / `-m <name>` / `--model=<name>` from the agent's arguments.
pub fn model_arg(args: &[String]) -> Option<String> {
    args.iter().enumerate().find_map(|(i, arg)| match arg.as_str() {
        "--model" | "-m" => args.get(i + 1).cloned(),
        _ => arg.strip_prefix("--model=").map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::{duration, file_name, fold_text, model_arg};

    #[test]
    fn reads_the_model_argument() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(model_arg(&args(&["--model", "sonnet"])).as_deref(), Some("sonnet"));
        assert_eq!(model_arg(&args(&["-x", "--model=o3"])).as_deref(), Some("o3"));
        assert_eq!(model_arg(&args(&["--model"])), None);
        assert_eq!(model_arg(&[]), None);
    }

    #[test]
    fn formats_durations_and_file_names() {
        assert_eq!(duration(4), "4s");
        assert_eq!(duration(125), "2m 05s");
        assert_eq!(duration(3780), "1h 03m");
        assert_eq!(file_name("src/chat/view.rs"), "view.rs");
        assert_eq!(file_name(""), "");
    }

    #[test]
    fn folds_long_messages_only() {
        assert_eq!(fold_text("short"), None);
        let many = (0..20).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
        let head = fold_text(&many).unwrap();
        assert_eq!(head.lines().count(), 12);
        assert!(head.ends_with('…'));
        assert!(fold_text(&"x".repeat(2000)).unwrap().chars().count() <= 1201);
    }
}
