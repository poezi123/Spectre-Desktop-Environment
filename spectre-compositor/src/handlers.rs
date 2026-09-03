//! Wayland protocol handlers.
//!
//! Policy lives here; the actual drawing lives in [`crate::render`] and the
//! layout maths in [`crate::layout`].

use smithay::delegate_compositor;
use smithay::delegate_data_device;
use smithay::delegate_dmabuf;
use smithay::delegate_layer_shell;
use smithay::delegate_output;
use smithay::delegate_primary_selection;
use smithay::delegate_seat;
use smithay::delegate_shm;
use smithay::delegate_xdg_decoration;
use smithay::delegate_xdg_shell;
use smithay::desktop::{
    find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output, PopupKind,
    PopupKeyboardGrab, PopupPointerGrab, PopupUngrabStrategy, Window, WindowSurfaceType,
};
use smithay::input::pointer::{CursorImageStatus, Focus};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Client;
use smithay::utils::Serial;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};
use smithay::wayland::compositor::{
    get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler,
    CompositorState,
};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::selection::primary_selection::{
    PrimarySelectionHandler, PrimarySelectionState,
};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::wlr_layer::{
    Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler,
    WlrLayerShellState,
};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{ShmHandler, ShmState};

use crate::state::{ClientState, Spectre};

// --- compositor --------------------------------------------------------------

impl CompositorHandler for Spectre {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        smithay::backend::renderer::utils::on_commit_buffer_handler::<Self>(surface);

        // A sync subsurface commits with its parent, so there is nothing to do
        // until the root surface commits.
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            // Pending windows need this just as much as mapped ones: their
            // geometry stays empty until `on_commit` reads the new buffer, and
            // `map_new_window` refuses to place a zero-sized window.
            let window = self
                .pending_windows
                .iter()
                .find(|w| w.wl_surface().as_deref() == Some(&root))
                .cloned()
                .or_else(|| self.window_for_surface(&root));
            if let Some(window) = window {
                window.on_commit();
            }
        }

        self.popups.commit(surface);
        self.ensure_initial_configure(surface);
        self.map_new_window(surface);

        // Any surface commit is new content on screen, including a panel
        // attaching its first buffer. Without this the compositor would sit
        // idle while a layer surface waited to be shown.
        self.update_layer_focus();
        self.mark_dirty();
    }
}

impl BufferHandler for Spectre {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for Spectre {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl OutputHandler for Spectre {}

impl DmabufHandler for Spectre {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        // The renderer lives in the backend, so the import is deferred to it.
        // Queue the request and let the next frame resolve it.
        self.pending_dmabufs.push((dmabuf, notifier));
        self.mark_dirty();
    }
}

delegate_dmabuf!(Spectre);

delegate_compositor!(Spectre);
delegate_shm!(Spectre);
delegate_output!(Spectre);

// --- xdg shell ---------------------------------------------------------------

impl XdgShellHandler for Spectre {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Spectre draws its own decorations, so every toplevel is told up front
        // that it is server-side decorated. Clients that insist on CSD say so
        // through xdg-decoration and are honoured in `request_mode`.
        surface.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        surface.send_configure();

