//! Key bindings.
//!
//! Bindings are written the way a user would type them — `"Mod+Shift+Q"` — and
//! parsed into a [`Keybind`]. The key half stays a plain string here: resolving
//! it to a keysym needs xkbcommon, which belongs in the compositor, not in the
//! configuration model.

use std::collections::BTreeMap;
use std::fmt;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

/// Modifier mask. `Mod`/`Super`/`Logo` all mean the same physical key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Modifiers {
    pub logo: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Modifiers {
    pub const NONE: Modifiers = Modifiers { logo: false, ctrl: false, alt: false, shift: false };

    pub const fn logo() -> Self {
        Self { logo: true, ..Self::NONE }
    }

    pub fn is_empty(self) -> bool {
        self == Self::NONE
    }

    fn parse_token(&mut self, token: &str) -> bool {
        match token.to_ascii_lowercase().as_str() {
            "mod" | "super" | "logo" | "meta" | "win" => self.logo = true,
            "ctrl" | "control" => self.ctrl = true,
            "alt" | "mod1" => self.alt = true,
            "shift" => self.shift = true,
            _ => return false,
        }
        true
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in
            [(self.logo, "Mod"), (self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift")]
        {
            if on {
                write!(f, "{name}+")?;
            }
        }
        Ok(())
    }
}

/// A parsed `Modifiers + key` combination.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Keybind {
    pub mods: Modifiers,
    /// xkb keysym name, normalised to lowercase (`return`, `q`, `xf86audioraisevolume`).
    pub key: String,
}

impl Keybind {
    pub fn new(mods: Modifiers, key: impl Into<String>) -> Self {
        Self { mods, key: key.into().to_ascii_lowercase() }
    }
}

impl std::str::FromStr for Keybind {
    type Err = ParseKeybindError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut mods = Modifiers::default();
        let mut key = None;

        for token in s.split('+').map(str::trim).filter(|t| !t.is_empty()) {
            if mods.parse_token(token) {
                continue;
            }
            if key.replace(token.to_ascii_lowercase()).is_some() {
                return Err(ParseKeybindError(format!("`{s}` names more than one key")));
            }
        }

        match key {
            Some(key) => Ok(Self { mods, key }),
            None => Err(ParseKeybindError(format!("`{s}` has no key, only modifiers"))),
        }
    }
}

impl fmt::Display for Keybind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.mods, self.key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseKeybindError(String);

impl fmt::Display for ParseKeybindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseKeybindError {}

impl Serialize for Keybind {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Keybind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(de::Error::custom)
    }
}

/// What a binding does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum Action {
    /// Run a command. Split by the compositor with shell-like quoting.
    Spawn { command: String },
    CloseWindow,
    /// Leave the session.
    Quit,
    FocusNext,
    FocusPrev,
    FocusDirection { direction: Direction },
    MoveDirection { direction: Direction },
    ToggleMaximize,
    ToggleFullscreen,
    ToggleFloating,
    /// Switch to workspace `index`, 1-based to match what the panel shows.
    Workspace { index: u8 },
    MoveToWorkspace { index: u8 },
    NextWorkspace,
    PrevWorkspace,
    /// Take the focused window along to the next or previous workspace.
    MoveToNextWorkspace,
    MoveToPrevWorkspace,
    /// Flip the global animation kill switch.
    ToggleAnimations,
    /// Cycle Performance -> Balanced -> Spectre.
    CycleProfile,
    ToggleLauncher,
    LockSession,
    Screenshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// The whole binding table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Keybinds(pub BTreeMap<Keybind, Action>);

impl Keybinds {
    pub fn get(&self, bind: &Keybind) -> Option<&Action> {
        self.0.get(bind)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Keybind, &Action)> {
        self.0.iter()
    }

    /// Merge user bindings over the defaults, so a config only has to list what
    /// it changes. Binding an action to `[]` is how a user removes a default.
    pub fn merged_with(mut self, overrides: Keybinds) -> Self {
        self.0.extend(overrides.0);
        self
    }
}

