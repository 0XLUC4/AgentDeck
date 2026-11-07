//! Box drawing (U+2500..U+257F) and block elements (U+2580..U+259F) drawn as rectangles snapped
//! to device pixels, like xterm's WebGL custom glyphs: lines join with no gaps between cells,
//! so the Claude and OpenCode logos and TUI borders look solid.

use gpui_kit::*;

/// Arm weights of U+2500..U+257F, as "up right down left" digits: 0 none, 1 light, 2 heavy, 3 double.
/// "" marks the characters drawn by a special case (dashes, arcs, diagonals).
const BOX: [&str; 128] = [
    "0101", "0202", "1010", "2020", "", "", "", "", "", "", "", "", // 2500..250B
    "0110", "0210", "0120", "0220", "0011", "0012", "0021", "0022", // 250C..2513
    "1100", "1200", "2100", "2200", "1001", "1002", "2001", "2002", // 2514..251B
    "1110", "1210", "2110", "1120", "2120", "2210", "1220", "2220", // 251C..2523
    "1011", "1012", "2011", "1021", "2021", "2012", "1022", "2022", // 2524..252B
    "0111", "0112", "0211", "0212", "0121", "0122", "0221", "0222", // 252C..2533
    "1101", "1102", "1201", "1202", "2101", "2102", "2201", "2202", // 2534..253B
    "1111", "1112", "1211", "1212", "2111", "1121", "2121", "2112", // 253C..2543
    "2211", "1122", "1221", "2212", "1222", "2122", "2221", "2222", // 2544..254B
    "", "", "", "", // 254C..254F dashes
    "0303", "3030", "0310", "0130", "0330", "0013", "0031", "0033", // 2550..2557
    "1300", "3100", "3300", "1003", "3001", "3003", "1310", "3130", // 2558..255F
    "3330", "1013", "3031", "3033", "0313", "0131", "0333", "1303", // 2560..2567
    "3101", "3303", "1313", "3131", "3333", // 2568..256C
    "", "", "", "", "", "", "", // 256D..2573 arcs and diagonals
    "0001", "1000", "0100", "0010", "0002", "2000", "0200", "0020", // 2574..257B
    "0201", "1020", "0102", "2010", // 257C..257F
];

/// Paints `c` into one cell when it is a box or block character. Returns false for other text.
pub fn paint(c: char, cell: Bounds<Pixels>, color: Hsla, font_size: Pixels, window: &mut Window) -> bool {
    let code = c as u32;
    let scale = window.scale_factor();
    let painter = Painter { cell, color, scale, light: light_width(font_size, scale) };
    match code {
        0x2500..=0x257f => painter.box_char(code - 0x2500, window),
        0x2580..=0x259f => {
            painter.block(code, window);
            true
        }
        _ => false,
    }
}

/// Light line width in logical px: a whole number of device pixels, at least one.
fn light_width(font_size: Pixels, scale: f32) -> f32 {
    (f32::from(font_size) * scale / 13.0).round().max(1.0) / scale
}

struct Painter {
    cell: Bounds<Pixels>,
    color: Hsla,
    scale: f32,
    light: f32,
}

impl Painter {
    fn snap(&self, v: f32) -> f32 {
        (v * self.scale).round() / self.scale
    }

    fn x(&self) -> f32 {
        f32::from(self.cell.origin.x)
    }
    fn y(&self) -> f32 {
        f32::from(self.cell.origin.y)
    }
    fn w(&self) -> f32 {
        f32::from(self.cell.size.width)
    }
    fn h(&self) -> f32 {
        f32::from(self.cell.size.height)
    }

