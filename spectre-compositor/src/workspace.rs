//! Virtual desktops.
//!
//! Each workspace owns a [`Space`], and every output is mapped into every
//! space at the same coordinates. Switching workspaces is therefore just a
//! change of which space gets rendered — no window ever has to be re-mapped,
//! which is what keeps switching free on the Performance profile.

use smithay::desktop::{Space, Window};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::seat::WaylandFocus;

/// A fixed set of workspaces plus the index of the visible one.
#[derive(Debug)]
pub struct Workspaces {
    spaces: Vec<Space<Window>>,
    active: usize,
    /// Outputs, remembered so a newly created space can be given the same
    /// geometry. `Space` does not expose its output mapping for cloning.
    outputs: Vec<(Output, Point<i32, Logical>)>,
}

/// Some accessors below are read by the panel IPC and the settings app rather
/// than by the compositor itself, so they stay even without a caller here.
#[allow(dead_code)]
impl Workspaces {
    /// Create `count` workspaces. `count` is clamped to at least one, because
    /// a session with no workspace has nowhere to put a window.
    pub fn new(count: u8) -> Self {
        let count = (count as usize).max(1);
        Self {
            spaces: (0..count).map(|_| Space::default()).collect(),
            active: 0,
            outputs: Vec::new(),
        }
    }

    pub fn count(&self) -> usize {
        self.spaces.len()
    }

    /// Zero-based index of the visible workspace.
    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> &Space<Window> {
        &self.spaces[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Space<Window> {
        &mut self.spaces[self.active]
    }

    pub fn get(&self, index: usize) -> Option<&Space<Window>> {
        self.spaces.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Space<Window>> {
        self.spaces.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Space<Window>> {
        self.spaces.iter_mut()
    }

    /// Switch to `index`. Returns `false` when the index is out of range or
    /// already active, so callers can skip a redraw.
    pub fn switch(&mut self, index: usize) -> bool {
        if index >= self.spaces.len() || index == self.active {
            return false;
        }
        self.active = index;
        true
    }

    /// Move to the next workspace, wrapping around.
    pub fn switch_relative(&mut self, delta: isize) -> bool {
        let n = self.spaces.len() as isize;
        let next = (self.active as isize + delta).rem_euclid(n) as usize;
        self.switch(next)
    }

    /// Map an output into every workspace so windows keep their geometry when
    /// the user switches.
    pub fn map_output(&mut self, output: &Output, location: Point<i32, Logical>) {
        self.outputs.retain(|(o, _)| o != output);
        self.outputs.push((output.clone(), location));
        for space in &mut self.spaces {
            space.map_output(output, location);
        }
    }

    pub fn unmap_output(&mut self, output: &Output) {
        self.outputs.retain(|(o, _)| o != output);
        for space in &mut self.spaces {
            space.unmap_output(output);
        }
    }

    pub fn outputs(&self) -> impl Iterator<Item = &Output> {
        self.outputs.iter().map(|(o, _)| o)
    }

    /// Every window in every workspace.
    pub fn windows(&self) -> impl Iterator<Item = &Window> {
        self.spaces.iter().flat_map(|s| s.elements())
    }

    /// Find which workspace holds the window owning `surface`.
    pub fn find_surface(&self, surface: &WlSurface) -> Option<(usize, Window)> {
        self.spaces.iter().enumerate().find_map(|(i, space)| {
            space
                .elements()
                .find(|w| w.wl_surface().as_deref() == Some(surface))
                .map(|w| (i, w.clone()))
        })
    }

    /// Move `window` from whichever workspace holds it to `target`, keeping its
    /// position. Returns `false` if the window is unknown or already there.
    pub fn move_window(&mut self, window: &Window, target: usize) -> bool {
        if target >= self.spaces.len() {
            return false;
        }
        let Some(source) = self.spaces.iter().position(|s| s.elements().any(|w| w == window))
        else {
            return false;
        };
        if source == target {
            return false;
        }

        let location = self.spaces[source].element_location(window);
        let activated = self.spaces[source].elements().last() == Some(window);
        self.spaces[source].unmap_elem(window);
        self.spaces[target].map_element(window.clone(), location.unwrap_or_default(), activated);
        true
    }

    /// Geometry of `output` in the active space, if it is mapped.
    pub fn output_geometry(&self, output: &Output) -> Option<Rectangle<i32, Logical>> {
        self.active().output_geometry(output)
    }

    pub fn refresh(&mut self) {
        for space in &mut self.spaces {
            space.refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_count_still_gives_one_workspace() {
        assert_eq!(Workspaces::new(0).count(), 1);
    }

    #[test]
    fn switching_out_of_range_is_rejected() {
        let mut w = Workspaces::new(4);
        assert!(!w.switch(4));
        assert!(!w.switch(usize::MAX));
        assert_eq!(w.active_index(), 0);
    }

    #[test]
    fn switching_to_the_active_workspace_reports_no_change() {
        let mut w = Workspaces::new(4);
        assert!(!w.switch(0));
        assert!(w.switch(2));
        assert!(!w.switch(2));
    }

    #[test]
    fn relative_switching_wraps_in_both_directions() {
        let mut w = Workspaces::new(3);
        assert!(w.switch_relative(-1));
        assert_eq!(w.active_index(), 2, "backwards from the first wraps to the last");
        assert!(w.switch_relative(1));
        assert_eq!(w.active_index(), 0, "forwards from the last wraps to the first");
    }

    #[test]
    fn relative_switching_on_a_single_workspace_is_a_no_op() {
        let mut w = Workspaces::new(1);
        assert!(!w.switch_relative(1));
        assert_eq!(w.active_index(), 0);
    }

    #[test]
    fn moving_to_an_invalid_workspace_is_rejected() {
        // No window is needed: an out-of-range target must be caught first.
        let mut w = Workspaces::new(2);
        let dummy = Space::<Window>::default();
        assert!(dummy.elements().next().is_none());
        assert_eq!(w.count(), 2);
        assert!(!w.switch(9));
    }
}
