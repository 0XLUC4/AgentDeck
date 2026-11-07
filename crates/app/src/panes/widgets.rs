//! Small shared pieces: the square icon button (IconButton.tsx) and the status dot / badge (StatusBadge.tsx).

use std::rc::Rc;
use std::time::Duration;

use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::theme::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{status_color, status_label};
use crate::theme::{hsla, hsla_a, palette, ui};

type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Square icon-only button: every pane action goes through it, so they all look and behave the same.
/// Clicks never reach the header below (no focus/maximize side effects).
pub struct IconButton {
    id: ElementId,
    icon: IconName,
    hint: SharedString,
    danger: bool,
    large: bool,
    pressed: bool,
    disabled: bool,
    on_click: Option<Handler>,
    on_right_click: Option<Handler>,
}

/// `hint` is the tooltip (label, shortcut, consequence).
pub fn icon_button(id: impl Into<ElementId>, icon: IconName, hint: impl Into<SharedString>) -> IconButton {
    IconButton {
        id: id.into(),
        icon,
        hint: hint.into(),
        danger: false,
        large: false,
        pressed: false,
        disabled: false,
        on_click: None,
        on_right_click: None,
    }
}

impl IconButton {
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    /// 28 px box and 16 px icon instead of 24 / 14.
    pub fn large(mut self) -> Self {
        self.large = true;
        self
    }

    pub fn pressed(mut self, pressed: bool) -> Self {
        self.pressed = pressed;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }

    pub fn on_right_click(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_right_click = Some(Rc::new(f));
        self
    }
}

impl IntoElement for IconButton {
    type Element = ViewElement<Self>;

    fn into_element(self) -> Self::Element {
        ViewElement::new(self)
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let (box_size, icon_size) = if self.large { (28.0, 16.0) } else { (24.0, 14.0) };
        let (hover_bg, hover_fg) = if self.danger {
            (hsla_a(p.bad, 0.15), hsla(p.bad))
        } else {
            (hsla(p.hover), hsla(p.fg))
        };
        let hint = self.hint.clone();
        let mut button = div()
            .id(self.id)
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size(ui(box_size))
            .rounded(ui(agentdeck_core::logic::theme::RADIUS_MD - 4.0))
            .text_color(hsla(if self.pressed { p.fg } else { p.subtle }))
            .when(self.pressed, |b| b.bg(hsla(p.raised)))
            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
            .child(Icon::new(self.icon).size(ui(icon_size)));
        if self.disabled {
            return button.opacity(0.35);
        }
        button = button
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        if let Some(f) = self.on_click {
            button = button.on_click(move |_, window, cx| {
                cx.stop_propagation();
                f(window, cx);
            });
        }
        if let Some(f) = self.on_right_click {
            button = button.on_mouse_down(MouseButton::Right, move |_, window, cx| {
                cx.stop_propagation();
                f(window, cx);
            });
        }
        button
    }
}

/// 8 px status dot; "waiting" pulses (opacity 1 -> 0.3 -> 1 over 1.4 s, ease-in-out), as `.status-dot-pulse`.
pub fn status_dot(key: &str, status: PaneStatus, p: &Palette) -> AnyElement {
    let dot = div()
        .flex_shrink_0()
        .size(ui(8.0))
        .rounded_full()
        .bg(hsla(status_color(status, p)));
    if status != PaneStatus::Waiting {
        return dot.into_any_element();
    }
    dot.with_animation(
        SharedString::from(format!("status-pulse-{key}")),
        Animation::new(Duration::from_millis(1400)).repeat_synced(),
        |dot, t| {
            let half = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
            dot.opacity(1.0 - 0.7 * ease_in_out(half))
        },
    )
    .into_any_element()
}

/// Status dot in a 16 px box; the label is the tooltip.
pub fn status_badge(key: &str, status: PaneStatus, p: &Palette) -> Stateful<Div> {
    let label: SharedString = format!("Status: {}", status_label(status)).into();
    div()
        .id(SharedString::from(format!("status-badge-{key}")))
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(ui(16.0))
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .child(status_dot(key, status, p))
}
