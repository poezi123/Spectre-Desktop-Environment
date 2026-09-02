//! The compositor state: every Wayland global Spectre exposes, plus the
//! desktop model (workspaces, focus, outputs) that the backends drive.

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Instant;

use smithay::desktop::{PopupManager, Window};
use smithay::input::keyboard::XkbConfig;
use smithay::input::pointer::{CursorImageStatus, PointerHandle};
use smithay::input::{Seat, SeatState};
use smithay::output::Output;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use smithay::utils::{Clock, Monotonic};
use smithay::wayland::compositor::{CompositorClientState, CompositorState};
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufState};
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shm::ShmState;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::socket::ListeningSocketSource;
use spectre_config::{Config, Keybinds};

use crate::workspace::Workspaces;

/// Per-client data. Smithay needs the compositor part; the rest is ours.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _id: ClientId) {}
    fn disconnected(&self, _id: ClientId, _reason: DisconnectReason) {}
}

/// Everything the compositor owns.
pub struct Spectre {
    pub display_handle: DisplayHandle,
    /// Kept so later phases (idle timers, config reload watchers) can add
    /// sources without threading the handle through every call site.
    #[allow(dead_code)]
    pub loop_handle: LoopHandle<'static, Spectre>,
    pub loop_signal: LoopSignal,
    pub clock: Clock<Monotonic>,
    pub start_time: Instant,
    pub running: bool,

    pub config: Config,
    pub keybinds: Keybinds,

    // Wayland globals.
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    /// Holds the `zxdg_decoration_manager_v1` global alive; the handler reaches
    /// it through the delegate macro rather than through this field.
    #[allow(dead_code)]
    pub xdg_decoration_state: XdgDecorationState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    /// Lets GPU clients hand us buffers directly instead of copying through
    /// shared memory. Most toolkits refuse to start without it.
    pub dmabuf_state: DmabufState,
    /// Kept alive for the lifetime of the compositor; dropping it would remove
    /// the `zwp_linux_dmabuf_v1` global from under running clients.
    pub dmabuf_global: Option<DmabufGlobal>,
    /// Holds the `xdg_output_manager` global alive.
    #[allow(dead_code)]
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub popups: PopupManager,

    pub seat: Seat<Self>,
    #[allow(dead_code)]
    pub seat_name: String,
    pub pointer: PointerHandle<Self>,
    pub cursor_status: CursorImageStatus,

    pub workspaces: Workspaces,
    /// Toplevels that have been created but not yet given a size, so they
    /// cannot be placed sensibly. Moved into a workspace on first commit.
    pub pending_windows: Vec<Window>,
    /// Windows hidden by the minimize button, with the position to restore
    /// them to. They stay reachable through focus cycling.
    pub minimized: Vec<(Window, smithay::utils::Point<i32, smithay::utils::Logical>)>,
    /// The window that currently has keyboard focus.
    pub focus: Option<Window>,
    /// Last title bar press, for double-click detection.
    pub last_click: Option<(Window, u32)>,
    /// The layer surface currently holding keyboard focus, if any. A launcher
    /// or a lock screen takes the keyboard away from windows while it is up.
    pub layer_focus: Option<WlSurface>,
    /// A workspace switch being animated.
    pub transition: Option<crate::transition::Transition>,
    /// Set while the logo key is held and nothing else has been pressed, so a
    /// tap of it on its own can open the application menu.
    pub logo_armed: bool,
    /// The launcher process, if one was started and may still be up.
    pub launcher: Option<u32>,
    /// The wallpaper, prepared for the current output size.
    pub wallpaper: Option<crate::render::Wallpaper>,
    /// Set whenever something visible changed; backends redraw and clear it.
    dirty: bool,
    /// Shaped and uploaded labels for title bars and, later, the panel.
    ///
    /// Behind a `RefCell` because the render pass borrows the state immutably
    /// while it walks the window stack, yet rasterising a caption mutates the
    /// cache.
    pub text: RefCell<crate::render::TextCache>,
    /// Dmabuf imports waiting for the backend's renderer to accept them.
    pub pending_dmabufs: Vec<(
        smithay::backend::allocator::dmabuf::Dmabuf,
        smithay::wayland::dmabuf::ImportNotifier,
    )>,
    /// Wayland socket clients connect to, e.g. `wayland-1`.
    pub socket_name: String,
    /// The control socket the panel and shell connect to. `None` when it could
    /// not be bound; the desktop still works, it just has no panel.
    pub ipc: Option<crate::ipc::Ipc>,
}

