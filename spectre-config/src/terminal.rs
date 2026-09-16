use serde::{Deserialize, Serialize};

pub const MIN_FONT_SIZE: f32 = 8.0;

pub const MAX_FONT_SIZE: f32 = 32.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Terminal {
    pub font_size: f32,
    pub scrollback: u32,
}

impl Default for Terminal {
    fn default() -> Self {
        Self { font_size: 14.0, scrollback: 10_000 }
    }
}

impl Terminal {
    pub fn font_size(&self) -> f32 {
        if !self.font_size.is_finite() {
            return Self::default().font_size;
        }
        self.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
    }

    pub fn scrollback(&self) -> usize {
        self.scrollback.min(200_000) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_terminal_reads_comfortably() {
        let terminal = Terminal::default();
        assert_eq!(terminal.font_size(), 14.0);
        assert_eq!(terminal.scrollback(), 10_000);
    }

    #[test]
    fn a_nonsense_font_size_falls_back_to_something_readable() {
        let terminal = Terminal { font_size: f32::NAN, ..Terminal::default() };
        assert_eq!(terminal.font_size(), 14.0);
    }

    #[test]
    fn the_font_size_stays_within_reach() {
        let tiny = Terminal { font_size: 1.0, ..Terminal::default() };
        let huge = Terminal { font_size: 400.0, ..Terminal::default() };
        assert_eq!(tiny.font_size(), MIN_FONT_SIZE);
        assert_eq!(huge.font_size(), MAX_FONT_SIZE);
    }

    #[test]
    fn an_absurd_history_is_capped() {
        let terminal = Terminal { scrollback: u32::MAX, ..Terminal::default() };
        assert_eq!(terminal.scrollback(), 200_000);
    }
}
