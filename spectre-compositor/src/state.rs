//! The compositor state: every Wayland global Spectre exposes, plus the
//! desktop model (workspaces, focus, outputs) that the backends drive.

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
    pub xdg_decoration_state: XdgDecorationState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    /// Lets GPU clients hand us buffers directly instead of copying through
    /// shared memory. Most toolkits refuse to start without it.
    pub dmabuf_state: DmabufState,
    /// Kept alive for the lifetime of the compositor; dropping it would remove
    /// the `zwp_linux_dmabuf_v1` global from under running clients.
    pub dmabuf_global: Option<DmabufGlobal>,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub popups: PopupManager,

    pub seat: Seat<Self>,
    pub seat_name: String,
    pub pointer: PointerHandle<Self>,
    pub cursor_status: CursorImageStatus,

    pub workspaces: Workspaces,
    /// Toplevels that have been created but not yet given a size, so they
    /// cannot be placed sensibly. Moved into a workspace on first commit.
    pub pending_windows: Vec<Window>,
    /// The window that currently has keyboard focus.
    pub focus: Option<Window>,
    /// Set whenever something visible changed; backends redraw and clear it.
    dirty: bool,
    /// Dmabuf imports waiting for the backend's renderer to accept them.
    pub pending_dmabufs: Vec<(
        smithay::backend::allocator::dmabuf::Dmabuf,
        smithay::wayland::dmabuf::ImportNotifier,
    )>,
    /// Wayland socket clients connect to, e.g. `wayland-1`.
    pub socket_name: String,
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
            focus: None,
            dirty: true,
            pending_dmabufs: Vec::new(),
            socket_name,
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

    /// Seconds since the compositor started, for pattern animation.
    pub fn elapsed_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    /// Current animation phase of the window/panel pattern.
    pub fn pattern_phase(&self) -> f32 {
        self.config.theme.window_pattern.phase(self.elapsed_secs())
    }

    /// Note that something visible changed, so the next frame is drawn.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Take the dirty flag, returning whether a redraw is needed.
    ///
    /// An animated pattern is always dirty: it changes every frame by
    /// definition, and asking the theme keeps that decision in one place.
    pub fn take_dirty(&mut self) -> bool {
        let animated = self.config.theme.needs_continuous_redraw();
        std::mem::replace(&mut self.dirty, false) || animated
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

    /// Housekeeping that has to run once per event loop iteration.
    pub fn refresh(&mut self) {
        self.workspaces.refresh();
        self.popups.cleanup();
        if let Err(err) = self.display_handle.flush_clients() {
            tracing::warn!(?err, "failed to flush clients");
        }
    }
}
