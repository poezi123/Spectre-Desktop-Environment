//! Window placement, focus and window-state changes.
//!
//! Spectre is a stacking desktop: windows float, keep their title bars and are
//! placed by the compositor when they first appear. That matches the window
//! concept and keeps the model simple enough to stay cheap.

use smithay::desktop::{layer_map_for_output, Window, WindowSurfaceType};
use smithay::output::Output;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::wayland::seat::WaylandFocus;
use spectre_config::Direction;

use crate::state::Spectre;

/// Offset applied to each successive window that lands on the same spot, so
/// three terminals opened in a row do not hide each other perfectly.
const CASCADE_STEP: i32 = 28;
/// How far a directional move nudges a floating window.
const MOVE_STEP: i32 = 64;

impl Spectre {
    /// Output the pointer is on, else the first mapped output.
    pub fn active_output(&self) -> Option<Output> {
        let pos = self.pointer.current_location();
        self.workspaces
            .active()
            .output_under(pos)
            .next()
            .cloned()
            .or_else(|| self.outputs().first().cloned())
    }

    /// Area available to normal windows: the output minus panels and other
    /// layer surfaces that reserved an exclusive zone.
    pub fn working_area(&self, output: &Output) -> Rectangle<i32, Logical> {
        let geometry = self
            .workspaces
            .output_geometry(output)
            .unwrap_or_else(|| Rectangle::from_size(Size::from((0, 0))));
        let mut area = layer_map_for_output(output).non_exclusive_zone();
        area.loc += geometry.loc;
        if area.size.w <= 0 || area.size.h <= 0 {
            return geometry;
        }
        area
    }

    /// Place a freshly mapped window and give it focus.
    pub fn place_window(&mut self, window: Window) {
        let Some(output) = self.active_output() else {
            tracing::warn!("no output to place a window on; dropping it");
            return;
        };

        let area = self.working_area(&output);
        let size = window.geometry().size;
        let mut loc = Point::from((
            area.loc.x + (area.size.w - size.w).max(0) / 2,
            area.loc.y + (area.size.h - size.h).max(0) / 2,
        ));

        // Cascade while something already sits exactly here.
        let occupied = |space: &smithay::desktop::Space<Window>, p: Point<i32, Logical>| {
            space.elements().any(|w| space.element_location(w) == Some(p))
        };
        let mut guard = 0;
        while occupied(self.workspaces.active(), loc) && guard < 16 {
            loc += Point::from((CASCADE_STEP, CASCADE_STEP));
            guard += 1;
        }
        // Never cascade a window off the bottom-right of the working area.
        loc.x = loc.x.min((area.loc.x + area.size.w - size.w).max(area.loc.x));
        loc.y = loc.y.min((area.loc.y + area.size.h - size.h).max(area.loc.y));

        self.workspaces.active_mut().map_element(window.clone(), loc, true);
        self.focus_window(Some(&window));
        self.mark_dirty();
    }

    /// Remove a window from whichever workspace holds it and move focus on.
    pub fn unmap_window(&mut self, window: &Window) {
        for space in self.workspaces.iter_mut() {
            space.unmap_elem(window);
        }
        if self.focus.as_ref() == Some(window) {
            self.focus = None;
            let next = self.workspaces.active().elements().last().cloned();
            self.focus_window(next.as_ref());
        }
        self.mark_dirty();
    }

    /// Give keyboard focus to `window`, or clear focus when `None`.
    pub fn focus_window(&mut self, window: Option<&Window>) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();

        // Deactivate everything else so only one title bar reads as focused.
        let windows: Vec<Window> = self.workspaces.active().elements().cloned().collect();
        for w in &windows {
            let active = Some(w) == window;
            if let Some(toplevel) = w.toplevel() {
                toplevel.with_pending_state(|state| {
                    if active {
                        state.states.set(xdg_toplevel::State::Activated);
                    } else {
                        state.states.unset(xdg_toplevel::State::Activated);
                    }
                });
                toplevel.send_pending_configure();
            }
        }

