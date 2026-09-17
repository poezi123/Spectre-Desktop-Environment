use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceTransition {
    None,
    Fade,
    #[default]
    Slide,
    Depth,
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

    pub fn needs_offscreen_pass(self) -> bool {
        matches!(self, WorkspaceTransition::Cube | WorkspaceTransition::Coverflow)
    }

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
    pub animation_speed: f32,
    pub workspace_transition: WorkspaceTransition,
    pub rgb_glow: f32,
    pub pattern_frames_per_second: u8,
}

impl Default for Effects {
    fn default() -> Self {
        Profile::Balanced.effects().expect("Balanced is not Custom")
    }
}

use crate::profile::Profile;

pub const PATTERN_RATES: [(u8, &str); 3] =
    [(30, "Smooth (30 fps)"), (15, "Light (15 fps)"), (8, "Very light (8 fps)")];

impl Effects {
    pub fn pattern_frame_gap(&self) -> std::time::Duration {
        let frames = self.pattern_frames_per_second.clamp(4, 60);
        std::time::Duration::from_secs_f32(1.0 / frames as f32)
    }

    pub fn pattern_interval(
        &self,
        pattern: &spectre_theme::Pattern,
        scale: f32,
    ) -> Option<std::time::Duration> {
        let interval = pattern.redraw_interval(scale)?;
        Some(interval.max(self.pattern_frame_gap()))
    }

    pub fn transition_duration_ms(&self) -> u32 {
        if !self.window_animations {
            return 0;
        }
        let speed = self.animation_speed.max(0.05);
        (self.workspace_transition.duration_ms() as f32 / speed) as u32
    }

    pub fn minimal(self) -> Self {
        Profile::Performance.effects().expect("Performance is not Custom")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pattern_never_asks_for_more_frames_than_the_setting_allows() {
        let pattern = spectre_theme::Pattern::default();
        let smooth = Effects { pattern_frames_per_second: 30, ..Default::default() };
        let light = Effects { pattern_frames_per_second: 8, ..Default::default() };

        let quick = smooth.pattern_interval(&pattern, 1.0).expect("it moves");
        let slow = light.pattern_interval(&pattern, 1.0).expect("it still moves");
        assert!(slow > quick, "{slow:?} must be rarer than {quick:?}");
        assert_eq!(slow, std::time::Duration::from_secs_f32(1.0 / 8.0));
    }

    #[test]
    fn a_nonsense_frame_rate_is_pulled_back_into_reach() {
        let none = Effects { pattern_frames_per_second: 0, ..Default::default() };
        let mad = Effects { pattern_frames_per_second: 240, ..Default::default() };
        assert_eq!(none.pattern_frame_gap(), std::time::Duration::from_secs_f32(1.0 / 4.0));
        assert_eq!(mad.pattern_frame_gap(), std::time::Duration::from_secs_f32(1.0 / 60.0));
    }

    #[test]
    fn a_still_pattern_asks_for_nothing_whatever_the_frame_rate() {
        let pattern = spectre_theme::Pattern::default().without_animation();
        let effects = Effects::default();
        assert!(effects.pattern_interval(&pattern, 1.0).is_none());
    }

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
