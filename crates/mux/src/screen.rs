//! A small VT screen model: it follows what a shell prints so the server can answer `capture`
//! (visible grid and scrollback as text) and repaint a client that attaches later. It never acts on
//! anything it parses: unknown sequences are dropped, OSC text is length-capped, and sizes are
//! bounded.

use std::collections::VecDeque;

pub const MAX_COLS: usize = 500;
pub const MAX_ROWS: usize = 300;
pub const MAX_TITLE: usize = 128;
/// Longest escape sequence kept while parsing; longer ones are dropped.
const MAX_SEQ: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Color {
    #[default]
    Default,
    Idx(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    /// bit 0 bold, 1 dim, 2 italic, 3 underline, 4 inverse, 5 strikethrough
    pub flags: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
    /// Second half of a wide character: skipped when printing.
    pub cont: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            style: Style::default(),
            cont: false,
        }
    }
}

type Row = Vec<Cell>;

#[derive(Clone)]
struct Grid {
    rows: Vec<Row>,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Ground,
    Esc,
    Csi,
    Osc,
    OscEsc,
    /// ESC ( ) * + and similar: swallow one more byte.
    Skip1,
    /// DCS / APC / PM / SOS: swallow until ST.
    Str,
    StrEsc,
}

pub struct Screen {
    cols: usize,
    rows: usize,
    main: Grid,
    alt: Option<Grid>,
    scrollback: VecDeque<Row>,
    max_scrollback: usize,
    x: usize,
    y: usize,
    wrap_pending: bool,
    saved: (usize, usize, Style),
    style: Style,
    top: usize,
    bottom: usize,
    cursor_visible: bool,
    title: String,
    mode: Mode,
    seq: Vec<u8>,
    utf: Vec<u8>,
}

fn blank_row(cols: usize) -> Row {
    vec![Cell::default(); cols]
}

