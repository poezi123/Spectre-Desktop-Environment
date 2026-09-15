use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
use smithay::wayland::xwayland_shell::XWaylandShellState;
use smithay::xwayland::X11Wm;
use spectre_config::{Config, Keybinds};
use spectre_theme::Color;

use crate::workspace::Workspaces;

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _id: ClientId) {}
    fn disconnected(&self, _id: ClientId, _reason: DisconnectReason) {}
}

const TRANSITION_INTERVAL: Duration = Duration::from_millis(16);

pub struct Spectre {
    pub display_handle: DisplayHandle,
    #[allow(dead_code)]
    pub loop_handle: LoopHandle<'static, Spectre>,
    pub loop_signal: LoopSignal,
    pub clock: Clock<Monotonic>,
    pub start_time: Instant,
    pub running: bool,

    pub config: Config,
    pub keybinds: Keybinds,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    #[allow(dead_code)]
    pub xdg_decoration_state: XdgDecorationState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
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
    pub pointer_surface: Option<WlSurface>,

    pub workspaces: Workspaces,
    pub pending_windows: Vec<Window>,
    pub minimized: Vec<(Window, smithay::utils::Point<i32, smithay::utils::Logical>)>,
    pub focus: Option<Window>,
    pub last_click: Option<(Window, u32)>,
    pub layer_focus: Option<WlSurface>,
    pub transition: Option<crate::transition::Transition>,
    pub logo_armed: bool,
    pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    pub launcher: Option<u32>,
    last_animation: Instant,
    pub panel: Option<u32>,
    pub wallpaper: Option<crate::render::Wallpaper>,
    pub cursor: Option<crate::render::CursorImage>,
    dirty: bool,
    display_dirty: bool,
    pub text: RefCell<crate::render::TextCache>,
    pub pending_dmabufs: Vec<(
        smithay::backend::allocator::dmabuf::Dmabuf,
        smithay::wayland::dmabuf::ImportNotifier,
    )>,
    pub socket_name: String,
    pub ipc: Option<crate::ipc::Ipc>,
    pub xwayland_shell_state: XWaylandShellState,
    pub xwm: Option<X11Wm>,
    pub xdisplay: Option<u32>,
    pub overview: Option<crate::overview::Overview>,
    pub corner_armed: bool,
    pub pending_overview_key: Option<smithay::input::keyboard::Keysym>,
    pub opening: Vec<(Window, crate::animation::Pop)>,
    pub closing: Vec<crate::animation::Closing>,
    pub launcher_opening: Option<crate::animation::Slide>,
    pub launcher_closing: Option<crate::animation::LauncherClosing>,
    pub launcher_shown: bool,
}

