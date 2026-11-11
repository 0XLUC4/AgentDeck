//! Rendering of a session's dock (Deck.tsx + dockview): split branches with invisible resize
//! sashes, groups whose header is the pane header (so it is the drag handle), stacked panes as
//! compact tabs, drop zones while a pane is dragged, maximize, detached placeholder.

use agentdeck_core::logic::theme::RADIUS_XL;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::DeckView;
use super::chrome::icon_button;
use super::dock::{Axis, Branch, Dock, Edge, Group, Node};
use crate::panes::{display_title, exit_bar, pane_frame, pane_frame_border, pane_overlays, steps_bar};
use crate::theme::{hsla, hsla_a, palette, ui};

/// Gap between panes (dockview theme `gap: 6`); the sash lives in it.
const GAP: f32 = 6.0;
const HEADER_HEIGHT: f32 = 30.0;

/// A pane header being dragged to another group or side.
#[derive(Clone)]
pub(super) struct PaneDrag {
    tab_id: String,
    view: String,
    label: SharedString,
}

impl Render for PaneDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        div()
            .px_3()
            .py_1()
            .rounded(ui(8.0))
            .bg(hsla(p.raised))
            .border_1()
            .border_color(hsla(p.accent))
            .text_color(hsla(p.fg))
            .text_size(ui(13.0))
            .child(self.label.clone())
    }
}

/// The sash after child `index` of the branch at `path` being dragged.
#[derive(Clone)]
struct SashDrag {
    tab_id: String,
    path: Vec<usize>,
    index: usize,
}

impl Render for SashDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn element_id(parts: impl std::fmt::Display) -> ElementId {
    ElementId::Name(parts.to_string().into())
}

impl DeckView {
    /// The dock of a session tab (the active one: other sessions keep their state but are not drawn).
    pub(super) fn render_dock(&mut self, tab_id: &str, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(dock) = self.docks.get(tab_id).cloned() else {
            return div().into_any_element();
        };
        if let Some(group) = dock
            .maximized
            .as_ref()
            .and_then(|m| dock.groups().into_iter().find(|g| &g.id == m).cloned())
        {
            return self.render_group(tab_id, &group, &dock, window, cx);
        }
        self.render_branch(tab_id, &dock.root, Vec::new(), &dock, window, cx)
    }

    fn render_branch(
        &mut self,
        tab_id: &str,
        branch: &Branch,
        path: Vec<usize>,
        dock: &Dock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut el = div().flex().size_full().min_w_0().min_h_0();
        el = if branch.axis == Axis::Row { el.flex_row() } else { el.flex_col() };
        for (i, (size, node)) in branch.children.iter().enumerate() {
            if i > 0 {
                el = el.child(sash(tab_id, &path, i - 1, branch.axis));
            }
            let mut child_path = path.clone();
            child_path.push(i);
            let content = match node {
                Node::Leaf(group) => self.render_group(tab_id, group, dock, window, cx),
                Node::Branch(inner) => self.render_branch(tab_id, inner, child_path, dock, window, cx),
            };
            let mut cell = div()
                .flex()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .flex_basis(px(0.0))
                .child(content);
            cell.style().flex_grow = Some(*size);
            cell.style().flex_shrink = Some(1.0);
            el = el.child(cell);
        }
        let tab = tab_id.to_owned();
        let axis = branch.axis;
        el.on_drag_move::<SashDrag>(cx.listener(move |this, e: &DragMoveEvent<SashDrag>, _, cx| {
            let drag = e.drag(cx).clone();
            if drag.tab_id != tab || drag.path != path {
                return;
            }
            let (pos, start, len) = match axis {
                Axis::Row => (e.event.position.x, e.bounds.origin.x, e.bounds.size.width),
                Axis::Column => (e.event.position.y, e.bounds.origin.y, e.bounds.size.height),
            };
            let len = f32::from(len);
            if len <= 0.0 {
                return;
            }
            let fraction = f32::from(pos - start) / len;
            if let Some(dock) = this.docks.get_mut(&tab) {
                dock.resize(&drag.path, drag.index, fraction);
                this.layout_changed(&tab, cx);
            }
        }))
        .into_any_element()
    }

