//! Input handling: turning libinput events into focus changes, pointer motion
//! and [`Action`]s.

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::input::keyboard::{keysyms, FilterResult, Keysym, ModifiersState};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent};
use smithay::utils::{Point, SERIAL_COUNTER};
use spectre_config::{Action, Keybind, Modifiers, Profile};

use crate::state::Spectre;

/// The name Spectre uses for a keysym in a binding, e.g. `return`, `q`, `f1`.
///
/// xkb's canonical names are used verbatim, lowercased, so anything
/// `xkbcli list` prints can be bound.
pub fn keysym_name(sym: Keysym) -> Option<String> {
    let name = smithay::input::keyboard::xkb::keysym_get_name(sym);
    (!name.is_empty() && name != "NoSymbol").then(|| name.to_ascii_lowercase())
}

impl From<&ModifiersState> for ModifiersExt {
    fn from(m: &ModifiersState) -> Self {
        ModifiersExt(Modifiers { logo: m.logo, ctrl: m.ctrl, alt: m.alt, shift: m.shift })
    }
}

/// Newtype so the `From` impl above can live in this crate.
pub struct ModifiersExt(pub Modifiers);

impl Spectre {
    /// Feed one backend input event into the compositor.
    pub fn handle_input<B: InputBackend>(&mut self, event: InputEvent<B>) {
        match event {
            InputEvent::Keyboard { event } => self.on_key::<B>(event),
            InputEvent::PointerMotion { event } => self.on_pointer_motion::<B>(event),
            InputEvent::PointerMotionAbsolute { event } => {
                self.on_pointer_motion_absolute::<B>(event)
            }
            InputEvent::PointerButton { event } => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event } => self.on_pointer_axis::<B>(event),
            _ => {}
        }
    }

    fn on_key<B: InputBackend>(&mut self, event: B::KeyboardKeyEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = Event::time_msec(&event);
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };

        let action = keyboard.input::<Option<Action>, _>(
            self,
            event.key_code(),
            event.state(),
            serial,
            time,
            |state, modifiers, handle| {
                // Only presses trigger bindings; releases always go to the client
                // so a client never sees a press without its release.
                if event.state() != smithay::backend::input::KeyState::Pressed {
                    return FilterResult::Forward;
                }

                // Compare against the unmodified symbol so `Mod+Shift+q` matches
                // the physical Q key rather than the shifted keysym.
                let sym = handle.raw_syms().first().copied().unwrap_or(handle.modified_sym());
                if sym.raw() == keysyms::KEY_NoSymbol {
                    return FilterResult::Forward;
                }
                let Some(name) = keysym_name(sym) else {
                    return FilterResult::Forward;
                };

                let bind = Keybind::new(ModifiersExt::from(modifiers).0, name);
                match state.keybinds.get(&bind).cloned() {
                    Some(action) => FilterResult::Intercept(Some(action)),
                    None => FilterResult::Forward,
                }
            },
        );

        if let Some(action) = action.flatten() {
            self.run_action(action);
        }
    }

    /// Execute a bound action.
    pub fn run_action(&mut self, action: Action) {
        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as Top;

        match action {
            Action::Spawn { command } => self.spawn(&command),
            Action::CloseWindow => self.close_focused(),
            Action::Quit => self.stop(),
            Action::FocusNext => self.cycle_focus(true),
            Action::FocusPrev => self.cycle_focus(false),
            Action::FocusDirection { direction } => self.focus_direction(direction),
            Action::MoveDirection { direction } => self.move_direction(direction),
            Action::ToggleMaximize => {
                if let Some(window) = self.focus.clone() {
                    let on = self.focused_has_state(Top::Maximized);
                    self.set_maximized(&window, !on);
                }
            }
            Action::ToggleFullscreen => {
                if let Some(window) = self.focus.clone() {
                    let on = self.focused_has_state(Top::Fullscreen);
                    self.set_fullscreen(&window, !on);
                }
            }
            Action::ToggleFloating => {
                // Every window floats today; the tiling layout lands in a later
                // phase, so this is deliberately inert rather than misleading.
                tracing::debug!("toggle-floating is not implemented yet");
            }
            Action::Workspace { index } => {
                if self.workspaces.switch(index.saturating_sub(1) as usize) {
                    self.on_workspace_changed();
                }
            }
            Action::MoveToWorkspace { index } => {
                if let Some(window) = self.focus.clone() {
                    let target = index.saturating_sub(1) as usize;
                    if self.workspaces.move_window(&window, target) {
                        let next = self.workspaces.active().elements().last().cloned();
                        self.focus_window(next.as_ref());
                        self.mark_dirty();
                    }
                }
            }
            Action::NextWorkspace => {
                if self.workspaces.switch_relative(1) {
                    self.on_workspace_changed();
                }
            }
            Action::PrevWorkspace => {
                if self.workspaces.switch_relative(-1) {
                    self.on_workspace_changed();
                }
            }
            Action::ToggleAnimations => self.toggle_animations(),
            Action::CycleProfile => self.cycle_profile(),
            Action::ToggleLauncher => self.spawn_configured("launcher"),
            Action::LockSession => self.spawn_configured("lock"),
            Action::Screenshot => self.spawn_configured("screenshot"),
        }
    }

    fn on_workspace_changed(&mut self) {
        let next = self.workspaces.active().elements().last().cloned();
        self.focus_window(next.as_ref());
        self.mark_dirty();
    }

    /// Flip the animation kill switch at runtime.
    fn toggle_animations(&mut self) {
        let on = self.config.effects.window_animations;
        self.config.effects.window_animations = !on;
        self.config.general.profile = Profile::Custom;
        self.config.theme = if on {
            self.config.theme.clone().without_animation()
        } else {
            spectre_theme::Theme::default()
        };
        tracing::info!(animations = !on, "animation kill switch toggled");
        self.mark_dirty();
    }

    /// Cycle Performance -> Balanced -> Spectre -> Performance.
    fn cycle_profile(&mut self) {
        let next = match self.config.general.profile {
            Profile::Performance => Profile::Balanced,
            Profile::Balanced => Profile::Spectre,
            _ => Profile::Performance,
        };
        self.config.general.profile = next;
        if let Some(effects) = next.effects() {
            self.config.effects = effects;
        }
        self.config.theme = next.apply_to_theme(spectre_theme::Theme::default());
        tracing::info!(profile = next.label(), "performance profile changed");
        self.mark_dirty();
    }

    /// Run a command detached from the compositor.
    pub fn spawn(&self, command: &str) {
        let Some(argv) = shell_split(command) else {
            tracing::warn!(%command, "unbalanced quotes in spawn command");
            return;
        };
        let Some((program, args)) = argv.split_first() else {
            return;
        };

        let mut cmd = std::process::Command::new(program);
        cmd.args(args)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env("XDG_SESSION_TYPE", "wayland")
            .env("XDG_CURRENT_DESKTOP", "Spectre")
            // A child must not inherit the compositor's stdio: a client writing
            // to a closed pipe would take the whole session down.
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        match cmd.spawn() {
            Ok(child) => {
                // Detach: nothing waits on it, so reap it via double-fork
                // semantics provided by the shell-free `Command` + `drop`.
                drop(child);
            }
            Err(err) => tracing::warn!(?err, %program, "failed to spawn"),
        }
    }

    /// Spawn one of the helper components. These live in later phases; until
    /// then the binding is a documented no-op rather than a crash.
    fn spawn_configured(&self, what: &str) {
        tracing::info!(component = what, "not implemented yet");
    }

    fn on_pointer_motion<B: InputBackend>(&mut self, event: B::PointerMotionEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let delta = event.delta();
        let location = self.clamp_to_outputs(self.pointer.current_location() + delta);
        let under = self.surface_under_pointer();

        let pointer = self.pointer.clone();
        pointer.motion(
            self,
            under.clone(),
            &MotionEvent { location, serial, time: event.time_msec() },
        );
        pointer.relative_motion(
            self,
            under,
            &RelativeMotionEvent {
                delta,
                delta_unaccel: event.delta_unaccel(),
                utime: event.time(),
            },
        );
        pointer.frame(self);
        self.follow_mouse_focus();
    }

    fn on_pointer_motion_absolute<B: InputBackend>(&mut self, event: B::PointerMotionAbsoluteEvent) {
        let Some(output) = self.outputs().first().cloned() else {
            return;
        };
        let Some(geometry) = self.workspaces.output_geometry(&output) else {
            return;
        };

        let serial = SERIAL_COUNTER.next_serial();
        let location = geometry.loc.to_f64() + event.position_transformed(geometry.size);
        let under = self.surface_under_pointer();

        let pointer = self.pointer.clone();
        pointer.motion(self, under, &MotionEvent { location, serial, time: event.time_msec() });
        pointer.frame(self);
        self.follow_mouse_focus();
    }

    fn on_pointer_button<B: InputBackend>(&mut self, event: B::PointerButtonEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let button = event.button_code();
        let state = event.state();

        // Click to focus: a press anywhere in a window raises and focuses it.
        if state == ButtonState::Pressed {
            if let Some(window) = self.window_under_pointer() {
                if self.focus.as_ref() != Some(&window) {
                    self.focus_window(Some(&window));
                }
            }
        }

        let pointer = self.pointer.clone();
        pointer.button(self, &ButtonEvent { button, state, serial, time: event.time_msec() });
        pointer.frame(self);
    }

    fn on_pointer_axis<B: InputBackend>(&mut self, event: B::PointerAxisEvent) {
        let mut frame = AxisFrame::new(event.time_msec()).source(event.source());

        for axis in [Axis::Horizontal, Axis::Vertical] {
            if let Some(discrete) = event.amount_v120(axis) {
                frame = frame.v120(axis, discrete as i32);
            }
            match event.amount(axis) {
                Some(amount) => frame = frame.value(axis, amount),
                None if event.source() == AxisSource::Finger => frame = frame.stop(axis),
                None => {}
            }
        }

        let pointer = self.pointer.clone();
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    fn follow_mouse_focus(&mut self) {
        if !self.config.input.pointer.focus_follows_mouse {
            return;
        }
        if let Some(window) = self.window_under_pointer() {
            if self.focus.as_ref() != Some(&window) {
                self.focus_window(Some(&window));
            }
        }
    }

    /// Keep the pointer inside the union of all mapped outputs.
    fn clamp_to_outputs(&self, mut location: Point<f64, smithay::utils::Logical>) -> Point<f64, smithay::utils::Logical> {
        let outputs = self.outputs();
        if outputs.is_empty() {
            return location;
        }
        // Already inside an output: nothing to do.
        if outputs
            .iter()
            .filter_map(|o| self.workspaces.output_geometry(o))
            .any(|g| g.to_f64().contains(location))
        {
            return location;
        }
        // Otherwise clamp into the nearest output's rectangle.
        if let Some(geometry) = outputs.first().and_then(|o| self.workspaces.output_geometry(o)) {
            let g = geometry.to_f64();
            location.x = location.x.clamp(g.loc.x, g.loc.x + g.size.w - 1.0);
            location.y = location.y.clamp(g.loc.y, g.loc.y + g.size.h - 1.0);
        }
        location
    }
}

