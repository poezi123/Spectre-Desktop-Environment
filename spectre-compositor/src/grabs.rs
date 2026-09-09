use smithay::desktop::Window;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
    RelativeMotionEvent,
};
use smithay::utils::{Logical, Point};

use crate::state::Spectre;

pub const BTN_LEFT: u32 = 0x110;

pub struct MoveGrab {
    start_data: GrabStartData<Spectre>,
    window: Window,
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
