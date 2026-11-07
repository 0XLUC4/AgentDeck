//! Notes graph: map of the workspace's Markdown notes and the links between them, with the
//! selected note beside it. Original: components/Graph/{GraphDialog,forceLayout}.ts.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use agentdeck_core::logic::theme::{FONT_SANS, Palette};
use agentdeck_core::notes_graph::{NotesGraph, note_read, notes_graph};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{modal_frame, modal_header};
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

// Small force-directed layout: links pull, every pair repels, a weak pull to the center.
const LINK_LENGTH: f32 = 60.0;
const REPULSION: f32 = 1800.0;
const CENTER_PULL: f32 = 0.004;
const DAMPING: f32 = 0.6;

#[derive(Clone, Copy, Debug, Default)]
pub struct SimNode {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    /// Held by the pointer: not moved by the forces.
    pub pinned: bool,
}

/// Nodes on a spiral, so the first frames already look spread out.
pub fn initial_positions(count: usize) -> Vec<SimNode> {
    (0..count)
        .map(|i| {
            let angle = i as f32 * 2.4;
            let radius = 12.0 * (i as f32).sqrt();
            SimNode { x: angle.cos() * radius, y: angle.sin() * radius, ..SimNode::default() }
        })
        .collect()
}

/// `a` or a tiny offset when it is zero (JS `a || 0.01`).
fn nonzero(a: f32) -> f32 {
    if a == 0.0 { 0.01 } else { a }
}

/// One simulation step scaled by `alpha` (cooling from 1 to 0). Edges are index pairs.
/// shortcut: repulsion is O(n²); fine for a few hundred notes, switch to Barnes-Hut if workspaces get much bigger.
pub fn step(nodes: &mut [SimNode], edges: &[(usize, usize)], alpha: f32) {
    for i in 0..nodes.len() {
        for j in i + 1..nodes.len() {
            let dx = nonzero(nodes[j].x - nodes[i].x);
            let dy = nonzero(nodes[j].y - nodes[i].y);
            let dist2 = (dx * dx + dy * dy).max(25.0);
            let force = REPULSION / dist2 * alpha;
            let dist = dist2.sqrt();
            let (fx, fy) = (dx / dist * force, dy / dist * force);
            nodes[i].vx -= fx;
            nodes[i].vy -= fy;
            nodes[j].vx += fx;
            nodes[j].vy += fy;
        }
    }
    for &(i, j) in edges {
        let dx = nodes[j].x - nodes[i].x;
        let dy = nodes[j].y - nodes[i].y;
        let dist = match (dx * dx + dy * dy).sqrt() {
            d if d == 0.0 => 1.0,
            d => d,
        };
        let force = (dist - LINK_LENGTH) / dist * 0.08 * alpha;
        nodes[i].vx += dx * force;
        nodes[i].vy += dy * force;
        nodes[j].vx -= dx * force;
        nodes[j].vy -= dy * force;
    }
    for node in nodes.iter_mut() {
        node.vx = (node.vx - node.x * CENTER_PULL * alpha) * DAMPING;
        node.vy = (node.vy - node.y * CENTER_PULL * alpha) * DAMPING;
        if node.pinned {
            node.vx = 0.0;
            node.vy = 0.0;
            continue;
        }
        node.x += node.vx;
        node.y += node.vy;
    }
}

/// Pan and zoom of the canvas.
#[derive(Clone, Copy)]
struct ViewTransform {
    x: f32,
    y: f32,
    k: f32,
}

struct Drag {
    /// Node under the pointer at press, or None to pan.
    node: Option<usize>,
    start: Point<Pixels>,
    last: Point<Pixels>,
    moved: bool,
}

/// Simulation and interaction state of a loaded graph.
struct Sim {
    nodes: Vec<SimNode>,
    edges: Vec<(usize, usize)>,
    neighbors: Vec<HashSet<usize>>,
    radius: Vec<f32>,
    alpha: f32,
    view: ViewTransform,
    hovered: Option<usize>,
    drag: Option<Drag>,
}

