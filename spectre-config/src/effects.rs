//! Individually switchable visual effects.
//!
//! Each field here must be independently toggleable — that is the "every
//! expensive animation is optional" rule, and the reason profiles are only a
//! preset over this struct rather than a separate code path.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceTransition {
    /// Instant switch. Always available, costs nothing.
    None,
    Fade,
    #[default]
    Slide,
    /// Slide with a scale-back, giving a sense of depth.
    Depth,
    /// Workspaces mapped onto the faces of a rotating cube.
    Cube,
    Coverflow,
}

impl WorkspaceTransition {
    pub const ALL: [WorkspaceTransition; 6] = [
        WorkspaceTransition::None,
        WorkspaceTransition::Fade,
        WorkspaceTransition::Slide,
        WorkspaceTransition::Depth,
        WorkspaceTransition::Cube,
        WorkspaceTransition::Coverflow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WorkspaceTransition::None => "None",
            WorkspaceTransition::Fade => "Fade",
            WorkspaceTransition::Slide => "Slide",
            WorkspaceTransition::Depth => "Depth",
            WorkspaceTransition::Cube => "Cube",
            WorkspaceTransition::Coverflow => "Coverflow",
        }
    }

    /// Whether the transition needs the full scene rendered off-screen first.
    /// These are the ones a low-end GPU should avoid.
    pub fn needs_offscreen_pass(self) -> bool {
        matches!(self, WorkspaceTransition::Cube | WorkspaceTransition::Coverflow)
    }

    /// Default duration in milliseconds.
    pub fn duration_ms(self) -> u32 {
        match self {
            WorkspaceTransition::None => 0,
            WorkspaceTransition::Fade => 140,
            WorkspaceTransition::Slide => 180,
            WorkspaceTransition::Depth => 220,
            WorkspaceTransition::Cube => 320,
            WorkspaceTransition::Coverflow => 320,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Effects {
    pub blur: bool,
    pub shadows: bool,
    pub rounded_corners: bool,
    pub window_animations: bool,
    /// Multiplier on every animation duration. Higher is faster.
    pub animation_speed: f32,
    pub workspace_transition: WorkspaceTransition,
    /// Strength of the RGB accent glow, 0..1. `0.0` leaves a flat accent border.
    pub rgb_glow: f32,
}

impl Default for Effects {
    fn default() -> Self {
        // Mirrors the Balanced profile, which is the documented default.
        Profile::Balanced.effects().expect("Balanced is not Custom")
    }
}

use crate::profile::Profile;

impl Effects {
    /// Duration of a workspace transition under the current settings.
    ///
    /// Returns `0` whenever animations are off, so callers can branch on the
    /// duration alone instead of checking two flags.
    pub fn transition_duration_ms(&self) -> u32 {
        if !self.window_animations {
            return 0;
        }
        let speed = self.animation_speed.max(0.05);
        (self.workspace_transition.duration_ms() as f32 / speed) as u32
    }

    /// Turn off everything that costs a GPU pass, keeping layout intact.
    pub fn minimal(self) -> Self {
        Profile::Performance.effects().expect("Performance is not Custom")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabling_animations_zeroes_the_transition() {
        let e = Effects { window_animations: false, ..Default::default() };
        assert_eq!(e.transition_duration_ms(), 0);
    }

    #[test]
    fn animation_speed_scales_the_duration() {
        let base = Effects::default().transition_duration_ms();
        let fast = Effects { animation_speed: 2.0, ..Default::default() }.transition_duration_ms();
        assert!(fast < base);
    }

    #[test]
    fn a_zero_speed_cannot_divide_by_zero() {
        let e = Effects { animation_speed: 0.0, ..Default::default() };
        assert!(e.transition_duration_ms() < 100_000);
    }

    #[test]
    fn only_the_3d_transitions_need_an_offscreen_pass() {
        assert!(WorkspaceTransition::Cube.needs_offscreen_pass());
        assert!(!WorkspaceTransition::Slide.needs_offscreen_pass());
        assert!(!WorkspaceTransition::None.needs_offscreen_pass());
    }
}