    /// Fills the rectangle between two absolute corners, snapped to device pixels.
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, alpha: f32, window: &mut Window) {
        let (x0, y0, x1, y1) = (self.snap(x0), self.snap(y0), self.snap(x1), self.snap(y1));
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let mut color = self.color;
        color.a *= alpha;
        window.paint_quad(fill(
            Bounds::from_corners(point(px(x0), px(y0)), point(px(x1), px(y1))),
            color,
        ));
    }

    /// Fills a fraction of the cell: (left, top, right, bottom) in 0..1.
    fn part(&self, l: f32, t: f32, r: f32, b: f32, window: &mut Window) {
        let (x, y, w, h) = (self.x(), self.y(), self.w(), self.h());
        self.rect(x + l * w, y + t * h, x + r * w, y + b * h, 1.0, window);
    }

    fn block(&self, code: u32, window: &mut Window) {
        let eighth = |n: u32| n as f32 / 8.0;
        match code {
            0x2580 => self.part(0.0, 0.0, 1.0, 0.5, window),
            0x2581..=0x2588 => self.part(0.0, 1.0 - eighth(code - 0x2580), 1.0, 1.0, window),
            0x2589..=0x258f => self.part(0.0, 0.0, eighth(0x2590 - code), 1.0, window),
            0x2590 => self.part(0.5, 0.0, 1.0, 1.0, window),
            0x2591..=0x2593 => {
                let alpha = (code - 0x2590) as f32 * 0.25;
                let (x, y) = (self.x(), self.y());
                self.rect(x, y, x + self.w(), y + self.h(), alpha, window);
            }
            0x2594 => self.part(0.0, 0.0, 1.0, 0.125, window),
            0x2595 => self.part(0.875, 0.0, 1.0, 1.0, window),
            _ => {
                // Quadrants: bit 1 upper left, 2 upper right, 4 lower left, 8 lower right.
                const QUADS: [u8; 10] = [4, 8, 1, 1 | 4 | 8, 1 | 8, 1 | 2 | 4, 1 | 2 | 8, 2, 2 | 4, 2 | 4 | 8];
                let mask = QUADS[(code - 0x2596) as usize];
                for (bit, l, t) in [(1, 0.0, 0.0), (2, 0.5, 0.0), (4, 0.0, 0.5), (8, 0.5, 0.5)] {
                    if mask & bit != 0 {
                        self.part(l, t, l + 0.5, t + 0.5, window);
                    }
                }
            }
        }
    }

    fn box_char(&self, index: u32, window: &mut Window) -> bool {
        let spec = BOX[index as usize].as_bytes();
        if spec.len() == 4 {
            let w = |i: usize| spec[i] - b'0';
            self.arms([w(0), w(1), w(2), w(3)], window);
            return true;
        }
        let code = 0x2500 + index;
        match code {
            // Dashed lines: (horizontal, heavy, dash count).
            0x2504..=0x250b | 0x254c..=0x254f => {
                let (horizontal, heavy, dashes) = match code {
                    0x2504 => (true, false, 3),
                    0x2505 => (true, true, 3),
                    0x2506 => (false, false, 3),
                    0x2507 => (false, true, 3),
                    0x2508 => (true, false, 4),
                    0x2509 => (true, true, 4),
                    0x250a => (false, false, 4),
                    0x250b => (false, true, 4),
                    0x254c => (true, false, 2),
                    0x254d => (true, true, 2),
                    0x254e => (false, false, 2),
                    _ => (false, true, 2),
                };
                self.dashes(horizontal, if heavy { 2 } else { 1 }, dashes, window);
                true
            }
            0x256d..=0x2570 => self.arc(code, window),
            0x2571..=0x2573 => self.diagonals(code, window),
            _ => false,
        }
    }

    /// Lines from the cell center to each edge with the given weights.
    fn arms(&self, [up, right, down, left]: [u8; 4], window: &mut Window) {
        let (x, y, w, h) = (self.x(), self.y(), self.w(), self.h());
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let thick = |weight: u8| if weight == 2 { self.light * 2.0 } else { self.light };
        // Half width of the widest crossing arm, so perpendicular arms meet it fully.
        let reach = |a: u8, b: u8| {
            let widest = a.max(b);
            match widest {
                0 => 0.0,
                3 => self.light * 1.5,
                _ => thick(widest) / 2.0,
            }
        };
        let vertical_reach = reach(up, down);
        let horizontal_reach = reach(left, right);
        let horizontal = |weight: u8, x0: f32, x1: f32, window: &mut Window| match weight {
            0 => {}
            3 => {
                for off in [-self.light, self.light] {
                    let top = cy + off - self.light / 2.0;
                    self.rect(x0, top, x1, top + self.light, 1.0, window);
                }
            }
            _ => {
                let t = thick(weight);
                self.rect(x0, cy - t / 2.0, x1, cy + t / 2.0, 1.0, window);
            }
        };
        let vertical = |weight: u8, y0: f32, y1: f32, window: &mut Window| match weight {
            0 => {}
            3 => {
                for off in [-self.light, self.light] {
                    let l = cx + off - self.light / 2.0;
                    self.rect(l, y0, l + self.light, y1, 1.0, window);
                }
            }
            _ => {
                let t = thick(weight);
                self.rect(cx - t / 2.0, y0, cx + t / 2.0, y1, 1.0, window);
            }
        };
        // shortcut: double-line corners overlap at the center instead of mitering; draw miters if it shows.
        horizontal(left, x, cx + vertical_reach, window);
        horizontal(right, cx - vertical_reach, x + w, window);
        vertical(up, y, cy + horizontal_reach, window);
        vertical(down, cy - horizontal_reach, y + h, window);
    }

    fn dashes(&self, horizontal: bool, weight: u8, count: u32, window: &mut Window) {
        let (x, y, w, h) = (self.x(), self.y(), self.w(), self.h());
        let t = self.light * weight as f32;
        let span = if horizontal { w } else { h };
        let step = span / count as f32;
        for i in 0..count {
            let start = i as f32 * step + step * 0.15;
            let end = start + step * 0.7;
            if horizontal {
                let cy = y + h / 2.0;
                self.rect(x + start, cy - t / 2.0, x + end, cy + t / 2.0, 1.0, window);
            } else {
                let cx = x + w / 2.0;
                self.rect(cx - t / 2.0, y + start, cx + t / 2.0, y + end, 1.0, window);
            }
        }
    }

    fn stroke(&self, build: impl FnOnce(&mut PathBuilder), window: &mut Window) -> bool {
        let mut path = PathBuilder::stroke(px(self.light));
        build(&mut path);
        match path.build() {
            Ok(path) => {
                window.paint_path(path, self.color);
                true
            }
            Err(_) => false,
        }
    }

    /// Rounded corners: a quarter curve from one edge midpoint to the other through the center.
    fn arc(&self, code: u32, window: &mut Window) -> bool {
        let (x, y, w, h) = (self.x(), self.y(), self.w(), self.h());
        // Center snapped on the same pixel column/row as the straight lines.
        let cx = self.snap(x + w / 2.0 - self.light / 2.0) + self.light / 2.0;
        let cy = self.snap(y + h / 2.0 - self.light / 2.0) + self.light / 2.0;
        let r = (w.min(h) / 2.0).max(1.0);
        // (vertical edge y, horizontal edge x, vertical direction, horizontal direction)
        let (edge_y, edge_x, dy, dx) = match code {
            0x256d => (y + h, x + w, 1.0, 1.0),   // down + right
            0x256e => (y + h, x, 1.0, -1.0),      // down + left
            0x256f => (y, x, -1.0, -1.0),         // up + left
            _ => (y, x + w, -1.0, 1.0),           // up + right
        };
        self.stroke(
            |p| {
                p.move_to(point(px(cx), px(edge_y)));
                p.line_to(point(px(cx), px(cy + dy * r)));
                p.curve_to(point(px(cx + dx * r), px(cy)), point(px(cx), px(cy)));
                p.line_to(point(px(edge_x), px(cy)));
            },
            window,
        )
    }

    fn diagonals(&self, code: u32, window: &mut Window) -> bool {
        let (x, y, w, h) = (self.x(), self.y(), self.w(), self.h());
        let mut ok = true;
        if code != 0x2572 {
            ok &= self.stroke(
                |p| {
                    p.move_to(point(px(x), px(y + h)));
                    p.line_to(point(px(x + w), px(y)));
                },
                window,
            );
        }
        if code != 0x2571 {
            ok &= self.stroke(
                |p| {
                    p.move_to(point(px(x), px(y)));
                    p.line_to(point(px(x + w), px(y + h)));
                },
                window,
            );
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob brings its own `test` attribute.
    use super::BOX;

    #[test]
    fn box_table_is_aligned() {
        let at = |c: char| BOX[c as usize - 0x2500];
        assert_eq!(at('─'), "0101");
        assert_eq!(at('┌'), "0110");
        assert_eq!(at('┼'), "1111");
        assert_eq!(at('╋'), "2222");
        assert_eq!(at('═'), "0303");
        assert_eq!(at('╬'), "3333");
        assert_eq!(at('╭'), "");
        assert_eq!(at('╴'), "0001");
        assert_eq!(at('╿'), "2010");
    }
}
