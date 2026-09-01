//! The Spectre configuration model.
//!
//! One TOML file, `$XDG_CONFIG_HOME/spectre/spectre.toml`, describes the whole
//! desktop. Every section is optional; anything missing falls back to the
//! documented default, so a zero-byte config produces the shipped experience.
//!
//! ```
//! use spectre_config::{Config, Profile};
//!
//! let cfg: Config = toml::from_str("[general]\nprofile = \"performance\"").unwrap();
//! let cfg = cfg.resolved();
//! assert_eq!(cfg.general.profile, Profile::Performance);
//! assert!(!cfg.effects.blur, "the profile must win over the effect defaults");
//! ```

pub mod effects;
pub mod input;
pub mod keybind;
pub mod profile;

pub use effects::{Effects, WorkspaceTransition};
pub use input::{Input, Keyboard, Pointer};
pub use keybind::{Action, Direction, Keybind, Keybinds, Modifiers};
pub use profile::Profile;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use spectre_theme::Theme;

/// File name looked up inside the `spectre` config directory.
pub const CONFIG_FILE: &str = "spectre.toml";
/// Directory name under `$XDG_CONFIG_HOME` and `/etc/xdg`.
pub const CONFIG_DIR: &str = "spectre";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct General {
    pub profile: Profile,
    /// Number of virtual desktops.
    pub workspaces: u8,
    /// Commands started once the session is up.
    pub autostart: Vec<String>,
}

impl Default for General {
    fn default() -> Self {
        Self { profile: Profile::default(), workspaces: 4, autostart: Vec::new() }
    }
}

