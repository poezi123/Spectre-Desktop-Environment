use smithay::input::pointer::PointerHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::pointer_constraints::{
    with_pointer_constraint, PointerConstraint, PointerConstraintsHandler,
};
use smithay::wayland::seat::WaylandFocus;
use smithay::{delegate_pointer_constraints, delegate_relative_pointer};

use crate::state::Spectre;

impl Spectre {
    pub fn pointer_is_locked(&self) -> bool {
        let Some(surface) = self.pointer_surface() else {
            return false;
        };
        let pointer = self.pointer.clone();
        with_pointer_constraint(&surface, &pointer, |constraint| {
            let Some(constraint) = constraint else {
                return false;
            };
            matches!(&*constraint, PointerConstraint::Locked(_)) && constraint.is_active()
        })
    }

    pub fn keep_the_pointer_inside(
        &self,
        wanted: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        let Some(surface) = self.pointer_surface() else {
            return wanted;
        };
        let Some(origin) = self.pointer_surface_origin() else {
            return wanted;
        };
        let pointer = self.pointer.clone();
        with_pointer_constraint(&surface, &pointer, |constraint| {
            let Some(constraint) = constraint else {
                return wanted;
            };
            if !matches!(&*constraint, PointerConstraint::Confined(_)) {
                return wanted;
            }
            if !constraint.is_active() {
                return wanted;
            }
            let Some(region) = constraint.region() else {
                return wanted;
            };
            let local = wanted - origin;
            match region.contains(local.to_i32_round()) {
                true => wanted,
                false => self.pointer_position(),
            }
        })
    }

    fn pointer_surface(&self) -> Option<WlSurface> {
        let window = self.focus.as_ref()?;
        window.wl_surface().map(|surface| surface.into_owned())
    }

    fn pointer_surface_origin(&self) -> Option<Point<f64, Logical>> {
        let window = self.focus.as_ref()?;
        let space = self.workspaces.active();
        let location = space.element_location(window)?;
        Some((location - window.geometry().loc).to_f64())
    }
}

impl PointerConstraintsHandler for Spectre {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        let holds_the_focus = self
            .pointer_surface()
            .is_some_and(|focused| focused == *surface);
        if !holds_the_focus {
            return;
        }
        with_pointer_constraint(surface, pointer, |constraint| {
            if let Some(constraint) = constraint {
                constraint.activate();
            }
        });
    }

    fn cursor_position_hint(
        &mut self,
        surface: &WlSurface,
        pointer: &PointerHandle<Self>,
        location: Point<f64, Logical>,
    ) {
        let active = with_pointer_constraint(surface, pointer, |constraint| match constraint {
            Some(constraint) => {
                matches!(&*constraint, PointerConstraint::Locked(_)) && constraint.is_active()
            }
            None => false,
        });
        if !active {
            return;
        }
        let Some(origin) = self.pointer_surface_origin() else {
            return;
        };
        self.set_pointer_position(origin + location);
    }
}

impl smithay::wayland::fractional_scale::FractionalScaleHandler for Spectre {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let scale = self
            .outputs()
            .into_iter()
            .next()
            .map(|output| output.current_scale().fractional_scale())
            .unwrap_or(1.0);
        smithay::wayland::compositor::with_states(&surface, |states| {
            smithay::wayland::fractional_scale::with_fractional_scale(states, |fractional| {
                fractional.set_preferred_scale(scale);
            });
        });
    }
}

delegate_pointer_constraints!(Spectre);
delegate_relative_pointer!(Spectre);
smithay::delegate_viewporter!(Spectre);
smithay::delegate_fractional_scale!(Spectre);
