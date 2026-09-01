//! Workspace transitions.
//!
//! The animation itself is pure arithmetic: given a kind, a start time and a
//! duration, it says where each workspace should be drawn and how opaque it
//! should be. The compositor then renders the outgoing and incoming workspaces
//! into the same frame with those offsets.
//!
//! Nothing here allocates or touches the renderer, so the timing and easing can
//! be tested without a GPU.

use std::time::{Duration, Instant};

use spectre_config::WorkspaceTransition;

/// How a single workspace is placed while a transition runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Horizontal offset from the workspace's resting position, in logical
    /// pixels. Positive is to the right.
    pub offset_x: i32,
    /// Uniform scale about the output's centre. `1.0` is untransformed.
    pub scale: f64,
    /// Opacity, `0.0..=1.0`.
    pub alpha: f32,
}

impl Placement {
    /// A workspace sitting exactly where it belongs, fully opaque.
    pub const RESTING: Placement = Placement { offset_x: 0, scale: 1.0, alpha: 1.0 };

    /// Whether drawing this placement would change any pixel.
    pub fn is_visible(&self) -> bool {
        self.alpha > 0.001 && self.scale > 0.001
    }
}

/// A workspace switch in progress.
#[derive(Debug, Clone)]
pub struct Transition {
    /// Index of the workspace being left.
    pub from: usize,
    /// Index of the workspace being entered.
    pub to: usize,
    kind: WorkspaceTransition,
    started: Instant,
    duration: Duration,
}

impl Transition {
    /// Start a transition, or `None` when it would not animate.
    ///
    /// Returning `None` for a zero duration keeps the "animations off" path
    /// free of any per-frame work at all, rather than running a transition that
    /// completes instantly.
    pub fn start(
        from: usize,
        to: usize,
        kind: WorkspaceTransition,
        duration_ms: u32,
        now: Instant,
    ) -> Option<Self> {
        if from == to || duration_ms == 0 || kind == WorkspaceTransition::None {
            return None;
        }
        Some(Self {
            from,
            to,
            kind,
            started: now,
            duration: Duration::from_millis(duration_ms as u64),
        })
    }

    /// Linear progress through the transition, `0.0..=1.0`.
    pub fn linear_progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f32();
        let total = self.duration.as_secs_f32();
        if total <= 0.0 {
            return 1.0;
        }
        (elapsed / total).clamp(0.0, 1.0)
    }

    /// Eased progress, which is what the placements use.
    pub fn progress(&self, now: Instant) -> f32 {
        ease_out_cubic(self.linear_progress(now))
    }

    pub fn is_done(&self, now: Instant) -> bool {
        self.linear_progress(now) >= 1.0
    }

    /// `true` when the new workspace enters from the right.
    ///
    /// Workspaces are a row, so going from 1 to 3 moves right and 3 to 1 moves
    /// left. Wrapping around the ends is treated as a normal move rather than a
    /// long sweep back, because a wrap is a short-cut, not a journey.
    fn moves_right(&self) -> bool {
        self.to > self.from
    }

    /// Where to draw the outgoing and incoming workspaces.
    ///
    /// `width` is the output width in logical pixels.
    pub fn placements(&self, now: Instant, width: i32) -> (Placement, Placement) {
        let t = self.progress(now);
        let direction = if self.moves_right() { 1.0 } else { -1.0 };
        let travel = width as f32;

        match self.kind {
            WorkspaceTransition::None => (Placement::RESTING, Placement::RESTING),

            WorkspaceTransition::Fade => (
                Placement { alpha: 1.0 - t, ..Placement::RESTING },
                Placement { alpha: t, ..Placement::RESTING },
            ),

            WorkspaceTransition::Slide => (
                Placement {
                    offset_x: (-direction * travel * t) as i32,
                    ..Placement::RESTING
                },
                Placement {
                    offset_x: (direction * travel * (1.0 - t)) as i32,
                    ..Placement::RESTING
                },
            ),

            // Depth slides and pushes the outgoing workspace back, so the two
            // read as layers rather than as a strip being dragged past.
            WorkspaceTransition::Depth => (
                Placement {
                    offset_x: (-direction * travel * 0.35 * t) as i32,
                    scale: 1.0 - 0.15 * t as f64,
                    alpha: 1.0 - 0.6 * t,
                },
                Placement {
                    offset_x: (direction * travel * (1.0 - t)) as i32,
                    scale: 0.85 + 0.15 * t as f64,
                    alpha: 0.4 + 0.6 * t,
                },
            ),

            // Cube and Coverflow need each workspace rendered to a texture and
            // mapped onto a perspective-projected quad, which the flat element
            // pipeline cannot express. Until that render pass exists they run
            // as Depth rather than silently doing nothing: the user asked for
            // motion and gets motion, just not the shape they picked.
            WorkspaceTransition::Cube | WorkspaceTransition::Coverflow => Transition {
                kind: WorkspaceTransition::Depth,
                ..self.clone()
            }
            .placements(now, width),
        }
    }
}

