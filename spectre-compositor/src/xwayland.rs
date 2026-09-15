use std::process::Stdio;
use std::os::unix::io::OwnedFd;
use std::time::Duration;

use smithay::delegate_xwayland_shell;
use smithay::desktop::Window;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState};
use smithay::xwayland::xwm::{Reorder, ResizeEdge, XwmId};
use smithay::wayland::selection::data_device::{
    clear_data_device_selection, current_data_device_selection_userdata,
    request_data_device_client_selection, set_data_device_selection,
};
use smithay::wayland::selection::primary_selection::{
    clear_primary_selection, current_primary_selection_userdata, request_primary_client_selection,
    set_primary_selection,
};
use smithay::wayland::selection::SelectionTarget;
use smithay::xwayland::{X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler};

use crate::state::Spectre;

const XWAYLAND_STARTUP_LIMIT: Duration = Duration::from_secs(10);

impl Spectre {
    pub fn start_xwayland(&mut self) {
        let spawned = crate::with_default_child_signal(|| {
            XWayland::spawn(
                &self.display_handle,
                None,
                std::iter::empty::<(String, String)>(),
                true,
                Stdio::null(),
                Stdio::inherit(),
                |_| {},
            )
        });
        let (xwayland, client) = match spawned {
            Ok(pair) => pair,
            Err(err) => {
                tracing::warn!(?err, "XWayland did not start; X11 programs will not run");
                return;
            }
        };

        let watched = self.loop_handle.insert_source(xwayland, move |event, _, state| match event {
            XWaylandEvent::Ready { x11_socket, display_number } => {
                match X11Wm::start_wm(state.loop_handle.clone(), x11_socket, client.clone()) {
                    Ok(wm) => {
                        tracing::info!(display = display_number, "XWayland ready");
                        state.xwm = Some(wm);
                        state.xdisplay = Some(display_number);
                    }
                    Err(err) => tracing::warn!(?err, "could not start the X11 window manager"),
                }
            }
            XWaylandEvent::Error => tracing::warn!("XWayland stopped during startup"),
        });
        let token = match watched {
            Ok(token) => token,
            Err(err) => {
                tracing::warn!(?err, "could not watch XWayland");
                return;
            }
        };

        let deadline = Timer::from_duration(XWAYLAND_STARTUP_LIMIT);
        let timed = self.loop_handle.insert_source(deadline, move |_, _, state| {
            if state.xwm.is_none() {
                tracing::warn!("XWayland was not ready in time; X11 programs will not run");
                state.loop_handle.remove(token);
            }
            TimeoutAction::Drop
        });
        if let Err(err) = timed {
            tracing::warn!(?err, "could not time the XWayland startup");
        }
    }

    pub fn window_for_x11(&self, surface: &X11Surface) -> Option<Window> {
        for window in self.workspaces.windows() {
            if window.x11_surface() == Some(surface) {
                return Some(window.clone());
            }
        }
        for window in &self.pending_windows {
            if window.x11_surface() == Some(surface) {
                return Some(window.clone());
            }
        }
        for (window, _) in &self.minimized {
            if window.x11_surface() == Some(surface) {
                return Some(window.clone());
            }
        }
        None
    }

    fn forget_x11_window(&mut self, surface: &X11Surface) {
        if let Some(window) = self.window_for_x11(surface) {
            self.unmap_window(&window);
            self.minimized.retain(|(minimized, _)| minimized != &window);
        }
        self.pending_windows.retain(|window| window.x11_surface() != Some(surface));
    }
}

impl XwmHandler for Spectre {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("the X11 window manager is running")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn allow_selection_access(&mut self, _xwm: XwmId, _selection: SelectionTarget) -> bool {
        let Some(window) = self.focus.as_ref() else {
            return false;
        };
        window.x11_surface().is_some()
    }

    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        match selection {
            SelectionTarget::Clipboard => {
                if let Err(err) = request_data_device_client_selection(&self.seat, mime_type, fd) {
                    tracing::warn!(?err, "could not hand the clipboard to an X11 program");
                }
            }
            SelectionTarget::Primary => {
                if let Err(err) = request_primary_client_selection(&self.seat, mime_type, fd) {
                    tracing::warn!(?err, "could not hand the selection to an X11 program");
                }
            }
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        match selection {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display_handle, &self.seat, mime_types, ());
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &self.seat, mime_types, ());
            }
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat);
                }
            }
        }
    }

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(err) = window.set_mapped(true) {
            tracing::warn!(?err, "could not map an X11 window");
            return;
        }
        self.pending_windows.push(Window::new_x11_window(window.clone()));
        if let Some(surface) = window.wl_surface() {
            self.map_new_window(&surface);
        }
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let location = window.geometry().loc;
        let popup = Window::new_x11_window(window);
        self.workspaces.active_mut().map_element(popup, location, true);
        self.mark_dirty();
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.forget_x11_window(&window);
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.forget_x11_window(&window);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let mut geometry = window.geometry();
        if let Some(width) = w {
            geometry.size.w = width as i32;
        }
        if let Some(height) = h {
            geometry.size.h = height as i32;
        }
        if let Some(managed) = self.window_for_x11(&window) {
            if let Some(location) = self.workspaces.active().element_location(&managed) {
                geometry.loc = location;
            }
        }
        let _ = window.configure(geometry);
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        if !window.is_override_redirect() {
            return;
        }
        let Some(popup) = self.window_for_x11(&window) else {
            return;
        };
        self.workspaces.active_mut().map_element(popup, geometry.loc, false);
        self.mark_dirty();
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(managed) = self.window_for_x11(&window) {
            self.set_maximized(&managed, true);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(managed) = self.window_for_x11(&window) {
            self.set_maximized(&managed, false);
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(managed) = self.window_for_x11(&window) {
            self.set_fullscreen(&managed, true);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(managed) = self.window_for_x11(&window) {
            self.set_fullscreen(&managed, false);
        }
    }

    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(managed) = self.window_for_x11(&window) {
            self.minimize(&managed);
        }
    }

    fn unminimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(managed) = self.window_for_x11(&window) {
            self.restore(&managed);
        }
    }

    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32, edge: ResizeEdge) {
        let Some(found) = self.window_for_x11(&window) else {
            return;
        };
        let mut edges = crate::render::Edges::default();
        match edge {
            ResizeEdge::Top => edges.top = true,
            ResizeEdge::Bottom => edges.bottom = true,
            ResizeEdge::Left => edges.left = true,
            ResizeEdge::Right => edges.right = true,
            ResizeEdge::TopLeft => {
                edges.top = true;
                edges.left = true;
            }
            ResizeEdge::TopRight => {
                edges.top = true;
                edges.right = true;
            }
            ResizeEdge::BottomLeft => {
                edges.bottom = true;
                edges.left = true;
            }
            ResizeEdge::BottomRight => {
                edges.bottom = true;
                edges.right = true;
            }
        }
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        self.start_resize(&found, edges, serial);
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32) {
        let Some(managed) = self.window_for_x11(&window) else {
            return;
        };
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        self.start_move(&managed, serial);
    }
}

impl XWaylandShellHandler for Spectre {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(&mut self, _xwm: XwmId, surface: WlSurface, _window: X11Surface) {
        self.map_new_window(&surface);
    }
}

delegate_xwayland_shell!(Spectre);