impl General {
    /// Everything to launch once the session is up.
    ///
    /// The panel is part of the desktop rather than something a user has to
    /// remember to add, so it is prepended unless `[panel] enabled = false`.
    /// It stays a normal autostart entry, which means a user who wants a
    /// different panel just turns this one off.
    pub fn startup_commands(&self, panel_enabled: bool) -> Vec<String> {
        let mut commands = Vec::with_capacity(self.autostart.len() + 1);
        if panel_enabled {
            commands.push(String::from("spectre-panel"));
        }
        commands.extend(self.autostart.iter().cloned());
        commands
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Panel {
    pub enabled: bool,
    /// Screen edge the panel is anchored to.
    pub position: PanelPosition,
    /// Floating panels leave a margin on all sides instead of spanning the edge.
    pub floating: bool,
    /// Background opacity, 0..1.
    pub opacity: f32,
    /// Widgets in display order.
    pub widgets: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PanelPosition {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

impl Default for Panel {
    fn default() -> Self {
        // The widget order from Taskleiste Concept.png, left to right.
        Self {
            enabled: true,
            position: PanelPosition::Bottom,
            floating: false,
            opacity: 1.0,
            widgets: [
                "launcher",
                "workspaces",
                "spacer",
                "tasks",
                "spacer",
                "tray",
                "resources",
                "audio",
                "network",
                "bluetooth",
                "clock",
                "session",
            ]
            .map(String::from)
            .to_vec(),
        }
    }
}

/// The complete desktop configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    pub general: General,
    pub effects: Effects,
    pub input: Input,
    pub panel: Panel,
    pub theme: Theme,
    pub keybinds: Keybinds,
}

impl Config {
    /// Apply the performance profile over the effect and theme settings.
    ///
    /// Call this once after loading. [`Profile::Custom`] is the escape hatch for
    /// users who want their `[effects]` section respected verbatim.
    pub fn resolved(mut self) -> Self {
        if let Some(effects) = self.general.profile.effects() {
            self.effects = effects;
        }
        self.theme = self.general.profile.apply_to_theme(self.theme);
        if !self.effects.window_animations {
            self.theme = self.theme.without_animation();
        }
        self.general.workspaces = self.general.workspaces.clamp(1, 9);
        self
    }

    /// Parse a config from TOML text, applying profile resolution.
    pub fn from_toml(text: &str) -> Result<Self, Error> {
        let cfg: Config = toml::from_str(text).map_err(|e| Error::Parse(e.to_string()))?;
        Ok(cfg.resolved())
    }

    /// Read the config from `path`.
    pub fn load_from(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::Read { path: path.to_owned(), source: e })?;
        Self::from_toml(&text)
    }

    /// Load the user's config, falling back to defaults.
    ///
    /// A missing file is normal and yields defaults. A *malformed* file is not:
    /// it is reported so the caller can log it, because silently running with
    /// defaults after a typo is the more confusing failure.
    pub fn load() -> (Self, Option<Error>) {
        match Self::config_path() {
            Some(path) if path.exists() => match Self::load_from(&path) {
                Ok(cfg) => (cfg, None),
                Err(e) => (Config::default().resolved(), Some(e)),
            },
            _ => (Config::default().resolved(), None),
        }
    }

    /// `$XDG_CONFIG_HOME/spectre/spectre.toml`, or the first XDG fallback that
    /// exists.
    pub fn config_path() -> Option<PathBuf> {
        let dirs = xdg::BaseDirectories::with_prefix(CONFIG_DIR);
        dirs.find_config_file(CONFIG_FILE)
            .or_else(|| dirs.get_config_home().map(|h| h.join(CONFIG_FILE)))
    }

    /// Serialise back to TOML, for `spectre-settings` writing the file.
    pub fn to_toml(&self) -> Result<String, Error> {
        toml::to_string_pretty(self).map_err(|e| Error::Serialize(e.to_string()))
    }
}

#[derive(Debug)]
pub enum Error {
    Read { path: PathBuf, source: std::io::Error },
    Parse(String),
    Serialize(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Read { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Error::Parse(m) => write!(f, "invalid configuration: {m}"),
            Error::Serialize(m) => write!(f, "cannot serialise configuration: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_config_is_valid_and_balanced() {
        let cfg = Config::from_toml("").unwrap();
        assert_eq!(cfg.general.profile, Profile::Balanced);
        assert!(cfg.panel.enabled);
        assert_eq!(cfg.general.workspaces, 4);
    }

    #[test]
    fn profile_overrides_the_effects_section() {
        let cfg = Config::from_toml(
            r#"
            [general]
            profile = "performance"
            [effects]
            blur = true
            "#,
        )
        .unwrap();
        assert!(!cfg.effects.blur, "the profile must win");
    }

    #[test]
    fn custom_profile_keeps_the_effects_section() {
        let cfg = Config::from_toml(
            r#"
            [general]
            profile = "custom"
            [effects]
            blur = true
            workspace-transition = "cube"
            "#,
        )
        .unwrap();
        assert!(cfg.effects.blur);
        assert_eq!(cfg.effects.workspace_transition, WorkspaceTransition::Cube);
    }

    #[test]
    fn disabling_animations_also_freezes_the_patterns() {
        let cfg = Config::from_toml(
            r#"
            [general]
            profile = "custom"
            [effects]
            window-animations = false
            "#,
        )
        .unwrap();
        assert!(!cfg.theme.needs_continuous_redraw());
    }

    #[test]
    fn workspace_count_is_clamped_into_range() {
        let hi = Config::from_toml("[general]\nworkspaces = 250").unwrap();
        assert_eq!(hi.general.workspaces, 9);
        let lo = Config::from_toml("[general]\nworkspaces = 0").unwrap();
        assert_eq!(lo.general.workspaces, 1);
    }

    #[test]
    fn a_typo_is_an_error_rather_than_a_silent_default() {
        let err = Config::from_toml("[general]\nprofil = \"spectre\"");
        assert!(err.is_err(), "deny_unknown_fields must catch the typo");
    }

    #[test]
    fn user_keybinds_merge_over_the_defaults() {
        let cfg = Config::from_toml(
            r#"
            [keybinds]
            "Mod+Return" = { action = "spawn", command = "foot" }
            "#,
        )
        .unwrap();
        // A [keybinds] table replaces rather than merges, so the caller opts in:
        let merged = Keybinds::default().merged_with(cfg.keybinds);
        assert_eq!(
            merged.get(&"Mod+Return".parse().unwrap()),
            Some(&Action::Spawn { command: "foot".into() })
        );
        assert_eq!(merged.get(&"Mod+q".parse().unwrap()), Some(&Action::CloseWindow));
    }

    #[test]
    fn the_panel_starts_with_the_session() {
        let cfg = Config::default().resolved();
        let commands = cfg.general.startup_commands(cfg.panel.enabled);
        assert_eq!(commands.first().map(String::as_str), Some("spectre-panel"));
    }

    #[test]
    fn disabling_the_panel_leaves_it_out_of_startup() {
        let cfg = Config::from_toml("[panel]\nenabled = false").unwrap();
        let commands = cfg.general.startup_commands(cfg.panel.enabled);
        assert!(!commands.iter().any(|c| c.contains("spectre-panel")));
    }

    #[test]
    fn user_autostart_entries_come_after_the_panel() {
        let cfg = Config::from_toml(
            r#"
            [general]
            autostart = ["nm-applet", "foot"]
            "#,
        )
        .unwrap();
        let commands = cfg.general.startup_commands(cfg.panel.enabled);
        assert_eq!(commands, ["spectre-panel", "nm-applet", "foot"]);
    }

    #[test]
    fn config_round_trips_through_toml() {
        let cfg = Config::default().resolved();
        let back = Config::from_toml(&cfg.to_toml().unwrap()).unwrap();
        assert_eq!(cfg, back);
    }
}

#[cfg(test)]
mod shipped_config_tests {
    use super::*;

    /// The commented default that gets installed to
    /// `/usr/share/spectre/spectre.toml`.
    const SHIPPED: &str = include_str!("../../spectre-session/share/spectre/spectre.toml");

    #[test]
    fn the_shipped_default_config_parses() {
        let cfg = Config::from_toml(SHIPPED)
            .unwrap_or_else(|e| panic!("the config we ship does not parse: {e}"));
        assert_eq!(cfg.general.profile, Profile::Balanced);
        assert_eq!(cfg.general.workspaces, 4);
    }

    #[test]
    fn the_shipped_default_documents_every_effect_key() {
        // A key that silently disappears from the sample is a documentation
        // bug; deny_unknown_fields only catches the opposite direction.
        for key in [
            "blur",
            "shadows",
            "rounded-corners",
            "window-animations",
            "animation-speed",
            "workspace-transition",
            "rgb-glow",
        ] {
            assert!(SHIPPED.contains(key), "the sample config never mentions `{key}`");
        }
    }
}