impl Spectre {
    /// Build the compositor state and start listening for clients.
    ///
    /// `seat_name` shows up in client-visible seat objects; backends pass their
    /// own name so logs make it obvious which backend is running.
    pub fn new(
        display: Display<Spectre>,
        loop_handle: LoopHandle<'static, Spectre>,
        loop_signal: LoopSignal,
        config: Config,
        seat_name: &str,
    ) -> anyhow::Result<Self> {
        let display_handle = display.handle();
        let dh = &display_handle;

        let compositor_state = CompositorState::new::<Self>(dh);
        let xdg_shell_state = XdgShellState::new::<Self>(dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(dh);
        let shm_state = ShmState::new::<Self>(dh, Vec::new());
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(dh);
        let data_device_state = DataDeviceState::new::<Self>(dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(dh);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(dh, seat_name);

        let kb = &config.input.keyboard;
        let (repeat_delay, repeat_rate) = kb.sane_repeat();
        let xkb = XkbConfig {
            rules: "",
            model: spectre_config::Keyboard::xkb_field(&kb.model).unwrap_or(""),
            layout: spectre_config::Keyboard::xkb_field(&kb.layout).unwrap_or(""),
            variant: spectre_config::Keyboard::xkb_field(&kb.variant).unwrap_or(""),
            options: spectre_config::Keyboard::xkb_field(&kb.options).map(str::to_owned),
        };
        // A bad layout string must not take the session down with it: fall back
        // to the system default and log loudly instead.
        let keyboard = seat
            .add_keyboard(xkb, repeat_delay as i32, repeat_rate as i32)
            .or_else(|err| {
                tracing::error!(?err, layout = %kb.layout, "invalid keyboard layout, using default");
                seat.add_keyboard(XkbConfig::default(), repeat_delay as i32, repeat_rate as i32)
            })?;
        drop(keyboard);
        let pointer = seat.add_pointer();

        let socket_name = Self::init_socket(&loop_handle)?;
        Self::init_display(display, &loop_handle)?;

        let keybinds = Keybinds::default().merged_with(config.keybinds.clone());
        let workspaces = Workspaces::new(config.general.workspaces);

        Ok(Self {
            display_handle,
            loop_handle,
            loop_signal,
            clock: Clock::new(),
            start_time: Instant::now(),
            running: true,
            config,
            keybinds,
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            layer_shell_state,
            shm_state,
            dmabuf_state: DmabufState::new(),
            dmabuf_global: None,
            output_manager_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            popups: PopupManager::default(),
            seat,
            seat_name: seat_name.to_owned(),
            pointer,
            cursor_status: CursorImageStatus::default_named(),
            workspaces,
            pending_windows: Vec::new(),
            minimized: Vec::new(),
            focus: None,
            last_click: None,
            layer_focus: None,
            transition: None,
            logo_armed: false,
            launcher: None,
            wallpaper: None,
            dirty: true,
            text: RefCell::new(crate::render::TextCache::new()),
            pending_dmabufs: Vec::new(),
            socket_name,
            ipc: None,
        })
    }

    /// Bind an auto-numbered Wayland socket and accept clients on it.
    fn init_socket(loop_handle: &LoopHandle<'static, Spectre>) -> anyhow::Result<String> {
        let source = ListeningSocketSource::new_auto()?;
        let socket_name = source.socket_name().to_string_lossy().into_owned();

        loop_handle.insert_source(source, |client_stream, _, state| {
            if let Err(err) = state
                .display_handle
                .insert_client(client_stream, Arc::new(ClientState::default()))
            {
                tracing::warn!(?err, "rejected a client that could not be inserted");
            }
        })?;

        Ok(socket_name)
    }

    /// Drive the Wayland display from the event loop.
    fn init_display(
        display: Display<Spectre>,
        loop_handle: &LoopHandle<'static, Spectre>,
    ) -> anyhow::Result<()> {
        loop_handle.insert_source(
            Generic::new(display, Interest::READ, Mode::Level),
            |_, display, state| {
                // SAFETY: the display is only dispatched from this callback, and
                // calloop guarantees the callback is not re-entered.
                unsafe { display.get_mut().dispatch_clients(state)? };
                Ok(PostAction::Continue)
            },
        )?;
        Ok(())
    }

    /// Bind the control socket. Failing to is survivable: the desktop runs
    /// without a panel rather than not at all.
    pub fn start_ipc(&mut self) {
        match crate::ipc::Ipc::new(&self.loop_handle, &self.socket_name.clone()) {
            Ok(ipc) => {
                tracing::info!(socket = %ipc.path.display(), "control socket ready");
                self.ipc = Some(ipc);
            }
            Err(err) => tracing::error!(?err, "no control socket; the panel will not start"),
        }
    }

    /// Path children should be told to connect to.
    pub fn ipc_socket_path(&self) -> Option<std::path::PathBuf> {
        self.ipc.as_ref().map(|ipc| ipc.path.clone())
    }

    /// Seconds since the compositor started, for pattern animation.
    pub fn elapsed_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    /// Current animation phase of the window/panel pattern.
    pub fn pattern_phase(&self) -> f32 {
        self.config.theme.window_pattern.phase(self.elapsed_secs())
    }

    /// Pixel size of the first output, for sizing the wallpaper.
    pub fn output_pixel_size(&self) -> Option<(i32, i32)> {
        let output = self.outputs().into_iter().next()?;
        let mode = output.current_mode()?;
        Some((mode.size.w, mode.size.h))
    }

    /// Load or reload the wallpaper for an output of this size.
    pub fn refresh_wallpaper(&mut self, width: i32, height: i32) {
        let Some(path) = self.config.desktop.wallpaper_path().map(|p| p.to_owned()) else {
            self.wallpaper = None;
            return;
        };
        let mode = self.config.desktop.wallpaper_mode;
        if self.wallpaper.as_ref().is_some_and(|w| w.matches(&path, mode, width, height)) {
            return;
        }
        self.wallpaper = crate::render::Wallpaper::load(&path, mode, width, height);
        self.mark_dirty();
    }

    /// Where the pattern's colour cycle stands.
    pub fn color_phase(&self) -> f32 {
        self.config.theme.window_pattern.color_phase(self.elapsed_secs())
    }

    /// Note that something visible changed, so the next frame is drawn.
    ///
    /// Shell clients are told as well: focus, workspace and window changes all
    /// pass through here, which is exactly what a panel needs to know about.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        if let Some(ipc) = self.ipc.as_mut() {
            ipc.mark_dirty();
        }
    }

