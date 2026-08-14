//! Terminal emulator wrapper around alacritty_terminal.
//!
//! Abstraction layer: if we swap out alacritty_terminal later,
//! only this module changes. The rest of the system consumes
//! TerminalEmulator and RenderableCell.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{self, Config, Term};
use alacritty_terminal::vte::ansi::{Color as AColor, NamedColor, Processor};

use crate::palette;

pub use crate::palette::TermColor;

/// ANSI named color via process palette (Kitty theme if loaded).
fn named_color(c: NamedColor) -> TermColor {
    palette::global().named(c)
}

/// 256-color palette lookup.
fn indexed_color(idx: u8) -> TermColor {
    match idx {
        0..=15 => {
            let named = match idx {
                0 => NamedColor::Black,
                1 => NamedColor::Red,
                2 => NamedColor::Green,
                3 => NamedColor::Yellow,
                4 => NamedColor::Blue,
                5 => NamedColor::Magenta,
                6 => NamedColor::Cyan,
                7 => NamedColor::White,
                8 => NamedColor::BrightBlack,
                9 => NamedColor::BrightRed,
                10 => NamedColor::BrightGreen,
                11 => NamedColor::BrightYellow,
                12 => NamedColor::BrightBlue,
                13 => NamedColor::BrightMagenta,
                14 => NamedColor::BrightCyan,
                15 => NamedColor::BrightWhite,
                _ => unreachable!(),
            };
            named_color(named)
        }
        // 216 color cube (6x6x6)
        16..=231 => {
            let idx = idx - 16;
            let r = (idx / 36) % 6;
            let g = (idx / 6) % 6;
            let b = idx % 6;
            let to_val = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            TermColor::new(to_val(r), to_val(g), to_val(b))
        }
        // 24 grayscale
        232..=255 => {
            let v = 8 + 10 * (idx - 232);
            TermColor::new(v, v, v)
        }
    }
}

fn resolve_color(color: AColor) -> TermColor {
    match color {
        AColor::Named(n) => named_color(n),
        AColor::Spec(rgb) => TermColor::new(rgb.r, rgb.g, rgb.b),
        AColor::Indexed(idx) => indexed_color(idx),
    }
}

/// Text attributes for a cell.
#[derive(Debug, Clone, Copy, Default)]
pub struct CellAttrs {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub dim: bool,
    pub inverse: bool,
    pub wide: bool,
    pub wide_spacer: bool,
}

/// A renderable cell — our abstraction over alacritty's internal Cell.
#[derive(Debug, Clone)]
pub struct RenderableCell {
    pub line: usize,
    pub col: usize,
    pub c: char,
    pub fg: TermColor,
    pub bg: TermColor,
    pub attrs: CellAttrs,
    pub selected: bool,
}

/// Cursor info for rendering.
#[derive(Debug, Clone)]
pub struct CursorState {
    pub line: usize,
    pub col: usize,
    pub visible: bool,
}

/// Size helper implementing alacritty's Dimensions trait.
struct TermSize {
    cols: usize,
    lines: usize,
}

impl Dimensions for TermSize {
    fn columns(&self) -> usize {
        self.cols
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn total_lines(&self) -> usize {
        self.lines
    }
}

/// Terminal emulator — wraps alacritty_terminal::Term.
///
/// Feed PTY bytes in, read renderable cells out.
/// If we ever swap backends, this is the only thing that changes.
pub struct TerminalEmulator {
    term: Term<VoidListener>,
    parser: Processor,
    cols: usize,
    lines: usize,
}

impl TerminalEmulator {
    /// Create a new terminal emulator with given dimensions.
    pub fn new(cols: usize, lines: usize) -> Self {
        let config = Config {
            scrolling_history: 10_000,
            ..Default::default()
        };
        let size = TermSize { cols, lines };
        let term = Term::new(config, &size, VoidListener);
        let parser = Processor::new();

        Self {
            term,
            parser,
            cols,
            lines,
        }
    }

    /// Feed raw bytes from the PTY into the terminal state machine.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// Resize the terminal grid.
    pub fn resize(&mut self, cols: usize, lines: usize) {
        self.cols = cols;
        self.lines = lines;
        let size = TermSize { cols, lines };
        self.term.resize(size);
    }