    fn render_group(
        &mut self,
        tab_id: &str,
        group: &Group,
        dock: &Dock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let maximized = dock.maximized.as_deref() == Some(group.id.as_str());
        let stacked = group.views.len() > 1;
        for view in &group.views {
            self.sync_header(view, maximized, stacked, cx);
        }
        let body = self.panel_body(tab_id, &group.id, &group.active, window, cx);

        // One agent alone: the pane's own frame (border by status, exit bar, working outline).
        if !stacked && self.deck.read(cx).state.panes.contains_key(&group.active) {
            let header = self.header_handle(tab_id, &group.id, &group.active, cx);
            let deck = self.deck.read(cx);
            if let Some(pane) = deck.state.panes.get(&group.active) {
                return pane_frame(pane, deck, header, body, cx).into_any_element();
            }
        }

        let active_pane = self.deck.read(cx).state.panes.get(&group.active).cloned();
        let header = if stacked {
            self.tab_strip(tab_id, group, cx)
        } else {
            self.browser_header(tab_id, &group.id, &group.active, maximized, cx)
        };
        let border = active_pane.as_ref().map_or(hsla_a(p.base, 0.0), |pane| pane_frame_border(pane, self.deck.read(cx), &p));
        let overlays = match &active_pane {
            Some(pane) => pane_overlays(pane, self.deck.read(cx), cx),
            None => Vec::new(),
        };
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .rounded(ui(RADIUS_XL))
            .overflow_hidden()
            .bg(hsla(p.base))
            .border_1()
            .border_color(border)
            .child(
                div()
                    .flex_shrink_0()
                    .h(ui(HEADER_HEIGHT))
                    .border_b_1()
                    .border_color(hsla(p.line))
                    .child(header),
            )
            .children(active_pane.as_ref().and_then(|pane| steps_bar(pane, cx)))
            .child(div().relative().flex_1().min_h_0().child(body))
            .when_some(active_pane.filter(|pane| pane.exited), |el, pane| {
                el.child(exit_bar(&pane.id, crate::panes::runtime::is_asleep(&pane.id, cx), &p))
            })
            .children(overlays)
            .into_any_element()
    }

    /// Keeps the pane header's dock state (maximized, compact, detached) current; no notify:
    /// the header renders right after, in this frame.
    fn sync_header(&mut self, view: &str, maximized: bool, compact: bool, cx: &mut Context<Self>) {
        let detached = self.detached.contains_key(view);
        if let Some(header) = self.headers.get(view) {
            header.update(cx, |h, _| {
                h.maximized = maximized;
                h.compact = compact;
                h.detached = detached;
            });
        }
    }

    /// The pane header (panes module) wrapped as the drag handle and drop target of its group.
    fn header_handle(&mut self, tab_id: &str, group_id: &str, view: &str, cx: &mut Context<Self>) -> AnyElement {
        match self.headers.get(view).cloned() {
            Some(header) => self
                .drag_handle(tab_id, group_id, view, cx)
                .size_full()
                .child(header)
                .into_any_element(),
            None => div().into_any_element(),
        }
    }

