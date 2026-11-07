//! Small form primitives shared by the Settings sections (original: Settings/controls.tsx).

use agentdeck_core::logic::theme::{FONT_MONO, Palette};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::SelectItem;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::{hsla, hsla_a, ui};

/// "Section title" then rows separated by a line.
pub fn section(title: &str, rows: Vec<AnyElement>, p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .mb_1()
                .text_size(ui(15.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(hsla(p.fg))
                .child(title.to_owned()),
        )
        .children(rows.into_iter().enumerate().map(|(i, row)| {
            div()
                .when(i > 0, |d| d.border_t_1().border_color(hsla(p.line)))
                .child(row)
        }))
}

/// One setting: label (+ optional hint) on the left, control on the right.
pub fn row(label: &str, hint: Option<&str>, control: impl IntoElement, p: &Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_6()
        .py_3()
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(ui(2.0))
                .child(div().text_color(hsla(p.fg)).child(label.to_owned()))
                .children(hint.map(|h| hint_text(h, p))),
        )
        .child(div().flex().flex_shrink_0().items_center().gap_2().child(control))
        .into_any_element()
}

pub fn hint_text(text: &str, p: &Palette) -> Div {
    div().text_xs().text_color(hsla(p.subtle)).child(text.to_owned())
}

/// Lucide icon from the full catalog.
pub fn icon(name: IconName, size: f32, color: u32) -> Icon {
    Icon::new(name).size(ui(size)).text_color(hsla(color))
}

/// On/off switch. The caller adds `.on_click`.
pub fn toggle(id: impl Into<ElementId>, checked: bool, p: &Palette) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .h(ui(20.0))
        .w(ui(36.0))
        .px(ui(2.0))
        .rounded_full()
        .border_1()
        .cursor_pointer()
        .when(checked, |d| d.justify_end().border_color(hsla(p.accent)).bg(hsla(p.accent)))
        .when(!checked, |d| {
            d.border_color(hsla(p.line_strong))
                .bg(hsla(p.base))
                .hover(|s| s.border_color(hsla(p.subtle)))
        })
        .child(
            div()
                .size(ui(14.0))
                .rounded_full()
                .bg(hsla(if checked { p.on_accent } else { p.subtle })),
        )
}

/// Outlined button (h-8, muted text). Disabled buttons are dimmed; the caller skips `.on_click`.
pub fn button(id: impl Into<ElementId>, icon_name: Option<IconName>, label: &str, disabled: bool, p: &Palette) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(ui(6.0))
        .h_8()
        .px_3()
        .rounded_md()
        .border_1()
        .border_color(hsla(p.line))
        .text_color(hsla(p.muted))
        .when(disabled, |d| d.opacity(0.4))
        .when(!disabled, |d| {
            d.cursor_pointer()
                .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
        })
        .children(icon_name.map(|n| icon(n, 14.0, p.muted)))
        .when(!label.is_empty(), |d| d.child(label.to_owned()))
}

/// Square icon-only button (IconButton).
pub fn icon_button(id: impl Into<ElementId>, name: IconName, danger: bool, p: &Palette) -> Stateful<Div> {
    let hover = if danger { p.bad } else { p.fg };
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size_7()
        .rounded_md()
        .cursor_pointer()
        .text_color(hsla(p.muted))
        .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(hover)))
        .child(Icon::new(name).size(ui(14.0)))
}

/// Red alert box.
pub fn error_text(message: &str, p: &Palette) -> Div {
    div()
        .rounded_md()
        .bg(hsla_a(p.bad, 0.1))
        .px_3()
        .py_2()
        .text_xs()
        .text_color(hsla(p.bad))
        .child(message.to_owned())
}

/// Text field of a fixed width (`None` = fills the row).
pub fn field(state: &Entity<InputState>, width: Option<f32>, mono: bool) -> Div {
    div()
        .when_some(width, |d, w| d.w(ui(w)))
        .when(width.is_none(), |d| d.flex_1())
        .min_w_0()
        .when(mono, |d| d.font_family(FONT_MONO).text_xs())
        .child(Input::new(state).h(ui(32.0)))
}

/// Segmented control (Usage/Modal `Segmented`).
pub fn segmented<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    value: T,
    p: &Palette,
    on_change: impl Fn(T, &mut Window, &mut App) + 'static,
) -> Div {
    let on_change = std::rc::Rc::new(on_change);
    div()
        .flex()
        .p(ui(2.0))
        .gap(ui(2.0))
        .rounded_md()
        .border_1()
        .border_color(hsla(p.line))
        .children(options.iter().enumerate().map(|(i, &(option, label))| {
            let on_change = on_change.clone();
            let selected = option == value;
            div()
                .id((id, i))
                .px_3()
                .h(ui(26.0))
                .flex()
                .items_center()
                .rounded(ui(5.0))
                .cursor_pointer()
                .text_xs()
                .when(selected, |d| d.bg(hsla(p.raised)).text_color(hsla(p.fg)))
                .when(!selected, |d| {
                    d.text_color(hsla(p.muted)).hover(|s| s.text_color(hsla(p.fg)))
                })
                .on_click(move |_, window, cx| on_change(option, window, cx))
                .child(label)
        }))
}

/// Item of a Select: shows `label`, identified by `value`.
#[derive(Clone)]
pub struct Opt {
    pub value: SharedString,
    pub label: SharedString,
}

impl Opt {
    pub fn new(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

impl SelectItem for Opt {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &SharedString {
        &self.value
    }
}

/// Native yes/no question (window.confirm in the original).
pub fn confirm(question: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title("AgentDeck")
        .set_description(question)
        .set_buttons(rfd::MessageButtons::YesNo)
        .set_level(rfd::MessageLevel::Warning)
        .show()
        == rfd::MessageDialogResult::Yes
}

/// Number field rule shared by the minute and port fields: empty or 0 means off, else clamped.
pub fn clamp_number(raw: &str, min: u32, max: u32) -> u32 {
    let n = raw.trim().parse::<f64>().unwrap_or(0.0).round();
    if n <= 0.0 { 0 } else { (n as u32).clamp(min, max) }
}

/// "#rrggbb" of a color.
pub fn to_hex(color: Hsla) -> String {
    let c = Rgba::from(color);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn number_field_rule() {
        assert_eq!(clamp_number("", 0, 1440), 0);
        assert_eq!(clamp_number("abc", 0, 1440), 0);
        assert_eq!(clamp_number("0", 1024, 65000), 0);
        assert_eq!(clamp_number("5.6", 0, 1440), 6);
        assert_eq!(clamp_number("99999", 0, 1440), 1440);
        assert_eq!(clamp_number("80", 1024, 65000), 1024);
        assert_eq!(clamp_number("-3", 0, 600), 0);
    }

    #[test]
    fn hex_of_color() {
        assert_eq!(to_hex(rgb(0x4f8ff7).into()), "#4f8ff7");
    }
}