/// Display width of a character: 0 for combining marks, 2 for the common wide ranges.
fn width(c: char) -> usize {
    let u = c as u32;
    match u {
        0x0300..=0x036F | 0x200B..=0x200F | 0xFE00..=0xFE0F | 0x20D0..=0x20FF => 0,
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

impl Screen {
    pub fn new(cols: usize, rows: usize, max_scrollback: usize) -> Screen {
        let (cols, rows) = (cols.clamp(1, MAX_COLS), rows.clamp(1, MAX_ROWS));
        Screen {
            cols,
            rows,
            main: Grid {
                rows: (0..rows).map(|_| blank_row(cols)).collect(),
            },
            alt: None,
            scrollback: VecDeque::new(),
            max_scrollback,
            x: 0,
            y: 0,
            wrap_pending: false,
            saved: (0, 0, Style::default()),
            style: Style::default(),
            top: 0,
            bottom: rows - 1,
            cursor_visible: true,
            title: String::new(),
            mode: Mode::Ground,
            seq: Vec::new(),
            utf: Vec::new(),
        }
    }

    pub fn size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.x, self.y)
    }

    pub fn in_alt_screen(&self) -> bool {
        self.alt.is_some()
    }

    fn grid(&mut self) -> &mut Grid {
        self.alt.as_mut().unwrap_or(&mut self.main)
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.clamp(1, MAX_COLS), rows.clamp(1, MAX_ROWS));
        let fix = |g: &mut Grid, y: &mut usize| {
            for r in g.rows.iter_mut() {
                r.resize(cols, Cell::default());
            }
            while g.rows.len() > rows {
                // Drop blank rows from the bottom first, otherwise push the top into scrollback.
                let last_blank = g.rows.last().is_some_and(|r| r.iter().all(|c| c.ch == ' '));
                if last_blank && *y < g.rows.len() - 1 {
                    g.rows.pop();
                } else {
                    g.rows.remove(0);
                    *y = y.saturating_sub(1);
                }
            }
            while g.rows.len() < rows {
                g.rows.push(blank_row(cols));
            }
        };
        let mut y = self.y;
        // Rows pushed out of the main grid go to scrollback.
        while self.main.rows.len() > rows {
            let last_blank = self
                .main
                .rows
                .last()
                .is_some_and(|r| r.iter().all(|c| c.ch == ' '));
            if last_blank && y < self.main.rows.len() - 1 {
                self.main.rows.pop();
            } else {
                let top = self.main.rows.remove(0);
                self.push_scrollback(top);
                y = y.saturating_sub(1);
            }
        }
        fix(&mut self.main, &mut y);
        if let Some(alt) = self.alt.as_mut() {
            let mut ay = self.y.min(rows - 1);
            fix(alt, &mut ay);
            y = ay;
        }
        for r in self.scrollback.iter_mut() {
            r.resize(cols, Cell::default());
        }
        self.cols = cols;
        self.rows = rows;
        self.x = self.x.min(cols - 1);
        self.y = y.min(rows - 1);
        self.top = 0;
        self.bottom = rows - 1;
        self.wrap_pending = false;
    }

    fn push_scrollback(&mut self, row: Row) {
        if self.max_scrollback == 0 {
            return;
        }
        if self.scrollback.len() >= self.max_scrollback {
            self.scrollback.pop_front();
        }
        self.scrollback.push_back(row);
    }

    fn scroll_up(&mut self, n: usize) {
        let (top, bottom, cols) = (self.top, self.bottom, self.cols);
        let to_scrollback = self.alt.is_none() && top == 0;
        for _ in 0..n.min(bottom - top + 1) {
            let removed = self.grid().rows.remove(top);
            self.grid().rows.insert(bottom, blank_row(cols));
            if to_scrollback {
                self.push_scrollback(removed);
            }
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let (top, bottom, cols) = (self.top, self.bottom, self.cols);
        for _ in 0..n.min(bottom - top + 1) {
            self.grid().rows.remove(bottom);
            self.grid().rows.insert(top, blank_row(cols));
        }
    }

    fn linefeed(&mut self) {
        if self.y == self.bottom {
            self.scroll_up(1);
        } else if self.y + 1 < self.rows {
            self.y += 1;
        }
    }

    fn put(&mut self, c: char) {
        let w = width(c);
        if w == 0 {
            // Combining mark: attach to the previous cell when there is one (kept as a no-op for text).
            return;
        }
        if self.wrap_pending || self.x + w > self.cols {
            self.x = 0;
            self.linefeed();
            self.wrap_pending = false;
        }
        let (x, y, style) = (self.x, self.y, self.style);
        let row = &mut self.grid().rows[y];
        row[x] = Cell {
            ch: c,
            style,
            cont: false,
        };
        if w == 2 && x + 1 < row.len() {
            row[x + 1] = Cell {
                ch: ' ',
                style,
                cont: true,
            };
        }
        if x + w >= self.cols {
            self.x = self.cols - 1;
            self.wrap_pending = true;
        } else {
            self.x = x + w;
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b);
        }
    }

    fn byte(&mut self, b: u8) {
        match self.mode {
            Mode::Ground => self.ground(b),
            Mode::Esc => self.esc(b),
            Mode::Csi => {
                if (0x40..=0x7e).contains(&b) {
                    let seq = std::mem::take(&mut self.seq);
                    self.mode = Mode::Ground;
                    self.csi(&seq, b);
                } else if b == 0x1b {
                    self.seq.clear();
                    self.mode = Mode::Esc;
                } else if self.seq.len() < MAX_SEQ {
                    self.seq.push(b);
                } else {
                    self.seq.clear();
                    self.mode = Mode::Ground;
                }
            }
            Mode::Osc => match b {
                0x07 => self.finish_osc(),
                0x1b => self.mode = Mode::OscEsc,
                _ => {
                    if self.seq.len() < MAX_SEQ {
                        self.seq.push(b);
                    }
                }
            },
            Mode::OscEsc => {
                if b == b'\\' {
                    self.finish_osc();
                } else {
                    self.seq.clear();
                    self.mode = Mode::Ground;
                    self.ground(b);
                }
            }
            Mode::Skip1 => self.mode = Mode::Ground,
            Mode::Str => match b {
                0x07 => self.mode = Mode::Ground,
                0x1b => self.mode = Mode::StrEsc,
                _ => {}
            },
            Mode::StrEsc => {
                self.mode = if b == b'\\' { Mode::Ground } else { Mode::Str };
            }
        }
    }

    fn finish_osc(&mut self) {
        let seq = std::mem::take(&mut self.seq);
        self.mode = Mode::Ground;
        let text = String::from_utf8_lossy(&seq);
        if let Some((code, rest)) = text.split_once(';') {
            if code == "0" || code == "2" {
                self.title = rest
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_TITLE)
                    .collect();
            }
        }
    }

    fn ground(&mut self, b: u8) {
        match b {
            0x1b => {
                self.utf.clear();
                self.mode = Mode::Esc;
            }
            b'\n' | 0x0b | 0x0c => {
                self.wrap_pending = false;
                self.linefeed();
            }
            b'\r' => {
                self.x = 0;
                self.wrap_pending = false;
            }
            0x08 => {
                self.x = self.x.saturating_sub(1);
                self.wrap_pending = false;
            }
            b'\t' => {
                self.x = ((self.x / 8 + 1) * 8).min(self.cols - 1);
                self.wrap_pending = false;
            }
            0x00..=0x1f | 0x7f => {}
            0x20..=0x7e => {
                self.utf.clear();
                self.put(b as char);
            }
            _ => {
                self.utf.push(b);
                match std::str::from_utf8(&self.utf) {
                    Ok(s) => {
                        let c = s.chars().next().unwrap_or('\u{fffd}');
                        self.utf.clear();
                        self.put(c);
                    }
                    Err(e) if e.error_len().is_none() && self.utf.len() < 4 => {}
                    Err(_) => {
                        self.utf.clear();
                        self.put('\u{fffd}');
                    }
                }
            }
        }
    }

    fn esc(&mut self, b: u8) {
        self.mode = Mode::Ground;
        match b {
            b'[' => {
                self.seq.clear();
                self.mode = Mode::Csi;
            }
            b']' => {
                self.seq.clear();
                self.mode = Mode::Osc;
            }
            b'P' | b'_' | b'^' | b'X' => self.mode = Mode::Str,
            b'(' | b')' | b'*' | b'+' | b'#' | b'%' => self.mode = Mode::Skip1,
            b'7' => self.saved = (self.x, self.y, self.style),
            b'8' => {
                let (x, y, s) = self.saved;
                self.x = x.min(self.cols - 1);
                self.y = y.min(self.rows - 1);
                self.style = s;
                self.wrap_pending = false;
            }
            b'M' => {
                if self.y == self.top {
                    self.scroll_down(1);
                } else {
                    self.y = self.y.saturating_sub(1);
                }
            }
            b'D' => self.linefeed(),
            b'E' => {
                self.x = 0;
                self.linefeed();
            }
            b'c' => {
                let (c, r, sb) = (self.cols, self.rows, self.max_scrollback);
                *self = Screen::new(c, r, sb);
            }
            _ => {}
        }
    }

    fn params(seq: &[u8]) -> (bool, Vec<Option<usize>>) {
        let private = seq
            .first()
            .is_some_and(|b| matches!(b, b'?' | b'>' | b'<' | b'='));
        let body = if private { &seq[1..] } else { seq };
        let text: String = body
            .iter()
            .filter(|b| b.is_ascii_digit() || **b == b';' || **b == b':')
            .map(|b| *b as char)
            .collect();
        let params = text
            .split(';')
            .map(|p| p.split(':').next().and_then(|n| n.parse::<usize>().ok()))
            .collect();
        (private, params)
    }

    fn csi(&mut self, seq: &[u8], fin: u8) {
        let (private, p) = Self::params(seq);
        let n = |i: usize, d: usize| p.get(i).copied().flatten().filter(|v| *v > 0).unwrap_or(d);
        let raw = |i: usize| p.get(i).copied().flatten().unwrap_or(0);
        self.wrap_pending = false;
        let (cols, rows) = (self.cols, self.rows);
        match (private, fin) {
            (false, b'A') => self.y = self.y.saturating_sub(n(0, 1)).max(self.top.min(self.y)),
            (false, b'B') => self.y = (self.y + n(0, 1)).min(rows - 1),
            (false, b'C') => self.x = (self.x + n(0, 1)).min(cols - 1),
            (false, b'D') => self.x = self.x.saturating_sub(n(0, 1)),
            (false, b'E') => {
                self.y = (self.y + n(0, 1)).min(rows - 1);
                self.x = 0;
            }
            (false, b'F') => {
                self.y = self.y.saturating_sub(n(0, 1));
                self.x = 0;
            }
            (false, b'G') => self.x = (n(0, 1) - 1).min(cols - 1),
            (false, b'd') => self.y = (n(0, 1) - 1).min(rows - 1),
            (false, b'H') | (false, b'f') => {
                self.y = (n(0, 1) - 1).min(rows - 1);
                self.x = (n(1, 1) - 1).min(cols - 1);
            }
            (false, b'J') => {
                let (x, y) = (self.x, self.y);
                match raw(0) {
                    0 => {
                        self.erase_cells(y, x, cols);
                        for r in y + 1..rows {
                            self.erase_cells(r, 0, cols);
                        }
                    }
                    1 => {
                        for r in 0..y {
                            self.erase_cells(r, 0, cols);
                        }
                        self.erase_cells(y, 0, x + 1);
                    }
                    2 => {
                        for r in 0..rows {
                            self.erase_cells(r, 0, cols);
                        }
                    }
                    3 => self.scrollback.clear(),
                    _ => {}
                }
            }
            (false, b'K') => {
                let (x, y) = (self.x, self.y);
                match raw(0) {
                    0 => self.erase_cells(y, x, cols),
                    1 => self.erase_cells(y, 0, x + 1),
                    2 => self.erase_cells(y, 0, cols),
                    _ => {}
                }
            }
            (false, b'L') | (false, b'M') => {
                if self.y >= self.top && self.y <= self.bottom {
                    let (y, bottom, k) =
                        (self.y, self.bottom, n(0, 1).min(self.bottom - self.y + 1));
                    for _ in 0..k {
                        if fin == b'L' {
                            self.grid().rows.remove(bottom);
                            self.grid().rows.insert(y, blank_row(cols));
                        } else {
                            self.grid().rows.remove(y);
                            self.grid().rows.insert(bottom, blank_row(cols));
                        }
                    }
                }
            }
            (false, b'@') => {
                let (x, y, k) = (self.x, self.y, n(0, 1).min(cols - self.x));
                let row = &mut self.grid().rows[y];
                for _ in 0..k {
                    row.pop();
                    row.insert(x, Cell::default());
                }
            }
            (false, b'P') => {
                let (x, y, k) = (self.x, self.y, n(0, 1).min(cols - self.x));
                let row = &mut self.grid().rows[y];
                for _ in 0..k {
                    row.remove(x);
                    row.push(Cell::default());
                }
            }
            (false, b'X') => {
                let (x, y) = (self.x, self.y);
                self.erase_cells(y, x, (x + n(0, 1)).min(cols));
            }
            (false, b'S') => self.scroll_up(n(0, 1)),
            (false, b'T') => self.scroll_down(n(0, 1)),
            (false, b'r') => {
                let (t, b) = (n(0, 1) - 1, n(1, rows) - 1);
                if t < b && b < rows {
                    self.top = t;
                    self.bottom = b;
                    self.x = 0;
                    self.y = 0;
                }
            }
            (false, b's') => self.saved = (self.x, self.y, self.style),
            (false, b'u') => {
                let (x, y, s) = self.saved;
                self.x = x.min(cols - 1);
                self.y = y.min(rows - 1);
                self.style = s;
            }
            (false, b'm') => self.sgr(&p),
            (true, b'h') | (true, b'l') => {
                let on = fin == b'h';
                for v in p.iter().flatten() {
                    match v {
                        25 => self.cursor_visible = on,
                        47 | 1047 | 1049 => self.set_alt(on),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn set_alt(&mut self, on: bool) {
        if on && self.alt.is_none() {
            self.alt = Some(Grid {
                rows: (0..self.rows).map(|_| blank_row(self.cols)).collect(),
            });
            self.saved = (self.x, self.y, self.style);
        } else if !on && self.alt.is_some() {
            self.alt = None;
            let (x, y, _) = self.saved;
            self.x = x.min(self.cols - 1);
            self.y = y.min(self.rows - 1);
        }
    }

    fn erase_cells(&mut self, row: usize, from: usize, to: usize) {
        let style = Style {
            bg: self.style.bg,
            ..Style::default()
        };
        let cols = self.cols;
        for c in self.grid().rows[row]
            .iter_mut()
            .take(to.min(cols))
            .skip(from)
        {
            *c = Cell {
                ch: ' ',
                style,
                cont: false,
            };
        }
    }

    fn sgr(&mut self, p: &[Option<usize>]) {
        let mut i = 0;
        while i < p.len() {
            let v = p[i].unwrap_or(0);
            let s = &mut self.style;
            match v {
                0 => *s = Style::default(),
                1 => s.flags |= 1,
                2 => s.flags |= 2,
                3 => s.flags |= 4,
                4 => s.flags |= 8,
                7 => s.flags |= 16,
                9 => s.flags |= 32,
                22 => s.flags &= !3,
                23 => s.flags &= !4,
                24 => s.flags &= !8,
                27 => s.flags &= !16,
                29 => s.flags &= !32,
                30..=37 => s.fg = Color::Idx((v - 30) as u8),
                39 => s.fg = Color::Default,
                40..=47 => s.bg = Color::Idx((v - 40) as u8),
                49 => s.bg = Color::Default,
                90..=97 => s.fg = Color::Idx((v - 90 + 8) as u8),
                100..=107 => s.bg = Color::Idx((v - 100 + 8) as u8),
                38 | 48 => {
                    let color = match p.get(i + 1).copied().flatten() {
                        Some(5) => {
                            let c = p
                                .get(i + 2)
                                .copied()
                                .flatten()
                                .map(|n| Color::Idx(n.min(255) as u8));
                            i += 2;
                            c
                        }
                        Some(2) => {
                            let g = |k: usize| {
                                p.get(i + k)
                                    .copied()
                                    .flatten()
                                    .map_or(0, |n| n.min(255) as u8)
                            };
                            let c = Color::Rgb(g(2), g(3), g(4));
                            i += 4;
                            Some(c)
                        }
                        _ => None,
                    };
                    if let Some(c) = color {
                        if v == 38 {
                            s.fg = c;
                        } else {
                            s.bg = c;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn row_text(row: &Row) -> String {
        let s: String = row.iter().filter(|c| !c.cont).map(|c| c.ch).collect();
        s.trim_end().to_string()
    }

    /// The visible grid (and, optionally, the scrollback before it) as plain text.
    pub fn text(&self, with_scrollback: bool) -> String {
        let grid = self.alt.as_ref().unwrap_or(&self.main);
        let mut lines: Vec<String> = Vec::new();
        if with_scrollback {
            lines.extend(self.scrollback.iter().map(Self::row_text));
        }
        lines.extend(grid.rows.iter().map(Self::row_text));
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }

    /// Bytes that repaint a freshly attached terminal to look like this screen.
    pub fn redraw(&self) -> Vec<u8> {
        let grid = self.alt.as_ref().unwrap_or(&self.main);
        let mut out = String::new();
        if self.alt.is_some() {
            out.push_str("\x1b[?1049h");
        }
        out.push_str("\x1b[0m\x1b[H\x1b[2J");
        let mut cur = Style::default();
        for (i, row) in grid.rows.iter().enumerate() {
            if i > 0 {
                out.push_str("\r\n");
            }
            let used = row
                .iter()
                .rposition(|c| c.ch != ' ' || c.style != Style::default())
                .map_or(0, |p| p + 1);
            for c in row.iter().take(used).filter(|c| !c.cont) {
                if c.style != cur {
                    out.push_str(&sgr_to(&c.style));
                    cur = c.style;
                }
                out.push(c.ch);
            }
        }
        out.push_str("\x1b[0m");
        out.push_str(&format!("\x1b[{};{}H", self.y + 1, self.x + 1));
        out.push_str(if self.cursor_visible {
            "\x1b[?25h"
        } else {
            "\x1b[?25l"
        });
        out.into_bytes()
    }
}

fn sgr_to(s: &Style) -> String {
    let mut p: Vec<String> = vec!["0".into()];
    for (bit, code) in [
        (1u8, "1"),
        (2, "2"),
        (4, "3"),
        (8, "4"),
        (16, "7"),
        (32, "9"),
    ] {
        if s.flags & bit != 0 {
            p.push(code.into());
        }
    }
    for (c, base) in [(s.fg, 38), (s.bg, 48)] {
        match c {
            Color::Default => {}
            Color::Idx(n) => p.push(format!("{base};5;{n}")),
            Color::Rgb(r, g, b) => p.push(format!("{base};2;{r};{g};{b}")),
        }
    }
    format!("\x1b[{}m", p.join(";"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(cols: usize, rows: usize) -> Screen {
        Screen::new(cols, rows, 100)
    }

    #[test]
    fn plain_text_wraps_scrolls_and_keeps_scrollback() {
        let mut sc = s(5, 2);
        sc.feed(b"abcdefgh\r\nxy\r\nlast");
        assert_eq!(sc.text(false), "xy\nlast");
        assert_eq!(sc.text(true), "abcde\nfgh\nxy\nlast");
        let mut sc = Screen::new(5, 2, 1);
        sc.feed(b"1\r\n2\r\n3\r\n4");
        assert_eq!(sc.text(true), "2\n3\n4");
    }

    #[test]
    fn cursor_erase_and_editing() {
        let mut sc = s(20, 3);
        sc.feed(b"hello world\r\x1b[2Cxx\x1b[K");
        assert_eq!(sc.text(false), "hexx");
        sc.feed(b"\x1b[2J\x1b[H\x1b[2;3Hab\x1b[1;1Hc");
        assert_eq!(sc.text(false), "c\n  ab");
        sc.feed(b"\x1b[2;1H\x1b[2P");
        assert_eq!(sc.text(false), "c\nab");
        sc.feed(b"\x1b[2;1H\x1b[@z");
        assert_eq!(sc.text(false), "c\nzab");
    }

    #[test]
    fn scroll_region_and_insert_delete_lines() {
        let mut sc = s(4, 4);
        sc.feed(b"1\r\n2\r\n3\r\n4\x1b[2;3r\x1b[3;1H\n");
        assert_eq!(sc.text(false), "1\n3\n\n4");
        sc.feed(b"\x1b[1;1H\x1b[r\x1b[2;1H\x1b[M");
        assert_eq!(sc.text(false), "1\n\n4");
    }

    #[test]
    fn alt_screen_is_separate_and_restored() {
        let mut sc = s(6, 2);
        sc.feed(b"shell\x1b[?1049h\x1b[Hvim");
        assert!(sc.in_alt_screen());
        assert_eq!(sc.text(false), "vim");
        sc.feed(b"\x1b[?1049l");
        assert_eq!(sc.text(false), "shell");
    }

    #[test]
    fn utf8_wide_and_split_sequences() {
        let mut sc = s(6, 2);
        let bytes = "é日x".as_bytes();
        for b in bytes {
            sc.feed(std::slice::from_ref(b));
        }
        assert_eq!(sc.text(false), "é日x");
        assert_eq!(sc.cursor(), (4, 0));
        let mut sc = s(6, 2);
        for b in b"\x1b[3" {
            sc.feed(&[*b]);
        }
        sc.feed(b"1mA\x1b[0mB");
        let row = &sc.main.rows[0];
        assert_eq!(row[0].style.fg, Color::Idx(1));
        assert_eq!(row[1].style, Style::default());
        sc.feed(&[0xff, b'q']);
        assert!(sc.text(false).contains('\u{fffd}'));
    }

    #[test]
    fn titles_are_capped_and_stripped_and_strings_are_swallowed() {
        let mut sc = s(10, 2);
        let long = "t".repeat(1000);
        sc.feed(format!("\x1b]0;{long}\x07").as_bytes());
        assert!(sc.title().chars().count() <= MAX_TITLE);
        sc.feed(b"\x1b]2;a\x01b\x1b\\");
        assert_eq!(sc.title(), "ab");
        sc.feed(b"\x1bPsecret data\x1b\\ok");
        assert_eq!(sc.text(false), "ok");
        // OSC 52 (clipboard) is never acted on, only swallowed.
        sc.feed(b"\x1b]52;c;QUJD\x07!");
        assert_eq!(sc.text(false), "ok!");
    }

    #[test]
    fn redraw_reproduces_the_screen() {
        let mut a = s(12, 3);
        a.feed(b"\x1b[1;31mred\x1b[0m plain\r\n\x1b[38;2;1;2;3mrgb\x1b[0m\x1b[3;4H");
        let mut b = s(12, 3);
        b.feed(&a.redraw());
        assert_eq!(a.text(false), b.text(false));
        assert_eq!(a.cursor(), b.cursor());
        assert_eq!(a.main.rows[0][0].style, b.main.rows[0][0].style);
        assert_eq!(a.main.rows[1][0].style, b.main.rows[1][0].style);
    }

    #[test]
    fn resize_keeps_content_and_bounds() {
        let mut sc = s(10, 4);
        sc.feed(b"a\r\nb\r\nc\r\nd");
        sc.resize(10, 2);
        assert_eq!(sc.text(true), "a\nb\nc\nd");
        sc.resize(3, 5);
        assert_eq!(sc.size(), (3, 5));
        sc.resize(100_000, 100_000);
        assert_eq!(sc.size(), (MAX_COLS, MAX_ROWS));
        sc.resize(0, 0);
        assert_eq!(sc.size(), (1, 1));
        sc.feed(b"xyz\r\n\r\n");
    }

    #[test]
    fn garbage_never_panics() {
        let mut sc = s(7, 3);
        let mut x: u32 = 12345;
        let mut data = Vec::new();
        for _ in 0..200_000 {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            data.push(match (x >> 24) % 6 {
                0 => 0x1b,
                1 => b'[',
                2 => (x >> 8) as u8,
                3 => b';',
                4 => b'0' + (x >> 16) as u8 % 10,
                _ => b"HJKLMPSTXrmhl@"[(x >> 12) as usize % 14],
            });
        }
        for chunk in data.chunks(97) {
            sc.feed(chunk);
            if chunk[0] % 31 == 0 {
                sc.resize(1 + (chunk[1] as usize % 20), 1 + (chunk[2] as usize % 10));
            }
        }
        let _ = (sc.text(true), sc.redraw());
    }
}