impl Spectre {
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
        let xwayland_shell_state = XWaylandShellState::new::<Self>(dh);

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
        let cursor = Some(Self::build_cursor(&config));
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
            pointer_surface: None,
            workspaces,
            pending_windows: Vec::new(),
            minimized: Vec::new(),
            focus: None,
            last_click: None,
            layer_focus: None,
            transition: None,
            logo_armed: false,
            pointer_location: (0.0, 0.0).into(),
            launcher: None,
            last_animation: Instant::now(),
            panel: None,
            wallpaper: None,
            cursor,
            dirty: true,
            display_dirty: false,
            text: RefCell::new(crate::render::TextCache::new()),
            pending_dmabufs: Vec::new(),
            socket_name,
            ipc: None,
            xwayland_shell_state,
            xwm: None,
            xdisplay: None,
            overview: None,
            corner_armed: false,
            pending_overview_key: None,
            opening: Vec::new(),
            closing: Vec::new(),
            launcher_opening: None,
            launcher_closing: None,
            launcher_shown: false,
        })
    }

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

    fn init_display(
        display: Display<Spectre>,
        loop_handle: &LoopHandle<'static, Spectre>,
    ) -> anyhow::Result<()> {
        loop_handle.insert_source(
            Generic::new(display, Interest::READ, Mode::Level),
            |_, display, state| {
                unsafe { display.get_mut().dispatch_clients(state)? };
                Ok(PostAction::Continue)
            },
        )?;
        Ok(())
    }

    pub fn start_ipc(&mut self) {
        match crate::ipc::Ipc::new(&self.loop_handle, &self.socket_name.clone()) {
            Ok(ipc) => {
                tracing::info!(socket = %ipc.path.display(), "control socket ready");
                self.ipc = Some(ipc);
            }
            Err(err) => tracing::error!(?err, "no control socket; the panel will not start"),
        }
    }

    pub fn ipc_socket_path(&self) -> Option<std::path::PathBuf> {
        self.ipc.as_ref().map(|ipc| ipc.path.clone())
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    fn animation_clock(&self) -> f64 {
        let elapsed = self.elapsed_secs();
        let Some(step) = self.animation_interval().map(|i| i.as_secs_f64()) else {
            return elapsed;
        };
        if step <= 0.0 {
            return elapsed;
        }
        (elapsed / step).floor() * step
    }

    fn build_cursor(config: &Config) -> crate::render::CursorImage {
        let (fill, outline) = config.input.cursor.colors(Color::hex(0x000000), Color::hex(0xffffff));
        crate::render::CursorImage::new(config.input.cursor.height(config.display.scale), fill, outline)
    }

    pub fn refresh_cursor(&mut self, previous: &Config) {
        let palette = |c: &Config| c.theme.palette.clone();
        let unchanged = previous.input.cursor == self.config.input.cursor
            && previous.display.scale == self.config.display.scale
            && palette(previous) == palette(&self.config);
        if unchanged && self.cursor.is_some() {
            return;
        }
        self.cursor = Some(Self::build_cursor(&self.config));
        self.mark_dirty();
    }

    pub fn pattern_phase(&self) -> f32 {
        self.config.theme.window_pattern.phase(self.animation_clock())
    }

    pub fn desktop_phase(&self) -> f32 {
        self.config.theme.desktop_pattern.phase(self.animation_clock())
    }

    pub fn desktop_color_phase(&self) -> f32 {
        self.config.theme.desktop_pattern.color_phase(self.animation_clock())
    }

    pub fn output_pixel_size(&self) -> Option<(i32, i32)> {
        let output = self.outputs().into_iter().next()?;
        let mode = output.current_mode()?;
        Some((mode.size.w, mode.size.h))
    }

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

    pub fn pointer_position(&self) -> smithay::utils::Point<f64, smithay::utils::Logical> {
        self.pointer_location
    }

    pub fn set_pointer_position(
        &mut self,
        location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        self.pointer_location = location;
    }

    pub fn color_phase(&self) -> f32 {
        self.config.theme.window_pattern.color_phase(self.animation_clock())
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        if let Some(ipc) = self.ipc.as_mut() {
            ipc.mark_dirty();
        }
    }

    pub fn take_display_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.display_dirty, false)
    }

    pub fn mark_display_dirty(&mut self) {
        self.display_dirty = true;
        self.mark_dirty();
    }

    pub fn take_dirty(&mut self) -> bool {
        if std::mem::replace(&mut self.dirty, false) {
            self.last_animation = Instant::now();
            return true;
        }
        let Some(interval) = self.animation_interval() else {
            return false;
        };
        let now = Instant::now();
        if now.duration_since(self.last_animation) < interval {
            return false;
        }
        self.last_animation = now;
        true
    }

    pub fn animation_interval(&self) -> Option<Duration> {
        if self.transition.is_some() {
            return Some(TRANSITION_INTERVAL);
        }
        let windows_moving = !self.opening.is_empty() || !self.closing.is_empty();
        let launcher_moving = self.launcher_opening.is_some() || self.launcher_closing.is_some();
        if windows_moving || launcher_moving {
            return Some(TRANSITION_INTERVAL);
        }
        if let Some(overview) = self.overview.as_ref() {
            if overview.is_moving(Instant::now()) {
                return Some(TRANSITION_INTERVAL);
            }
            return None;
        }
        let scale = self.animation_scale();
        let theme = &self.config.theme;
        let desktop = theme.desktop_pattern.redraw_interval(scale);
        let window = self
            .workspaces
            .active()
            .elements()
            .any(|w| self.is_decorated(w))
            .then(|| theme.window_pattern.redraw_interval(scale))
            .flatten();
        match (desktop, window) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (only, None) | (None, only) => only,
        }
    }

    fn animation_scale(&self) -> f32 {
        self.outputs()
            .into_iter()
            .map(|output| output.current_scale().fractional_scale() as f32)
            .fold(1.0f32, f32::max)
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.loop_signal.stop();
        self.loop_signal.wakeup();
    }

    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.workspaces.find_surface(surface).map(|(_, w)| w)
    }

    pub fn outputs(&self) -> Vec<Output> {
        self.workspaces.outputs().cloned().collect()
    }

    pub fn window_app_id(&self, window: &Window) -> String {
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

        if let Some(x11) = window.x11_surface() {
            return x11.class();
        }
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

    pub fn window_title(&self, window: &Window) -> String {
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

        if let Some(x11) = window.x11_surface() {
            let title = x11.title();
            if !title.trim().is_empty() {
                return title;
            }
            let class = x11.class();
            if !class.trim().is_empty() {
                return class;
            }
            return String::from("Window");
        }
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

    pub fn is_decorated(&self, window: &Window) -> bool {
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;

        if let Some(x11) = window.x11_surface() {
            if x11.is_override_redirect() || x11.is_popup() || x11.is_decorated() {
                return false;
            }
            return !x11.is_fullscreen();
        }
        let Some(toplevel) = window.toplevel() else {
            return false;
        };
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

    pub fn has_state(&self, window: &Window, wanted: xdg_toplevel::State) -> bool {
        if let Some(toplevel) = window.toplevel() {
            return toplevel.with_pending_state(|state| state.states.contains(wanted));
        }
        if let Some(x11) = window.x11_surface() {
            return match wanted {
                xdg_toplevel::State::Maximized => x11.is_maximized(),
                xdg_toplevel::State::Fullscreen => x11.is_fullscreen(),
                xdg_toplevel::State::Activated => x11.is_activated(),
                _ => false,
            };
        }
        false
    }

    pub fn refresh(&mut self) {
        self.finish_transition();
        self.finish_window_animations();
        self.workspaces.refresh();
        self.popups.cleanup();
        self.prune_ipc_windows();
        self.publish_desktop_state();
        if let Err(err) = self.display_handle.flush_clients() {
            tracing::warn!(?err, "failed to flush clients");
        }
    }
}
