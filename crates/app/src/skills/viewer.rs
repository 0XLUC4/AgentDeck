//! Interactive model preview (ModelViewer.tsx): drag to orbit, scroll to zoom, click a part to select it.
//! Frames are rendered off the UI thread by `model3d` and drawn as an image.

use std::path::PathBuf;
use std::sync::Arc;

use agentdeck_core::assets::asset_source;
use gpui_kit::*;

use super::model3d::{self, Model, Orbit};

/// A press that moves less than this is a click (picks a part), more is an orbit drag.
const CLICK_SLOP: f32 = 4.0;

/// Emitted when the selected part changes (`None`: empty space was clicked).
pub struct PartSelected(pub Option<String>);

pub struct ModelViewer {
    model: Option<Arc<Model>>,
    pub error: Option<String>,
    /// "Converting with Blender…" / "Loading…" until the model is in.
    pub loading: &'static str,
    orbit: Orbit,
    selected: Option<usize>,
    image: Option<Arc<RenderImage>>,
    /// Part ids of the shown frame, with its pixel size.
    ids: Option<Arc<model3d::Frame>>,
    /// Device pixel size the next frame should have, and the element bounds.
    want: (u32, u32),
    bounds: Bounds<Pixels>,
    rendering: bool,
    dirty: bool,
    drag: Option<(Point<Pixels>, Orbit, bool)>,
    /// Images to free on the next window access (GPUI atlas).
    /// shortcut: the last frame stays in the atlas when the viewer closes (one image per opened model).
    stale: Vec<Arc<RenderImage>>,
}

impl EventEmitter<PartSelected> for ModelViewer {}

/// GPUI image from BGRA pixels.
pub fn render_image(width: u32, height: u32, bgra: Vec<u8>) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(width, height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

impl ModelViewer {
    pub fn new(cwd: String, path: String, ext: String, cx: &mut Context<Self>) -> Self {
        let loading = if ext == "blend" { "Converting with Blender…" } else { "Loading…" };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let source = asset_source(cwd, path)?;
                    model3d::load(&PathBuf::from(source), &ext)
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(model) => this.model = Some(Arc::new(model)),
                    Err(e) => this.error = Some(e),
                }
                this.loading = "";
                this.request_frame(cx);
            })
            .ok();
        })
        .detach();
        Self {
            model: None,
            error: None,
            loading,
            orbit: Orbit::default(),
            selected: None,
            image: None,
            ids: None,
            want: (0, 0),
            bounds: Bounds::default(),
            rendering: false,
            dirty: false,
            drag: None,
            stale: Vec::new(),
        }
    }

    /// Renders a new frame in the background; while one is running, the latest request waits for it.
    fn request_frame(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.model.clone() else { return };
        let (w, h) = self.want;
        if w == 0 || h == 0 {
            return;
        }
        if self.rendering {
            self.dirty = true;
            return;
        }
        self.rendering = true;
        let (orbit, selected) = (self.orbit, self.selected);
        cx.spawn(async move |this, cx| {
            let frame = cx
                .background_executor()
                .spawn(async move { model3d::render(&model, &orbit, w, h, selected) })
                .await;
            this.update(cx, |this, cx| {
                this.rendering = false;
                let frame = Arc::new(frame);
                if let Some(image) = render_image(frame.width, frame.height, frame.bgra.clone()) {
                    this.stale.extend(this.image.replace(image));
                }
                this.ids = Some(frame);
                if std::mem::take(&mut this.dirty) {
                    this.request_frame(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn on_down(&mut self, e: &MouseDownEvent, _: &mut Window, _: &mut Context<Self>) {
        self.drag = Some((e.position, self.orbit, false));
    }

    fn on_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some((start, orbit, moved)) = self.drag.as_mut() else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let delta = e.position - *start;
        if !*moved && f32::from(delta.x).hypot(f32::from(delta.y)) <= CLICK_SLOP {
            return;
        }
        *moved = true;
        let mut next = *orbit;
        next.rotate(f32::from(delta.x), f32::from(delta.y));
        self.orbit = next;
        self.request_frame(cx);
    }

    fn on_up(&mut self, e: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, _, moved)) = self.drag.take() else { return };
        if moved {
            return;
        }
        let (Some(model), Some(frame)) = (self.model.clone(), self.ids.clone()) else { return };
        let local = e.position - self.bounds.origin;
        let scale = window.scale_factor();
        let (x, y) = (f32::from(local.x) * scale, f32::from(local.y) * scale);
        if x < 0.0 || y < 0.0 {
            return;
        }
        let picked = frame.pick(&model, x as u32, y as u32);
        if picked != self.selected {
            self.selected = picked;
            cx.emit(PartSelected(picked.map(|i| model.part_name(i))));
            self.request_frame(cx);
        }
    }

    fn on_scroll(&mut self, e: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let dy = f32::from(e.delta.pixel_delta(px(20.0)).y);
        if dy == 0.0 {
            return;
        }
        self.orbit.zoom(-dy / 40.0);
        self.request_frame(cx);
    }
}

impl Render for ModelViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in self.stale.drain(..) {
            if let Err(e) = window.drop_image(image) {
                eprintln!("model preview: free frame: {e}");
            }
        }
        let view = cx.entity();
        let image = self.image.clone();
        div()
            .id("model-viewer")
            .size_full()
            .cursor_crosshair()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .on_scroll(cx.listener(Self::on_scroll))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let scale = window.scale_factor();
                        let want = (
                            (f32::from(bounds.size.width) * scale).round() as u32,
                            (f32::from(bounds.size.height) * scale).round() as u32,
                        );
                        view.update(cx, |v, cx| {
                            v.bounds = bounds;
                            if v.want != want {
                                v.want = want;
                                v.request_frame(cx);
                            }
                        });
                    },
                    move |bounds, _, window, _| {
                        if let Some(image) = image
                            && let Err(e) = window.paint_image(bounds, bounds, Corners::default(), image, 0, false)
                        {
                            eprintln!("model preview paint: {e}");
                        }
                    },
                )
                .size_full(),
            )
    }
}
