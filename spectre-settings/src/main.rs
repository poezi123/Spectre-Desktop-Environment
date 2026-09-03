//! The Spectre settings application.
//!
//! An ordinary xdg-shell window, so the compositor decorates it like any other.
//! Every change is written to the configuration file and applied to the running
//! session over the control socket.

mod model;
mod ui;

use std::time::{Duration, Instant};

use anyhow::Context;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client as wayland_client;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::xdg::window::{
    Window, WindowConfigure, WindowDecorations, WindowHandler,
};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use spectre_config::Config;
use spectre_draw::{Canvas, PatternMask};
use spectre_ipc::{Client, Request};
use spectre_text::TextRenderer;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

use crate::model::{Control, Field, Settings};

const BTN_LEFT: u32 = 0x110;
/// Repaint interval while the pattern is moving.
const ANIMATION_INTERVAL: Duration = Duration::from_millis(33);

fn main() -> anyhow::Result<()> {
    init_tracing();

    let (config, error) = Config::load_active();
    if let Some(error) = error {
        tracing::error!(%error, "using built-in defaults");
    }
    // The file as written, not as resolved: the settings window edits what is
    // in the file, and the profile is one of the rows.
    let mut settings = Settings::new(config);

    // Ask the session what the display can actually do, so the resolution row
    // offers real modes rather than a guessed list.
    let mut ipc = Client::connect().ok();
    if let Some(client) = ipc.as_mut() {
        match client.request_state() {
            Ok(Some(desktop)) => {
                if let Some(output) = desktop.outputs.first() {
                    settings.set_modes(&output.modes);
                }
            }
            Ok(None) => {}
            Err(err) => tracing::warn!(?err, "could not read the display's modes"),
        }
    }

    let conn = Connection::connect_to_env().context("no Wayland compositor to connect to")?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor is missing")?;
    let xdg_shell = XdgShell::bind(&globals, &qh).context("xdg-shell is missing")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm is missing")?;

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title("Spectre Settings");
    window.set_app_id("spectre-settings");
    window.set_min_size(Some((560, 420)));
    window.commit();

    let (width, height) = ui::window_size(1280, 800);
    let pool = SlotPool::new((width * height * 4) as usize, &shm)
        .context("could not allocate the settings window's shared memory")?;

    let mut event_loop: EventLoop<App> = EventLoop::try_new()?;
    let mut app = App {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        window,
        keyboard: None,
        pointer: None,
        width,
        height,
        scale: 1,
        configured: false,
        exit: false,
        dirty: true,
        canvas: Canvas::new(0, 0),
        mask: PatternMask::new(),
        text: TextRenderer::new(),
        settings,
        section: 0,
        row: 0,
        dragging: None,
        status: String::new(),
        ipc,
        started: Instant::now(),
    };

    WaylandSource::new(conn, event_queue).insert(event_loop.handle())?;
    event_loop
        .handle()
        .insert_source(Timer::from_duration(ANIMATION_INTERVAL), |_, _, app: &mut App| {
            if app.settings.config.theme.window_pattern.needs_continuous_redraw() {
                app.dirty = true;
                app.redraw_if_needed();
            }
            TimeoutAction::ToDuration(ANIMATION_INTERVAL)
        })
        .map_err(|err| anyhow::anyhow!("could not start the settings animation: {err}"))?;

    let signal = event_loop.get_signal();
    event_loop.run(None, &mut app, move |app| {
        if app.exit {
            signal.stop();
        }
    })?;
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("SPECTRE_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
}

struct App {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,

    width: i32,
    height: i32,
    scale: i32,
    configured: bool,
    exit: bool,
    dirty: bool,

    canvas: Canvas,
    mask: PatternMask,
    text: TextRenderer,

    settings: Settings,
    section: usize,
    row: usize,
    /// The slider the button is being held down on, if any.
    dragging: Option<Field>,
    status: String,
    ipc: Option<Client>,
    started: Instant,
}

impl App {
    fn sections(&self) -> Vec<model::Section> {
        self.settings.sections()
    }

    fn rows_in_section(&self) -> usize {
        self.sections().get(self.section).map(|s| s.rows.len()).unwrap_or(0)
    }

    fn move_row(&mut self, delta: isize) {
        let count = self.rows_in_section();
        if count == 0 {
            return;
        }
        self.row = (self.row as isize + delta).rem_euclid(count as isize) as usize;
        self.dirty = true;
    }

    fn move_section(&mut self, delta: isize) {
        let count = self.sections().len();
        if count == 0 {
            return;
        }
        self.section = (self.section as isize + delta).rem_euclid(count as isize) as usize;
        self.row = 0;
        self.dirty = true;
    }

    /// Change the selected row and apply the result.
    fn edit(&mut self, delta: i32) {
        let Some(section) = self.sections().into_iter().nth(self.section) else {
            return;
        };
        let Some(row) = section.rows.get(self.row) else {
            return;
        };
        if self.settings.step(row.field, delta) {
            self.apply();
        }
        self.dirty = true;
    }

    /// Set a slider directly, for a click on the bar.
    fn set_slider(&mut self, value: f32) {
        let Some(section) = self.sections().into_iter().nth(self.section) else {
            return;
        };

        let Some(row) = section.rows.get(self.row) else {
            return;
        };

        if self.settings.set_slider(row.field, value) {
            self.dirty = true;
        }
    }

    /// Write the config and ask the session to re-read it.
    fn apply(&mut self) {
        match self.settings.save() {
            Ok(_) => {
                self.status = match self.reload() {
                    true => String::from("Applied"),
                    false => String::from("Saved; restart to apply"),
                };
            }
            Err(err) => {
                tracing::warn!(%err, "could not write the configuration");
                self.status = format!("Could not save: {err}");
            }
        }
    }

    fn reload(&mut self) -> bool {
        let Some(ipc) = self.ipc.as_mut() else {
            return false;
        };
        match ipc.send(&Request::ReloadConfig) {
            Ok(()) => true,
            Err(err) => {
                tracing::warn!(?err, "the compositor did not take the reload");
                self.ipc = None;
                false
            }
        }
    }

    fn redraw_if_needed(&mut self) {
        if self.dirty && self.configured && self.width > 0 {
            self.draw();
        }
    }

    fn draw(&mut self) {
        self.dirty = false;
        let (width, height) = (self.width * self.scale, self.height * self.scale);
        self.canvas.resize(width, height);

        let elapsed = self.started.elapsed().as_secs_f64();
        let pattern = ui::settings_pattern(&self.settings.config.theme);
        self.mask.prepare(width, height, &pattern, pattern.phase(elapsed), self.scale as f32);

        let sections = self.settings.sections();
        let frame = ui::Frame {
            theme: &self.settings.config.theme,
            sections: &sections,
            section: self.section,
            row: self.row,
            mask: &self.mask,
            color_phase: pattern.color_phase(elapsed),
            status: &self.status,
        };
        ui::draw(&mut self.canvas, &mut self.text, &frame);

        let stride = width * 4;
        let Ok((buffer, target)) =
            self.pool.create_buffer(width, height, stride, wl_shm::Format::Argb8888)
        else {
            tracing::warn!("could not get a buffer for the settings window");
            return;
        };
        let bytes = self.canvas.as_bytes();
        let len = target.len().min(bytes.len());
        target[..len].copy_from_slice(&bytes[..len]);

        let surface = self.window.wl_surface();
        surface.set_buffer_scale(self.scale);
        surface.damage_buffer(0, 0, width, height);
        if let Err(err) = buffer.attach_to(surface) {
            tracing::warn!(?err, "could not attach the settings buffer");
            return;
        }
        self.window.commit();
    }
}

impl WindowHandler for App {
    fn request_close(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _w: &Window) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _w: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let (width, height) = ui::window_size(1280, 800);
        self.width = configure.new_size.0.map(|w| w.get() as i32).unwrap_or(width);
        self.height = configure.new_size.1.map(|h| h.get() as i32).unwrap_or(height);
        self.configured = true;
        self.dirty = true;
        self.redraw_if_needed();
    }
}

impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _k: &wl_keyboard::WlKeyboard,
        _s: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _k: &wl_keyboard::WlKeyboard,
        _s: &wl_surface::WlSurface,
        _serial: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _k: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        match event.keysym {
            Keysym::Escape | Keysym::q => self.exit = true,
            Keysym::Down => self.move_row(1),
            Keysym::Up => self.move_row(-1),
            Keysym::Tab | Keysym::Page_Down => self.move_section(1),
            Keysym::ISO_Left_Tab | Keysym::Page_Up => self.move_section(-1),
            Keysym::Right | Keysym::plus | Keysym::equal => self.edit(1),
            Keysym::Left | Keysym::minus => self.edit(-1),
            Keysym::Return | Keysym::KP_Enter | Keysym::space => self.edit(1),
            _ => {}
        }
        self.redraw_if_needed();
    }

    fn release_key(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _k: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }

    fn repeat_key(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        keyboard: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        self.press_key(conn, qh, keyboard, serial, event);
    }

    fn update_modifiers(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _k: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _modifiers: Modifiers,
        _raw: RawModifiers,
        _layout: u32,
    ) {
    }
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _p: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if event.surface != *self.window.wl_surface() {
                continue;
            }
            let (x, y) =
                (event.position.0 as i32 * self.scale, event.position.1 as i32 * self.scale);
            let (w, h) = (self.width * self.scale, self.height * self.scale);
            match event.kind {
                PointerEventKind::Motion { .. } => match self.dragging {
                    // Held on a slider: follow the pointer. Writing the file
                    // on every pixel would be a hundred saves per drag, so it
                    // waits for the button to come up.
                    Some(field) => {
                        let rect = ui::control_rect(ui::row_rect(w, self.row));
                        self.settings.set_slider(field, ui::slider_value_at(rect, x));
                        self.dirty = true;
                    }
                    None => {
                        if let Some(row) = ui::row_at(w, h, x, y) {
                            if row < self.rows_in_section() && row != self.row {
                                self.row = row;
                                self.dirty = true;
                            }
                        }
                    }
                },
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    if self.dragging.take().is_some() {
                        self.apply();
                        // `apply` leaves a word in the status line; without a
                        // redraw nobody ever reads it.
                        self.dirty = true;
                    }
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    if let Some(section) = ui::section_at(self.sections().len(), x, y) {
                        if section != self.section {
                            self.section = section;
                            self.row = 0;
                            self.dirty = true;
                        }
                    } else if let Some(row) = ui::row_at(w, h, x, y) {
                        if row < self.rows_in_section() {
                            self.row = row;
                            self.click_row(x, y);
                        }
                    }
                }
                PointerEventKind::Axis { vertical, .. } if vertical.discrete != 0 => {
                    self.move_row(vertical.discrete as isize);
                }
                _ => {}
            }
        }
        self.redraw_if_needed();
    }
}