impl Sim {
    fn new(graph: &NotesGraph) -> Self {
        let index = |id: &str| graph.nodes.iter().position(|n| n.id == id).unwrap_or(0);
        let edges: Vec<(usize, usize)> = graph.edges.iter().map(|(a, b)| (index(a), index(b))).collect();
        let mut neighbors = vec![HashSet::new(); graph.nodes.len()];
        for &(a, b) in &edges {
            neighbors[a].insert(b);
            neighbors[b].insert(a);
        }
        let radius = neighbors.iter().map(|n| 3.0 + (n.len() as f32).sqrt() * 2.0).collect();
        Self {
            nodes: initial_positions(graph.nodes.len()),
            edges,
            neighbors,
            radius,
            alpha: 1.0,
            view: ViewTransform { x: 0.0, y: 0.0, k: 1.0 },
            hovered: None,
            drag: None,
        }
    }

    /// Canvas point (relative to the canvas origin) to world coordinates.
    fn to_world(&self, local: Point<Pixels>, size: Size<Pixels>) -> (f32, f32) {
        let v = self.view;
        (
            (f32::from(local.x) - f32::from(size.width) / 2.0 - v.x) / v.k,
            (f32::from(local.y) - f32::from(size.height) / 2.0 - v.y) / v.k,
        )
    }

    fn node_at(&self, local: Point<Pixels>, size: Size<Pixels>) -> Option<usize> {
        let (x, y) = self.to_world(local, size);
        let k = self.view.k;
        self.nodes
            .iter()
            .enumerate()
            .position(|(i, n)| (n.x - x).hypot(n.y - y) <= self.radius[i] + 3.0 / k)
    }
}

/// Map of the workspace's Markdown notes and the links between them, with the selected note beside it.
pub struct GraphDialog {
    deck: Entity<Deck>,
    workspace_id: String,
    pub focus: FocusHandle,
    graph: Option<NotesGraph>,
    error: Option<String>,
    sim: Option<Sim>,
    selected: Option<String>,
    text: String,
    /// Canvas bounds of the last paint, for pointer math.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    ticking: Option<Task<()>>,
}

