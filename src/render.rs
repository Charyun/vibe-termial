//! Terminal grid data and GPUI painting. All hit testing uses terminal cells.
use gpui_kit::*;
use std::path::Path;
use vibe_terminal::{
    paths::{FileLink, find_links},
    terminal::Buffer,
};

pub const CELL: f32 = 8.4;
pub const LINE: f32 = 20.0;
pub const INSET: f32 = 12.0;
pub const BACK: u32 = 0x181818;
pub const FORE: u32 = 0xdddddf;
pub const ACCENT: u32 = 0xa8d5a2;
#[derive(Clone, PartialEq)]
pub struct CellStyle {
    pub foreground: u32,
    pub background: u32,
    pub bold: bool,
    pub underline: bool,
}
#[derive(Clone)]
pub struct Run {
    pub col: u16,
    pub row: u16,
    pub cells: u16,
    pub text: String,
    pub style: CellStyle,
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub rows: u16,
    pub cols: u16,
    pub runs: Vec<Run>,
    pub lines: Vec<String>,
    pub links: Vec<Vec<FileLink>>,
    pub cursor: Option<(u16, u16)>,
    pub alternate: bool,
}
impl Snapshot {
    pub fn read(b: &Buffer, workspace: &Path) -> Self {
        let screen = b.parser.screen();
        let (rows, cols) = screen.size();
        let mut out = Self {
            rows,
            cols,
            alternate: screen.alternate_screen(),
            ..Self::default()
        };
        for row in 0..rows {
            let mut line = String::new();
            let mut pending: Option<Run> = None;
            for col in 0..cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let value = if cell.has_contents() {
                    cell.contents()
                } else {
                    " "
                };
                line.push_str(value);
                let (foreground, background) = if cell.inverse() {
                    (ansi(cell.bgcolor(), true), ansi(cell.fgcolor(), false))
                } else {
                    (ansi(cell.fgcolor(), false), ansi(cell.bgcolor(), true))
                };
                let style = CellStyle {
                    foreground,
                    background,
                    bold: cell.bold(),
                    underline: cell.underline(),
                };
                let cells = if cell.is_wide() { 2 } else { 1 };
                if value.is_ascii()
                    && pending.as_ref().is_some_and(|p| {
                        p.style == style && p.col + p.cells == col && p.text.is_ascii()
                    })
                {
                    let p = pending.as_mut().unwrap();
                    p.text.push_str(value);
                    p.cells += cells;
                } else {
                    if let Some(p) = pending.take() {
                        out.runs.push(p);
                    }
                    pending = Some(Run {
                        col,
                        row,
                        cells,
                        text: value.into(),
                        style,
                    });
                }
            }
            if let Some(p) = pending {
                out.runs.push(p);
            }
            out.lines.push(line);
        }
        out.links = vec![vec![]; rows as usize];
        let mut first = 0;
        while first < rows as usize {
            let mut last = first;
            while last + 1 < rows as usize && screen.row_wrapped(last as u16) {
                last += 1;
            }
            let joined = out.lines[first..=last].concat();
            for link in find_links(&joined, &b.parser.callbacks().cwd, workspace) {
                for row in first..=last {
                    let offset = (row - first) * cols as usize;
                    let start = link.start.max(offset);
                    let end = link.end.min(offset + cols as usize);
                    if start < end {
                        let mut part = link.clone();
                        part.start = start - offset;
                        part.end = end - offset;
                        out.links[row].push(part);
                    }
                }
            }
            first = last + 1;
        }
        if !screen.hide_cursor() && screen.scrollback() == 0 {
            out.cursor = Some(screen.cursor_position());
        }
        out
    }
    pub fn link_at(&self, row: usize, col: usize) -> Option<&FileLink> {
        self.links
            .get(row)?
            .iter()
            .find(|l| col >= l.start && col < l.end)
    }
    pub fn selected_text(&self, a: (usize, usize), b: (usize, usize)) -> String {
        use unicode_width::UnicodeWidthChar;
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let mut lines = vec![];
        for row in start.0..=end.0 {
            let Some(line) = self.lines.get(row) else {
                break;
            };
            let left = if row == start.0 { start.1 } else { 0 };
            let right = if row == end.0 {
                end.1
            } else {
                self.cols as usize
            };
            let mut col = 0;
            let mut value = String::new();
            for ch in line.chars() {
                if col >= left && col < right {
                    value.push(ch);
                }
                col += ch.width().unwrap_or(0);
            }
            lines.push(value.trim_end().to_owned());
        }
        lines.join("\n")
    }
}
pub fn cell_at(
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    rows: u16,
    cols: u16,
) -> (usize, usize) {
    let x = f32::from(position.x - bounds.origin.x) - INSET;
    let y = f32::from(position.y - bounds.origin.y) - INSET;
    (
        (y / LINE).floor().clamp(0.0, rows.saturating_sub(1) as f32) as usize,
        (x / CELL).floor().clamp(0.0, cols as f32) as usize,
    )
}
pub struct PaintState {
    pub lines: Vec<(Point<Pixels>, ShapedLine)>,
    pub quads: Vec<PaintQuad>,
}
pub fn prepare(
    snapshot: &Snapshot,
    bounds: Bounds<Pixels>,
    selection: Option<((usize, usize), (usize, usize))>,
    preedit: &str,
    focused: bool,
    window: &mut Window,
) -> PaintState {
    let mut state = PaintState {
        lines: vec![],
        quads: vec![],
    };
    let origin = bounds.origin + point(px(INSET), px(INSET));
    let quad = |col: f32, row: f32, width: f32, height: f32, c: Hsla| {
        fill(
            Bounds::new(
                origin + point(px(col * CELL), px(row * LINE)),
                size(px(width), px(height)),
            ),
            c,
        )
    };
    for run in &snapshot.runs {
        if run.style.background != BACK {
            state.quads.push(quad(
                run.col as f32,
                run.row as f32,
                run.cells as f32 * CELL,
                LINE,
                rgb(run.style.background).into(),
            ));
        }
        if !run.text.trim().is_empty() {
            let font = Font {
                family: "Geist Mono".into(),
                weight: if run.style.bold {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                },
                ..Font::default()
            };
            let text: SharedString = run.text.clone().into();
            let line = window.text_system().shape_line(
                text.clone(),
                px(14.0),
                &[TextRun {
                    len: text.len(),
                    font,
                    color: rgb(run.style.foreground).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            );
            state.lines.push((
                origin + point(px(run.col as f32 * CELL), px(run.row as f32 * LINE)),
                line,
            ));
        }
        if run.style.underline {
            state.quads.push(fill(
                Bounds::new(
                    origin
                        + point(
                            px(run.col as f32 * CELL),
                            px((run.row + 1) as f32 * LINE - 2.0),
                        ),
                    size(px(run.cells as f32 * CELL), px(1.0)),
                ),
                rgb(run.style.foreground),
            ));
        }
    }
    for (row, links) in snapshot.links.iter().enumerate() {
        for link in links {
            state.quads.push(fill(
                Bounds::new(
                    origin
                        + point(
                            px(link.start as f32 * CELL),
                            px((row + 1) as f32 * LINE - 2.0),
                        ),
                    size(px((link.end - link.start) as f32 * CELL), px(1.0)),
                ),
                rgb(ACCENT),
            ));
        }
    }
    if let Some((a, b)) = selection {
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        for row in start.0..=end.0.min(snapshot.rows.saturating_sub(1) as usize) {
            let left = if row == start.0 { start.1 } else { 0 };
            let right = if row == end.0 {
                end.1
            } else {
                snapshot.cols as usize
            };
            state.quads.push(quad(
                left as f32,
                row as f32,
                right.saturating_sub(left) as f32 * CELL,
                LINE,
                rgba(0x88b9ff40).into(),
            ));
        }
    }
    if focused && let Some((row, col)) = snapshot.cursor {
        state
            .quads
            .push(quad(col as f32, row as f32, 2.0, LINE, rgb(ACCENT).into()));
        if !preedit.is_empty() {
            let font = Font {
                family: "Geist Mono".into(),
                ..Font::default()
            };
            let text: SharedString = preedit.to_owned().into();
            let line = window.text_system().shape_line(
                text.clone(),
                px(14.0),
                &[TextRun {
                    len: text.len(),
                    font,
                    color: rgb(FORE).into(),
                    background_color: Some(rgb(0x313131).into()),
                    underline: Some(UnderlineStyle {
                        color: Some(rgb(ACCENT).into()),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    strikethrough: None,
                }],
                None,
            );
            state.lines.push((
                origin + point(px(col as f32 * CELL), px(row as f32 * LINE)),
                line,
            ));
        }
    }
    state
}
fn ansi(c: vt100::Color, background: bool) -> u32 {
    const COLORS: [u32; 16] = [
        0x272727, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xca9ee6, 0x81c8be, 0xc6d0f5, 0x737994,
        0xea999c, 0xb5df9c, 0xefdaa3, 0xa5bdf4, 0xdfb5f5, 0x99d9d0, 0xf2f3f5,
    ];
    match c {
        vt100::Color::Default => {
            if background {
                BACK
            } else {
                FORE
            }
        }
        vt100::Color::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
        vt100::Color::Idx(i) if i < 16 => COLORS[i as usize],
        vt100::Color::Idx(i) if i >= 232 => {
            let v = (8 + (i - 232) * 10) as u32;
            v << 16 | v << 8 | v
        }
        vt100::Color::Idx(i) => {
            let i = i - 16;
            let component = |v: u8| if v == 0 { 0 } else { 55 + v * 40 } as u32;
            component(i / 36) << 16 | component(i / 6 % 6) << 8 | component(i % 6)
        }
    }
}
