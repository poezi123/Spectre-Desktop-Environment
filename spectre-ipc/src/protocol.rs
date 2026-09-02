//! Messages exchanged between the compositor and its shell components.
//!
//! Newline-delimited JSON over a Unix stream socket. JSON rather than a packed
//! binary format because the traffic is a handful of messages per second and
//! being able to debug the desktop with `socat` is worth more than the bytes.

use serde::{Deserialize, Serialize};
use spectre_config::Profile;

/// Opaque, stable-for-its-lifetime handle to a window.
pub type WindowId = u64;

/// Sent by a client to the compositor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "kebab-case")]
pub enum Request {
    /// Send the current state now, and again whenever it changes.
    Subscribe,
    /// Send the current state once.
    GetState,
    /// Switch to a workspace. 1-based, matching what the panel shows.
    SwitchWorkspace { index: u8 },
    /// Focus a window, restoring it first if it is minimized.
    ActivateWindow { id: WindowId },
    MinimizeWindow { id: WindowId },
    CloseWindow { id: WindowId },
    /// Change the performance profile at runtime.
    SetProfile { profile: Profile },
    /// Turn every animation on or off.
    SetAnimations { enabled: bool },
    /// Re-read the configuration file and apply it.
    ReloadConfig,
    /// End the session.
    Quit,
}

/// Sent by the compositor to a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    /// The full desktop state. Sent on subscribe and after every change.
    State(Desktop),
    /// The configuration was re-read; shell components should reload too.
    ConfigChanged,
    /// A request could not be carried out. Advisory: the connection stays open.
    Error { message: String },
}

/// Everything a panel needs to draw itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct Desktop {
    pub workspaces: Vec<Workspace>,
    pub windows: Vec<Window>,
    pub profile: Profile,
    /// Mirrors the animation kill switch.
    pub animations: bool,
}

impl Desktop {
    pub fn active_workspace(&self) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.active)
    }

    pub fn focused_window(&self) -> Option<&Window> {
        self.windows.iter().find(|w| w.focused)
    }

    /// Windows on the visible workspace, in stacking order.
    pub fn visible_windows(&self) -> impl Iterator<Item = &Window> {
        let active = self.active_workspace().map(|w| w.index);
        self.windows.iter().filter(move |w| Some(w.workspace) == active)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Workspace {
    /// 1-based.
    pub index: u8,
    pub active: bool,
    /// How many windows live here, including minimized ones.
    pub windows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Window {
    pub id: WindowId,
    pub title: String,
    pub app_id: String,
    /// 1-based workspace index.
    pub workspace: u8,
    pub focused: bool,
    pub minimized: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(index: u8, active: bool) -> Workspace {
        Workspace { index, active, windows: 0 }
    }

    fn window(id: WindowId, workspace: u8, focused: bool) -> Window {
        Window {
            id,
            title: format!("window {id}"),
            app_id: "test".into(),
            workspace,
            focused,
            minimized: false,
        }
    }

    #[test]
    fn requests_round_trip_through_json() {
        let all = [
            Request::Subscribe,
            Request::GetState,
            Request::SwitchWorkspace { index: 3 },
            Request::ActivateWindow { id: 7 },
            Request::MinimizeWindow { id: 7 },
            Request::CloseWindow { id: 7 },
            Request::SetProfile { profile: Profile::Spectre },
            Request::SetAnimations { enabled: false },
            Request::Quit,
        ];
        for request in all {
            let line = serde_json::to_string(&request).unwrap();
            assert!(!line.contains('\n'), "a message must fit on one line");
            assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), request);
        }
    }

    #[test]
    fn events_round_trip_through_json() {
        let event = Event::State(Desktop {
            workspaces: vec![workspace(1, true), workspace(2, false)],
            windows: vec![window(1, 1, true)],
            profile: Profile::Balanced,
            animations: true,
        });
        let line = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), event);
    }

    #[test]
    fn a_title_with_a_newline_cannot_break_the_framing() {
        let event = Event::State(Desktop {
            windows: vec![Window { title: "evil\ntitle".into(), ..window(1, 1, false) }],
            ..Default::default()
        });
        let line = serde_json::to_string(&event).unwrap();
        assert!(!line.contains('\n'), "JSON must escape the newline, not emit it");
        assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), event);
    }

    #[test]
    fn an_unknown_request_is_rejected_rather_than_guessed() {
        assert!(serde_json::from_str::<Request>(r#"{"request":"self-destruct"}"#).is_err());
        assert!(serde_json::from_str::<Request>("not json").is_err());
    }

    #[test]
    fn active_workspace_and_focus_are_found() {
        let d = Desktop {
            workspaces: vec![workspace(1, false), workspace(2, true)],
            windows: vec![window(1, 1, false), window(2, 2, true)],
            ..Default::default()
        };
        assert_eq!(d.active_workspace().unwrap().index, 2);
        assert_eq!(d.focused_window().unwrap().id, 2);
        assert_eq!(d.visible_windows().map(|w| w.id).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn an_empty_desktop_answers_without_panicking() {
        let d = Desktop::default();
        assert!(d.active_workspace().is_none());
        assert!(d.focused_window().is_none());
        assert_eq!(d.visible_windows().count(), 0);
    }
}
