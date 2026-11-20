//! Explorer-like grid of the workspace's 3D models and images; a click opens one, where a change
//! can be described and sent to an agent of the workspace. "Changed deliverables" lists instead the
//! images, models and Markdown reports modified or added in one Git worktree of the project (the
//! main checkout or an agent's), read off the UI thread; Markdown opens in the app's reader.
//! Original: components/Assets/{AssetsDialog,AssetsHost,ModelViewer}.tsx.
//! Models are previewed by the CPU renderer in `model3d` (see its notes), tiles get a rendered thumbnail.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agentdeck_core::AppState;
use agentdeck_core::assets::{AssetFile, asset_source, assets_list, changed_deliverables};
use agentdeck_core::logic::deck::Pane;
use agentdeck_core::logic::queue::{SUBMIT_DELAY_MS, prompt_text};
use agentdeck_core::logic::text::agent_name;
use agentdeck_core::pty::write_to_pane;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::model3d::{self, Orbit};
use super::viewer::{ModelViewer, PartSelected, render_image};
use super::{modal_header, page_frame, segmented};
use crate::panes::agent_logo;
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};
use agentdeck_core::logic::theme::{RADIUS_LG, RADIUS_MD, RADIUS_XL};

pub(super) const MODEL_EXTS: &[&str] = &["glb", "gltf", "obj", "fbx", "stl", "blend"];

/// Fixed light studio backdrop of the original viewer and thumbnails (not themed there either).
pub(super) const BACKDROP: u32 = 0xe8e8ea;
const THUMB_SIZE: u32 = 256;

fn is_model(file: &AssetFile) -> bool {
    MODEL_EXTS.contains(&file.ext.as_str())
}

fn is_markdown(file: &AssetFile) -> bool {
    matches!(file.ext.as_str(), "md" | "markdown")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Models,
    Images,
    /// Deliverables changed in the picked Git worktree.
    Changed,
}

/// Worktrees the "Changed deliverables" view offers: the project folder, then each agent's own
/// worktree, deduplicated by folder. `panes`: (agent name, folder it works in).
fn worktree_choices(project: &str, main: &str, panes: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut choices = vec![(format!("{project} (project folder)"), main.to_owned())];
    for (name, dir) in panes {
        if !choices.iter().any(|(_, d)| d == &dir) {
            let folder = std::path::Path::new(&dir).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            choices.push((format!("{name} · {folder}"), dir));
        }
    }
    choices
}

pub struct AssetsDialog {
    deck: Entity<Deck>,
    workspace_id: String,
    pub focus: FocusHandle,
    files: Option<Vec<AssetFile>>,
    list_error: Option<String>,
    /// Index into `files` of the opened asset.
    opened: Option<usize>,
    filter: Filter,
    instruction: Entity<InputState>,
    /// Picked agent pane; the first agent when unset or gone.
    target: Option<String>,
    /// 3D preview of the opened model, and the part clicked in it.
    viewer: Option<(Entity<ModelViewer>, Subscription)>,
    part: Option<String>,
    /// Rendered model thumbnails by path (absent: not rendered yet or failed, the tile shows an icon).
    thumbs: HashMap<String, Arc<RenderImage>>,
    /// "Changed deliverables": worktrees to pick from (label, folder), None until resolved.
    worktrees: Option<Vec<(String, String)>>,
    /// Picked worktree folder.
    worktree: Option<String>,
    /// Its changed files (None while listing), the worktree root they are relative to, or why not.
    changed: Option<Vec<AssetFile>>,
    changed_root: String,
    changed_error: Option<String>,
    /// Text of the opened Markdown file.
    markdown: Option<SharedString>,
    _instruction_events: Subscription,
}

