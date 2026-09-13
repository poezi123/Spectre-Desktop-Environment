use std::f32::consts::TAU;
use std::time::{Duration, Instant};

pub const CORNER_DWELL: Duration = Duration::from_millis(600);

const SNAP_DURATION: Duration = Duration::from_millis(220);

const FACES_PER_SCREEN_WIDTH: f32 = 1.6;

const CLICK_SLOP: f64 = 8.0;

#[derive(Debug, Clone)]
pub struct Overview {
    faces: usize,
    angle: f32,
    drag: Option<Drag>,
    snap: Option<Snap>,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    start_x: f64,
    start_angle: f32,
    moved: bool,
}

#[derive(Debug, Clone, Copy)]
struct Snap {
    from: f32,
    to: f32,
    started: Instant,
}

impl Overview {
    pub fn open(faces: usize, active: usize) -> Self {
        Self { faces: faces.max(1), angle: active as f32, drag: None, snap: None }
    }

    pub fn faces(&self) -> usize {
        self.faces
    }

    pub fn angle(&self, now: Instant) -> f32 {
        let Some(snap) = self.snap else {
            return self.angle;
        };
        let elapsed = now.saturating_duration_since(snap.started).as_secs_f32();
        let t = (elapsed / SNAP_DURATION.as_secs_f32()).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        snap.from + (snap.to - snap.from) * eased
    }

    pub fn press(&mut self, x: f64, now: Instant) {
        self.angle = self.angle(now);
        self.snap = None;
        self.drag = Some(Drag { start_x: x, start_angle: self.angle, moved: false });
    }

    pub fn drag_to(&mut self, x: f64, screen_width: f64) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let distance = x - drag.start_x;
        if distance.abs() > CLICK_SLOP {
            drag.moved = true;
        }
        let turned = (distance / screen_width.max(1.0)) as f32 * FACES_PER_SCREEN_WIDTH;
        self.angle = drag.start_angle - turned;
    }

    pub fn release(&mut self, now: Instant) -> Option<usize> {
        let drag = self.drag.take()?;
        if !drag.moved {
            return Some(self.front(now));
        }
        self.snap = Some(Snap { from: self.angle, to: self.angle.round(), started: now });
        None
    }

    pub fn step(&mut self, delta: i32, now: Instant) {
        let current = self.angle(now);
        self.angle = current;
        self.drag = None;
        let target = current.round() + delta as f32;
        self.snap = Some(Snap { from: current, to: target, started: now });
    }

    pub fn front(&self, now: Instant) -> usize {
        let rounded = self.angle(now).round() as i64;
        rounded.rem_euclid(self.faces as i64) as usize
    }

    pub fn face_angle(&self, index: usize, now: Instant) -> f32 {
        (index as f32 - self.angle(now)) * TAU / self.faces as f32
    }

    pub fn is_visible(&self, index: usize, now: Instant) -> bool {
        if self.faces == 1 {
            return index == 0;
        }
        self.face_angle(index, now).cos() > 0.001
    }

    pub fn is_moving(&self, now: Instant) -> bool {
        if self.drag.is_some() {
            return true;
        }
        match self.snap {
            Some(snap) => now.saturating_duration_since(snap.started) < SNAP_DURATION,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn later(now: Instant) -> Instant {
        now + SNAP_DURATION + Duration::from_millis(10)
    }

    #[test]
    fn it_opens_facing_the_active_workspace() {
        let now = Instant::now();
        let overview = Overview::open(4, 2);
        assert_eq!(overview.front(now), 2);
        assert!(overview.face_angle(2, now).abs() < 1e-6);
    }

    #[test]
    fn a_click_without_dragging_picks_the_front_workspace() {
        let now = Instant::now();
        let mut overview = Overview::open(4, 1);
        overview.press(500.0, now);
        overview.drag_to(503.0, 1920.0);
        assert_eq!(overview.release(now), Some(1));
    }

    #[test]
    fn dragging_left_brings_the_next_workspace_round() {
        let now = Instant::now();
        let mut overview = Overview::open(4, 0);
        overview.press(1500.0, now);
        overview.drag_to(1500.0 - 1920.0 * 0.5, 1920.0);
        assert_eq!(overview.release(now), None, "a drag is not a choice");
        assert_eq!(overview.front(later(now)), 1);
    }

    #[test]
    fn a_drag_snaps_to_a_whole_face() {
        let now = Instant::now();
        let mut overview = Overview::open(4, 0);
        overview.press(1000.0, now);
        overview.drag_to(700.0, 1920.0);
        overview.release(now);
        let angle = overview.angle(later(now));
        assert!((angle - angle.round()).abs() < 1e-5, "it came to rest at {angle}");
        assert!(!overview.is_moving(later(now)));
    }

    #[test]
    fn stepping_wraps_round_the_cube() {
        let now = Instant::now();
        let mut overview = Overview::open(4, 3);
        overview.step(1, now);
        assert_eq!(overview.front(later(now)), 0);
        overview.step(-1, later(now));
        assert_eq!(overview.front(later(later(now))), 3);
    }

    #[test]
    fn only_the_faces_turned_towards_the_viewer_are_drawn() {
        let now = Instant::now();
        let overview = Overview::open(4, 0);
        assert!(overview.is_visible(0, now));
        assert!(!overview.is_visible(2, now), "the back face is hidden");
    }

    #[test]
    fn a_single_workspace_still_has_a_face() {
        let now = Instant::now();
        let overview = Overview::open(1, 0);
        assert!(overview.is_visible(0, now));
        assert_eq!(overview.front(now), 0);
    }

    #[test]
    fn a_release_without_a_press_does_nothing() {
        let now = Instant::now();
        let mut overview = Overview::open(4, 0);
        assert_eq!(overview.release(now), None);
    }
}
