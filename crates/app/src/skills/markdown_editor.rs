//! Visual Markdown editor of the Files page (Obsidian-like live preview). The document is a list
//! of blocks (`agentdeck_core::logic::markdown_blocks`): each one is shown formatted, and the
//! focused one becomes an input holding its inline Markdown, so `**bold**` shows its markers only
//! while the caret is in it. A toolbar changes the block type and wraps the selection.

use std::ops::Range;
use std::path::PathBuf;

use agentdeck_core::logic::markdown_blocks::{self as md, Block, BlockKind, Doc, InlineStyle};
use agentdeck_core::logic::theme::{FONT_MONO, Palette};
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{
    Backspace, Escape, IndentInline, InputEvent, MoveDown, MoveLeft, MoveRight, MoveUp, OutdentInline, Textarea,
    TextareaState,
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{MARKDOWN_CONTEXT, MarkdownBold, MarkdownItalic, MarkdownStrike};
use crate::theme::{hsla, palette, ui};
use agentdeck_core::logic::theme::{RADIUS_MD};

/// Width of the text column.
const COLUMN: f32 = 760.0;
/// Side padding of the input (gpui-component's extra small size), repeated on shown blocks so the
/// text does not move when a block gets the caret.
const INPUT_PX: f32 = 4.0;

/// Text size, weight and line height (times the size) of a block.
fn block_text(kind: BlockKind) -> (f32, FontWeight, f32) {
    match kind {
        BlockKind::Heading(1) => (26.0, FontWeight::BOLD, 1.3),
        BlockKind::Heading(2) => (21.0, FontWeight::BOLD, 1.3),
        BlockKind::Heading(3) => (17.0, FontWeight::SEMIBOLD, 1.4),
        BlockKind::Heading(_) => (15.0, FontWeight::SEMIBOLD, 1.4),
        BlockKind::Code | BlockKind::Raw => (12.5, FontWeight::NORMAL, 1.5),
        _ => (14.0, FontWeight::NORMAL, 1.6),
    }
}

/// Same block type for the toolbar (any task matches the checklist button).
fn same_kind(a: BlockKind, b: BlockKind) -> bool {
    a == b || matches!((a, b), (BlockKind::Task(_), BlockKind::Task(_)))
}

/// Inline Markdown shown formatted: markers hidden, bold, italic, struck, `code` chips, links.
fn inline_text(text: &str, struck: bool, p: Palette) -> StyledText {
    let mut shown = String::new();
    let mut highlights = Vec::new();
    let mut mono = Vec::new();
    for span in md::inline_spans(text) {
        let range = shown.len()..shown.len() + span.text.len();
        shown.push_str(&span.text);
        let style = span.style;
        let mut highlight = HighlightStyle::default();
        if style.bold {
            highlight.font_weight = Some(FontWeight::BOLD);
        }
        if style.italic {
            highlight.font_style = Some(FontStyle::Italic);
        }
        if style.strike || struck {
            highlight.strikethrough = Some(StrikethroughStyle { thickness: px(1.0), color: None });
        }
        if style.code {
            highlight.background_color = Some(hsla(p.raised));
            mono.push((range.clone(), SharedString::from(FONT_MONO)));
        }
        if style.link {
            highlight.color = Some(hsla(p.accent));
            highlight.underline = Some(UnderlineStyle { thickness: px(1.0), color: None, wavy: false });
        }
        if highlight != HighlightStyle::default() {
            highlights.push((range, highlight));
        }
    }
    StyledText::new(shown).with_highlights(highlights).with_font_family_overrides(mono)
}

pub struct MarkdownEditor {
    doc: Doc,
    /// `doc` serialized, refreshed on each change (compared with the saved text for the dirty mark).
    text: String,
    /// Folder of the file: relative image paths start there.
    dir: PathBuf,
    /// The one live input, drawn in place of the focused block.
    input: Entity<TextareaState>,
    focused: Option<usize>,
    /// The "…" tools (code block, divider, table) are shown.
    more: bool,
    /// Focused when no block is (Esc), so the page shortcuts keep working.
    focus: FocusHandle,
    scroll: ScrollHandle,
    _input_events: Subscription,
}

impl MarkdownEditor {
    pub fn new(text: &str, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 400).submit_on_enter(true));
        let input_events = cx.subscribe_in(&input, window, Self::on_input);
        let doc = Doc::parse(text);
        Self {
            text: doc.serialize(),
            doc,
            dir,
            input,
            focused: None,
            more: false,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _input_events: input_events,
        }
    }

    /// The document as Markdown.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the document (switch from source, reload from disk).
    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if text == self.text {
            return;
        }
        self.doc = Doc::parse(text);
        self.text = self.doc.serialize();
        if self.focused.take().is_some() {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Headings as (block index, level, text).
    pub fn outline(&self) -> Vec<(usize, u8, String)> {
        self.doc.headings().into_iter().map(|(i, level, text)| (i, level, text.to_owned())).collect()
    }

    /// Scrolls to block `index` and puts the caret at its end.
    pub fn jump_to(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let end = self.doc.blocks.get(index).map_or(0, |b| b.text.len());
        self.focus_block(index, end..end, window, cx);
        self.scroll.scroll_to_item(index);
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.text = self.doc.serialize();
        cx.notify();
    }

    /// Shows block `index` in the input with `caret` selected, and focuses it.
    fn focus_block(&mut self, index: usize, caret: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(block) = self.doc.blocks.get(index) else { return };
        let (text, multiline) = (block.text.clone(), block.kind.is_multiline());
        self.focused = Some(index);
        self.input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.set_submit_on_enter(!multiline, cx);
            input.set_selected_range(caret, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Puts the focused block's text back in the input with a new selection (undoable).
    fn replace_input(&mut self, index: usize, text: String, selection: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.doc.blocks[index].text = text.clone();
        self.input.update(cx, |input, cx| {
            input.replace_all(text, window, cx);
            input.set_selected_range(selection, cx);
            input.focus(window, cx);
        });
        self.changed(cx);
    }

    fn focused_index(&self) -> Option<usize> {
        self.focused.filter(|&i| i < self.doc.blocks.len())
    }

    fn on_input(&mut self, _: &Entity<TextareaState>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.focused_index() else { return };
        match event {
            InputEvent::Change => {
                let value = self.input.read(cx).value().to_string();
                let block = &mut self.doc.blocks[i];
                if let Some((kind, marker, len)) = md::typed_marker(block.kind, &value) {
                    // "## ", "- ", "1. ", "[] ", "> " typed at the start: the block takes that type.
                    let indent = if kind.is_list() { block.indent().to_owned() } else { String::new() };
                    block.kind = kind;
                    block.prefix = indent + &marker;
                    block.text = value[len..].to_owned();
                    if kind == BlockKind::Numbered {
                        self.doc.renumber(i);
                    }
                    self.focus_block(i, 0..0, window, cx);
                } else {
                    block.text = value;
                }
                self.changed(cx);
            }
            InputEvent::PressEnter { shift: false, .. } if !self.doc.blocks[i].kind.is_multiline() => self.enter(i, window, cx),
            _ => {}
        }
    }

    /// Enter: splits the block at the caret (an empty list item or quote leaves the list instead).
    fn enter(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (mut text, selection) = {
            let input = self.input.read(cx);
            (input.value().to_string(), input.selected_range())
        };
        let kind = self.doc.blocks[i].kind;
        if text.is_empty() && (kind.is_list() || kind == BlockKind::Quote) {
            self.doc.to_paragraph(i);
            self.changed(cx);
            return;
        }
        let next = if kind.is_text() {
            text.replace_range(selection.clone(), "");
            self.doc.blocks[i].text = text;
            self.doc.split(i, selection.start)
        } else {
            self.doc.insert_after(i, Block::new(BlockKind::Paragraph, ""))
        };
        self.focus_block(next, 0..0, window, cx);
        self.changed(cx);
    }

    /// Backspace with the caret at the start of the block (the input passes it up only then).
    fn backspace_at_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.focused_index() else { return };
        let block = &self.doc.blocks[i];
        let (kind, empty) = (block.kind, block.text.is_empty());
        if kind.is_text() && kind != BlockKind::Paragraph {
            // A heading, list item or quote turns back into a paragraph.
            self.doc.to_paragraph(i);
        } else if kind == BlockKind::Paragraph
            && let Some(join) = self.doc.merge_up(i)
        {
            self.focus_block(i - 1, join..join, window, cx);
        } else if i > 0 && empty {
            self.doc.remove(i);
            let end = self.doc.blocks[i - 1].text.len();
            self.focus_block(i - 1, end..end, window, cx);
        } else if i > 0 && matches!(self.doc.blocks[i - 1].kind, BlockKind::Rule | BlockKind::Image) {
            self.doc.remove(i - 1);
            self.focus_block(i - 1, 0..0, window, cx);
        } else {
            return;
        }
        self.changed(cx);
    }

    /// Arrow at the start (back) or end of the block: moves to the neighbor block. True when moved.
    fn step(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(i) = self.focused_index() else { return false };
        let (cursor, len, selecting) = {
            let input = self.input.read(cx);
            (input.cursor(), input.value().len(), !input.selected_range().is_empty())
        };
        if selecting {
            return false;
        }
        if back && cursor == 0 && i > 0 {
            let end = self.doc.blocks[i - 1].text.len();
            self.focus_block(i - 1, end..end, window, cx);
            return true;
        }
        if !back && cursor == len && i + 1 < self.doc.blocks.len() {
            self.focus_block(i + 1, 0..0, window, cx);
            return true;
        }
        false
    }

    /// Tab / Shift+Tab on a list item: nests it one level deeper or shallower. True when done.
    fn indent(&mut self, deeper: bool, cx: &mut Context<Self>) -> bool {
        let Some(i) = self.focused_index().filter(|&i| self.doc.blocks[i].kind.is_list()) else { return false };
        let prefix = &mut self.doc.blocks[i].prefix;
        if deeper {
            prefix.insert_str(0, "  ");
        } else if prefix.starts_with("  ") {
            prefix.drain(..2);
        } else if prefix.starts_with('\t') {
            prefix.remove(0);
        }
        self.changed(cx);
        true
    }

    /// Toolbar block type: applies it to the focused block, or back to a paragraph when it has it.
    fn set_kind(&mut self, kind: BlockKind, cx: &mut Context<Self>) {
        let Some(i) = self.focused_index() else { return };
        let current = self.doc.blocks[i].kind;
        if !current.is_text() || current == BlockKind::Paragraph && kind == BlockKind::Paragraph {
            return;
        }
        if kind == BlockKind::Paragraph || same_kind(current, kind) {
            self.doc.to_paragraph(i);
        } else {
            self.doc.blocks[i].set_kind(kind);
            if kind == BlockKind::Numbered {
                self.doc.renumber(i);
            }
        }
        self.changed(cx);
    }

    /// Bold, italic or strike on the selection (or the word at the caret).
    fn wrap(&mut self, marker: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.focused_index().filter(|&i| self.doc.blocks[i].kind.is_text()) else { return };
        let (value, selection) = {
            let input = self.input.read(cx);
            (input.value().to_string(), input.selected_range())
        };
        let (text, selection) = md::toggle_wrap(&value, selection, marker);
        self.replace_input(i, text, selection, window, cx);
    }

    fn link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.focused_index().filter(|&i| self.doc.blocks[i].kind.is_text()) else { return };
        let (value, selection) = {
            let input = self.input.read(cx);
            (input.value().to_string(), input.selected_range())
        };
        let (text, selection) = md::insert_link(&value, selection);
        self.replace_input(i, text, selection, window, cx);
    }

    /// Puts `block` below the focused block (or in place of an empty focused paragraph, or at the
    /// end) and focuses it with `caret` selected.
    fn insert(&mut self, block: Block, caret: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let index = match self.focused_index() {
            Some(i) if self.doc.blocks[i].kind == BlockKind::Paragraph && self.doc.blocks[i].text.is_empty() => {
                let target = &mut self.doc.blocks[i];
                (target.kind, target.prefix, target.text) = (block.kind, block.prefix, block.text);
                i
            }
            Some(i) => self.doc.insert_after(i, block),
            None => self.doc.insert_after(self.doc.blocks.len().saturating_sub(1), block),
        };
        self.focus_block(index, caret, window, cx);
        self.changed(cx);
    }

    /// Click below the last block: caret at the end of the text, or a new paragraph.
    fn click_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.doc.blocks.last() {
            Some(last) if last.kind.is_text() => {
                let (index, end) = (self.doc.blocks.len() - 1, last.text.len());
                self.focus_block(index, end..end, window, cx);
            }
            _ => self.insert_paragraph_at_end(window, cx),
        }
    }

    fn insert_paragraph_at_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.doc.insert_after(self.doc.blocks.len().saturating_sub(1), Block::new(BlockKind::Paragraph, ""));
        self.focus_block(index, 0..0, window, cx);
        self.changed(cx);
    }

    fn toggle_task(&mut self, index: usize, cx: &mut Context<Self>) {
        self.doc.toggle_task(index);
        self.changed(cx);
    }
}