impl App {
    /// A click inside the selected row: on the control, or anywhere else.
    fn click_row(&mut self, x: i32, y: i32) {
        let rect = ui::row_rect(self.width * self.scale, self.row);
        let control = ui::control_rect(rect);
        let Some(section) = self.sections().into_iter().nth(self.section) else {
            return;
        };
        let Some(row) = section.rows.get(self.row) else {
            return;
        };

        match &row.control {
            Control::Slider { .. } if control.contains(x, y) => {
                self.dragging = Some(row.field);
                self.set_slider(ui::slider_value_at(control, x));
            }
            // Clicking the left half of a choice steps back, the right half
            // forward, which is what the arrows drawn there promise.
            Control::Choice { .. } if control.contains(x, y) => {
                let forward = x > control.x + control.w / 2;
                self.edit(if forward { 1 } else { -1 })
            }
            _ => self.edit(1),
        }
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        self.scale = new_factor.max(1);
        self.dirty = true;
        self.redraw_if_needed();
    }

    fn transform_changed(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: &wl_surface::WlSurface,
        _t: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: &wl_surface::WlSurface,
        _time: u32,
    ) {
        self.redraw_if_needed();
    }

    fn surface_enter(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: &wl_surface::WlSurface,
        _o: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: &wl_surface::WlSurface,
        _o: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: wl_output::WlOutput) {}
    fn update_output(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: wl_output::WlOutput) {
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _s: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _c: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard if self.keyboard.is_none() => {
                match self.seat_state.get_keyboard(qh, &seat, None) {
                    Ok(keyboard) => self.keyboard = Some(keyboard),
                    Err(err) => tracing::warn!(?err, "no keyboard for the settings window"),
                }
            }
            Capability::Pointer if self.pointer.is_none() => {
                match self.seat_state.get_pointer(qh, &seat) {
                    Ok(pointer) => self.pointer = Some(pointer),
                    Err(err) => tracing::warn!(?err, "no pointer for the settings window"),
                }
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard => {
                if let Some(keyboard) = self.keyboard.take() {
                    keyboard.release();
                }
            }
            Capability::Pointer => {
                if let Some(pointer) = self.pointer.take() {
                    pointer.release();
                }
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _s: wl_seat::WlSeat) {}
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

delegate_registry!(App);
smithay_client_toolkit::delegate_dispatch2!(App);