/// Decelerating ease. Fast at the start so the switch feels immediate, settling
/// at the end so it does not look like it stopped short.
fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: i32 = 1920;

    fn transition(kind: WorkspaceTransition, from: usize, to: usize) -> (Transition, Instant) {
        let now = Instant::now();
        (Transition::start(from, to, kind, 200, now).unwrap(), now)
    }

    #[test]
    fn a_transition_to_the_same_workspace_never_starts() {
        let now = Instant::now();
        assert!(Transition::start(1, 1, WorkspaceTransition::Slide, 200, now).is_none());
    }

    #[test]
    fn a_zero_duration_never_starts() {
        let now = Instant::now();
        assert!(Transition::start(0, 1, WorkspaceTransition::Slide, 0, now).is_none());
    }

    #[test]
    fn the_none_kind_never_starts() {
        let now = Instant::now();
        assert!(Transition::start(0, 1, WorkspaceTransition::None, 200, now).is_none());
    }

    #[test]
    fn progress_runs_from_zero_to_one_and_stops_there() {
        let (t, now) = transition(WorkspaceTransition::Slide, 0, 1);
        assert_eq!(t.linear_progress(now), 0.0);
        assert!(!t.is_done(now));
        assert_eq!(t.linear_progress(now + Duration::from_millis(200)), 1.0);
        assert_eq!(t.linear_progress(now + Duration::from_secs(60)), 1.0);
        assert!(t.is_done(now + Duration::from_millis(200)));
    }

    #[test]
    fn easing_keeps_the_endpoints_and_front_loads_the_motion() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert!(ease_out_cubic(0.5) > 0.5, "an ease-out is past halfway at the midpoint");
        // Monotonic.
        let mut previous = 0.0;
        for i in 0..=100 {
            let value = ease_out_cubic(i as f32 / 100.0);
            assert!(value >= previous);
            previous = value;
        }
    }

    #[test]
    fn a_slide_starts_and_ends_where_the_workspaces_rest() {
        let (t, now) = transition(WorkspaceTransition::Slide, 0, 1);

        let (from, to) = t.placements(now, WIDTH);
        assert_eq!(from.offset_x, 0, "the outgoing workspace starts in place");
        assert_eq!(to.offset_x, WIDTH, "the incoming one starts off screen");

        let (from, to) = t.placements(now + Duration::from_millis(200), WIDTH);
        assert_eq!(from.offset_x, -WIDTH, "the outgoing one ends off screen");
        assert_eq!(to.offset_x, 0, "the incoming one ends in place");
    }

    #[test]
    fn the_slide_direction_follows_the_workspace_order() {
        let now = Instant::now();
        let forward = Transition::start(0, 2, WorkspaceTransition::Slide, 200, now).unwrap();
        let backward = Transition::start(2, 0, WorkspaceTransition::Slide, 200, now).unwrap();

        assert!(forward.placements(now, WIDTH).1.offset_x > 0, "next comes from the right");
        assert!(backward.placements(now, WIDTH).1.offset_x < 0, "previous comes from the left");
    }

    #[test]
    fn a_fade_swaps_opacity_without_moving_anything() {
        let (t, now) = transition(WorkspaceTransition::Fade, 0, 1);

        let (from, to) = t.placements(now, WIDTH);
        assert_eq!((from.offset_x, to.offset_x), (0, 0));
        assert!((from.alpha - 1.0).abs() < 1e-6);
        assert!(to.alpha < 1e-6);

        let (from, to) = t.placements(now + Duration::from_millis(200), WIDTH);
        assert!(from.alpha < 1e-6);
        assert!((to.alpha - 1.0).abs() < 1e-6);
    }

    #[test]
    fn depth_scales_as_well_as_slides() {
        let (t, now) = transition(WorkspaceTransition::Depth, 0, 1);
        let mid = now + Duration::from_millis(100);
        let (from, to) = t.placements(mid, WIDTH);

        assert!(from.scale < 1.0, "the outgoing workspace recedes");
        assert!(to.scale < 1.0 && to.scale > 0.85, "the incoming one grows into place");
        assert_ne!(from.offset_x, 0);

        let (from, to) = t.placements(now + Duration::from_millis(200), WIDTH);
        assert!((to.scale - 1.0).abs() < 1e-6, "it must land at full size");
        assert!((to.alpha - 1.0).abs() < 1e-6);
        assert!(from.alpha < 0.45);
    }

    #[test]
    fn the_three_dimensional_kinds_animate_as_depth_for_now() {
        let now = Instant::now();
        let cube = Transition::start(0, 1, WorkspaceTransition::Cube, 200, now).unwrap();
        let depth = Transition::start(0, 1, WorkspaceTransition::Depth, 200, now).unwrap();
        let mid = now + Duration::from_millis(90);
        assert_eq!(cube.placements(mid, WIDTH), depth.placements(mid, WIDTH));
    }

    #[test]
    fn every_kind_lands_both_workspaces_somewhere_sensible() {
        let now = Instant::now();
        for kind in WorkspaceTransition::ALL {
            let Some(t) = Transition::start(0, 1, kind, 200, now) else {
                continue;
            };
            for step in 0..=10 {
                let at = now + Duration::from_millis(step * 20);
                let (from, to) = t.placements(at, WIDTH);
                for placement in [from, to] {
                    assert!((0.0..=1.0).contains(&placement.alpha), "{kind:?} alpha");
                    assert!(placement.scale > 0.0 && placement.scale <= 1.5, "{kind:?} scale");
                    assert!(placement.offset_x.abs() <= WIDTH, "{kind:?} offset");
                }
            }
        }
    }

    #[test]
    fn an_invisible_placement_is_reported_as_such() {
        assert!(Placement::RESTING.is_visible());
        assert!(!Placement { alpha: 0.0, ..Placement::RESTING }.is_visible());
        assert!(!Placement { scale: 0.0, ..Placement::RESTING }.is_visible());
    }

    #[test]
    fn a_zero_width_output_does_not_produce_nonsense() {
        let (t, now) = transition(WorkspaceTransition::Slide, 0, 1);
        let (from, to) = t.placements(now + Duration::from_millis(100), 0);
        assert_eq!((from.offset_x, to.offset_x), (0, 0));
    }
}
