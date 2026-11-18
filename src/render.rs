//! Terminal grid data and GPUI painting. All hit testing uses terminal cells.
#[cfg(all(test, windows))]
#[path = "cursor_native_test.rs"]
mod cursor_native_test;
use gpui_kit::*;
use std::path::Path;
use vibe_terminal::{
    model::InputCursorStyle,
    paths::{FileLink, FileLinkResolver, WebLink, find_web_links},
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
    pub web_links: Vec<Vec<WebLink>>,
    pub cursor: Option<(u16, u16)>,
    /// Real terminal caret, retained even when a TUI hides the painted cursor.
    pub input_cursor: Option<(u16, u16)>,
}
/// The PTY lock is needed only to copy visible terminal cells. Resolve paths
/// after releasing it so filesystem latency cannot stall the PTY reader.
pub struct CapturedFrame {
    snapshot: Snapshot,
    cwd: std::path::PathBuf,
    wrapped: Vec<bool>,
    remote: bool,
}
impl CapturedFrame {
    pub fn resolve_links(self, workspace: &Path) -> Snapshot {
        let Self {
            snapshot: mut out,
            cwd,
            wrapped,
            remote,
        } = self;
        let rows = out.rows;
        let cols = out.cols;
        let mut resolver = FileLinkResolver::default();
        out.links = vec![vec![]; rows as usize];
        out.web_links = vec![vec![]; rows as usize];
        let mut first = 0;
        while first < rows as usize {
            let mut last = first;
            while last + 1 < rows as usize && wrapped[last] {
                last += 1;
            }
            let joined = out.lines[first..=last].concat();
            for link in if remote {
                vec![]
            } else {
                resolver.find(&joined, &cwd, workspace)
            } {
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
            for link in find_web_links(&joined) {
                for row in first..=last {
                    let offset = (row - first) * cols as usize;
                    let start = link.start.max(offset);
                    let end = link.end.min(offset + cols as usize);
                    if start < end {
                        let mut part = link.clone();
                        part.start = start - offset;
                        part.end = end - offset;
                        out.web_links[row].push(part);
                    }
                }
            }
            first = last + 1;
        }
        out
    }
}

#[derive(Default)]
pub struct SnapshotCache {
    key: Option<SnapshotKey>,
}
struct SnapshotKey {
    source: uuid::Uuid,
    revision: u64,
    scrollback: usize,
    size: (u16, u16),
    cwd: std::path::PathBuf,
    workspace: std::path::PathBuf,
}
impl SnapshotCache {
    pub fn capture(
        &mut self,
        source: uuid::Uuid,
        buffer: &Buffer,
        workspace: &Path,
        force: bool,
    ) -> Option<CapturedFrame> {
        let screen = buffer.parser.screen();
        let cwd = &buffer.parser.callbacks().cwd;
        if !force
            && self.key.as_ref().is_some_and(|key| {
                key.source == source
                    && key.revision == buffer.revision
                    && key.scrollback == screen.scrollback()
                    && key.size == screen.size()
                    && key.cwd == *cwd
                    && key.workspace == workspace
            })
        {
            return None;
        }
        self.key = Some(SnapshotKey {
            source,
            revision: buffer.revision,
            scrollback: screen.scrollback(),
            size: screen.size(),
            cwd: cwd.clone(),
            workspace: workspace.to_owned(),
        });
        Some(Snapshot::capture(buffer))
    }

    pub fn clear(&mut self) -> bool {
        self.key.take().is_some()
    }
}

impl Snapshot {
    #[cfg(test)]
    pub fn read(b: &Buffer, workspace: &Path) -> Self {
        Self::capture(b).resolve_links(workspace)
    }