impl GraphDialog {
    pub fn new(deck: Entity<Deck>, workspace_id: String, cx: &mut Context<Self>) -> Self {
        let cwd = workspace_path(&deck, &workspace_id, cx);
        if !cwd.is_empty() {
            cx.spawn(async move |this, cx| {
                let result = cx.background_executor().spawn(async move { notes_graph(cwd) }).await;
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(graph) => {
                            this.sim = Some(Sim::new(&graph));
                            this.graph = Some(graph);
                            this.run(cx);
                        }
                        Err(e) => this.error = Some(e),
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
            graph: None,
            error: None,
            sim: None,
            selected: None,
            text: String::new(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            ticking: None,
        }
    }

    /// Runs the simulation at ~60 fps until it cools down.
    fn run(&mut self, cx: &mut Context<Self>) {
        if self.ticking.is_some() {
            return;
        }
        self.ticking = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let running = this.update(cx, |this, cx| {
                    let Some(sim) = this.sim.as_mut() else {
                        return false;
                    };
                    step(&mut sim.nodes, &sim.edges, sim.alpha);
                    sim.alpha *= 0.985;
                    cx.notify();
                    sim.alpha > 0.02
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
            let _ = this.update(cx, |this, _| this.ticking = None);
        }));
    }

    fn reheat(&mut self, cx: &mut Context<Self>) {
        if let Some(sim) = self.sim.as_mut() {
            sim.alpha = sim.alpha.max(0.3);
        }
        self.run(cx);
    }

    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected = Some(id.clone());
        self.text.clear();
        let cwd = workspace_path(&self.deck, &self.workspace_id, cx);
        cx.spawn(async move |this, cx| {
            let read_id = id.clone();
            let result = cx.background_executor().spawn(async move { note_read(cwd, read_id) }).await;
            let _ = this.update(cx, |this, cx| {
                if this.selected.as_deref() == Some(id.as_str()) {
                    this.text = result.unwrap_or_else(|e| e);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn on_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let bounds = self.bounds.get();
        let Some(sim) = self.sim.as_mut() else { return };
        let node = sim.node_at(event.position - bounds.origin, bounds.size);
        if let Some(i) = node {
            sim.nodes[i].pinned = true;
        }
        sim.drag = Some(Drag { node, start: event.position, last: event.position, moved: false });
        cx.notify();
    }

    fn on_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = self.bounds.get();
        let Some(sim) = self.sim.as_mut() else { return };
        let local = event.position - bounds.origin;
        let Some(drag) = sim.drag.as_mut() else {
            let next = sim.node_at(local, bounds.size);
            if next != sim.hovered {
                sim.hovered = next;
                cx.notify();
            }
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            // Released outside the canvas.
            return self.on_up(cx);
        }
        let delta = event.position - drag.start;
        drag.moved |= f32::from(delta.x).hypot(f32::from(delta.y)) > 3.0;
        let step_by = event.position - drag.last;
        drag.last = event.position;
        match drag.node {
            Some(i) => {
                let (x, y) = sim.to_world(local, bounds.size);
                sim.nodes[i].x = x;
                sim.nodes[i].y = y;
                self.reheat(cx);
            }
            None => {
                sim.view.x += f32::from(step_by.x);
                sim.view.y += f32::from(step_by.y);
                cx.notify();
            }
        }
        let _ = window;
    }

    fn on_up(&mut self, cx: &mut Context<Self>) {
        let Some(sim) = self.sim.as_mut() else { return };
        let Some(drag) = sim.drag.take() else { return };
        if let Some(i) = drag.node {
            sim.nodes[i].pinned = false;
            if !drag.moved {
                if let Some(id) = self.graph.as_ref().and_then(|g| g.nodes.get(i)).map(|n| n.id.clone()) {
                    self.select(id, cx);
                }
            }
        }
        cx.notify();
    }

    fn on_wheel(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = self.bounds.get();
        let Some(sim) = self.sim.as_mut() else { return };
        let local = event.position - bounds.origin;
        let (bx, by) = sim.to_world(local, bounds.size);
        let dy = f32::from(event.delta.pixel_delta(window.line_height()).y);
        // Wheel down (negative GPUI delta) zooms out, as `deltaY > 0` does in the browser.
        let k = (sim.view.k * (dy * 0.0015).exp()).clamp(0.15, 6.0);
        // Zoom around the cursor: the world point under it stays put.
        sim.view = ViewTransform {
            k,
            x: sim.view.x + bx * (sim.view.k - k),
            y: sim.view.y + by * (sim.view.k - k),
        };
        cx.stop_propagation();
        cx.notify();
    }

    fn render_canvas(&self, cx: &mut Context<Self>) -> AnyElement {
        let (Some(sim), Some(graph)) = (&self.sim, &self.graph) else {
            return div().into_any_element();
        };
        let p = palette(cx);
        let focus = sim.hovered.or_else(|| {
            let selected = self.selected.as_deref()?;
            graph.nodes.iter().position(|n| n.id == selected)
        });
        let scene = Scene {
            nodes: sim.nodes.iter().map(|n| (n.x, n.y)).collect(),
            edges: sim.edges.clone(),
            lit: (0..sim.nodes.len())
                .map(|i| focus.is_none_or(|f| f == i || sim.neighbors[f].contains(&i)))
                .collect(),
            hub: sim.neighbors.iter().map(|n| n.len() > 3).collect(),
            radius: sim.radius.clone(),
            titles: graph.nodes.iter().map(|n| SharedString::from(n.title.clone())).collect(),
            focus,
            view: sim.view,
            palette: p,
        };
        let bounds_cell = self.bounds.clone();
        let cursor = if sim.hovered.is_some() { CursorStyle::PointingHand } else { CursorStyle::OpenHand };
        div()
            .id("notes-graph-canvas")
            .size_full()
            .cursor(cursor)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e, _, cx| this.on_down(e, cx)))
            .on_mouse_move(cx.listener(|this, e, window, cx| this.on_move(e, window, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.on_up(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.on_up(cx)))
            .on_scroll(cx.listener(|this, e, window, cx| this.on_wheel(e, window, cx)))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !*hovered {
                    if let Some(sim) = this.sim.as_mut() {
                        sim.hovered = None;
                        cx.notify();
                    }
                }
            }))
            .child(
                canvas(
                    move |bounds, _, _| bounds_cell.set(bounds),
                    move |bounds, (), window, cx| scene.paint(bounds, window, cx),
                )
                .size_full(),
            )
            .into_any_element()
    }

    fn render_side(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let side = div().w(ui(320.0)).flex_shrink_0().flex().flex_col().gap_2().min_h_0();
        let Some(selected) = self.selected.clone() else {
            return side.child(div().text_color(hsla(p.subtle)).child(
                "Click a note to read it. Drag the background to pan, scroll to zoom, drag a note to move it.",
            ));
        };
        let mut linked: Vec<String> = self
            .graph
            .iter()
            .flat_map(|g| &g.edges)
            .filter(|(a, b)| *a == selected || *b == selected)
            .map(|(a, b)| if *a == selected { b.clone() } else { a.clone() })
            .collect();
        linked.sort();
        side.child(div().truncate().font_weight(FontWeight::MEDIUM).child(selected))
            .when(!linked.is_empty(), |d| {
                d.child(div().flex().flex_wrap().gap_1().children(linked.into_iter().enumerate().map(
                    |(i, id)| {
                        let label = id.rsplit('/').next().unwrap_or(&id).trim_end_matches(".md").to_string();
                        div()
                            .id(("linked-note", i))
                            .max_w_full()
                            .truncate()
                            .rounded(ui(8.0))
                            .border_1()
                            .border_color(hsla(p.line))
                            .px_2()
                            .py(ui(2.0))
                            .text_size(ui(12.0))
                            .text_color(hsla(p.muted))
                            .cursor_pointer()
                            .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx)))
                    },
                )))
            })
            .child(
                div()
                    .id("note-text")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .rounded(ui(12.0))
                    .border_1()
                    .border_color(hsla(p.line))
                    .bg(hsla(p.base))
                    .p_3()
                    .text_size(ui(12.0))
                    .text_color(hsla(p.muted))
                    .whitespace_normal()
                    .child(self.text.clone()),
            )
    }
}