        match window {
            Some(w) => {
                self.workspaces.active_mut().raise_element(w, true);
                keyboard.set_focus(self, w.wl_surface().map(|s| s.into_owned()), serial);
                self.focus = Some(w.clone());
            }
            None => {
                keyboard.set_focus(self, None, serial);
                self.focus = None;
            }
        }
        self.mark_dirty();
    }

    /// Focus the next (or previous) window in the active workspace.
    pub fn cycle_focus(&mut self, forward: bool) {
        let windows: Vec<Window> = self.workspaces.active().elements().cloned().collect();
        if windows.is_empty() {
            return;
        }
        let current = self
            .focus
            .as_ref()
            .and_then(|f| windows.iter().position(|w| w == f))
            .unwrap_or(0);
        let n = windows.len();
        let next = if forward { (current + 1) % n } else { (current + n - 1) % n };
        let target = windows[next].clone();
        self.focus_window(Some(&target));
    }

    /// Focus the nearest window in `direction`, measured between centres.
    pub fn focus_direction(&mut self, direction: Direction) {
        let space = self.workspaces.active();
        let Some(current) = self.focus.clone() else {
            let first = space.elements().last().cloned();
            self.focus_window(first.as_ref());
            return;
        };
        let Some(origin) = space.element_geometry(&current).map(center) else {
            return;
        };

        let best = space
            .elements()
            .filter(|w| **w != current)
            .filter_map(|w| space.element_geometry(w).map(|g| (w.clone(), center(g))))
            .filter(|(_, c)| in_direction(origin, *c, direction))
            .min_by_key(|(_, c)| distance_sq(origin, *c))
            .map(|(w, _)| w);

        if let Some(window) = best {
            self.focus_window(Some(&window));
        }
    }

    /// Nudge the focused floating window in `direction`, clamped to the working
    /// area so it can never be pushed off screen.
    pub fn move_direction(&mut self, direction: Direction) {
        let Some(window) = self.focus.clone() else {
            return;
        };
        let Some(output) = self.active_output() else {
            return;
        };
        let area = self.working_area(&output);
        let space = self.workspaces.active_mut();
        let Some(mut loc) = space.element_location(&window) else {
            return;
        };
        let size = window.geometry().size;

        match direction {
            Direction::Left => loc.x -= MOVE_STEP,
            Direction::Right => loc.x += MOVE_STEP,
            Direction::Up => loc.y -= MOVE_STEP,
            Direction::Down => loc.y += MOVE_STEP,
        }
        loc.x = loc.x.clamp(area.loc.x, (area.loc.x + area.size.w - size.w).max(area.loc.x));
        loc.y = loc.y.clamp(area.loc.y, (area.loc.y + area.size.h - size.h).max(area.loc.y));

        space.map_element(window, loc, true);
        self.mark_dirty();
    }

    /// Maximize or restore `window`.
    pub fn set_maximized(&mut self, window: &Window, maximized: bool) {
        let Some(output) = self.active_output() else {
            return;
        };
        let area = self.working_area(&output);
        let Some(toplevel) = window.toplevel().cloned() else {
            return;
        };

        toplevel.with_pending_state(|state| {
            if maximized {
                state.states.set(xdg_toplevel::State::Maximized);
                state.size = Some(area.size);
            } else {
                state.states.unset(xdg_toplevel::State::Maximized);
                state.size = None;
            }
        });
        toplevel.send_pending_configure();

        if maximized {
            self.workspaces.active_mut().map_element(window.clone(), area.loc, true);
        }
        self.mark_dirty();
    }

    /// Fullscreen covers the whole output, ignoring panels.
    pub fn set_fullscreen(&mut self, window: &Window, fullscreen: bool) {
        let Some(output) = self.active_output() else {
            return;
        };
        let Some(geometry) = self.workspaces.output_geometry(&output) else {
            return;
        };
        let Some(toplevel) = window.toplevel().cloned() else {
            return;
        };

        toplevel.with_pending_state(|state| {
            if fullscreen {
                state.states.set(xdg_toplevel::State::Fullscreen);
                state.size = Some(geometry.size);
            } else {
                state.states.unset(xdg_toplevel::State::Fullscreen);
                state.size = None;
            }
        });
        toplevel.send_pending_configure();

        if fullscreen {
            self.workspaces.active_mut().map_element(window.clone(), geometry.loc, true);
        }
        self.mark_dirty();
    }

    /// True when the focused window is in the given toplevel state.
    pub fn focused_has_state(&self, wanted: xdg_toplevel::State) -> bool {
        self.focus
            .as_ref()
            .and_then(|w| w.toplevel().cloned())
            .map(|t| t.with_pending_state(|state| state.states.contains(wanted)))
            .unwrap_or(false)
    }

    /// Ask the focused window to close.
    pub fn close_focused(&mut self) {
        if let Some(toplevel) = self.focus.as_ref().and_then(|w| w.toplevel().cloned()) {
            toplevel.send_close();
        }
    }

    /// Re-arrange layer surfaces on `output` and refit anything maximized.
    pub fn reflow_output(&mut self, output: &Output) {
        layer_map_for_output(output).arrange();

        let area = self.working_area(output);
        let windows: Vec<Window> = self.workspaces.active().elements().cloned().collect();
        for window in windows {
            let Some(toplevel) = window.toplevel().cloned() else {
                continue;
            };
            let maximized = toplevel
                .with_pending_state(|s| s.states.contains(xdg_toplevel::State::Maximized));
            if maximized {
                toplevel.with_pending_state(|state| state.size = Some(area.size));
                toplevel.send_pending_configure();
                self.workspaces.active_mut().map_element(window, area.loc, false);
            }
        }
        self.mark_dirty();
    }

    /// Window under the pointer in the active workspace.
    pub fn window_under_pointer(&self) -> Option<Window> {
        let pos = self.pointer.current_location();
        self.workspaces
            .active()
            .element_under(pos)
            .map(|(w, _)| w.clone())
    }

    /// Surface under the pointer, with the position of its top-left corner.
    pub fn surface_under_pointer(
        &self,
    ) -> Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<f64, Logical>,
    )> {
        let pos = self.pointer.current_location();
        let output = self.active_output()?;
        let output_geo = self.workspaces.output_geometry(&output)?;
        let layers = layer_map_for_output(&output);

        // Overlay and top layers sit above windows; bottom and background below.
        let above = layers
            .layer_under(smithay::wayland::shell::wlr_layer::Layer::Overlay, pos)
            .or_else(|| layers.layer_under(smithay::wayland::shell::wlr_layer::Layer::Top, pos));

        if let Some(layer) = above {
            let loc = layers.layer_geometry(layer)?.loc + output_geo.loc;
            return layer
                .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                .map(|(s, p)| (s, (p + loc).to_f64()));
        }

        if let Some((window, loc)) = self.workspaces.active().element_under(pos) {
            return window
                .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                .map(|(s, p)| (s, (p + loc).to_f64()));
        }

        let below = layers
            .layer_under(smithay::wayland::shell::wlr_layer::Layer::Bottom, pos)
            .or_else(|| {
                layers.layer_under(smithay::wayland::shell::wlr_layer::Layer::Background, pos)
            })?;
        let loc = layers.layer_geometry(below)?.loc + output_geo.loc;
        below
            .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
            .map(|(s, p)| (s, (p + loc).to_f64()))
    }
}

