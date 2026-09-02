//! The pointer.
//!
//! Clients set their own cursor surface as soon as the pointer is over them;
//! over the desktop, the panel's gaps and anything that has not asked for one,
//! Spectre draws its own arrow. Without that last part there is simply no
//! pointer on screen, which is what a compositor that only forwards client
//! cursors ends up with.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::{Size, Transform};
use spectre_theme::Color;

/// The arrow, as pixels: `.` transparent, `X` outline, `#` fill.
const ARROW: [&str; 19] = [
    "X...........",
    "XX..........",
    "X#X.........",
    "X##X........",
    "X###X.......",
    "X####X......",
    "X#####X.....",
    "X######X....",
    "X#######X...",
    "X########X..",
    "X#####XXXXX.",
    "X##X##X.....",
    "X#X.X##X....",
    "XX..X##X....",
    "X....X##X...",
    ".....X##X...",
    "......X##X..",
    "......X##X..",
    ".......XX...",
];

/// How many device pixels one cell of [`ARROW`] becomes.
const CELL: i32 = 2;

/// Spectre's own pointer, ready to hand to the renderer.
pub struct CursorImage {
    pub buffer: MemoryRenderBuffer,
    /// Where the tip sits inside the buffer, in device pixels.
    pub hotspot: (i32, i32),
    pub size: (i32, i32),
}

impl std::fmt::Debug for CursorImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CursorImage").field("size", &self.size).finish()
    }
}

impl CursorImage {
    /// Build the default arrow. `outline` and `fill` are its two colours.
    pub fn new(fill: Color, outline: Color) -> Self {
        let width = ARROW[0].len() as i32 * CELL;
        let height = ARROW.len() as i32 * CELL;
        let pixels = rasterise(fill, outline);
        let buffer = MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            Size::from((width, height)),
            1,
            Transform::Normal,
            None,
        );
        Self { buffer, hotspot: (0, 0), size: (width, height) }
    }
}

/// Draw [`ARROW`] into an `Argb8888` buffer.
fn rasterise(fill: Color, outline: Color) -> Vec<u8> {
    let cols = ARROW[0].len() as i32;
    let rows = ARROW.len() as i32;
    let (width, height) = (cols * CELL, rows * CELL);
    let mut out = vec![0u8; (width * height * 4) as usize];

    for (row, line) in ARROW.iter().enumerate() {
        for (col, cell) in line.chars().enumerate() {
            let color = match cell {
                '#' => fill,
                'X' => outline,
                _ => continue,
            };
            let [r, g, b, a] = color.to_rgba8();
            for dy in 0..CELL {
                for dx in 0..CELL {
                    let x = col as i32 * CELL + dx;
                    let y = row as i32 * CELL + dy;
                    let i = ((y * width + x) * 4) as usize;
                    // Argb8888 is [B, G, R, A] in memory, premultiplied.
                    let m = |c: u8| ((c as u16 * a as u16) / 255) as u8;
                    out[i] = m(b);
                    out[i + 1] = m(g);
                    out[i + 2] = m(r);
                    out[i + 3] = a;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels() -> (Vec<u8>, i32, i32) {
        let width = ARROW[0].len() as i32 * CELL;
        let height = ARROW.len() as i32 * CELL;
        (rasterise(Color::hex(0xffffff), Color::hex(0x000000)), width, height)
    }

    #[test]
    fn the_art_is_rectangular() {
        let width = ARROW[0].len();
        assert!(ARROW.iter().all(|line| line.len() == width), "every row must be as wide");
    }

    #[test]
    fn the_art_only_uses_the_three_cells_that_mean_something() {
        for line in ARROW {
            assert!(line.chars().all(|c| matches!(c, '.' | '#' | 'X')), "{line}");
        }
    }

    #[test]
    fn the_tip_is_the_top_left_pixel() {
        let (pixels, width, _) = pixels();
        let alpha = |x: i32, y: i32| pixels[((y * width + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 255, "the hotspot must be on the arrow itself");
    }

    #[test]
    fn the_arrow_is_outlined_so_it_shows_on_any_background() {
        let (pixels, width, _) = pixels();
        let at = |x: i32, y: i32| {
            let i = ((y * width + x) * 4) as usize;
            (pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3])
        };
        // Row 2 of the art is "X#X": outline, fill, outline.
        assert_eq!(at(0, 2 * CELL).3, 255);
        assert_eq!(at(0, 2 * CELL).0, 0, "the edge is the dark colour");
        assert_eq!(at(CELL, 2 * CELL).0, 255, "and the inside the light one");
    }

    #[test]
    fn most_of_the_buffer_is_transparent() {
        let (pixels, _, _) = pixels();
        let opaque = pixels.chunks(4).filter(|p| p[3] != 0).count();
        assert!(opaque > 0 && opaque < pixels.len() / 4 / 2, "an arrow, not a block");
    }

    #[test]
    fn the_image_reports_the_size_it_drew() {
        let image = CursorImage::new(Color::hex(0xffffff), Color::hex(0x000000));
        assert_eq!(image.size, (ARROW[0].len() as i32 * CELL, ARROW.len() as i32 * CELL));
        assert_eq!(image.hotspot, (0, 0));
    }
}