impl Render for GraphDialog {
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
        let counts = self
            .graph
            .as_ref()
            .map(|g| format!(" · {} notes, {} links", g.nodes.len(), g.edges.len()))
            .unwrap_or_default();
        let title = format!("Graph · {}{counts}", workspace.name);
        let body = if workspace.path.is_empty() {
            div().text_color(hsla(p.muted)).child("Link a folder to this workspace to map its notes.")
        } else {
            let empty = self.graph.as_ref().is_some_and(|g| g.nodes.is_empty());
            let message = match (&self.error, &self.graph) {
                (Some(e), _) => e.clone(),
                (None, None) => "Reading notes…".into(),
                (None, Some(_)) if empty => "No Markdown note in this folder.".into(),
                _ => String::new(),
            };
            div()
                .flex()
                .gap_4()
                .h(window.viewport_size().height * 0.72)
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .rounded(ui(12.0))
                        .border_1()
                        .border_color(hsla(p.line))
                        .bg(hsla(p.base))
                        .when(!empty, |d| d.child(self.render_canvas(cx)))
                        .when(!message.is_empty(), |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(hsla(p.subtle))
                                    .child(message),
                            )
                        }),
                )
                .child(self.render_side(cx))
        };
        let panel = div()
            .child(modal_header(title, None, cx))
            .child(div().min_h_0().flex_1().px_5().py_4().child(body));
        modal_frame(&self.focus, 1280.0, panel, cx).into_any_element()
    }
}

