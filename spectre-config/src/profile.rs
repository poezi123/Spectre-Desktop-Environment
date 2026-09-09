use serde::{Deserialize, Serialize};
use spectre_theme::{Pattern, Theme};

use crate::effects::{Effects, WorkspaceTransition};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    Performance,
    #[default]
    Balanced,
    Spectre,
    Custom,
}

impl Profile {
    pub const ALL: [Profile; 4] =
        [Profile::Performance, Profile::Balanced, Profile::Spectre, Profile::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Profile::Performance => "Performance",
            Profile::Balanced => "Balanced",
            Profile::Spectre => "Spectre",
            Profile::Custom => "Custom",
        }
    }

    pub fn effects(self) -> Option<Effects> {
        Some(match self {
            Profile::Performance => Effects {
                blur: false,
                shadows: false,
                rounded_corners: false,
                window_animations: false,
                animation_speed: 1.0,
                workspace_transition: WorkspaceTransition::None,
                rgb_glow: 0.0,
            },
            Profile::Balanced => Effects {
                blur: false,
                shadows: true,
                rounded_corners: true,
                window_animations: true,
                animation_speed: 1.0,
                workspace_transition: WorkspaceTransition::Slide,
                rgb_glow: 0.45,
            },
            Profile::Spectre => Effects {
                blur: true,
                shadows: true,
                rounded_corners: true,
                window_animations: true,
                animation_speed: 1.0,
                workspace_transition: WorkspaceTransition::Cube,
                rgb_glow: 1.0,
            },
            Profile::Custom => return None,
        })
    }

    pub fn apply_to_theme(self, theme: Theme) -> Theme {
        match self {
            Profile::Performance => theme.without_animation(),
            Profile::Balanced => theme.with_static_lines(),
            Profile::Spectre => Theme {
                desktop_pattern: spectre_theme::DesktopPattern(Pattern::default()),
                ..theme
            },
            Profile::Custom => theme,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn performance_disables_every_expensive_effect() {
        let e = Profile::Performance.effects().unwrap();
        assert!(!e.blur && !e.window_animations);
        assert_eq!(e.workspace_transition, WorkspaceTransition::None);
        assert_eq!(e.rgb_glow, 0.0);
    }

    #[test]
    fn custom_leaves_effects_alone() {
        assert!(Profile::Custom.effects().is_none());
    }

    #[test]
    fn performance_freezes_patterns_instead_of_removing_them() {
        let t = Profile::Performance.apply_to_theme(Theme::default());
        assert!(!t.needs_continuous_redraw());
        assert!(!t.window_pattern.is_noop(), "the pattern must still be drawn");
    }

    #[test]
    fn spectre_turns_on_the_desktop_pattern() {
        let t = Profile::Spectre.apply_to_theme(Theme::default());
        assert!(!t.desktop_pattern.is_noop());
        assert!(t.needs_continuous_redraw(), "the Spectre profile is the animated one");
    }

    #[test]
    fn balanced_freezes_the_lines_but_keeps_the_colours_moving() {
        let t = Profile::Balanced.apply_to_theme(Theme::default());
        assert!(!t.window_pattern.is_noop(), "the texture must stay visible");
        assert_eq!(t.window_pattern.phase(10.0), 0.0, "the field must stand still");
        assert!(t.window_pattern.cycles_color());
    }
}
