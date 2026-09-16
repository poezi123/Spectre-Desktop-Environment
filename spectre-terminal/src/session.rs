use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;

use alacritty_terminal::event::{OnResize, VoidListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::Processor;
use smithay_client_toolkit::reexports::calloop::channel::Sender;

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
    pub term: Term<VoidListener>,
    pub size: Size,
    processor: Processor,
    pty: tty::Pty,
    writer: std::fs::File,
}

impl Session {
    pub fn start(
        size: Size,
        cell: (u16, u16),
        scrollback: usize,
        output: Sender<Vec<u8>>,
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
                if output.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });

        let config = Config { scrolling_history: scrollback, ..Config::default() };
        let term = Term::new(config, &size, VoidListener);
        Ok(Self { term, size, processor: Processor::new(), pty, writer })
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

    pub fn feed(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
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
        self.pty.on_resize(window_size(size, cell));
    }
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