impl MarkdownEditor {
    /// A small toolbar button; acts on mouse down without taking the focus from the block.
    fn tool(
        &self,
        id: impl Into<ElementId>,
        icon: IconName,
        tip: &'static str,
        active: bool,
        cx: &mut Context<Self>,
        run: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        let p = palette(cx);
        div()
            .id(id)
            .size(ui(26.0))
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .rounded(ui(RADIUS_MD))
            .text_size(ui(14.0))
            .cursor_pointer()
            .when(active, |d| d.bg(hsla(p.raised)).text_color(hsla(p.fg)))
            .when(!active, |d| d.text_color(hsla(p.muted)).hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg))))
            .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    run(this, window, cx);
                }),
            )
            .child(icon)
    }

    fn toolbar(&self, p: Palette, cx: &mut Context<Self>) -> Div {
        let block = self.focused_index().map(|i| &self.doc.blocks[i]);
        let kind = block.map(|b| b.kind);
        let inline = block
            .filter(|b| b.kind.is_text())
            .map_or_else(InlineStyle::default, |b| md::style_at(&b.text, self.input.read(cx).cursor()));
        let separator = || div().flex_shrink_0().w(px(1.0)).h(ui(16.0)).mx_1().bg(hsla(p.line));
        let kinds = [
            (IconName::Pilcrow, "Paragraph", BlockKind::Paragraph),
            (IconName::Heading1, "Heading 1", BlockKind::Heading(1)),
            (IconName::Heading2, "Heading 2", BlockKind::Heading(2)),
            (IconName::Heading3, "Heading 3", BlockKind::Heading(3)),
        ];
        let lists = [
            (IconName::List, "Bulleted list", BlockKind::Bullet),
            (IconName::ListOrdered, "Numbered list", BlockKind::Numbered),
            (IconName::ListChecks, "Checklist", BlockKind::Task(false)),
            (IconName::TextQuote, "Quote", BlockKind::Quote),
        ];
        let marks = [
            (IconName::Bold, "Bold (Ctrl+B)", "**", inline.bold),
            (IconName::Italic, "Italic (Ctrl+I)", "*", inline.italic),
            (IconName::Strikethrough, "Strikethrough (Ctrl+Shift+X)", "~~", inline.strike),
        ];
        let kind_tool = |n: usize, (icon, tip, target): (IconName, &'static str, BlockKind), cx: &mut Context<Self>| {
            let active = kind.is_some_and(|k| same_kind(k, target));
            self.tool(("md-kind", n), icon, tip, active, cx, move |this, _, cx| this.set_kind(target, cx))
        };
        let mut bar = div()
            .flex()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            .gap(px(2.0))
            .px_3()
            .py(ui(5.0))
            .border_b_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.panel));
        for (n, item) in kinds.into_iter().enumerate() {
            bar = bar.child(kind_tool(n, item, cx));
        }
        bar = bar.child(separator());
        for (n, (icon, tip, marker, active)) in marks.into_iter().enumerate() {
            bar = bar.child(self.tool(("md-mark", n), icon, tip, active, cx, move |this, window, cx| this.wrap(marker, window, cx)));
        }
        bar = bar.child(separator());
        for (n, item) in lists.into_iter().enumerate() {
            bar = bar.child(kind_tool(n + 10, item, cx));
        }
        bar = bar
            .child(separator())
            .child(self.tool("md-link", IconName::Link, "Link", false, cx, |this, window, cx| this.link(window, cx)))
            .child(self.tool("md-image", IconName::Image, "Image", false, cx, |this, window, cx| {
                this.insert(Block::new(BlockKind::Image, "![](image.png)"), 4..13, window, cx)
            }))
            .child(self.tool("md-more", IconName::Ellipsis, "More blocks", self.more, cx, |this, _, cx| {
                this.more = !this.more;
                cx.notify();
            }));
        bar.when(self.more, |bar| {
            bar.child(self.tool("md-code", IconName::SquareCode, "Code block", false, cx, |this, window, cx| {
                this.insert(Block::new(BlockKind::Code, "```\n\n```"), 4..4, window, cx)
            }))
            .child(self.tool("md-rule", IconName::Minus, "Divider", false, cx, |this, window, cx| {
                this.insert(Block::new(BlockKind::Rule, "---"), 3..3, window, cx)
            }))
            .child(self.tool("md-table", IconName::Table, "Table", false, cx, |this, window, cx| {
                this.insert(Block::new(BlockKind::Raw, "| Column | Column |\n| --- | --- |\n|  |  |"), 2..8, window, cx)
            }))
        })
    }

    /// The block's content when it does not have the caret.
    fn shown(&self, block: &Block, p: Palette) -> AnyElement {
        match block.kind {
            BlockKind::Rule => div().w_full().py(ui(8.0)).child(div().h(px(1.0)).bg(hsla(p.line_strong))).into_any_element(),
            BlockKind::Code => div().child(md::code_parts(&block.text).1).into_any_element(),
            BlockKind::Raw => div().child(block.text.clone()).into_any_element(),
            BlockKind::Image => match md::image_source(&block.text) {
                Some((_, src)) => {
                    let source: ImageSource = if src.starts_with("http://") || src.starts_with("https://") {
                        src.clone().into()
                    } else {
                        self.dir.join(&src).into()
                    };
                    img(source)
                        .max_w_full()
                        .max_h(ui(360.0))
                        .object_fit(ObjectFit::Contain)
                        .with_fallback(move || {
                            div().text_color(hsla(p.bad)).child(format!("Image not found: {src}")).into_any_element()
                        })
                        .into_any_element()
                }
                None => inline_text(&block.text, false, p).into_any_element(),
            },
            BlockKind::Heading(_) => inline_text(md::heading_title(&block.text), false, p).into_any_element(),
            kind => inline_text(&block.text, kind == BlockKind::Task(true), p).into_any_element(),
        }
    }

    fn render_block(&self, i: usize, p: Palette, cx: &mut Context<Self>) -> AnyElement {
        let block = &self.doc.blocks[i];
        let (size, weight, line) = block_text(block.kind);
        let line_px = size * line;
        let marker_box = || div().flex_shrink_0().w(ui(22.0)).h(ui(line_px)).flex().items_center().text_color(hsla(p.muted));
        let marker = match block.kind {
            BlockKind::Bullet => Some(marker_box().child("•").into_any_element()),
            BlockKind::Numbered => Some(marker_box().text_size(ui(13.0)).child(block.prefix.trim().to_owned()).into_any_element()),
            BlockKind::Task(checked) => Some(
                marker_box()
                    .id(("md-check", i))
                    .cursor_pointer()
                    .text_size(ui(15.0))
                    .when(checked, |d| d.text_color(hsla(p.accent)))
                    .child(if checked { IconName::SquareCheckBig } else { IconName::Square })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.toggle_task(i, cx);
                        }),
                    )
                    .into_any_element(),
            ),
            _ => None,
        };
        let mono = block.kind.is_multiline();
        let frame = div().flex_1().min_w_0();
        let frame = match block.kind {
            BlockKind::Quote => frame.border_l_2().border_color(hsla(p.line_strong)).pl_2().text_color(hsla(p.muted)),
            BlockKind::Code => frame.rounded(ui(RADIUS_MD)).bg(hsla(p.raised)).px_2().py_2(),
            BlockKind::Raw => frame.rounded(ui(RADIUS_MD)).border_1().border_color(hsla(p.line)).px_2().py_1().text_color(hsla(p.muted)),
            BlockKind::Task(true) => frame.text_color(hsla(p.subtle)),
            _ => frame,
        };
        let content = if self.focused == Some(i) {
            Textarea::new(&self.input)
                .appearance(false)
                .xsmall()
                .w_full()
                .text_size(ui(size))
                .font_weight(weight)
                .line_height(relative(line))
                .when(mono, |t| t.font_family(FONT_MONO))
                .into_any_element()
        } else {
            div()
                .px(px(INPUT_PX))
                .min_h(ui(line_px))
                .cursor_text()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        window.prevent_default();
                        let end = this.doc.blocks.get(i).map_or(0, |b| b.text.len());
                        this.focus_block(i, end..end, window, cx);
                    }),
                )
                .child(self.shown(block, p))
                .into_any_element()
        };
        let gap = if block.blank_after.is_empty() { 2.0 } else { 10.0 };
        div()
            .id(("md-block", i))
            .w_full()
            .max_w(ui(COLUMN))
            .flex()
            .items_start()
            .mb(ui(gap))
            .when(matches!(block.kind, BlockKind::Heading(1 | 2)) && i > 0, |d| d.mt(ui(8.0)))
            .pl(ui(22.0 * block.depth() as f32))
            .text_size(ui(size))
            .font_weight(weight)
            .line_height(relative(line))
            .when(mono, |d| d.font_family(FONT_MONO))
            .children(marker)
            .child(frame.child(content))
            .into_any_element()
    }
}

