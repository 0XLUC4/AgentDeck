//! Paints the terminal grid: background runs, selection, text shaped per style run and pinned to
//! its cell column, box/block glyphs as rectangles, and the cursor.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::point_to_viewport;
use alacritty_terminal::vte::ansi::CursorShape;
use gpui_kit::*;

use super::{Metrics, TerminalView, colors, glyphs};
use crate::theme::hsla;

/// Text style of a run of cells.
#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: u32,
    bold: bool,
    italic: bool,
    underline: bool,
    undercurl: bool,
    strike: bool,
}

/// One run of same-style text starting at a cell.
struct Run {
    row: usize,
    col: usize,
    text: String,
    style: Style,
}

enum CursorPaint {
    /// Outline (unfocused terminal or hollow block).
    Hollow(Bounds<Pixels>),
    /// Filled bar (beam or underline).
    Bar(Bounds<Pixels>),
}

pub(super) struct Frame {
    area: Bounds<Pixels>,
    rects: Vec<(Bounds<Pixels>, Hsla)>,
    glyphs: Vec<(char, Bounds<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    cursor: Option<CursorPaint>,
    /// IME composition text at the cursor cell.
    preedit: Option<(Point<Pixels>, ShapedLine)>,
    cell_height: Pixels,
    font_size: Pixels,
    focus: FocusHandle,
}

pub(super) fn terminal_canvas(view: Entity<TerminalView>, pad_left: f32, pad_y: f32) -> impl IntoElement {
    let paint_view = view.clone();
    canvas(
        move |bounds, window, cx| prepaint(&view, bounds, pad_left, pad_y, window, cx),
        move |_, frame, window, cx| paint(&paint_view, frame, window, cx),
    )
    .size_full()
}

/// Cell size of the font: advance of "m" and ascent + descent, rounded to device pixels.
fn cell_size(font: &Font, font_size: Pixels, scale: f32, cx: &App) -> Size<Pixels> {
    let text = cx.text_system();
    let id = text.resolve_font(font);
    let width = text
        .advance(id, font_size, 'm')
        .map(|s| s.width)
        .unwrap_or(font_size * 0.6);
    let height = text.ascent(id, font_size) + text.descent(id, font_size).abs();
    let snap = |v: Pixels| px(((f32::from(v) * scale).round() / scale).max(1.0));
    size(snap(width), snap(height))
}

fn prepaint(
    view: &Entity<TerminalView>,
    bounds: Bounds<Pixels>,
    pad_left: f32,
    pad_y: f32,
    window: &mut Window,
    cx: &mut App,
) -> Frame {
    let (family, font_size) = {
        let v = view.read(cx);
        (v.font_family.clone(), px(v.font_size))
    };
    let base_font = font(family);
    let cell = cell_size(&base_font, font_size, window.scale_factor(), cx);
    let area = Bounds::from_corners(
        point(bounds.origin.x + px(pad_left), bounds.origin.y + px(pad_y)),
        point(bounds.right(), bounds.bottom() - px(pad_y)),
    );
    let cols = ((f32::from(area.size.width) / f32::from(cell.width)) as usize).max(2);
    let rows = ((f32::from(area.size.height) / f32::from(cell.height)) as usize).max(1);
    let hidden = area.size.width <= px(0.0) || area.size.height <= px(0.0);
    view.update(cx, |v, cx| {
        // Hidden panes keep their size (no reflow of background tabs).
        if !hidden {
            v.resize_to(cols, rows, Metrics { origin: area.origin, cell }, cx);
        }
    });

    let theme = colors::theme(cx);
    let v = view.read(cx);
    let focus = v.focus_handle(cx);
    let term = &v.term;
    let content = term.renderable_content();
    let offset = content.display_offset;
    let selection = content.selection;
    let cursor_point = point_to_viewport(offset, content.cursor.point);
    let cursor_shape = content.cursor.shape;
    let show_cursor = cursor_shape != CursorShape::Hidden && (!v.focused || v.blink_on);
    let solid_cursor = v.focused && matches!(cursor_shape, CursorShape::Block);
    let screen_cols = term.columns();

    let cell_bounds = |row: usize, col: usize, width: usize| {
        Bounds::new(
            point(area.origin.x + cell.width * col as f32, area.origin.y + cell.height * row as f32),
            size(cell.width * width as f32, cell.height),
        )
    };

    let mut rects: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
    let mut glyph_cells = Vec::new();
    let mut runs: Vec<Run> = Vec::new();
    // Background run being extended on the current row: (row, start col, end col, color).
    let mut bg_run: Option<(usize, usize, usize, u32)> = None;
    let flush_bg = |run: Option<(usize, usize, usize, u32)>, rects: &mut Vec<(Bounds<Pixels>, Hsla)>| {
        if let Some((row, start, end, color)) = run {
            rects.push((cell_bounds(row, start, end - start), hsla(color)));
        }
    };

    for indexed in content.display_iter {
        let row = (indexed.point.line.0 + offset as i32) as usize;
        let col = indexed.point.column.0;
        let cell = indexed.cell;
        if row >= rows || col >= screen_cols {
            continue;
        }
        if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
        let bold = cell.flags.contains(Flags::BOLD);
        let mut fg = colors::resolve(cell.fg, &content.colors, bold, theme);
        let mut bg = colors::resolve(cell.bg, &content.colors, false, theme);
        if cell.flags.contains(Flags::DIM) {
            fg = colors::dim(fg, theme);
        }
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if selection.is_some_and(|s| s.contains(indexed.point)) {
            bg = theme.selection;
        }
        let at_cursor = show_cursor && solid_cursor && cursor_point == Some(alac_point(row, col));
        if at_cursor {
            bg = theme.cursor;
            fg = theme.background;
        }

        // Background: merge equal neighbours, skip the default background (already painted).
        let extends = matches!(bg_run, Some((r, _, end, c)) if r == row && end == col && c == bg);
        if extends {
            if let Some(run) = bg_run.as_mut() {
                run.2 = col + width;
            }
        } else {
            flush_bg(bg_run.take(), &mut rects);
            if bg != theme.background {
                bg_run = Some((row, col, col + width, bg));
            }
        }

        if cell.flags.contains(Flags::HIDDEN) || cell.c == ' ' || cell.c == '\t' {
            continue;
        }
        let code = cell.c as u32;
        if (0x2500..=0x259f).contains(&code) {
            glyph_cells.push((cell.c, cell_bounds(row, col, 1), hsla(fg)));
            continue;
        }
        let style = Style {
            fg,
            bold,
            italic: cell.flags.contains(Flags::ITALIC),
            underline: cell.flags.intersects(Flags::ALL_UNDERLINES & !Flags::UNDERCURL),
            undercurl: cell.flags.contains(Flags::UNDERCURL),
            strike: cell.flags.contains(Flags::STRIKEOUT),
        };
        let mut text = cell.c.to_string();
        for c in cell.zerowidth().into_iter().flatten() {
            text.push(*c);
        }
        // ASCII runs stay together; anything else gets its own run so fallback fonts and wide
        // chars cannot push the following columns out of the grid.
        let ascii = cell.c.is_ascii() && width == 1 && text.len() == 1;
        match runs.last_mut() {
            Some(run)
                if ascii
                    && run.row == row
                    && run.style == style
                    && run.text.is_ascii()
                    && run.col + run.text.chars().count() == col =>
            {
                run.text.push(cell.c)
            }
            _ => runs.push(Run { row, col, text, style }),
        }
    }
    flush_bg(bg_run.take(), &mut rects);

    let lines = runs
        .into_iter()
        .map(|run| {
            let mut f = base_font.clone();
            if run.style.bold {
                f.weight = FontWeight::BOLD;
            }
            if run.style.italic {
                f.style = FontStyle::Italic;
            }
            let color = hsla(run.style.fg);
            let underline = (run.style.underline || run.style.undercurl).then_some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(color),
                wavy: run.style.undercurl,
            });
            let strikethrough = run.style.strike.then_some(StrikethroughStyle { thickness: px(1.0), color: Some(color) });
            let text_run = TextRun {
                len: run.text.len(),
                font: f,
                color,
                background_color: None,
                underline,
                strikethrough,
            };
            let origin = cell_bounds(run.row, run.col, 1).origin;
            let shaped = window
                .text_system()
                .shape_line(SharedString::from(run.text), font_size, &[text_run], None);
            (origin, shaped)
        })
        .collect();

    let cursor = cursor_point
        .filter(|p| show_cursor && !solid_cursor && p.line < rows && p.column.0 < screen_cols)
        .map(|p| {
            let b = cell_bounds(p.line, p.column.0, 1);
            let bar = px(2.0);
            match cursor_shape {
                CursorShape::Beam => CursorPaint::Bar(Bounds::new(b.origin, size(bar, b.size.height))),
                CursorShape::Underline => CursorPaint::Bar(Bounds::new(
                    point(b.origin.x, b.bottom() - bar),
                    size(b.size.width, bar),
                )),
                _ => CursorPaint::Hollow(b),
            }
        });

    let preedit = v.preedit.clone().zip(cursor_point.filter(|p| p.line < rows)).map(|(text, p)| {
        let run = TextRun {
            len: text.len(),
            font: base_font.clone(),
            color: hsla(theme.foreground),
            background_color: Some(hsla(theme.background)),
            underline: Some(UnderlineStyle { thickness: px(1.0), color: None, wavy: false }),
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(SharedString::from(text), font_size, &[run], None);
        (cell_bounds(p.line, p.column.0, 1).origin, shaped)
    });

    Frame { area, rects, glyphs: glyph_cells, lines, cursor, preedit, cell_height: cell.height, font_size, focus }
}

