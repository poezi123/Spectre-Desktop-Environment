//! Pointer grabs.
//!
//! While a grab is active the pointer stops being routed to clients and is
//! handled by the compositor instead. Spectre uses one: dragging a title bar
//! to move a window.

use smithay::desktop::Window;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
    RelativeMotionEvent,
};
use smithay::utils::{Logical, Point};

use crate::state::Spectre;

/// Left mouse button, from `linux/input-event-codes.h`.
pub const BTN_LEFT: u32 = 0x110;

/// Moves a window with the pointer until the button that started it is released.
pub struct MoveGrab {
    start_data: GrabStartData<Spectre>,
    window: Window,
    /// Where the window's top-left sat relative to the pointer when the drag
    /// began. Keeping the offset rather than the absolute position is what
    /// stops the window from snapping its corner to the cursor.
    offset: Point<f64, Logical>,
}

impl MoveGrab {
    pub fn new(
        start_data: GrabStartData<Spectre>,
        window: Window,
        window_location: Point<i32, Logical>,
    ) -> Self {
        let offset = window_location.to_f64() - start_data.location;
        Self { start_data, window, offset }
    }

    fn move_to(&self, state: &mut Spectre, pointer: Point<f64, Logical>) {
        let target = pointer + self.offset;
        let location = Point::<i32, Logical>::from((
            target.x.round() as i32,
            target.y.round() as i32,
        ));
        state.move_window_to(&self.window, location);
    }
}

impl PointerGrab<Spectre> for MoveGrab {
    fn motion(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        _focus: Option<(smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // Focus stays where it was: a client must not receive enter/leave
        // events for surfaces the pointer sweeps over mid-drag.
        handle.motion(state, None, event);
        self.move_to(state, event.location);
    }

    fn relative_motion(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        _focus: Option<(smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(state, None, event);
    }

    fn button(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        event: &ButtonEvent,
    ) {
        handle.button(state, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, state, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        details: AxisFrame,
    ) {
        handle.axis(state, details);
    }

    fn frame(&mut self, state: &mut Spectre, handle: &mut PointerInnerHandle<'_, Spectre>) {
        handle.frame(state);
    }

    fn gesture_swipe_begin(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(state, event);
    }

    fn gesture_swipe_update(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(state, event);
    }

    fn gesture_swipe_end(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(state, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(state, event);
    }

    fn gesture_pinch_update(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(state, event);
    }

    fn gesture_pinch_end(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(state, event);
    }

    fn gesture_hold_begin(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(state, event);
    }

    fn gesture_hold_end(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(state, event);
    }

    fn start_data(&self) -> &GrabStartData<Spectre> {
        &self.start_data
    }

    fn unset(&mut self, _state: &mut Spectre) {}
}
