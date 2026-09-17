use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, OnResize, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::search::{Match, RegexSearch};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::Processor;
use smithay_client_toolkit::reexports::calloop::channel::Sender;

pub enum FromShell {
    Output { id: u64, bytes: Vec<u8> },
    Ended { id: u64 },
}

struct Shared {
    title: Option<String>,
    answers: Vec<u8>,
    window: WindowSize,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            title: None,
            answers: Vec::new(),
            window: WindowSize {
                num_lines: 1,
                num_cols: 1,
                cell_width: 1,
                cell_height: 1,
            },
        }
    }
}

#[derive(Clone, Default)]
pub struct Notifier {
    shared: Arc<Mutex<Shared>>,
}

impl Notifier {
    fn set_window(&self, window: WindowSize) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.window = window;
        }
    }

    fn title(&self) -> Option<String> {
        self.shared.lock().ok().and_then(|shared| shared.title.clone())
    }

    fn take_answers(&self) -> Vec<u8> {
        match self.shared.lock() {
            Ok(mut shared) => std::mem::take(&mut shared.answers),
            Err(_) => Vec::new(),
        }
    }
}

impl EventListener for Notifier {
    fn send_event(&self, event: Event) {
        let Ok(mut shared) = self.shared.lock() else {
            return;
        };
        match event {
            Event::Title(title) => shared.title = Some(title),
            Event::ResetTitle => shared.title = None,
            Event::PtyWrite(text) => shared.answers.extend_from_slice(text.as_bytes()),
            Event::TextAreaSizeRequest(reply) => {
                let answer = reply(shared.window);
                shared.answers.extend_from_slice(answer.as_bytes());
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub columns: usize,
    pub lines: usize,
}

impl Size {
    pub fn new(columns: usize, lines: usize) -> Self {
        Self { columns: columns.max(1), lines: lines.max(1) }
    }
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

pub struct Session {
    pub id: u64,
    pub term: Term<Notifier>,
    pub size: Size,
    processor: Processor,
    pty: tty::Pty,
    writer: std::fs::File,
    notifier: Notifier,
}

impl Session {
    pub fn start(
        id: u64,
        size: Size,
        cell: (u16, u16),
        scrollback: usize,
        output: Sender<FromShell>,
    ) -> anyhow::Result<Self> {
        let mut options = tty::Options::default();
        options.env.insert(String::from("TERM"), String::from("xterm-256color"));

        let pty = tty::new(&options, window_size(size, cell), 0)?;
        let mut reader = pty.file().try_clone()?;
        let writer = pty.file().try_clone()?;
        wait_for_output(&reader);

        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                let read = reader.read(&mut buffer);
                let count = match read {
                    Ok(0) => break,
                    Ok(count) => count,
                    Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                    Err(err) => {
                        tracing::debug!(?err, "the shell closed its side");
                        break;
                    }
                };
                let bytes = buffer[..count].to_vec();
                if output.send(FromShell::Output { id, bytes }).is_err() {
                    break;
                }
            }
            let _ = output.send(FromShell::Ended { id });
        });

        let config = Config { scrolling_history: scrollback, ..Config::default() };
        let notifier = Notifier::default();
        notifier.set_window(window_size(size, cell));
        let term = Term::new(config, &size, notifier.clone());
        Ok(Self { id, term, size, processor: Processor::new(), pty, writer, notifier })
    }

    pub fn title(&self) -> Option<String> {
        self.notifier.title()
    }

    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_page(&mut self, up: bool) {
        match up {
            true => self.term.scroll_display(Scroll::PageUp),
            false => self.term.scroll_display(Scroll::PageDown),
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn start_selection(&mut self, point: Point, side: Side) {
        self.term.selection = Some(Selection::new(SelectionType::Simple, point, side));
    }

    pub fn update_selection(&mut self, point: Point, side: Side) {
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn link_at(&self, point: Point) -> Option<(usize, usize, String)> {
        let columns = self.size.columns;
        if columns == 0 || point.line < self.term.topmost_line() {
            return None;
        }
        if point.line > self.term.bottommost_line() {
            return None;
        }
        let row: Vec<char> =
            (0..columns).map(|column| self.term.grid()[point.line][Column(column)].c).collect();
        link_in_row(&row, point.column.0.min(columns - 1))
    }

    pub fn look_for(&self, query: &str, origin: Point, forward: bool) -> Option<Match> {
        if query.is_empty() {
            return None;
        }
        let mut search = RegexSearch::new(&as_literal(query)).ok()?;
        let direction = match forward {
            true => Direction::Right,
            false => Direction::Left,
        };
        self.term.search_next(&mut search, origin, direction, Side::Left, None)
    }

    pub fn show_match(&mut self, found: &Match) {
        self.term.scroll_to_point(*found.start());
        let mut selection = Selection::new(SelectionType::Simple, *found.start(), Side::Left);
        selection.update(*found.end(), Side::Right);
        self.term.selection = Some(selection);
    }

    pub fn last_point(&self) -> Point {
        Point::new(self.term.bottommost_line(), Column(self.size.columns.saturating_sub(1)))
    }

    pub fn first_point(&self) -> Point {
        Point::new(self.term.topmost_line(), Column(0))
    }

    pub fn after(&self, point: Point) -> Point {
        let last = self.size.columns.saturating_sub(1);
        if point.column.0 < last {
            return Point::new(point.line, Column(point.column.0 + 1));
        }
        if point.line >= self.term.bottommost_line() {
            return self.first_point();
        }
        Point::new(point.line + 1, Column(0))
    }

    pub fn before(&self, point: Point) -> Point {
        if point.column.0 > 0 {
            return Point::new(point.line, Column(point.column.0 - 1));
        }
        if point.line <= self.term.topmost_line() {
            return self.last_point();
        }
        Point::new(point.line - 1, Column(self.size.columns.saturating_sub(1)))
    }

    pub fn selected_text(&self) -> Option<String> {
        let text = self.term.selection_to_string()?;
        if text.is_empty() {
            return None;
        }
        Some(text)
    }

    pub fn wraps_a_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
        let answers = self.notifier.take_answers();
        if !answers.is_empty() {
            self.write(&answers);
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if let Err(err) = self.writer.write_all(bytes) {
            tracing::warn!(?err, "could not send the keystroke to the shell");
        }
    }

    pub fn resize(&mut self, size: Size, cell: (u16, u16)) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.term.resize(size);
        self.notifier.set_window(window_size(size, cell));
        self.pty.on_resize(window_size(size, cell));
    }
}

fn link_in_row(row: &[char], at: usize) -> Option<(usize, usize, String)> {
    if at >= row.len() || !is_link_letter(row[at]) {
        return None;
    }
    let mut start = at;
    while start > 0 && is_link_letter(row[start - 1]) {
        start -= 1;
    }
    let mut end = at;
    while end + 1 < row.len() && is_link_letter(row[end + 1]) {
        end += 1;
    }
    let mut text: String = row[start..=end].iter().collect();
    while text.ends_with(['.', ',', ';', ':', '!', '?']) {
        text.pop();
        end -= 1;
    }
    let address = as_address(&text)?;
    Some((start, end, address))
}

fn is_link_letter(letter: char) -> bool {
    if letter.is_whitespace() || letter == '\0' {
        return false;
    }
    !matches!(letter, '<' | '>' | '"' | '`' | '|' | '\'' | '(' | ')' | '[' | ']' | '{' | '}')
}

fn as_address(text: &str) -> Option<String> {
    const SCHEMES: [&str; 5] = ["http://", "https://", "file://", "ftp://", "mailto:"];
    for scheme in SCHEMES {
        if text.starts_with(scheme) && text.len() > scheme.len() {
            return Some(text.to_owned());
        }
    }
    if text.starts_with("www.") && text.len() > 4 {
        return Some(format!("https://{text}"));
    }
    None
}

fn as_literal(query: &str) -> String {
    let mut out = String::new();
    for letter in query.chars() {
        if !letter.is_alphanumeric() && !letter.is_whitespace() {
            out.push('\\');
        }
        out.push(letter);
    }
    out
}

fn wait_for_output(reader: &std::fs::File) {
    let fd = reader.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
        }
    }
}

fn window_size(size: Size, cell: (u16, u16)) -> WindowSize {
    WindowSize {
        num_lines: size.lines as u16,
        num_cols: size.columns as u16,
        cell_width: cell.0.max(1),
        cell_height: cell.1.max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    #[test]
    fn a_link_is_read_out_of_the_line_around_the_click() {
        let line = row("siehe https://spectre.de/x und sonst nichts          ");
        let found = link_in_row(&line, 10).expect("the click sits on the link");
        assert_eq!(found.2, "https://spectre.de/x");
        assert_eq!(&line[found.0..=found.1].iter().collect::<String>(), "https://spectre.de/x");

        assert!(link_in_row(&line, 2).is_none(), "the word before is not a link");
        assert!(link_in_row(&line, 5).is_none(), "a space is not a link");
    }

    #[test]
    fn a_full_stop_at_the_end_of_a_sentence_is_not_part_of_the_link() {
        let line = row("lies https://spectre.de/x.");
        let found = link_in_row(&line, 12).expect("a link");
        assert_eq!(found.2, "https://spectre.de/x");
        assert_eq!(found.1, line.len() - 2);
    }

    #[test]
    fn a_link_at_the_very_end_of_the_line_still_counts() {
        let line = row("www.archlinux.org");
        let found = link_in_row(&line, 0).expect("a link");
        assert_eq!(found.2, "https://www.archlinux.org");
        assert_eq!((found.0, found.1), (0, line.len() - 1));
    }

    #[test]
    fn only_real_addresses_count_as_links() {
        assert_eq!(as_address("https://spectre.de/x"), Some(String::from("https://spectre.de/x")));
        assert_eq!(as_address("www.spectre.de"), Some(String::from("https://www.spectre.de")));
        assert_eq!(as_address("spectre.de"), None, "a bare word is not a link");
        assert_eq!(as_address("https://"), None, "a scheme on its own is not a link");
    }

    #[test]
    fn brackets_and_quotes_never_belong_to_a_link() {
        assert!(!is_link_letter('('));
        assert!(!is_link_letter('"'));
        assert!(!is_link_letter(' '));
        assert!(is_link_letter('/'));
        assert!(is_link_letter('-'));
    }

    #[test]
    fn a_search_word_is_taken_as_it_is_typed() {
        assert_eq!(as_literal("main.rs"), "main\\.rs");
        assert_eq!(as_literal("a+b"), "a\\+b");
        assert_eq!(as_literal("two words"), "two words");
    }

    #[test]
    fn a_size_is_never_empty() {
        let size = Size::new(0, 0);
        assert_eq!(size.columns, 1);
        assert_eq!(size.lines, 1);
    }

    #[test]
    fn the_window_size_carries_the_cell_size() {
        let window = window_size(Size::new(80, 24), (9, 18));
        assert_eq!(window.num_cols, 80);
        assert_eq!(window.num_lines, 24);
        assert_eq!(window.cell_width, 9);
        assert_eq!(window.cell_height, 18);
    }

    #[test]
    fn a_cell_of_no_size_is_refused() {
        let window = window_size(Size::new(80, 24), (0, 0));
        assert_eq!(window.cell_width, 1);
        assert_eq!(window.cell_height, 1);
    }
}