impl Render for MarkdownEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let blocks: Vec<AnyElement> = (0..self.doc.blocks.len()).map(|i| self.render_block(i, p, cx)).collect();
        let empty = self.doc.blocks.is_empty();
        div()
            .id("md-editor")
            .track_focus(&self.focus)
            .key_context(MARKDOWN_CONTEXT)
            .on_action(cx.listener(|this, _: &MarkdownBold, window, cx| this.wrap("**", window, cx)))
            .on_action(cx.listener(|this, _: &MarkdownItalic, window, cx| this.wrap("*", window, cx)))
            .on_action(cx.listener(|this, _: &MarkdownStrike, window, cx| this.wrap("~~", window, cx)))
            // The input passes Backspace up only with the caret at its start.
            .on_action(cx.listener(|this, _: &Backspace, window, cx| this.backspace_at_start(window, cx)))
            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                if this.step(true, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveLeft, window, cx| {
                if this.step(true, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                if this.step(false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveRight, window, cx| {
                if this.step(false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, _, cx| {
                if this.indent(true, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &OutdentInline, _, cx| {
                if this.indent(false, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.focused.take().is_some() {
                    this.focus.focus(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(hsla(p.base))
            .text_color(hsla(p.fg))
            .child(self.toolbar(p, cx))
            .child(
                div()
                    .id("md-blocks")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .items_center()
                    .px_6()
                    .pt_5()
                    .children(blocks)
                    // Below the last block: a click continues the text.
                    .child(
                        div()
                            .id("md-end")
                            .w_full()
                            .max_w(ui(COLUMN))
                            .min_h(ui(160.0))
                            .px(px(INPUT_PX))
                            .cursor_text()
                            .text_color(hsla(p.subtle))
                            .when(empty, |d| d.child("Empty file. Click to write."))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    window.prevent_default();
                                    this.click_end(window, cx);
                                }),
                            ),
                    ),
            )
    }
}
