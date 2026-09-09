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
}

impl Default for Effects {
    fn default() -> Self {
        Profile::Balanced.effects().expect("Balanced is not Custom")
    }
}

use crate::profile::Profile;

impl Effects {
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
