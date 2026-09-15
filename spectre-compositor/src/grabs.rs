use smithay::desktop::Window;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
    RelativeMotionEvent,
};
use smithay::input::pointer::CursorIcon;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::render::Edges;
use crate::state::Spectre;

pub const BTN_LEFT: u32 = 0x110;

pub const MIN_WIDTH: i32 = 160;

pub const MIN_HEIGHT: i32 = 100;

#[derive(Debug, Clone)]
pub struct ActiveResize {
    pub window: Window,
    pub edges: Edges,
    pub initial: Rectangle<i32, Logical>,
    pub released: bool,
}

impl ActiveResize {
    pub fn location_for(&self, size: Size<i32, Logical>) -> Point<i32, Logical> {
        resized_location(self.initial, self.edges, size)
    }
}

pub fn resized_location(
    initial: Rectangle<i32, Logical>,
    edges: Edges,
    size: Size<i32, Logical>,
) -> Point<i32, Logical> {
    let mut location = initial.loc;
    if edges.left {
        location.x += initial.size.w - size.w;
    }
    if edges.top {
        location.y += initial.size.h - size.h;
    }
    location
}

pub fn resize_icon(edges: Edges) -> CursorIcon {
    let falling = (edges.top && edges.left) || (edges.bottom && edges.right);
    let rising = (edges.top && edges.right) || (edges.bottom && edges.left);
    if falling {
        return CursorIcon::NwseResize;
    }
    if rising {
        return CursorIcon::NeswResize;
    }
    if edges.left || edges.right {
        return CursorIcon::EwResize;
    }
    CursorIcon::NsResize
}

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

pub struct ResizeGrab {
    start_data: GrabStartData<Spectre>,
    window: Window,
    edges: Edges,
    initial: Rectangle<i32, Logical>,
}

impl ResizeGrab {
    pub fn new(
        start_data: GrabStartData<Spectre>,
        window: Window,
        edges: Edges,
        initial: Rectangle<i32, Logical>,
    ) -> Self {
        Self { start_data, window, edges, initial }
    }

    fn size_at(&self, pointer: Point<f64, Logical>) -> Size<i32, Logical> {
        let dx = (pointer.x - self.start_data.location.x).round() as i32;
        let dy = (pointer.y - self.start_data.location.y).round() as i32;
        let mut width = self.initial.size.w;
        let mut height = self.initial.size.h;
        if self.edges.left {
            width -= dx;
        }
        if self.edges.right {
            width += dx;
        }
        if self.edges.top {
            height -= dy;
        }
        if self.edges.bottom {
            height += dy;
        }
        Size::from((width.max(MIN_WIDTH), height.max(MIN_HEIGHT)))
    }
}

impl PointerGrab<Spectre> for ResizeGrab {
    fn motion(
        &mut self,
        state: &mut Spectre,
        handle: &mut PointerInnerHandle<'_, Spectre>,
        _focus: Option<(smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(state, None, event);
        let size = self.size_at(event.location);
        state.resize_window_to(&self.window, size);
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

    fn unset(&mut self, state: &mut Spectre) {
        state.finish_resize(&self.window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initial() -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((100, 100)), Size::from((400, 300)))
    }

    #[test]
    fn dragging_the_right_edge_keeps_the_window_in_place() {
        let edges = Edges { right: true, bottom: true, ..Edges::default() };
        let location = resized_location(initial(), edges, Size::from((500, 350)));
        assert_eq!(location, Point::from((100, 100)));
    }

    #[test]
    fn dragging_the_left_edge_moves_the_window_with_it() {
        let edges = Edges { left: true, top: true, ..Edges::default() };
        let location = resized_location(initial(), edges, Size::from((300, 250)));
        assert_eq!(location, Point::from((200, 150)));
    }

    #[test]
    fn corners_get_diagonal_arrows() {
        let top_left = Edges { top: true, left: true, ..Edges::default() };
        let bottom_left = Edges { bottom: true, left: true, ..Edges::default() };
        let right = Edges { right: true, ..Edges::default() };
        assert_eq!(resize_icon(top_left), CursorIcon::NwseResize);
        assert_eq!(resize_icon(bottom_left), CursorIcon::NeswResize);
        assert_eq!(resize_icon(right), CursorIcon::EwResize);
    }
}