impl Default for Keybinds {
    fn default() -> Self {
        let mut m = BTreeMap::new();
        let logo = Modifiers::logo();
        let logo_shift = Modifiers { shift: true, ..logo };

        let mut bind = |mods: Modifiers, key: &str, action: Action| {
            m.insert(Keybind::new(mods, key), action);
        };

        bind(logo, "return", Action::Spawn { command: "konsole".into() });
        bind(logo, "d", Action::ToggleLauncher);
        bind(logo, "comma", Action::Spawn { command: "spectre-settings".into() });
        bind(logo, "q", Action::CloseWindow);
        bind(logo, "f", Action::ToggleFullscreen);
        bind(logo, "m", Action::ToggleMaximize);
        bind(logo, "space", Action::ToggleFloating);
        bind(logo, "l", Action::LockSession);
        bind(logo, "tab", Action::FocusNext);
        bind(logo_shift, "tab", Action::FocusPrev);
        bind(logo_shift, "q", Action::Quit);
        bind(logo_shift, "a", Action::ToggleAnimations);
        bind(logo_shift, "p", Action::CycleProfile);
        bind(Modifiers::NONE, "print", Action::Screenshot);

        // The way every other desktop moves between workspaces.
        let logo_ctrl = Modifiers { ctrl: true, ..logo };
        bind(logo_ctrl, "left", Action::PrevWorkspace);
        bind(logo_ctrl, "right", Action::NextWorkspace);
        let logo_ctrl_shift = Modifiers { shift: true, ..logo_ctrl };
        bind(logo_ctrl_shift, "left", Action::MoveToPrevWorkspace);
        bind(logo_ctrl_shift, "right", Action::MoveToNextWorkspace);

        for (key, dir) in [
            ("left", Direction::Left),
            ("right", Direction::Right),
            ("up", Direction::Up),
            ("down", Direction::Down),
            ("h", Direction::Left),
            ("l", Direction::Right),
            ("k", Direction::Up),
            ("j", Direction::Down),
        ] {
            // Mod+L is the lock binding above; direction gets the arrow only.
            if !(key == "l") {
                bind(logo, key, Action::FocusDirection { direction: dir });
            }
            bind(logo_shift, key, Action::MoveDirection { direction: dir });
        }

        for i in 1..=9u8 {
            let key = i.to_string();
            bind(logo, &key, Action::Workspace { index: i });
            bind(logo_shift, &key, Action::MoveToWorkspace { index: i });
        }

        Self(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Keybind {
        s.parse().unwrap()
    }

    #[test]
    fn parses_modifier_aliases_to_the_same_bind() {
        assert_eq!(parse("Mod+Q"), parse("Super+q"));
        assert_eq!(parse("Mod+Q"), parse("logo+Q"));
        assert_eq!(parse("Ctrl+Alt+T"), parse("control+mod1+t"));
    }

    #[test]
    fn key_is_case_insensitive() {
        assert_eq!(parse("Mod+Return").key, "return");
    }

    #[test]
    fn rejects_bindings_without_a_key() {
        assert!("Mod+Shift".parse::<Keybind>().is_err());
        assert!("".parse::<Keybind>().is_err());
    }

    #[test]
    fn rejects_two_keys() {
        assert!("Mod+q+w".parse::<Keybind>().is_err());
    }

    #[test]
    fn display_round_trips() {
        let b = parse("Mod+Shift+q");
        assert_eq!(b.to_string(), "Mod+Shift+q");
        assert_eq!(parse(&b.to_string()), b);
    }

    #[test]
    fn defaults_cover_all_nine_workspaces() {
        let k = Keybinds::default();
        for i in 1..=9u8 {
            let bind = Keybind::new(Modifiers::logo(), i.to_string());
            assert_eq!(k.get(&bind), Some(&Action::Workspace { index: i }), "Mod+{i} missing");
        }
    }

    #[test]
    fn mod_l_locks_rather_than_moving_focus() {
        let k = Keybinds::default();
        assert_eq!(k.get(&parse("Mod+l")), Some(&Action::LockSession));
    }

    #[test]
    fn user_bindings_override_defaults() {
        let mine = Keybinds(
            [(parse("Mod+Return"), Action::Spawn { command: "foot".into() })].into_iter().collect(),
        );
        let merged = Keybinds::default().merged_with(mine);
        assert_eq!(
            merged.get(&parse("Mod+Return")),
            Some(&Action::Spawn { command: "foot".into() })
        );
        assert_eq!(merged.get(&parse("Mod+q")), Some(&Action::CloseWindow), "others survive");
    }

    #[test]
    fn actions_round_trip_through_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W {
            binds: Keybinds,
        }
        let w = W { binds: Keybinds::default() };
        let back: W = toml::from_str(&toml::to_string(&w).unwrap()).unwrap();
        assert_eq!(w, back);
    }
}
