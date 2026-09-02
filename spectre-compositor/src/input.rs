//! Input handling: turning libinput events into focus changes, pointer motion
//! and [`Action`]s.

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::input::keyboard::{keysyms, FilterResult, Keysym, ModifiersState};
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, Focus, GrabStartData, MotionEvent, RelativeMotionEvent,
};
use smithay::utils::{Point, SERIAL_COUNTER};
use spectre_config::{Action, Keybind, Modifiers, Profile};

use crate::grabs::{MoveGrab, BTN_LEFT};
use crate::render::Part;
use crate::state::Spectre;

/// Two clicks closer together than this on a title bar count as a double click.
const DOUBLE_CLICK_MS: u32 = 400;

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
                // Compare against the unmodified symbol so `Mod+Shift+q` matches
                // the physical Q key rather than the shifted keysym.
                let sym = handle.raw_syms().first().copied().unwrap_or(handle.modified_sym());
                let is_logo =
                    matches!(sym.raw(), keysyms::KEY_Super_L | keysyms::KEY_Super_R);

                // Only presses trigger bindings; releases always go to the client
                // so a client never sees a press without its release.
                if event.state() != smithay::backend::input::KeyState::Pressed {
                    if is_logo && std::mem::take(&mut state.logo_armed) {
                        return FilterResult::Intercept(Some(Action::ToggleLauncher));
                    }
                    return FilterResult::Forward;
                }

                // A tap of the logo key on its own opens the menu; pressing it
                // as part of a binding, or pressing anything else while it is
                // held, disarms that.
                state.logo_armed =
                    is_logo && !modifiers.ctrl && !modifiers.alt && !modifiers.shift;

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
                self.switch_workspace(index.saturating_sub(1) as usize);
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
                self.switch_workspace_relative(1);
            }
            Action::PrevWorkspace => {
                self.switch_workspace_relative(-1);
            }
            Action::ToggleAnimations => self.toggle_animations(),
            Action::CycleProfile => self.cycle_profile(),
            Action::ToggleLauncher => self.toggle_launcher(),
            Action::LockSession => self.spawn_configured("lock"),
            Action::Screenshot => self.spawn_configured("screenshot"),
        }
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
        let _ = self.spawn_pid(command);
    }

    /// Spawn `command`, returning the child's pid.
    pub fn spawn_pid(&self, command: &str) -> Option<u32> {
        let Some(argv) = shell_split(command) else {
            tracing::warn!(%command, "unbalanced quotes in spawn command");
            return None;
        };
        let Some((program, args)) = argv.split_first() else {
            return None;
        };

        let mut cmd = std::process::Command::new(program);
        if let Some(socket) = self.ipc_socket_path() {
            cmd.env(spectre_ipc::SOCKET_ENV, socket);
        }
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
                // Nothing waits on it: SIGCHLD is ignored, so the kernel reaps.
                let pid = child.id();
                drop(child);
                Some(pid)
            }
            Err(err) => {
                tracing::warn!(?err, %program, "failed to spawn");
                None
            }
        }
    }

    /// Open the application menu, or close it if it is already up.
    fn toggle_launcher(&mut self) {
        if let Some(pid) = self.launcher.take() {
            // Signal 0 only asks whether the process is still there.
            let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
            if alive {
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
                return;
            }
        }
        self.launcher = self.spawn_pid("spectre-launcher");
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
        let time = event.time_msec();

        if state == ButtonState::Pressed {
            // Clicking while the logo key is held is a drag, not a menu tap.
            self.logo_armed = false;
            // Decorations are ours: a press on one is handled here and never
            // reaches the client, which would otherwise see a stray click.
            if let Some((window, part)) = self.decoration_under_pointer() {
                self.focus_window(Some(&window));
                self.on_decoration_press(&window, part, button, serial, time);
                return;
            }
            // Click to focus anywhere inside a window.
            if let Some(window) = self.window_under_pointer() {
                if self.focus.as_ref() != Some(&window) {
                    self.focus_window(Some(&window));
                }
            }
        }

        let pointer = self.pointer.clone();
        pointer.button(self, &ButtonEvent { button, state, serial, time });
        pointer.frame(self);
    }

    /// Act on a press over a window's frame.
    fn on_decoration_press(
        &mut self,
        window: &smithay::desktop::Window,
        part: Part,
        button: u32,
        serial: smithay::utils::Serial,
        time: u32,
    ) {
        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as Top;

        if button != BTN_LEFT {
            return;
        }

        match part {
            Part::Close => {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_close();
                }
            }
            Part::Minimize => self.minimize(window),
            Part::Maximize => {
                let on = self.has_state(window, Top::Maximized);
                self.set_maximized(window, !on);
            }
            Part::Titlebar => {
                if self.is_double_click(window, time) {
                    let on = self.has_state(window, Top::Maximized);
                    self.set_maximized(window, !on);
                    return;
                }
                // A maximized window is not draggable: it has no position to
                // drag to until the user restores it.
                if self.has_state(window, Top::Maximized) {
                    return;
                }
                self.start_move(window, serial);
            }
            Part::Border => {}
        }
    }

    /// Whether this title bar press completes a double click.
    fn is_double_click(&mut self, window: &smithay::desktop::Window, time: u32) -> bool {
        let same = self.last_click.as_ref().is_some_and(|(w, t)| {
            w == window && time.saturating_sub(*t) <= DOUBLE_CLICK_MS
        });
        // Consume the click either way, so a triple click is not two doubles.
        self.last_click = if same { None } else { Some((window.clone(), time)) };
        same
    }

    /// Begin dragging `window` by its title bar.
    fn start_move(&mut self, window: &smithay::desktop::Window, serial: smithay::utils::Serial) {
        let Some(location) = self.workspaces.active().element_location(window) else {
            return;
        };
        let start_data = GrabStartData {
            focus: None,
            button: BTN_LEFT,
            location: self.pointer.current_location(),
        };
        let grab = MoveGrab::new(start_data, window.clone(), location);
        let pointer = self.pointer.clone();
        pointer.set_grab(self, grab, serial, Focus::Clear);
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