    /// Get all renderable cells for the current viewport.
    ///
    /// Alacritty viewport: line 0 on screen = `Line(-display_offset)`.
    /// Indexing `Line(0)` always hits the live bottom — scroll would be invisible.
    pub fn renderable_cells(&self) -> Vec<RenderableCell> {
        let grid = self.term.grid();
        let offset = grid.display_offset() as i32;
        let mut cells = Vec::with_capacity(self.cols * self.lines);

        for line_idx in 0..self.lines {
            let line = Line(line_idx as i32 - offset);
            for col_idx in 0..self.cols {
                let point = Point::new(line, Column(col_idx));
                let cell = &grid[point];

                let mut fg = resolve_color(cell.fg);
                let mut bg = resolve_color(cell.bg);

                let attrs = CellAttrs {
                    bold: cell.flags.contains(Flags::BOLD),
                    italic: cell.flags.contains(Flags::ITALIC),
                    underline: cell.flags.intersects(
                        Flags::ALL_UNDERLINES,
                    ),
                    strikethrough: cell.flags.contains(Flags::STRIKEOUT),
                    dim: cell.flags.contains(Flags::DIM),
                    inverse: cell.flags.contains(Flags::INVERSE),
                    wide: cell.flags.contains(Flags::WIDE_CHAR),
                    wide_spacer: cell.flags.contains(Flags::WIDE_CHAR_SPACER),
                };

                // Handle inverse
                if attrs.inverse {
                    std::mem::swap(&mut fg, &mut bg);
                }

                // Handle dim
                if attrs.dim {
                    fg.r = fg.r / 2;
                    fg.g = fg.g / 2;
                    fg.b = fg.b / 2;
                }

                let selected = self.term.selection.as_ref().and_then(|s| s.to_range(&self.term)).map(|r| {
                    r.contains(point)
                }).unwrap_or(false);

                cells.push(RenderableCell {
                    line: line_idx,
                    col: col_idx,
                    c: cell.c,
                    fg,
                    bg,
                    attrs,
                    selected,
                });
            }
        }

        cells
    }

    /// Get cursor state in viewport coordinates (hidden when scrolled out).
    pub fn cursor(&self) -> CursorState {
        let grid = self.term.grid();
        let cursor = grid.cursor.point;
        let offset = grid.display_offset() as i32;
        let viewport_line = cursor.line.0 + offset;
        let in_view = viewport_line >= 0 && viewport_line < self.lines as i32;
        let visible =
            self.term.mode().contains(term::TermMode::SHOW_CURSOR) && in_view;

        CursorState {
            line: viewport_line.max(0) as usize,
            col: cursor.column.0,
            visible,
        }
    }

    /// Scroll the display by delta lines (positive = up/back, negative = down/forward).
    pub fn scroll(&mut self, delta: i32) {
        use alacritty_terminal::grid::Scroll;
        self.term.scroll_display(Scroll::Delta(delta));
    }

    /// Get current scroll offset (0 = bottom/live).
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Reset scroll to bottom (live view).
    pub fn scroll_to_bottom(&mut self) {
        use alacritty_terminal::grid::Scroll;
        self.term.grid_mut().scroll_display(Scroll::Bottom);
    }

    /// Get terminal dimensions.
    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn lines(&self) -> usize {
        self.lines
    }

    /// Get a specific *viewport* line as a string (for testing/debugging/copy).
    pub fn line_text(&self, line_idx: usize) -> String {
        let grid = self.term.grid();
        let offset = grid.display_offset() as i32;
        let line = Line(line_idx as i32 - offset);
        let mut text = String::with_capacity(self.cols);

        for col_idx in 0..self.cols {
            let point = Point::new(line, Column(col_idx));
            let cell = &grid[point];
            if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                text.push(cell.c);
            }
        }

        text.trim_end().to_string()
    }

    /// Get all visible text as a single string (for testing).
    pub fn screen_text(&self) -> String {
        (0..self.lines)
            .map(|i| self.line_text(i))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn viewport_point(&self, col: usize, line: usize) -> Point {
        let offset = self.term.grid().display_offset() as i32;
        Point::new(Line(line as i32 - offset), Column(col.min(self.cols.saturating_sub(1))))
    }

    pub fn selection_begin(&mut self, col: usize, line: usize) {
        let point = self.viewport_point(col, line);
        self.term.selection = Some(Selection::new(SelectionType::Simple, point, Side::Left));
    }

    pub fn selection_update(&mut self, col: usize, line: usize) {
        let point = self.viewport_point(col, line);
        if let Some(sel) = self.term.selection.as_mut() {
            sel.update(point, Side::Right);
        }
    }

    pub fn selection_clear(&mut self) {
        self.term.selection = None;
    }

    pub fn selected_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }
}