fn alac_point(row: usize, col: usize) -> alacritty_terminal::index::Point<usize> {
    alacritty_terminal::index::Point::new(row, alacritty_terminal::index::Column(col))
}

fn paint(view: &Entity<TerminalView>, frame: Frame, window: &mut Window, cx: &mut App) {
    window.handle_input(&frame.focus, ElementInputHandler::new(frame.area, view.clone()), cx);
    let theme = colors::theme(cx);
    window.paint_quad(fill(frame.area, hsla(theme.background)));
    for (bounds, color) in frame.rects {
        window.paint_quad(fill(bounds, color));
    }
    for (origin, line) in &frame.lines {
        if let Err(e) = line.paint(*origin, frame.cell_height, TextAlign::Left, None, window, cx) {
            eprintln!("terminal text paint: {e}");
        }
    }
    for (c, bounds, color) in frame.glyphs {
        if !glyphs::paint(c, bounds, color, frame.font_size, window) {
            // Characters without a custom drawing fall back to the font.
            let run = TextRun {
                len: c.len_utf8(),
                font: font(view.read(cx).font_family.clone()),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = window.text_system().shape_line(c.to_string().into(), frame.font_size, &[run], None);
            if let Err(e) = shaped.paint(bounds.origin, frame.cell_height, TextAlign::Left, None, window, cx) {
                eprintln!("terminal glyph paint: {e}");
            }
        }
    }
    let cursor_color = hsla(theme.cursor);
    match frame.cursor {
        Some(CursorPaint::Bar(bounds)) => window.paint_quad(fill(bounds, cursor_color)),
        Some(CursorPaint::Hollow(bounds)) => window.paint_quad(outline(bounds, cursor_color, BorderStyle::Solid)),
        None => {}
    }
    if let Some((origin, line)) = &frame.preedit
        && let Err(e) = line.paint(*origin, frame.cell_height, TextAlign::Left, None, window, cx)
    {
        eprintln!("terminal preedit paint: {e}");
    }
}