    fn capture(b: &Buffer) -> CapturedFrame {
        let screen = b.parser.screen();
        let (rows, cols) = screen.size();
        let visible_cursor =
            (!screen.hide_cursor() && screen.scrollback() == 0).then(|| screen.cursor_position());
        let mut out = Self {
            rows,
            cols,
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
                        p.style == style
                            && p.col + p.cells == col
                            && p.text.is_ascii()
                            // Keep the block cursor's character separate so it
                            // can use contrasting ink without tinting its row.
                            && visible_cursor != Some((row, col))
                            && visible_cursor != Some((p.row, p.col))
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
        if screen.scrollback() == 0 {
            out.input_cursor = Some(screen.cursor_position());
            if !screen.hide_cursor() {
                out.cursor = out.input_cursor;
            }
        }
        CapturedFrame {
            snapshot: out,
            cwd: b.parser.callbacks().cwd.clone(),
            wrapped: (0..rows).map(|row| screen.row_wrapped(row)).collect(),
            remote: b.parser.callbacks().remote,
        }
    }
    /// IME geometry is in window logical pixels; the platform applies DPI once.
    pub fn ime_bounds(
        &self,
        bounds: Bounds<Pixels>,
        preedit: &str,
        offset_utf16: usize,
    ) -> Option<Bounds<Pixels>> {
        use unicode_width::UnicodeWidthChar;
        let (row, col) = self.input_cursor?;
        if self.rows == 0 || self.cols == 0 {
            return None;
        }
        let mut units = 0;
        let mut cells = 0;
        for ch in preedit.chars() {
            units += ch.len_utf16();
            if units > offset_utf16 {
                break;
            }
            cells += ch.width().unwrap_or(0);
        }
        let col = (col as usize + cells).min(self.cols.saturating_sub(1) as usize);
        let row = row.min(self.rows.saturating_sub(1));
        Some(Bounds::new(
            bounds.origin + point(px(INSET + col as f32 * CELL), px(INSET + row as f32 * LINE)),
            size(px(CELL), px(LINE)),
        ))
    }
    pub fn link_at(&self, row: usize, col: usize) -> Option<&FileLink> {
        self.links
            .get(row)?
            .iter()
            .find(|l| col >= l.start && col < l.end)
    }
    pub fn web_link_at(&self, row: usize, col: usize) -> Option<&WebLink> {
        self.web_links
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
/// Retain only shaped text used by the current frame. Cursor, selection and
/// IME overlays are still prepared each time; their geometry is never cached.
#[derive(Default)]
pub struct TextCache {
    current: std::collections::HashMap<TextKey, ShapedLine>,
    previous: std::collections::HashMap<TextKey, ShapedLine>,
    scale: f32,
}
#[derive(Clone, Hash, PartialEq, Eq)]
struct TextKey {
    text: SharedString,
    bold: bool,
    foreground: u32,
}
impl TextCache {
    fn begin(&mut self, scale: f32) {
        if self.scale != scale {
            self.current.clear();
            self.previous.clear();
            self.scale = scale;
        }
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.clear();
    }

    fn shape(
        &mut self,
        text: SharedString,
        bold: bool,
        foreground: u32,
        create: impl FnOnce() -> ShapedLine,
    ) -> ShapedLine {
        let key = TextKey {
            text,
            bold,
            foreground,
        };
        if let Some(line) = self.current.get(&key) {
            return line.clone();
        }
        let line = self.previous.remove(&key).unwrap_or_else(create);
        self.current.insert(key, line.clone());
        line
    }

    fn finish(&mut self) {
        self.previous.clear();
    }
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
    cursor_style: InputCursorStyle,
    focused: bool,
    cursor_visible: bool,
    text_cache: &mut TextCache,
    window: &mut Window,
) -> PaintState {
    text_cache.begin(window.scale_factor());
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
        // TUI scrollbars use box-drawing characters and full blocks. Font glyphs
        // leave leading inside our 20px rows, so paint these cell graphics at
        // the full row height to keep adjacent scrollbar segments connected.
        let graphic_width = match run.text.as_str() {
            "│" => Some(CELL / 8.0),
            "┃" => Some(CELL / 4.0),
            "█" => Some(CELL),
            _ => None,
        };
        if let Some(width) = graphic_width {
            state.quads.push(fill(
                Bounds::new(
                    origin
                        + point(
                            px(run.col as f32 * CELL + (CELL - width) / 2.0),
                            px(run.row as f32 * LINE),
                        ),
                    size(px(width), px(LINE)),
                ),
                rgb(run.style.foreground),
            ));
        } else if !run.text.trim().is_empty() {
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
            let foreground = if focused
                && cursor_visible
                && cursor_style == InputCursorStyle::Block
                && snapshot.cursor == Some((run.row, run.col))
            {
                BACK
            } else {
                run.style.foreground
            };
            let line = text_cache.shape(text.clone(), run.style.bold, foreground, || {
                window.text_system().shape_line(
                    text.clone(),
                    px(14.0),
                    &[TextRun {
                        len: text.len(),
                        font,
                        color: rgb(foreground).into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                )
            });
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
    for row in 0..snapshot.rows as usize {
        let links = snapshot
            .links
            .get(row)
            .into_iter()
            .flatten()
            .map(|l| (l.start, l.end))
            .chain(
                snapshot
                    .web_links
                    .get(row)
                    .into_iter()
                    .flatten()
                    .map(|l| (l.start, l.end)),
            );
        for (start, end) in links {
            state.quads.push(fill(
                Bounds::new(
                    origin + point(px(start as f32 * CELL), px((row + 1) as f32 * LINE - 2.0)),
                    size(px((end - start) as f32 * CELL), px(1.0)),
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
    if focused && cursor_visible && let Some((row, col)) = snapshot.cursor {
        let width = if cursor_style == InputCursorStyle::Bar {
            2.0
        } else {
            snapshot
                .runs
                .iter()
                .find(|run| run.row == row && run.col == col)
                .map_or(CELL, |run| run.cells as f32 * CELL)
        };
        state.quads.push(quad(
            col as f32,
            row as f32,
            width,
            LINE,
            rgb(ACCENT).into(),
        ));
    }
    if focused
        && !preedit.is_empty()
        && let Some((row, col)) = snapshot.input_cursor
    {
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
    text_cache.finish();
    state
}
#[cfg(test)]
mod tests {
    use super::{CELL, INSET, LINE, Snapshot};
    use gpui_kit::{Bounds, point, px, size};
    use vibe_terminal::terminal::Buffer;

    #[test]
    #[ignore = "explicit snapshot microbenchmark; excludes PTY parsing and GPU painting"]
    fn unchanged_frame_capture_microbenchmark() {
        use std::{hint::black_box, time::Instant};
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("sample.rs"), "fn main() {}").unwrap();
        let mut buffer = Buffer::new(30, 100, root.clone());
        buffer.process(
            "sample.rs:12:3 repeated terminal output\r\n"
                .repeat(25)
                .as_bytes(),
        );
        let source = uuid::Uuid::new_v4();
        let mut cache = super::SnapshotCache::default();
        let frame = cache
            .capture(source, &buffer, &root, false)
            .unwrap()
            .resolve_links(&root);
        assert_eq!(frame.lines, Snapshot::read(&buffer, &root).lines);
        let rounds = 1000;
        let started = Instant::now();
        for _ in 0..rounds {
            black_box(Snapshot::read(&buffer, &root));
        }
        let full = started.elapsed();
        let started = Instant::now();
        for _ in 0..rounds {
            assert!(black_box(cache.capture(source, &buffer, &root, false)).is_none());
        }
        let reused = started.elapsed();
        eprintln!(
            "SNAPSHOT_MICROBENCH rounds={rounds} full_capture_ms={:.3} unchanged_check_ms={:.3}",
            full.as_secs_f64() * 1000.0,
            reused.as_secs_f64() * 1000.0
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn background_output_reuses_foreground_frame_and_new_output_invalidates_it() {
        let root = std::env::temp_dir();
        let mut foreground = Buffer::new(5, 40, root.clone());
        foreground.process(b"foreground");
        let mut background = Buffer::new(5, 40, root.clone());
        let source = uuid::Uuid::new_v4();
        let mut cache = super::SnapshotCache::default();
        assert!(cache.capture(source, &foreground, &root, false).is_some());
        for _ in 0..1000 {
            background.process(b"background\r\n");
            assert!(cache.capture(source, &foreground, &root, false).is_none());
        }
        foreground.process(b" updated");
        let frame = cache.capture(source, &foreground, &root, false).unwrap();
        assert!(frame.resolve_links(&root).lines[0].contains("updated"));
        // A restarted PTY can have the same revision/size, but is a different source.
        assert!(
            cache
                .capture(uuid::Uuid::new_v4(), &foreground, &root, false)
                .is_some()
        );
    }

    #[test]
    fn frame_cache_tracks_scrollback_resize_cwd_and_workspace() {
        let root = std::env::temp_dir();
        let source = uuid::Uuid::new_v4();
        let mut buffer = Buffer::new(3, 20, root.clone());
        buffer.process(b"one\r\ntwo\r\nthree\r\nfour");
        let mut cache = super::SnapshotCache::default();
        assert!(cache.capture(source, &buffer, &root, false).is_some());
        buffer.scroll_lines(1);
        assert!(cache.capture(source, &buffer, &root, false).is_some());
        // Session::send returns to the live grid without incrementing revision.
        buffer.parser.screen_mut().set_scrollback(0);
        let frame = cache.capture(source, &buffer, &root, false).unwrap();
        assert!(frame.resolve_links(&root).input_cursor.is_some());
        buffer.parser.screen_mut().set_size(4, 24);
        let frame = cache.capture(source, &buffer, &root, false).unwrap();
        assert_eq!(frame.snapshot.cols, 24);
        buffer.parser.callbacks_mut().cwd = root.join("different-cwd");
        assert!(cache.capture(source, &buffer, &root, false).is_some());
        assert!(
            cache
                .capture(source, &buffer, &root.join("other-workspace"), false)
                .is_some()
        );
        assert!(cache.clear());
        assert!(!cache.clear());
        assert!(cache.capture(source, &buffer, &root, false).is_some());
    }

    #[test]
    fn detached_frame_uses_captured_cwd_and_revalidates_files_when_forced() {
        use std::sync::{Arc, Mutex};
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let cwd = root.join("cwd");
        std::fs::create_dir_all(&cwd).unwrap();
        let buffer = Arc::new(Mutex::new(Buffer::new(4, 60, cwd.clone())));
        buffer.lock().unwrap().process(b"source.rs:12:3\r\n");
        let source = uuid::Uuid::new_v4();
        let mut cache = super::SnapshotCache::default();
        let frame = {
            let guard = buffer.lock().unwrap();
            cache.capture(source, &guard, &root, false).unwrap()
        };
        // Link resolution must not retain the PTY mutex or borrow its live state.
        let mut guard = buffer.try_lock().unwrap();
        guard.parser.callbacks_mut().cwd = root.clone();
        std::fs::write(cwd.join("source.rs"), "fn main() {}").unwrap();
        let snapshot = frame.resolve_links(&root);
        assert_eq!(snapshot.links[0][0].path, cwd.join("source.rs"));
        assert_eq!(snapshot.links[0][0].line, Some(12));
        assert_eq!(snapshot.links[0][0].column, Some(3));
        guard.parser.callbacks_mut().cwd = cwd.clone();
        assert!(cache.capture(source, &guard, &root, false).is_none());
        std::fs::remove_file(cwd.join("source.rs")).unwrap();
        let refreshed = cache
            .capture(source, &guard, &root, true)
            .unwrap()
            .resolve_links(&root);
        assert!(refreshed.links.iter().all(Vec::is_empty));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shaped_text_reuses_styles_and_evicts_unused_text_and_dpi() {
        use std::cell::Cell;
        let calls = Cell::new(0);
        let create = || {
            calls.set(calls.get() + 1);
            gpui_kit::ShapedLine::default()
        };
        let mut cache = super::TextCache::default();
        for _ in 0..1000 {
            cache.begin(1.0);
            cache.shape("稳定的输出".into(), false, super::FORE, create);
            cache.shape("稳定的输出".into(), false, super::FORE, create);
            cache.finish();
            assert_eq!(cache.current.len(), 1);
            assert!(cache.previous.is_empty());
        }
        assert_eq!(calls.get(), 1);
        cache.begin(1.0);
        cache.shape("稳定的输出".into(), true, super::FORE, create);
        cache.shape("稳定的输出".into(), true, super::BACK, create);
        cache.finish();
        assert_eq!(calls.get(), 3); // Weight and block-cursor ink have distinct entries.
        cache.begin(1.5);
        cache.shape("稳定的输出".into(), true, super::BACK, create);
        cache.finish();
        assert_eq!(calls.get(), 4); // A DPI change drops previously shaped lines.
        cache.begin(1.5);
        cache.finish();
        assert!(cache.current.is_empty());
        assert!(cache.previous.is_empty());
    }

    #[test]
    fn hidden_tui_cursor_still_anchors_ime_in_window_logical_coordinates() {
        let root = std::env::temp_dir();
        let mut buffer = Buffer::new(30, 100, root.clone());
        buffer.process(b"\x1b[?25l\x1b[20;9H");
        let snapshot = Snapshot::read(&buffer, &root);
        assert!(snapshot.cursor.is_none());
        assert_eq!(snapshot.input_cursor, Some((19, 8)));
        let bounds = Bounds::new(point(px(240.0), px(80.0)), size(px(900.0), px(650.0)));
        let caret = snapshot.ime_bounds(bounds, "ni中😀", 5).unwrap();
        assert_eq!(
            caret.origin,
            bounds.origin + point(px(INSET + 14.0 * CELL), px(INSET + 19.0 * LINE))
        );
        assert_eq!(caret.size, size(px(CELL), px(LINE)));
        // A UTF-16 index inside a surrogate pair must not advance half a cell.
        assert_eq!(
            snapshot.ime_bounds(bounds, "😀", 1),
            snapshot.ime_bounds(bounds, "", 0)
        );
    }

    #[test]
    fn ime_anchor_is_clamped_to_grid_and_absent_in_scrollback() {
        let root = std::env::temp_dir();
        let mut buffer = Buffer::new(3, 10, root.clone());
        buffer.process(b"one\r\ntwo\r\nthree\r\nfour\x1b[3;10H");
        let bounds = Bounds::new(point(px(100.0), px(50.0)), size(px(200.0), px(100.0)));
        let snapshot = Snapshot::read(&buffer, &root);
        assert_eq!(
            snapshot
                .ime_bounds(bounds, "long preedit", 12)
                .unwrap()
                .origin,
            bounds.origin + point(px(INSET + 9.0 * CELL), px(INSET + 2.0 * LINE))
        );
        buffer.scroll_lines(1);
        let snapshot = Snapshot::read(&buffer, &root);
        assert!(snapshot.ime_bounds(bounds, "", 0).is_none());
    }

    #[test]
    fn remote_terminal_paths_never_resolve_against_local_files() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut buffer = Buffer::new(5, 100, root.clone());
        buffer.parser.callbacks_mut().remote = true;
        buffer.process(b"Cargo.toml http://localhost:3000/\r\n");
        let snapshot = Snapshot::read(&buffer, &root);
        assert!(snapshot.links.iter().all(Vec::is_empty));
        assert!(snapshot.web_links.iter().any(|row| !row.is_empty()));
    }
    #[test]
    fn wrapped_web_links_hit_the_same_complete_url_on_both_rows() {
        let root = std::env::temp_dir();
        let mut buffer = Buffer::new(5, 24, root.clone());
        let url = "http://localhost:3000/long/path?x=1#part";
        buffer.process(format!("中 {url}\r\n").as_bytes());
        let snapshot = Snapshot::read(&buffer, &root);
        assert_eq!(snapshot.web_link_at(0, 3).unwrap().url, url);
        assert_eq!(snapshot.web_link_at(1, 0).unwrap().url, url);
        assert!(snapshot.web_link_at(0, 2).is_none());
        assert!(snapshot.web_link_at(2, 0).is_none());
    }
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
