//! Software rendering for Spectre's shell surfaces.
//!
//! The panel, the launcher and the notification popups are all small, mostly
//! static surfaces. Compositing them on the CPU straight into the shared-memory
//! buffer keeps each one to its own pixels and a few megabytes of code, where a
//! GL context would cost more than the surface it is drawing.
//!
//! ```
//! use spectre_draw::{Canvas, Rect};
//! use spectre_theme::palette;
//!
//! let mut canvas = Canvas::new(64, 32);
//! canvas.fill_rect(Rect::new(0, 0, 64, 32), palette::SURFACE);
//! assert_eq!(canvas.as_bytes().len(), 64 * 32 * 4);
//! ```

mod canvas;
mod logo;

pub use canvas::{Canvas, Rect};
pub use logo::logo;