fn center(rect: Rectangle<i32, Logical>) -> Point<i32, Logical> {
    Point::from((rect.loc.x + rect.size.w / 2, rect.loc.y + rect.size.h / 2))
}

fn distance_sq(a: Point<i32, Logical>, b: Point<i32, Logical>) -> i64 {
    let dx = (a.x - b.x) as i64;
    let dy = (a.y - b.y) as i64;
    dx * dx + dy * dy
}

/// Whether `target` lies in `direction` from `origin`.
///
/// The dominant-axis test keeps a window that is slightly up and far right from
/// stealing an "up" press.
fn in_direction(
    origin: Point<i32, Logical>,
    target: Point<i32, Logical>,
    direction: Direction,
) -> bool {
    let dx = target.x - origin.x;
    let dy = target.y - origin.y;
    match direction {
        Direction::Left => dx < 0 && dx.abs() >= dy.abs(),
        Direction::Right => dx > 0 && dx.abs() >= dy.abs(),
        Direction::Up => dy < 0 && dy.abs() > dx.abs(),
        Direction::Down => dy > 0 && dy.abs() > dx.abs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i32, y: i32) -> Point<i32, Logical> {
        Point::from((x, y))
    }

    #[test]
    fn direction_test_picks_the_dominant_axis() {
        let origin = p(100, 100);
        assert!(in_direction(origin, p(500, 120), Direction::Right));
        assert!(!in_direction(origin, p(500, 120), Direction::Up));
        assert!(in_direction(origin, p(110, 10), Direction::Up));
        assert!(!in_direction(origin, p(110, 10), Direction::Right));
    }

    #[test]
    fn nothing_is_in_a_direction_from_itself() {
        let origin = p(50, 50);
        for d in [Direction::Left, Direction::Right, Direction::Up, Direction::Down] {
            assert!(!in_direction(origin, origin, d));
        }
    }

    #[test]
    fn centre_of_a_rectangle_is_its_middle() {
        let r = Rectangle::new(p(10, 20), Size::from((100, 50)));
        assert_eq!(center(r), p(60, 45));
    }

    #[test]
    fn distance_is_symmetric_and_zero_at_the_same_point() {
        assert_eq!(distance_sq(p(0, 0), p(3, 4)), 25);
        assert_eq!(distance_sq(p(3, 4), p(0, 0)), 25);
        assert_eq!(distance_sq(p(7, 7), p(7, 7)), 0);
    }

    #[test]
    fn distance_does_not_overflow_on_huge_coordinates() {
        // i32 squared overflows i32; the i64 accumulator must hold it.
        let d = distance_sq(p(i32::MIN / 2, 0), p(i32::MAX / 2, 0));
        assert!(d > 0);
    }
}