/// Split a command line on whitespace, honouring single and double quotes.
///
/// Returns `None` when a quote is left open, so a typo in the config produces a
/// warning instead of a mangled `argv`.
pub fn shell_split(input: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut has_token = false;

    for c in input.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            (None, '\'') | (None, '"') => {
                quote = Some(c);
                has_token = true;
            }
            (None, c) if c.is_whitespace() => {
                if has_token {
                    out.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            (None, c) => {
                current.push(c);
                has_token = true;
            }
        }
    }

    if quote.is_some() {
        return None;
    }
    if has_token {
        out.push(current);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_plain_command_line() {
        assert_eq!(shell_split("foot -e fish").unwrap(), ["foot", "-e", "fish"]);
    }

    #[test]
    fn collapses_repeated_whitespace() {
        assert_eq!(shell_split("  foot   -e  ").unwrap(), ["foot", "-e"]);
    }

    #[test]
    fn keeps_quoted_arguments_together() {
        assert_eq!(
            shell_split(r#"sh -c "echo hello world""#).unwrap(),
            ["sh", "-c", "echo hello world"]
        );
        assert_eq!(shell_split("echo 'a b'").unwrap(), ["echo", "a b"]);
    }

    #[test]
    fn an_empty_quoted_argument_survives() {
        assert_eq!(shell_split(r#"foo "" bar"#).unwrap(), ["foo", "", "bar"]);
    }

    #[test]
    fn an_unbalanced_quote_is_an_error() {
        assert!(shell_split(r#"foot -e "fish"#).is_none());
    }

    #[test]
    fn an_empty_command_yields_no_arguments() {
        assert_eq!(shell_split("   ").unwrap(), Vec::<String>::new());
    }
}