fn workspace_path(deck: &Entity<Deck>, workspace_id: &str, cx: &App) -> String {
    deck.read(cx)
        .state
        .workspaces
        .iter()
        .find(|w| w.id == workspace_id)
        .map(|w| w.path.clone())
        .unwrap_or_default()
}

/// Snapshot of what one frame draws.
struct Scene {
    nodes: Vec<(f32, f32)>,
    edges: Vec<(usize, usize)>,
    lit: Vec<bool>,
    hub: Vec<bool>,
    radius: Vec<f32>,
    titles: Vec<SharedString>,
    focus: Option<usize>,
    view: ViewTransform,
    palette: Palette,
}

impl Scene {
    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let p = &self.palette;
        let v = self.view;
        let center = bounds.center();
        let screen = |(x, y): (f32, f32)| {
            point(center.x + px(v.x + x * v.k), center.y + px(v.y + y * v.k))
        };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            // Edges: dim, then the focused node's links on top.
            for active in [false, true] {
                let mut path = PathBuilder::stroke(px(1.0));
                let mut any = false;
                for &(a, b) in &self.edges {
                    let on = self.focus.is_some_and(|f| a == f || b == f);
                    if on != active {
                        continue;
                    }
                    path.move_to(screen(self.nodes[a]));
                    path.line_to(screen(self.nodes[b]));
                    any = true;
                }
                if !any {
                    continue;
                }
                let alpha = if self.focus.is_none() || active { 0.7 } else { 0.15 };
                let color = if active { p.ok } else { p.line_strong };
                if let Ok(path) = path.build() {
                    window.paint_path(path, hsla_a(color, alpha));
                }
            }
            let font_size = px(12.0);
            for (i, &node) in self.nodes.iter().enumerate() {
                let alpha = if self.lit[i] { 1.0 } else { 0.2 };
                let color = if Some(i) == self.focus {
                    p.ok
                } else if self.hub[i] {
                    p.fg
                } else {
                    p.subtle
                };
                let r = self.radius[i] * v.k;
                let at = screen(node);
                window.paint_quad(
                    fill(Bounds::new(at - point(px(r), px(r)), size(px(2.0 * r), px(2.0 * r))), hsla_a(color, alpha))
                        .corner_radii(px(r)),
                );
                // Labels when zoomed in, or for the focused note and its neighbors.
                if v.k > 1.2 || (self.focus.is_some() && self.lit[i]) {
                    let title = self.titles[i].clone();
                    let run = TextRun {
                        len: title.len(),
                        font: font(FONT_SANS),
                        color: hsla(p.muted),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let line = window.text_system().shape_line(title, font_size, &[run], None);
                    let origin = point(at.x - line.width / 2.0, at.y + px(r) + px(2.0));
                    let _ = line.paint(origin, px(14.0), TextAlign::Left, None, window, cx);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's glob exports its own `test` attribute.
    use super::{initial_positions, step};

    #[test]
    fn pulls_linked_notes_together_and_pushes_the_others_away() {
        let mut nodes = initial_positions(4);
        let edges = [(0, 1)];
        for i in 0..300 {
            step(&mut nodes, &edges, 1.0 - i as f32 / 300.0);
        }
        let dist = |a: usize, b: usize| (nodes[a].x - nodes[b].x).hypot(nodes[a].y - nodes[b].y);
        assert!(dist(0, 1) < dist(2, 3));
        assert!(nodes.iter().all(|n| n.x.is_finite() && n.y.is_finite()));
    }
}
