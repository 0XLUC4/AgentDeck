//! Explorer-like grid of the workspace's 3D models and images; a click opens one, where a change
//! can be described and sent to an agent of the workspace.
//! Original: components/Assets/{AssetsDialog,AssetsHost,ModelViewer}.tsx.
//! Models are previewed by the CPU renderer in `model3d` (see its notes), tiles get a rendered thumbnail.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agentdeck_core::AppState;
use agentdeck_core::assets::{AssetFile, asset_source, assets_list};
use agentdeck_core::logic::deck::Pane;
use agentdeck_core::logic::queue::{SUBMIT_DELAY_MS, prompt_text};
use agentdeck_core::logic::text::agent_name;
use agentdeck_core::pty::write_to_pane;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::model3d::{self, Orbit};
use super::viewer::{ModelViewer, PartSelected, render_image};
use super::{modal_frame, modal_header, segmented};
use crate::panes::agent_logo;
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

const MODEL_EXTS: &[&str] = &["glb", "gltf", "obj", "fbx", "stl", "blend"];

/// Fixed light studio backdrop of the original viewer and thumbnails (not themed there either).
const BACKDROP: u32 = 0xe8e8ea;
const THUMB_SIZE: u32 = 256;

fn is_model(file: &AssetFile) -> bool {
    MODEL_EXTS.contains(&file.ext.as_str())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Models,
    Images,
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
                            this.render_thumbnails(&files, cx);
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
            _instruction_events: instruction_events,
        }
    }

    /// Renders the models' tiles one after another in the background (renderThumbnail's queue).
    /// Stops when the dialog closes.
    fn render_thumbnails(&mut self, files: &[AssetFile], cx: &mut Context<Self>) {
        let cwd = cwd_of(&self.deck, &self.workspace_id, cx);
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

    /// Live agent panes of this workspace, in tab order.
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
            self.opened.and_then(|i| self.files.as_ref()?.get(i)),
        ) else {
            return;
        };
        if instruction.is_empty() {
            return;
        }
        let whole = if is_model(file) {
            format!("the 3D model {}", file.path)
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

    fn open(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.files.as_ref().and_then(|f| f.get(index)) else { return };
        let (path, ext, model) = (file.path.clone(), file.ext.clone(), is_model(file));
        self.part = None;
        self.viewer = model.then(|| {
            let cwd = cwd_of(&self.deck, &self.workspace_id, cx);
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
        let model = self.viewer.is_some();
        let placeholder = match &self.part {
            Some(part) => format!("What should change on \"{part}\"?"),
            None => format!("What should change on this {}?", if model { "model" } else { "image" }),
        };
        self.instruction.update(cx, |i, cx| i.set_placeholder(placeholder, window, cx));
        cx.notify();
    }

    fn render_grid(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let filter = self.filter;
        let shown: Vec<(usize, &AssetFile)> = self
            .files
            .iter()
            .flatten()
            .enumerate()
            .filter(|(_, f)| filter == Filter::All || (filter == Filter::Models) == is_model(f))
            .collect();
        div()
            .min_h(ui(320.0))
            .flex()
            .flex_col()
            .gap_2()
            .when_some(self.list_error.clone(), |d, e| d.child(div().text_color(hsla(p.bad)).child(e)))
            .when(self.files.is_none() && self.list_error.is_none(), |d| {
                d.child(div().text_color(hsla(p.subtle)).child("Scanning…"))
            })
            .when(self.files.is_some() && shown.is_empty(), |d| {
                d.child(div().text_color(hsla(p.subtle)).child("No 3D model or image here."))
            })
            .child(div().flex().flex_wrap().gap_2().children(shown.into_iter().map(|(i, file)| {
                let name = file.rel.rsplit(['\\', '/']).next().unwrap_or(&file.rel).to_string();
                div()
                    .id(("asset-tile", i))
                    .w(ui(144.0))
                    .flex()
                    .flex_col()
                    .gap(ui(6.0))
                    .rounded(ui(12.0))
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
                            .rounded(ui(12.0))
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
                                    .rounded(ui(6.0))
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
                    .on_click(cx.listener(move |this, _, window, cx| this.open(i, window, cx)))
            })))
            .into_any_element()
    }

    fn render_detail(&self, file: &AssetFile, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let model = is_model(file);
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
                        .rounded(ui(8.0))
                        .bg(hsla_a(0x000000, 0.55))
                        .px_2()
                        .py_1()
                        .text_size(ui(12.0))
                        .text_color(hsla(0xffffff))
                        .child(hint),
                )
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
                .rounded(ui(12.0))
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
            .h(window.viewport_size().height * 0.72)
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("assets-back")
                            .ghost()
                            .small()
                            .icon(IconName::ArrowLeft)
                            .tooltip("Back to all assets")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.opened = None;
                                this.viewer = None;
                                this.part = None;
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
                    .when(!model, |d| d.p_4())
                    .overflow_hidden()
                    .rounded(ui(12.0))
                    .border_1()
                    .border_color(hsla(p.line))
                    .bg(hsla(BACKDROP))
                    .child(preview),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(ui(6.0))
                    .when(agents.is_empty(), |d| {
                        d.child(
                            div()
                                .text_color(hsla(p.subtle))
                                .child("Start an agent in this workspace to send changes."),
                        )
                    })
                    .children(chips),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let opened = self.opened.and_then(|i| self.files.as_ref()?.get(i));
        let actions = opened.is_none().then(|| {
            let this = cx.entity().downgrade();
            segmented(
                "assets-filter",
                &[(Filter::All, "All"), (Filter::Models, "3D"), (Filter::Images, "Images")],
                self.filter,
                move |filter, _, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.filter = filter;
                        cx.notify();
                    });
                },
                cx,
            )
            .into_any_element()
        });
        let body = if workspace.path.is_empty() {
            div()
                .text_color(hsla(p.muted))
                .child("Link a folder to this workspace to browse its 3D assets.")
                .into_any_element()
        } else if let Some(file) = opened {
            self.render_detail(file, window, cx)
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
                    .px_5()
                    .py_4()
                    .child(body),
            );
        modal_frame(&self.focus, 1152.0, panel, cx).into_any_element()
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