    /// Several panes stacked in one group: one compact header per pane, the visible one highlighted.
    fn tab_strip(&mut self, tab_id: &str, group: &Group, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let mut strip = div()
            .id(element_id(format_args!("tabs-{}", group.id)))
            .flex()
            .size_full()
            .overflow_x_scroll();
        for view in &group.views {
            let visible = *view == group.active;
            let content = match (view.strip_prefix("browser:"), self.headers.get(view).cloned()) {
                (None, Some(header)) => header.into_any_element(),
                _ => browser_label(&p).into_any_element(),
            };
            let select = view.clone();
            strip = strip.child(
                self.drag_handle(tab_id, &group.id, view, cx)
                    .h_full()
                    .flex_shrink_0()
                    .w(ui(220.0))
                    .border_r_1()
                    .border_color(hsla(p.line))
                    .when(!visible, |el| el.opacity(0.7))
                    .child(content)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.select_panel(&select, cx))),
            );
        }
        strip.into_any_element()
    }

    /// Header of a browser panel alone in its group: drag handle + maximize. Its address bar and
    /// close button are part of the panel (browser module).
    fn browser_header(
        &mut self,
        tab_id: &str,
        group_id: &str,
        view: &str,
        maximized: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let (icon, hint) = if maximized {
            (IconName::Minimize2, "Restore (Esc)")
        } else {
            (IconName::Maximize2, "Maximize (double-click the header)")
        };
        let (max_id, click_id) = (view.to_owned(), view.to_owned());
        self.drag_handle(tab_id, group_id, view, cx)
            .size_full()
            .bg(hsla(p.panel))
            .child(div().flex_1().min_w_0().child(browser_label(&p)))
            .child(
                icon_button(element_id(format_args!("max-{view}")), icon, hint, &p)
                    .mr_1()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.toggle_maximize(&max_id, cx);
                    })),
            )
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.click_count() == 2 {
                    this.toggle_maximize(&click_id, cx);
                }
            }))
            .into_any_element()
    }

    /// Drag handle + drop target (into this group, as a tab) shared by every header. Pressing it
    /// focuses the shell (so Esc restores a maximized pane) and selects the panel.
    fn drag_handle(&mut self, tab_id: &str, group_id: &str, view: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette(cx);
        let drag = PaneDrag {
            tab_id: tab_id.to_owned(),
            view: view.to_owned(),
            label: self.panel_label(view, cx).into(),
        };
        let (press, tab, group) = (view.to_owned(), tab_id.to_owned(), group_id.to_owned());
        let focus = self.focus.clone();
        div()
            .id(element_id(format_args!("handle-{view}")))
            .flex()
            .items_center()
            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    window.focus(&focus, cx);
                    this.select_panel(&press, cx);
                }),
            )
            .drag_over::<PaneDrag>(move |style, _, _, _| style.bg(hsla_a(p.accent, 0.12)))
            .on_drop(cx.listener(move |this, drag: &PaneDrag, _, cx| {
                this.drop_pane(&tab, drag, &group, None, cx);
            }))
    }

    /// Body of a panel: terminal, browser, or the detached placeholder; drop zones while dragging.
    fn panel_body(
        &mut self,
        tab_id: &str,
        group_id: &str,
        view: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let content = if let Some(pane_id) = view.strip_prefix("browser:") {
            self.browser(pane_id, window, cx).into_any_element()
        } else if self.detached.contains_key(view) {
            // The detached window draws the terminal; it comes back here on reattach.
            let id = view.to_owned();
            div()
                .flex()
                .size_full()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(hsla(p.muted))
                .child("Detached in a window")
                .child(
                    div()
                        .id(element_id(format_args!("reattach-{view}")))
                        .flex()
                        .items_center()
                        .gap(ui(6.0))
                        .px(ui(10.0))
                        .py_1()
                        .rounded(ui(6.0))
                        .border_1()
                        .border_color(hsla(p.line_strong))
                        .text_color(hsla(p.fg))
                        .text_size(ui(14.0))
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(|s| s.bg(hsla(p.hover)))
                        .child(IconName::SquareArrowDownLeft)
                        .child(div().text_size(ui(13.0)).child("Reattach"))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.reattach(&id, cx))),
                )
                .into_any_element()
        } else {
            self.terminal(view, cx).into_any_element()
        };
        let select = view.to_owned();
        let tab = tab_id.to_owned();
        div()
            .relative()
            .size_full()
            .bg(hsla(p.base))
            // Capture: the terminal handles the press too (focus, selection).
            .capture_any_mouse_down(cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                if this.docks.get_mut(&tab).is_some_and(|d| d.set_active(&select)) {
                    this.focus_pane(&select, cx);
                    cx.notify();
                }
            }))
            .child(content)
            .when(cx.has_active_drag(), |el| el.children(self.drop_zones(tab_id, group_id, cx)))
            .into_any_element()
    }

    /// Drop targets over a group while a pane is dragged: four sides split, the center stacks.
    fn drop_zones(&self, tab_id: &str, group_id: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = palette(cx);
        let zones: [(Option<Edge>, f32, f32, f32, f32); 5] = [
            (Some(Edge::Left), 0.0, 0.0, 0.25, 1.0),
            (Some(Edge::Right), 0.75, 0.0, 0.25, 1.0),
            (Some(Edge::Top), 0.25, 0.0, 0.5, 0.25),
            (Some(Edge::Bottom), 0.25, 0.75, 0.5, 0.25),
            (None, 0.25, 0.25, 0.5, 0.5),
        ];
        zones
            .into_iter()
            .map(|(edge, left, top, width, height)| {
                let (tab, group) = (tab_id.to_owned(), group_id.to_owned());
                div()
                    .absolute()
                    .left(relative(left))
                    .top(relative(top))
                    .w(relative(width))
                    .h(relative(height))
                    .drag_over::<PaneDrag>(move |style, _, _, _| {
                        style
                            .bg(hsla_a(p.accent, 0.12))
                            .border_1()
                            .border_color(hsla(p.accent))
                    })
                    .on_drop(cx.listener(move |this, drag: &PaneDrag, _, cx| {
                        this.drop_pane(&tab, drag, &group, edge, cx);
                    }))
                    .into_any_element()
            })
            .collect()
    }

    fn drop_pane(&mut self, tab_id: &str, drag: &PaneDrag, group: &str, edge: Option<Edge>, cx: &mut Context<Self>) {
        if drag.tab_id != tab_id {
            return;
        }
        let Some(dock) = self.docks.get_mut(tab_id) else { return };
        let before = dock.clone();
        dock.move_view(&drag.view, group, edge);
        dock.set_active(&drag.view);
        if *dock != before {
            self.layout_changed(tab_id, cx);
        }
        self.focus_pane(&drag.view, cx);
    }

    /// Selects a panel of its session (tab click, header press).
    pub(super) fn select_panel(&mut self, view: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_of(view, cx) else { return };
        if self.docks.get_mut(&tab).is_some_and(|d| d.set_active(view)) {
            self.layout_changed(&tab, cx);
        }
        self.focus_pane(view, cx);
    }

    /// Label of a panel for the drag preview.
    fn panel_label(&self, view: &str, cx: &App) -> String {
        if view.starts_with("browser:") {
            return "Browser".into();
        }
        self.deck
            .read(cx)
            .state
            .panes
            .get(view)
            .map(display_title)
            .unwrap_or_else(|| "Agent".into())
    }
}

fn browser_label(p: &agentdeck_core::logic::theme::Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px(ui(10.0))
        .text_size(ui(14.0))
        .text_color(hsla(p.muted))
        .child(IconName::Globe)
        .child(div().text_size(ui(13.0)).child("Browser"))
}

/// Invisible resize handle in the gap between two children; only the cursor shows.
fn sash(tab_id: &str, path: &[usize], index: usize, axis: Axis) -> AnyElement {
    let drag = SashDrag {
        tab_id: tab_id.to_owned(),
        path: path.to_vec(),
        index,
    };
    let el = div()
        .id(element_id(format_args!("sash-{tab_id}-{path:?}-{index}")))
        .flex_none();
    let el = match axis {
        Axis::Row => el.w(px(GAP)).h_full().cursor_col_resize(),
        Axis::Column => el.h(px(GAP)).w_full().cursor_row_resize(),
    };
    el.on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
        .into_any_element()
}
