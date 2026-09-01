//! Performance profiles.
//!
//! A profile is a preset over the effect switches, never over functionality:
//! per the project principles, switching to `Performance` may remove blur and
//! 3D workspace transitions, but it must never remove a window button, a
//! keybinding or a panel widget.

use serde::{Deserialize, Serialize};
use spectre_theme::{Pattern, Theme};

use crate::effects::{Effects, WorkspaceTransition};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    /// Old laptops, VMs and low-power hardware.
    Performance,
    /// The intended default.
    #[default]
    Balanced,
    /// Modern hardware — everything on.
    Spectre,
    /// Nothing is forced; the `[effects]` section is taken verbatim.
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

    /// The effect set this profile prescribes, or `None` for [`Profile::Custom`].
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

    /// Apply the profile's pattern policy to a theme.
    ///
    /// Patterns are always drawn; profiles only decide whether they move. That
    /// is the design rule - a disabled animation makes the pattern static, it
    /// never makes it disappear.
    ///
    /// `Balanced` keeps them static on purpose. An animated pattern means a new
    /// frame every vblank for the whole output, and on the low-end hardware
    /// this project treats as a first-class target that cost buys a texture
    /// most people will not notice moving. `Spectre` is where motion lives.
    pub fn apply_to_theme(self, theme: Theme) -> Theme {
        match self {
            Profile::Performance | Profile::Balanced => theme.without_animation(),
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
    fn balanced_shows_the_pattern_without_animating_it() {
        let t = Profile::Balanced.apply_to_theme(Theme::default());
        assert!(!t.window_pattern.is_noop(), "the texture must stay visible");
        assert!(!t.needs_continuous_redraw(), "no frame may be drawn just to move it");
    }
}