impl std::fmt::Debug for TerminalEmulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalEmulator")
            .field("cols", &self.cols)
            .field("lines", &self.lines)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_terminal() {
        let term = TerminalEmulator::new(80, 24);
        assert_eq!(term.cols(), 80);
        assert_eq!(term.lines(), 24);
    }

    #[test]
    fn process_plain_text() {
        let mut term = TerminalEmulator::new(80, 24);
        term.process(b"Hello, World!");
        assert_eq!(term.line_text(0), "Hello, World!");
    }

    #[test]
    fn process_newlines() {
        let mut term = TerminalEmulator::new(80, 24);
        term.process(b"line1\r\nline2\r\nline3");
        assert_eq!(term.line_text(0), "line1");
        assert_eq!(term.line_text(1), "line2");
        assert_eq!(term.line_text(2), "line3");
    }

    #[test]
    fn cursor_tracks_input() {
        let mut term = TerminalEmulator::new(80, 24);
        term.process(b"abc");

        let cursor = term.cursor();
        assert_eq!(cursor.line, 0);
        assert_eq!(cursor.col, 3);
        assert!(cursor.visible);
    }

    #[test]
    fn cursor_moves_on_newline() {
        let mut term = TerminalEmulator::new(80, 24);
        term.process(b"abc\r\n");

        let cursor = term.cursor();
        assert_eq!(cursor.line, 1);
        assert_eq!(cursor.col, 0);
    }

    #[test]
    fn ansi_color_parsed() {
        let mut term = TerminalEmulator::new(80, 24);
        // Red text: ESC[31m
        term.process(b"\x1b[31mRED\x1b[0m");

        let cells = term.renderable_cells();
        let r_cell = &cells[0]; // 'R'
        assert_eq!(r_cell.c, 'R');
        assert_eq!(r_cell.fg, named_color(NamedColor::Red));
    }

    #[test]
    fn bold_flag() {
        let mut term = TerminalEmulator::new(80, 24);
        term.process(b"\x1b[1mBOLD\x1b[0m");

        let cells = term.renderable_cells();
        assert!(cells[0].attrs.bold);
        assert_eq!(cells[0].c, 'B');
    }

    #[test]
    fn resize_terminal() {
        let mut term = TerminalEmulator::new(80, 24);
        term.process(b"hello");
        term.resize(40, 12);

        assert_eq!(term.cols(), 40);
        assert_eq!(term.lines(), 12);
    }

    #[test]
    fn cursor_movement_escape() {
        let mut term = TerminalEmulator::new(80, 24);
        // Write, then move cursor to column 0 with \r
        term.process(b"hello\rX");

        assert_eq!(term.line_text(0), "Xello");
    }

    #[test]
    fn clear_screen() {
        let mut term = TerminalEmulator::new(80, 24);
        term.process(b"hello\x1b[2J\x1b[H");

        // After clear + home, screen should be empty
        assert_eq!(term.line_text(0), "");
    }

    #[test]
    fn renderable_cell_count() {
        let term = TerminalEmulator::new(80, 24);
        let cells = term.renderable_cells();
        assert_eq!(cells.len(), 80 * 24);
    }

    #[test]
    fn screen_text_multiline() {
        let mut term = TerminalEmulator::new(80, 24);
        term.process(b"first\r\nsecond");
        let text = term.screen_text();
        assert!(text.contains("first"));
        assert!(text.contains("second"));
    }

    #[test]
    fn scroll_changes_visible_cells() {
        let mut term = TerminalEmulator::new(40, 8);
        for i in 0..20 {
            term.process(format!("L{i:02}\r\n").as_bytes());
        }
        let live_top = term.line_text(0);
        assert_eq!(term.display_offset(), 0);

        term.scroll(5);
        assert_eq!(term.display_offset(), 5);
        let scrolled_top = term.line_text(0);
        assert_ne!(live_top, scrolled_top, "viewport must move with display_offset");
        assert!(
            scrolled_top.starts_with('L'),
            "expected history line, got {:?}",
            scrolled_top
        );
    }

    #[test]
    fn inverse_swaps_colors() {
        let mut term = TerminalEmulator::new(80, 24);
        // ESC[7m = inverse
        term.process(b"\x1b[7mX\x1b[0m");

        let cells = term.renderable_cells();
        let x_cell = &cells[0];
        assert_eq!(x_cell.c, 'X');
        assert_eq!(x_cell.fg, named_color(NamedColor::Background));
        assert_eq!(x_cell.bg, named_color(NamedColor::Foreground));
    }

    #[test]
    fn truecolor_rgb_parsed() {
        let mut term = TerminalEmulator::new(80, 24);
        // Starship-style: ESC[38;2;249;38;114m (#f92672)
        term.process(b"\x1b[38;2;249;38;114mX\x1b[0m");
        let cells = term.renderable_cells();
        assert_eq!(cells[0].c, 'X');
        assert_eq!(cells[0].fg, TermColor::new(249, 38, 114));
    }

    #[test]
    fn truecolor_bg_parsed() {
        let mut term = TerminalEmulator::new(80, 24);
        term.process(b"\x1b[48;2;102;217;239mX\x1b[0m");
        let cells = term.renderable_cells();
        assert_eq!(cells[0].bg, TermColor::new(102, 217, 239));
    }

    #[test]
    fn mouse_selection_extracts_text() {
        let mut term = TerminalEmulator::new(80, 24);
        term.process(b"hello world");
        term.selection_begin(0, 0);
        term.selection_update(4, 0);
        let text = term.selected_text().unwrap();
        assert!(text.starts_with("hello"), "got {text:?}");
        term.selection_clear();
        assert!(term.selected_text().is_none());
    }
}