        let window = Window::new_wayland_window(surface);
        // Placement waits for the first commit: the client has not told us its
        // size yet, so centring now would centre a zero-sized rectangle.
        self.pending_windows.push(window);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(err) = self.popups.track_popup(PopupKind::from(surface)) {
            tracing::warn!(?err, "failed to track popup");
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn grab(&mut self, surface: PopupSurface, seat: WlSeat, serial: Serial) {
        let seat: Seat<Self> = Seat::from_resource(&seat).unwrap();
        let kind = PopupKind::Xdg(surface);
        let Some(root) = find_popup_root_surface(&kind).ok() else {
            return;
        };

        let grab = self.popups.grab_popup(root, kind, &seat, serial);
        let Ok(mut grab) = grab else {
            return;
        };

        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed()
                && !(keyboard.has_grab(serial)
                    || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }

        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed()
                && !(pointer.has_grab(serial)
                    || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let window = self
            .workspaces
            .windows()
            .find(|w| w.toplevel().map(|t| t == &surface).unwrap_or(false))
            .cloned();

        if let Some(window) = window {
            self.unmap_window(&window);
        }
        self.pending_windows.retain(|w| w.toplevel() != Some(&surface));
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        if let Some(window) = self.window_for_toplevel(&surface) {
            self.set_fullscreen(&window, true);
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_toplevel(&surface) {
            self.set_fullscreen(&window, false);
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_toplevel(&surface) {
            self.set_maximized(&window, true);
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_toplevel(&surface) {
            self.set_maximized(&window, false);
        }
    }
}

impl smithay::wayland::shell::xdg::decoration::XdgDecorationHandler for Spectre {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        toplevel.send_configure();
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecorationMode) {
        // Client-side decoration is honoured when asked for: fighting a client
        // that wants to draw its own frame only produces a double title bar.
        toplevel.with_pending_state(|state| state.decoration_mode = Some(mode));
        toplevel.send_pending_configure();
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        toplevel.send_pending_configure();
    }
}

delegate_xdg_shell!(Spectre);
delegate_xdg_decoration!(Spectre);

// --- layer shell -------------------------------------------------------------

impl WlrLayerShellHandler for Spectre {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let output = wl_output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.outputs().first().cloned());

        let Some(output) = output else {
            tracing::warn!(%namespace, "layer surface requested with no output available");
            return;
        };

        let layer = smithay::desktop::LayerSurface::new(surface, namespace.clone());
        let mut map = layer_map_for_output(&output);
        if let Err(err) = map.map_layer(&layer) {
            tracing::warn!(?err, %namespace, "failed to map layer surface");
        }
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let found = self.outputs().into_iter().find(|output| {
            let mut map = layer_map_for_output(output);
            let layer = map.layers().find(|l| l.layer_surface() == &surface).cloned();
            match layer {
                Some(layer) => {
                    map.unmap_layer(&layer);
                    true
                }
                None => false,
            }
        });

        if let Some(output) = found {
            // Removing an exclusive zone changes how much room windows have,
            // and a dismissed launcher has to hand the keyboard back.
            self.reflow_output(&output);
            self.update_layer_focus();
        }
    }
}

delegate_layer_shell!(Spectre);

// --- seat and selection ------------------------------------------------------

impl SeatHandler for Spectre {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let window = focused.and_then(|s| self.window_for_surface(s));
        if self.focus.as_ref() != window.as_ref() {
            self.focus = window;
            self.mark_dirty();
        }
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
        self.mark_dirty();
    }
}

impl SelectionHandler for Spectre {
    type SelectionUserData = ();
}

impl DataDeviceHandler for Spectre {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for Spectre {}
impl ServerDndGrabHandler for Spectre {}

impl PrimarySelectionHandler for Spectre {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

delegate_seat!(Spectre);
delegate_data_device!(Spectre);
delegate_primary_selection!(Spectre);

// --- helpers used by the handlers above --------------------------------------

impl Spectre {
    /// The window wrapping `toplevel`, if it is mapped.
    pub fn window_for_toplevel(&self, toplevel: &ToplevelSurface) -> Option<Window> {
        self.workspaces
            .windows()
            .find(|w| w.toplevel().map(|t| t == toplevel).unwrap_or(false))
            .cloned()
    }

    /// Send the first configure once a toplevel has committed but has not been
    /// configured yet. Sending it earlier is a protocol error.
    fn ensure_initial_configure(&mut self, surface: &WlSurface) {
        if let Some(window) = self
            .pending_windows
            .iter()
            .chain(self.workspaces.windows())
            .find(|w| w.wl_surface().as_deref() == Some(surface))
            .cloned()
        {
            if let Some(toplevel) = window.toplevel() {
                let configured = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .map(|d| d.lock().unwrap().initial_configure_sent)
                        .unwrap_or(true)
                });
                if !configured {
                    toplevel.send_configure();
                }
            }
        }

        // Layer surfaces are laid out again on every commit, not only on the
        // first: a panel that moves to another edge changes its anchor and its
        // size on a surface that is already mapped, and without a fresh
        // arrange it would keep the geometry it was given when it started.
        for output in self.outputs() {
            let mut map = layer_map_for_output(&output);
            if map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).is_none() {
                continue;
            }
            map.arrange();
            let configured = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                // Only when something actually changed, or client and
                // compositor would configure each other in a circle.
                .and_then(|layer| layer.layer_surface().send_pending_configure())
                .is_some();
            drop(map);
            if configured || !initial_configure_sent(surface) {
                self.reflow_output(&output);
            }
            break;
        }
    }

    /// Move a toplevel from `pending_windows` into the active workspace once it
    /// has a real size.
    fn map_new_window(&mut self, surface: &WlSurface) {
        let Some(index) = self
            .pending_windows
            .iter()
            .position(|w| w.wl_surface().as_deref() == Some(surface))
        else {
            return;
        };

        if self.pending_windows[index].geometry().size.is_empty() {
            return;
        }

        let window = self.pending_windows.remove(index);
        self.place_window(window);
    }

    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = self.window_for_surface(&root) else {
            return;
        };
        let Some(output) = self.workspaces.active().outputs().next().cloned() else {
            return;
        };
        let Some(output_geo) = self.workspaces.output_geometry(&output) else {
            return;
        };
        let Some(window_loc) = self.workspaces.active().element_location(&window) else {
            return;
        };

        // The positioner works relative to the toplevel, so translate the
        // output rectangle into that space before clamping.
        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

/// Whether a layer surface has already received its first configure.
fn initial_configure_sent(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        states
            .data_map
            .get::<LayerSurfaceData>()
            .map(|data| data.lock().unwrap().initial_configure_sent)
            .unwrap_or(true)
    })
}