    /// Take the dirty flag, returning whether a redraw is needed.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false) || self.needs_animation_frames()
    }

    /// Whether an animation currently on screen needs a new frame every tick.
    ///
    /// Both patterns the compositor draws are asked, but the window pattern
    /// only counts while a decorated window is actually mapped: an empty
    /// desktop must fall back to zero frames per second.
    pub fn needs_animation_frames(&self) -> bool {
        if self.transition.is_some() {
            return true;
        }
        if self.config.theme.desktop_pattern.needs_continuous_redraw() {
            return true;
        }
        self.config.theme.window_pattern.needs_continuous_redraw()
            && self.workspaces.active().elements().any(|w| self.is_decorated(w))
    }

    /// Ask the event loop to stop; the session ends after the current iteration.
    pub fn stop(&mut self) {
        self.running = false;
        self.loop_signal.stop();
        self.loop_signal.wakeup();
    }

    /// Window under `surface`, searching every workspace.
    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.workspaces.find_surface(surface).map(|(_, w)| w)
    }

    /// All outputs currently mapped.
    pub fn outputs(&self) -> Vec<Output> {
        self.workspaces.outputs().cloned().collect()
    }

    /// The application id a window reports, or an empty string.
    pub fn window_app_id(&self, window: &Window) -> String {
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

        let Some(surface) = window.wl_surface() else {
            return String::new();
        };
        with_states(&surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|data| data.lock().unwrap().app_id.clone())
                .unwrap_or_default()
        })
    }

    /// Title a window asks to be shown in its title bar.
    ///
    /// Falls back to the app id, then to a generic name, so a window is never
    /// left with a blank bar.
    pub fn window_title(&self, window: &Window) -> String {
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

        let Some(surface) = window.wl_surface() else {
            return String::from("Window");
        };
        with_states(&surface, |states| {
            let Some(data) = states.data_map.get::<XdgToplevelSurfaceData>() else {
                return String::from("Window");
            };
            let data = data.lock().unwrap();
            data.title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .or_else(|| data.app_id.clone().filter(|a| !a.trim().is_empty()))
                .unwrap_or_else(|| String::from("Window"))
        })
    }

    /// Whether Spectre draws the frame for this window.
    ///
    /// Clients that asked for client-side decorations through xdg-decoration
    /// draw their own, and must not get a second one from us.
    pub fn is_decorated(&self, window: &Window) -> bool {
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;

        let Some(toplevel) = window.toplevel() else {
            return false;
        };
        // Fullscreen windows own the whole output; a frame would only cover it.
        if self.has_state(window, xdg_toplevel::State::Fullscreen) {
            return false;
        }
        toplevel.with_pending_state(|state| {
            !matches!(state.decoration_mode, Some(Mode::ClientSide))
        })
    }

    pub fn is_maximized(&self, window: &Window) -> bool {
        self.has_state(window, xdg_toplevel::State::Maximized)
    }

    /// Whether `window`'s toplevel carries `wanted`.
    pub fn has_state(&self, window: &Window, wanted: xdg_toplevel::State) -> bool {
        window
            .toplevel()
            .map(|t| t.with_pending_state(|state| state.states.contains(wanted)))
            .unwrap_or(false)
    }

    /// Housekeeping that has to run once per event loop iteration.
    pub fn refresh(&mut self) {
        self.finish_transition();
        self.workspaces.refresh();
        self.popups.cleanup();
        self.prune_ipc_windows();
        self.publish_desktop_state();
        if let Err(err) = self.display_handle.flush_clients() {
            tracing::warn!(?err, "failed to flush clients");
        }
    }
}
