mod session;
mod view;

use std::time::{Duration, Instant};

use anyhow::Context;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::{channel, Event as ChannelEvent};
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
use spectre_draw::Canvas;
use spectre_text::TextRenderer;
use spectre_theme::Theme;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

use crate::session::{Session, Size};
use crate::view::Metrics;

const MIN_REDRAW: Duration = Duration::from_millis(16);
const WHEEL_LINES: i32 = 3;
const START_COLUMNS: usize = 92;
const START_LINES: usize = 26;

fn main() -> anyhow::Result<()> {
    init_tracing();

    let (config, problem) = Config::load_active();
    if let Some(err) = problem {
        tracing::warn!(?err, "using the built-in settings");
    }

    let font_px = config.terminal.font_size();
    let mut text = TextRenderer::new();
    let metrics = Metrics::measure(&mut text, font_px);
    let width = metrics.cell_width * START_COLUMNS as i32 + view::PADDING * 2;
    let height = metrics.cell_height * START_LINES as i32 + view::PADDING * 2;

    let conn = Connection::connect_to_env().context("no Wayland compositor to connect to")?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor is missing")?;
    let xdg_shell = XdgShell::bind(&globals, &qh).context("xdg-shell is missing")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm is missing")?;

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title("Spectre Terminal");
    window.set_app_id("spectre-terminal");
    window.set_min_size(Some((320, 200)));
    window.commit();

    let pool = SlotPool::new((width * height * 4) as usize, &shm)
        .context("could not allocate the terminal's shared memory")?;

    let mut event_loop: EventLoop<App> = EventLoop::try_new()?;
    let (sender, receiver) = channel::<Vec<u8>>();
    let cell = (metrics.cell_width as u16, metrics.cell_height as u16);
    let history = config.terminal.scrollback();
    let session = Session::start(Size::new(START_COLUMNS, START_LINES), cell, history, sender)
        .context("could not start the shell")?;

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
        last_draw: Instant::now(),
        canvas: Canvas::new(0, 0),
        text,
        metrics,
        theme: config.theme.clone(),
        session,
        control: false,
        alt: false,
        shift: false,
        font_px,
    };

    WaylandSource::new(conn, event_queue).insert(event_loop.handle())?;
    event_loop
        .handle()
        .insert_source(receiver, |event, _, app: &mut App| match event {
            ChannelEvent::Msg(bytes) => {
                app.session.feed(&bytes);
                app.dirty = true;
                app.redraw_if_needed();
            }
            ChannelEvent::Closed => app.exit = true,
        })
        .map_err(|err| anyhow::anyhow!("could not listen to the shell: {err}"))?;

    let signal = event_loop.get_signal();
    event_loop.run(Duration::from_millis(16), &mut app, move |app| {
        app.redraw_if_needed();
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
    last_draw: Instant,

    canvas: Canvas,
    text: TextRenderer,
    metrics: Metrics,
    theme: Theme,
    session: Session,
    control: bool,
    alt: bool,
    shift: bool,
    font_px: f32,
}

impl App {
    fn redraw_if_needed(&mut self) {
        if !self.dirty || !self.configured || self.width <= 0 {
            return;
        }
        if self.last_draw.elapsed() < MIN_REDRAW {
            return;
        }
        self.last_draw = Instant::now();
        self.draw();
    }

    fn change_font(&mut self, steps: f32) {
        let wanted = (self.font_px + steps).clamp(
            spectre_config::terminal::MIN_FONT_SIZE,
            spectre_config::terminal::MAX_FONT_SIZE,
        );
        if wanted == self.font_px {
            return;
        }
        self.font_px = wanted;
        self.metrics = Metrics::measure(&mut self.text, self.font_px);
        self.fit_the_grid();
        self.dirty = true;
        self.redraw_if_needed();
    }

    fn fit_the_grid(&mut self) {
        let width = self.width * self.scale;
        let height = self.height * self.scale;
        let size = Size::new(self.metrics.columns(width), self.metrics.lines(height));
        let cell = (self.metrics.cell_width as u16, self.metrics.cell_height as u16);
        self.session.resize(size, cell);
    }

    fn draw(&mut self) {
        self.dirty = false;
        let (width, height) = (self.width * self.scale, self.height * self.scale);
        self.canvas.resize(width, height);
        view::draw(&mut self.canvas, &mut self.text, &self.session, &self.theme, &self.metrics);

        let stride = width * 4;
        let Ok((buffer, target)) =
            self.pool.create_buffer(width, height, stride, wl_shm::Format::Argb8888)
        else {
            tracing::warn!("could not get a buffer for the terminal window");
            return;
        };
        let bytes = self.canvas.as_bytes();
        let len = target.len().min(bytes.len());
        target[..len].copy_from_slice(&bytes[..len]);

        let surface = self.window.wl_surface();
        surface.set_buffer_scale(self.scale);
        surface.damage_buffer(0, 0, width, height);
        if let Err(err) = buffer.attach_to(surface) {
            tracing::warn!(?err, "could not attach the terminal buffer");
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
        let columns = self.metrics.cell_width * START_COLUMNS as i32 + view::PADDING * 2;
        let lines = self.metrics.cell_height * START_LINES as i32 + view::PADDING * 2;
        self.width = configure.new_size.0.map(|w| w.get() as i32).unwrap_or(columns);
        self.height = configure.new_size.1.map(|h| h.get() as i32).unwrap_or(lines);
        self.configured = true;
        self.dirty = true;
        self.fit_the_grid();
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
        if self.shift {
            let paged = match event.keysym {
                Keysym::Page_Up => Some(true),
                Keysym::Page_Down => Some(false),
                _ => None,
            };
            if let Some(up) = paged {
                self.session.scroll_page(up);
                self.dirty = true;
                self.redraw_if_needed();
                return;
            }
        }
        if self.control {
            let steps = match event.keysym {
                Keysym::plus | Keysym::equal | Keysym::KP_Add => Some(1.0),
                Keysym::minus | Keysym::KP_Subtract => Some(-1.0),
                _ => None,
            };
            if let Some(steps) = steps {
                self.change_font(steps);
                return;
            }
        }
        if let Some(bytes) = keys_to_bytes(&event, self.control, self.alt) {
            self.session.scroll_to_bottom();
            self.session.write(&bytes);
            self.dirty = true;
        }
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
        modifiers: Modifiers,
        _raw: RawModifiers,
        _layout: u32,
    ) {
        self.control = modifiers.ctrl;
        self.alt = modifiers.alt;
        self.shift = modifiers.shift;
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
        self.fit_the_grid();
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

    fn output_destroyed(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _o: wl_output::WlOutput,
    ) {
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
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            match self.seat_state.get_keyboard(qh, &seat, None) {
                Ok(keyboard) => self.keyboard = Some(keyboard),
                Err(err) => tracing::warn!(?err, "no keyboard for the terminal"),
            }
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => self.pointer = Some(pointer),
                Err(err) => tracing::warn!(?err, "no mouse for the terminal"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
        }
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
    }

    fn remove_seat(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _s: wl_seat::WlSeat) {}
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
            let PointerEventKind::Axis { vertical, .. } = &event.kind else {
                continue;
            };
            let mut lines = wheel_steps(vertical) * WHEEL_LINES;
            if lines == 0 {
                lines = (vertical.absolute / self.metrics.cell_height as f64).round() as i32;
            }
            if lines == 0 {
                continue;
            }
            self.session.scroll(-lines);
            self.dirty = true;
        }
        self.redraw_if_needed();
    }
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

fn wheel_steps(axis: &smithay_client_toolkit::seat::pointer::AxisScroll) -> i32 {
    if axis.value120 != 0 {
        return axis.value120 / 120;
    }
    axis.discrete
}

fn keys_to_bytes(event: &KeyEvent, control: bool, alt: bool) -> Option<Vec<u8>> {
    let plain = match event.keysym {
        Keysym::Return | Keysym::KP_Enter => Some(vec![b'\r']),
        Keysym::BackSpace => Some(vec![0x7f]),
        Keysym::Tab => Some(vec![b'\t']),
        Keysym::Escape => Some(vec![0x1b]),
        Keysym::Up => Some(b"\x1b[A".to_vec()),
        Keysym::Down => Some(b"\x1b[B".to_vec()),
        Keysym::Right => Some(b"\x1b[C".to_vec()),
        Keysym::Left => Some(b"\x1b[D".to_vec()),
        Keysym::Home => Some(b"\x1b[H".to_vec()),
        Keysym::End => Some(b"\x1b[F".to_vec()),
        Keysym::Page_Up => Some(b"\x1b[5~".to_vec()),
        Keysym::Page_Down => Some(b"\x1b[6~".to_vec()),
        Keysym::Delete => Some(b"\x1b[3~".to_vec()),
        _ => None,
    };
    if let Some(bytes) = plain {
        return Some(bytes);
    }

    let typed = event.utf8.as_ref()?;
    if typed.is_empty() {
        return None;
    }
    let mut bytes = match control {
        true => control_bytes(typed)?,
        false => typed.as_bytes().to_vec(),
    };
    if alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

fn control_bytes(typed: &str) -> Option<Vec<u8>> {
    let letter = typed.chars().next()?;
    let code = match letter {
        'a'..='z' => letter as u8 - b'a' + 1,
        'A'..='Z' => letter as u8 - b'A' + 1,
        '@' => 0,
        '[' => 0x1b,
        '\\' => 0x1c,
        ']' => 0x1d,
        '^' => 0x1e,
        '_' => 0x1f,
        ' ' => 0,
        _ => return Some(typed.as_bytes().to_vec()),
    };
    Some(vec![code])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(keysym: Keysym, typed: Option<&str>) -> KeyEvent {
        KeyEvent {
            time: 0,
            raw_code: 0,
            keysym,
            utf8: typed.map(|t| t.to_owned()),
        }
    }

    fn axis(value120: i32, discrete: i32) -> smithay_client_toolkit::seat::pointer::AxisScroll {
        smithay_client_toolkit::seat::pointer::AxisScroll {
            value120,
            discrete,
            ..Default::default()
        }
    }

    #[test]
    fn a_wheel_notch_counts_as_one_step() {
        assert_eq!(wheel_steps(&axis(-120, 0)), -1);
        assert_eq!(wheel_steps(&axis(240, 0)), 2);
    }

    #[test]
    fn an_older_compositor_is_still_understood() {
        assert_eq!(wheel_steps(&axis(0, -1)), -1);
    }

    #[test]
    fn typing_sends_the_letter() {
        let bytes = keys_to_bytes(&key(Keysym::a, Some("a")), false, false).unwrap();
        assert_eq!(bytes, b"a");
    }

    #[test]
    fn control_c_sends_the_interrupt() {
        let bytes = keys_to_bytes(&key(Keysym::c, Some("c")), true, false).unwrap();
        assert_eq!(bytes, vec![3]);
    }

    #[test]
    fn the_arrows_send_escape_sequences() {
        let bytes = keys_to_bytes(&key(Keysym::Up, None), false, false).unwrap();
        assert_eq!(bytes, b"\x1b[A");
    }

    #[test]
    fn alt_puts_an_escape_in_front() {
        let bytes = keys_to_bytes(&key(Keysym::b, Some("b")), false, true).unwrap();
        assert_eq!(bytes, vec![0x1b, b'b']);
    }

    #[test]
    fn a_key_without_a_letter_sends_nothing() {
        assert!(keys_to_bytes(&key(Keysym::Shift_L, None), false, false).is_none());
    }
}