impl AssetsDialog {
    pub fn new(
        deck: Entity<Deck>,
        workspace_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let instruction = cx.new(|cx| InputState::new(window, cx));
        let instruction_events =
            cx.subscribe_in(&instruction, window, |this, _, event: &InputEvent, window, cx| {
                match event {
                    InputEvent::PressEnter { .. } => this.send(window, cx),
                    _ => cx.notify(),
                }
            });
        let cwd = cwd_of(&deck, &workspace_id, cx);
        if !cwd.is_empty() {
            cx.spawn(async move |this, cx| {
                let result = cx.background_executor().spawn(async move { assets_list(cwd) }).await;
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(files) => {
                            let cwd = cwd_of(&this.deck, &this.workspace_id, cx);
                            this.render_thumbnails(&files, cwd, cx);
                            this.files = Some(files);
                        }
                        Err(e) => this.list_error = Some(e),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        Self {
            deck,
            workspace_id,
            focus: cx.focus_handle(),
            files: None,
            list_error: None,
            opened: None,
            filter: Filter::All,
            instruction,
            target: None,
            viewer: None,
            part: None,
            thumbs: HashMap::new(),
            worktrees: None,
            worktree: None,
            changed: None,
            changed_root: String::new(),
            changed_error: None,
            markdown: None,
            _instruction_events: instruction_events,
        }
    }

    /// Files of the current view: the changed deliverables or the whole gallery.
    fn list(&self) -> Option<&Vec<AssetFile>> {
        if self.filter == Filter::Changed { self.changed.as_ref() } else { self.files.as_ref() }
    }

    /// Folder the current view's files sit in (previews only read inside it).
    fn root(&self, cx: &App) -> String {
        if self.filter == Filter::Changed {
            self.changed_root.clone()
        } else {
            cwd_of(&self.deck, &self.workspace_id, cx)
        }
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        if filter == Filter::Changed && self.worktrees.is_none() {
            self.load_worktrees(cx);
        }
        cx.notify();
    }

    /// Resolves the worktree of each agent of the project off the UI thread, then lists the active
    /// agent's one (else the project folder's).
    fn load_worktrees(&mut self, cx: &mut Context<Self>) {
        let main = cwd_of(&self.deck, &self.workspace_id, cx);
        let deck = self.deck.read(cx);
        let project = deck.state.workspaces.iter().find(|w| w.id == self.workspace_id).map(|w| w.name.clone()).unwrap_or_default();
        let active = deck.state.active_pane_id.clone();
        // Local agents only: an ssh pane's folder is on another machine.
        let panes: Vec<(String, String, String, bool)> = self
            .agents(cx)
            .into_iter()
            .filter(|p| p.ssh.is_none())
            .map(|p| {
                let name = if p.title.is_empty() { agent_name(p.agent).to_owned() } else { p.title.clone() };
                (p.id.clone(), name, p.cwd.clone(), p.worktree)
            })
            .collect();
        cx.spawn(async move |this, cx| {
            let (choices, active_dir) = cx
                .background_executor()
                .spawn(async move {
                    let dirs: Vec<(String, String, String)> = panes
                        .into_iter()
                        .map(|(id, name, cwd, worktree)| {
                            let dir = agentdeck_core::git::git_pane_dir(cwd, id.clone(), worktree);
                            (id, name, dir)
                        })
                        .collect();
                    let active_dir = dirs.iter().find(|(id, ..)| active.as_ref() == Some(id)).map(|(_, _, dir)| dir.clone());
                    let choices = worktree_choices(&project, &main, dirs.into_iter().map(|(_, name, dir)| (name, dir)).collect());
                    (choices, active_dir)
                })
                .await;
            this.update(cx, |this, cx| {
                let pick = active_dir.filter(|dir| choices.iter().any(|(_, d)| d == dir)).unwrap_or_else(|| choices[0].1.clone());
                this.worktrees = Some(choices);
                this.load_changed(pick, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Lists the deliverables changed in `dir`'s worktree off the UI thread.
    fn load_changed(&mut self, dir: String, cx: &mut Context<Self>) {
        self.worktree = Some(dir.clone());
        self.changed = None;
        self.changed_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let asked = dir.clone();
            let result = cx.background_executor().spawn(async move { changed_deliverables(asked) }).await;
            this.update(cx, |this, cx| {
                if this.worktree.as_ref() != Some(&dir) {
                    return; // another worktree was picked meanwhile
                }
                match result {
                    Ok(changed) => {
                        this.render_thumbnails(&changed.files, changed.root.clone(), cx);
                        this.changed_root = changed.root;
                        this.changed = Some(changed.files);
                    }
                    Err(e) => this.changed_error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Renders the models' tiles one after another in the background (renderThumbnail's queue).
    /// Stops when the dialog closes.
    fn render_thumbnails(&mut self, files: &[AssetFile], cwd: String, cx: &mut Context<Self>) {
        let models: Vec<(String, String)> =
            files.iter().filter(|f| is_model(f)).map(|f| (f.path.clone(), f.ext.clone())).collect();
        cx.spawn(async move |this, cx| {
            for (path, ext) in models {
                let (cwd, key) = (cwd.clone(), path.clone());
                let frame = cx
                    .background_executor()
                    .spawn(async move {
                        let source = asset_source(cwd, path)?;
                        let model = model3d::load(&PathBuf::from(source), &ext)?;
                        Ok::<_, String>(model3d::render(&model, &Orbit::default(), THUMB_SIZE, THUMB_SIZE, None))
                    })
                    .await;
                let alive = this
                    .update(cx, |this, cx| {
                        if let Some(image) = frame.ok().and_then(|f| render_image(f.width, f.height, f.bgra)) {
                            this.thumbs.insert(key, image);
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    return;
                }
            }
        })
        .detach();
    }

    /// Live agent panes of this project, in tab order.
    fn agents(&self, cx: &App) -> Vec<Pane> {
        let state = &self.deck.read(cx).state;
        state
            .tabs
            .iter()
            .filter(|t| t.workspace_id == self.workspace_id)
            .flat_map(|t| &t.pane_ids)
            .filter_map(|id| state.panes.get(id))
            .filter(|p| !p.exited)
            .cloned()
            .collect()
    }

    fn target_id(&self, agents: &[Pane]) -> Option<String> {
        agents
            .iter()
            .find(|p| Some(&p.id) == self.target.as_ref())
            .or(agents.first())
            .map(|p| p.id.clone())
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let agents = self.agents(cx);
        let instruction = self.instruction.read(cx).value().trim().to_string();
        let (Some(target), Some(file)) = (
            self.target_id(&agents),
            self.opened.and_then(|i| self.list()?.get(i)),
        ) else {
            return;
        };
        if instruction.is_empty() {
            return;
        }
        let whole = if is_model(file) {
            format!("the 3D model {}", file.path)
        } else if is_markdown(file) {
            format!("the report {}", file.path)
        } else {
            format!("the image {}", file.path)
        };
        let subject = match &self.part {
            Some(part) => format!("the object \"{part}\" of {whole}"),
            None => whole,
        };
        let prompt = format!("In {subject}: {instruction}");
        let name = agents
            .iter()
            .find(|p| p.id == target)
            .map(|p| p.title.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "the agent".into());
        let backend = self.deck.read(cx).backend.clone();
        let deck = self.deck.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = send_prompt(backend, target, prompt, cx).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.instruction.update(cx, |i, cx| i.set_value("", window, cx));
                        deck.update(cx, |d, cx| d.toast(format!("Sent to {name}"), cx));
                    }
                    Err(e) => deck.update(cx, |d, cx| d.toast(format!("Could not send: {e}"), cx)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Checks off the UI thread that the clicked file still exists (reading a Markdown report's
    /// text), then opens it; a file deleted since the list was made is reported and the list redone.
    fn open_checked(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.list().and_then(|f| f.get(index)) else { return };
        let (path, rel, markdown) = (file.path.clone(), file.rel.clone(), is_markdown(file));
        cx.spawn_in(window, async move |this, cx| {
            let read = path.clone();
            let found = cx
                .background_executor()
                .spawn(async move {
                    if markdown {
                        std::fs::read_to_string(&read).map(|text| Some(SharedString::from(text)))
                    } else {
                        std::fs::metadata(&read).map(|_| None)
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                // The list may have been redone meanwhile: open only the file that was clicked.
                if this.list().and_then(|f| f.get(index)).is_none_or(|f| f.path != path) {
                    return;
                }
                match found {
                    Ok(text) => {
                        this.markdown = text;
                        this.open(index, window, cx);
                    }
                    Err(_) => {
                        this.deck.update(cx, |d, cx| d.toast(format!("{rel} no longer exists"), cx));
                        if let Some(dir) = this.worktree.clone().filter(|_| this.filter == Filter::Changed) {
                            this.load_changed(dir, cx);
                        }
                    }
                }
            });
        })
        .detach();
    }

    fn open(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.list().and_then(|f| f.get(index)) else { return };
        let (path, ext, model) = (file.path.clone(), file.ext.clone(), is_model(file));
        self.part = None;
        self.viewer = model.then(|| {
            let cwd = self.root(cx);
            let viewer = cx.new(|cx| ModelViewer::new(cwd, path, ext, cx));
            let sub = cx.subscribe_in(&viewer, window, |this, _, e: &PartSelected, window, cx| {
                this.part = e.0.clone();
                this.update_placeholder(window, cx);
            });
            (viewer, sub)
        });
        self.opened = Some(index);
        self.update_placeholder(window, cx);
    }

    fn update_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = if self.viewer.is_some() {
            "model"
        } else if self.markdown.is_some() {
            "report"
        } else {
            "image"
        };
        let placeholder = match &self.part {
            Some(part) => format!("What should change on \"{part}\"?"),
            None => format!("What should change on this {kind}?"),
        };
        self.instruction.update(cx, |i, cx| i.set_placeholder(placeholder, window, cx));
        cx.notify();
    }

    fn render_grid(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let filter = self.filter;
        let changed = filter == Filter::Changed;
        let shown: Vec<(usize, &AssetFile)> = self
            .list()
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|(_, f)| matches!(filter, Filter::All | Filter::Changed) || (filter == Filter::Models) == is_model(f))
            .collect();
        let (error, listed, empty) = if changed {
            (self.changed_error.clone(), self.changed.is_some(), "No changed image, 3D model or Markdown file in this worktree.")
        } else {
            (self.list_error.clone(), self.files.is_some(), "No 3D model or image here.")
        };
        div()
            .min_h(ui(320.0))
            .flex()
            .flex_col()
            .gap_2()
            .when(changed, |d| d.child(self.worktree_bar(cx)))
            .when_some(error.clone(), |d, e| d.child(div().text_color(hsla(p.bad)).child(e)))
            .when(!listed && error.is_none(), |d| d.child(div().text_color(hsla(p.subtle)).child("Scanning…")))
            .when(listed && shown.is_empty(), |d| d.child(div().text_color(hsla(p.subtle)).child(empty)))
            .child(div().flex().flex_wrap().gap_2().children(shown.into_iter().map(|(i, file)| {
                // Changed deliverables show where they sit in the worktree.
                let name = if changed {
                    file.rel.clone()
                } else {
                    file.rel.rsplit(['\\', '/']).next().unwrap_or(&file.rel).to_string()
                };
                div()
                    .id(("asset-tile", i))
                    .w(ui(144.0))
                    .flex()
                    .flex_col()
                    .gap(ui(6.0))
                    .rounded(ui(RADIUS_XL))
                    .p(ui(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(p.hover)))
                    .child(
                        div()
                            .relative()
                            .size(ui(132.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .rounded(ui(RADIUS_XL))
                            .border_1()
                            .border_color(hsla(p.line))
                            .bg(hsla(BACKDROP))
                            .child(match self.thumbs.get(&file.path) {
                                Some(image) => img(image.clone()).size_full().object_fit(ObjectFit::Contain).into_any_element(),
                                None => thumbnail(file, 28.0),
                            })
                            .child(
                                div()
                                    .absolute()
                                    .top(ui(6.0))
                                    .right(ui(6.0))
                                    .rounded(ui(RADIUS_MD))
                                    .bg(hsla_a(0x000000, 0.5))
                                    .px(ui(6.0))
                                    .text_size(ui(10.0))
                                    .text_color(hsla_a(0xffffff, 0.8))
                                    .child(file.ext.to_uppercase()),
                            ),
                    )
                    .child(
                        div()
                            .px(ui(2.0))
                            .truncate()
                            .text_size(ui(12.0))
                            .text_color(hsla(p.muted))
                            .child(name),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.open_checked(i, window, cx)))
            })))
            .into_any_element()
    }

    /// "Changed deliverables" header: one chip per worktree of the project, refresh, and what the
    /// list means.
    fn worktree_bar(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let chips = self.worktrees.iter().flatten().enumerate().map(|(i, (label, dir))| {
            let pressed = self.worktree.as_ref() == Some(dir);
            let dir = dir.clone();
            let hint = SharedString::from(dir.clone());
            div()
                .id(("asset-worktree", i))
                .h(ui(28.0))
                .max_w(ui(260.0))
                .flex()
                .items_center()
                .gap(ui(6.0))
                .px(ui(10.0))
                .rounded(ui(RADIUS_LG))
                .border_1()
                .cursor_pointer()
                .text_size(ui(12.0))
                .when(pressed, |d| d.border_color(hsla(p.line_strong)).bg(hsla(p.raised)).text_color(hsla(p.fg)))
                .when(!pressed, |d| {
                    d.border_color(hsla(p.line)).text_color(hsla(p.muted)).hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                })
                .child(Icon::new(IconName::GitBranch).size(ui(13.0)))
                .child(div().truncate().child(label.clone()))
                .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                .on_click(cx.listener(move |this, _, _, cx| this.load_changed(dir.clone(), cx)))
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .pb_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .children(chips)
                    .when(self.worktrees.is_none(), |d| d.child(div().text_color(hsla(p.subtle)).child("Finding worktrees…")))
                    .child(
                        Button::new("assets-changed-refresh")
                            .ghost()
                            .small()
                            .icon(IconName::RefreshCw)
                            .tooltip("List the changes again")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(dir) = this.worktree.clone() {
                                    this.load_changed(dir, cx);
                                }
                            })),
                    ),
            )
            .child(div().text_size(ui(12.0)).text_color(hsla(p.subtle)).child(
                "Images, 3D models and Markdown files modified or added in this Git worktree (deleted ones left \
                 out). Git tells where a file changed, not which agent made it.",
            ))
    }

    fn render_detail(&self, file: &AssetFile, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let model = is_model(file);
        let report = is_markdown(file) && self.markdown.is_some();
        let agents = self.agents(cx);
        let target = self.target_id(&agents);
        let can_send = target.is_some() && !self.instruction.read(cx).value().trim().is_empty();
        let preview = if let Some((viewer, _)) = &self.viewer {
            let v = viewer.read(cx);
            let (error, loading) = (v.error.clone(), v.loading);
            let hint = match &self.part {
                Some(part) => div()
                    .flex()
                    .gap_1()
                    .child("Selected:")
                    .child(div().font_weight(FontWeight::MEDIUM).child(part.clone())),
                None => div().child("Drag to orbit, scroll to zoom, click a part to select it"),
            };
            div()
                .relative()
                .size_full()
                .child(viewer.clone())
                .when(!loading.is_empty() && error.is_none(), |d| {
                    d.child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(hsla(0x525252))
                            .child(loading),
                    )
                })
                .when_some(error, |d, e| {
                    d.child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .px_6()
                            .text_color(hsla(p.bad))
                            .child(e),
                    )
                })
                .child(
                    div()
                        .absolute()
                        .top_2()
                        .left_2()
                        .rounded(ui(RADIUS_LG))
                        .bg(hsla_a(0x000000, 0.55))
                        .px_2()
                        .py_1()
                        .text_size(ui(12.0))
                        .text_color(hsla(0xffffff))
                        .child(hint),
                )
                .into_any_element()
        } else if let Some(text) = self.markdown.clone().filter(|_| is_markdown(file)) {
            // The app's Markdown reader, one id per file (two worktrees' copies stay apart).
            div()
                .size_full()
                .p_4()
                .bg(hsla(p.panel))
                .text_color(hsla(p.fg))
                .child(TextView::markdown(SharedString::from(format!("asset-md-{}", file.path)), text).selectable(true).scrollable(true))
                .into_any_element()
        } else {
            img(PathBuf::from(&file.path))
                .max_w_full()
                .max_h_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element()
        };
        let chips = agents.iter().enumerate().map(|(i, pane)| {
            let pressed = Some(&pane.id) == target.as_ref();
            let id = pane.id.clone();
            let label = if pane.title.is_empty() { agent_name(pane.agent).to_string() } else { pane.title.clone() };
            div()
                .id(("asset-target", i))
                .h(ui(32.0))
                .max_w(ui(224.0))
                .flex()
                .items_center()
                .gap_2()
                .px(ui(10.0))
                .rounded(ui(RADIUS_XL))
                .border_1()
                .cursor_pointer()
                .when(pressed, |d| d.border_color(hsla(p.line_strong)).bg(hsla(p.raised)).text_color(hsla(p.fg)))
                .when(!pressed, |d| {
                    d.border_color(hsla(p.line))
                        .text_color(hsla(p.muted))
                        .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                })
                .child(agent_logo(pane.agent, 15.0, None, &p))
                .child(div().truncate().child(label))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.target = Some(id.clone());
                    cx.notify();
                }))
        });
        div()
            // Fills the page: the agent choice and the prompt sit at its bottom edge.
            .h_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .pt_3()
                    .child(
                        Button::new("assets-back")
                            .ghost()
                            .small()
                            .icon(IconName::ArrowLeft)
                            .tooltip(if self.filter == Filter::Changed { "Back to the changed deliverables" } else { "Back to all assets" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.opened = None;
                                this.viewer = None;
                                this.part = None;
                                this.markdown = None;
                                cx.notify();
                            })),
                    )
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(file.rel.clone())),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!model && !report, |d| d.p_4())
                    .overflow_hidden()
                    .when(!report, |d| d.bg(hsla(BACKDROP)))
                    .child(preview),
            )
            // One line: who gets the change, what to change, send.
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .pb_3()
                    .when(agents.is_empty(), |d| {
                        d.child(
                            div()
                                .flex_shrink_0()
                                .text_color(hsla(p.subtle))
                                .child("Start an agent in this project to send changes."),
                        )
                    })
                    .children(chips)
                    .child(div().flex_1().min_w_0().child(Input::new(&self.instruction)))
                    .child(
                        Button::new("assets-send")
                            .primary()
                            .icon(IconName::Send)
                            .label("Send")
                            .disabled(!can_send)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
            .into_any_element()
    }
}

impl Render for AssetsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let Some(workspace) = self
            .deck
            .read(cx)
            .state
            .workspaces
            .iter()
            .find(|w| w.id == self.workspace_id)
            .cloned()
        else {
            return div().into_any_element();
        };
        let opened = self.opened.and_then(|i| self.list()?.get(i));
        let actions = opened.is_none().then(|| {
            let this = cx.entity().downgrade();
            segmented(
                "assets-filter",
                &[
                    (Filter::All, "All"),
                    (Filter::Models, "3D"),
                    (Filter::Images, "Images"),
                    (Filter::Changed, "Changed deliverables"),
                ],
                self.filter,
                move |filter, _, cx| {
                    let _ = this.update(cx, |this, cx| this.set_filter(filter, cx));
                },
                cx,
            )
            .into_any_element()
        });
        let body = if workspace.path.is_empty() {
            div()
                .text_color(hsla(p.muted))
                .child("Link a folder to this project to browse its 3D assets.")
                .into_any_element()
        } else if let Some(file) = opened {
            self.render_detail(file, cx)
        } else {
            self.render_grid(cx)
        };
        let panel = div()
            .child(modal_header(format!("Assets · {}", workspace.name), actions, cx))
            .child(
                div()
                    .id("assets-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    // The grid keeps its margins; an opened asset goes edge to edge.
                    .when(self.opened.is_none(), |d| d.px_5().py_4())
                    .child(body),
            );
        page_frame(&self.focus, panel, cx).into_any_element()
    }
}

/// Tile picture: the image itself, or a model icon (see the module note).
fn thumbnail(file: &AssetFile, icon_size: f32) -> AnyElement {
    let icon = move |name: IconName| {
        div()
            .size(ui(icon_size))
            .text_size(ui(icon_size))
            .text_color(hsla_a(0x000000, 0.25))
            .child(name)
            .into_any_element()
    };
    if is_model(file) {
        return icon(IconName::Box);
    }
    if is_markdown(file) {
        return icon(IconName::FileText);
    }
    img(PathBuf::from(&file.path))
        .size_full()
        .object_fit(ObjectFit::Contain)
        .with_fallback(move || icon(IconName::Image))
        .into_any_element()
}

fn cwd_of(deck: &Entity<Deck>, workspace_id: &str, cx: &App) -> String {
    deck.read(cx)
        .state
        .workspaces
        .iter()
        .find(|w| w.id == workspace_id)
        .map(|w| w.path.clone())
        .unwrap_or_default()
}

/// Types the prompt, then presses Enter in a separate write (agent TUIs read text + Enter in one
/// write as a paste). Same as Broadcast/send.ts `sendPrompt`.
async fn send_prompt(
    backend: agentdeck_core::AppHandle,
    pane_id: String,
    text: String,
    cx: &mut AsyncWindowContext,
) -> Result<(), String> {
    let state = backend
        .try_state::<AppState>()
        .ok_or_else(|| "backend not started".to_string())?;
    let (write_state, id) = (state.clone(), pane_id.clone());
    cx.background_executor()
        .spawn(async move { write_to_pane(&write_state, id, prompt_text(&text)) })
        .await?;
    cx.background_executor()
        .timer(Duration::from_millis(SUBMIT_DELAY_MS))
        .await;
    cx.background_executor()
        .spawn(async move { write_to_pane(&state, pane_id, "\r".into()) })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn worktree_choices_start_with_the_project_and_dedupe() {
        let panes = vec![
            ("Claude".to_owned(), r"D:\shop-agentdeck-1a2b3c4d".to_owned()),
            ("Codex".to_owned(), r"D:\shop".to_owned()), // works in the project folder itself
            ("Claude 2".to_owned(), r"D:\shop-agentdeck-1a2b3c4d".to_owned()),
        ];
        let choices = worktree_choices("shop", r"D:\shop", panes);
        let labels: Vec<&str> = choices.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, ["shop (project folder)", "Claude · shop-agentdeck-1a2b3c4d"]);
        assert_eq!(worktree_choices("p", "", Vec::new()).len(), 1);
    }

    #[test]
    fn markdown_is_a_deliverable_kind() {
        let file = |ext: &str| AssetFile { path: String::new(), rel: String::new(), ext: ext.into(), size: 0 };
        assert!(is_markdown(&file("md")) && !is_model(&file("md")));
        assert!(is_model(&file("glb")) && !is_markdown(&file("png")));
    }
}
