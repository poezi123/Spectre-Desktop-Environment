use serde::{Deserialize, Serialize};
use spectre_config::Profile;

pub type WindowId = u64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "kebab-case")]
pub enum Request {
    Subscribe,
    GetState,
    SwitchWorkspace { index: u8 },
    ActivateWindow { id: WindowId },
    MinimizeWindow { id: WindowId },
    CloseWindow { id: WindowId },
    SetProfile { profile: Profile },
    SetAnimations { enabled: bool },
    ReloadConfig,
    ToggleLauncher,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    State(Desktop),
    ConfigChanged,
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct Desktop {
    pub workspaces: Vec<Workspace>,
    pub windows: Vec<Window>,
    pub profile: Profile,
    pub animations: bool,
    #[serde(default)]
    pub outputs: Vec<Output>,
    #[serde(default)]
    pub resting: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Output {
    pub name: String,
    pub modes: Vec<Mode>,
    pub current: Option<Mode>,
    pub scale: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    pub refresh: u32,
}

impl Mode {
    pub fn label(&self) -> String {
        format!("{}x{}@{}", self.width, self.height, self.refresh)
    }
}

impl Desktop {
    pub fn active_workspace(&self) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.active)
    }

    pub fn focused_window(&self) -> Option<&Window> {
        self.windows.iter().find(|w| w.focused)
    }

    pub fn visible_windows(&self) -> impl Iterator<Item = &Window> {
        let active = self.active_workspace().map(|w| w.index);
        self.windows.iter().filter(move |w| Some(w.workspace) == active)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Workspace {
    pub index: u8,
    pub active: bool,
    pub windows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Window {
    pub id: WindowId,
    pub title: String,
    pub app_id: String,
    pub workspace: u8,
    pub focused: bool,
    pub minimized: bool,
    #[serde(default)]
    pub fullscreen: bool,
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
            fullscreen: false,
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
            Request::ReloadConfig,
            Request::ToggleLauncher,
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
            resting: false,
            outputs: vec![Output {
                name: String::from("Virtual-1"),
                modes: vec![Mode { width: 1920, height: 1080, refresh: 60 }],
                current: Some(Mode { width: 1920, height: 1080, refresh: 60 }),
                scale: 1.0,
            }],
        });
        let line = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), event);
    }

    #[test]
    fn a_client_built_before_outputs_existed_still_parses_a_state() {
        let line = r#"{"event":"state","workspaces":[],"windows":[],"profile":"balanced","animations":true}"#;
        let Event::State(desktop) = serde_json::from_str::<Event>(line).unwrap() else {
            panic!("not a state event");
        };
        assert!(desktop.outputs.is_empty());
    }

    #[test]
    fn a_mode_is_labelled_the_way_the_config_writes_it() {
        assert_eq!(Mode { width: 1920, height: 1080, refresh: 60 }.label(), "1920x1080@60");
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
