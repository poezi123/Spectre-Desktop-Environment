//! The Spectre panel.
//!
//! A `wlr-layer-shell` client that draws itself into shared memory and talks to
//! the compositor over the Spectre control socket. No GPU context, no toolkit:
//! at 1920x32 the whole surface is a quarter of a megabyte, and keeping it on
//! the CPU is what lets the panel cost single-digit megabytes of memory.

mod clock;
mod draw;
mod layout;
mod readout;

use std::io::{ErrorKind, Read};
use std::time::{Duration, Instant};

/// Repaint interval while the pattern is moving: 30 fps.
const ANIMATION_INTERVAL: Duration = Duration::from_millis(33);

use anyhow::Context;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::generic::Generic;
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{EventLoop, Interest, Mode, PostAction};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use spectre_config::{Config, PanelPosition};
use spectre_ipc::{Client, Desktop, Event, Request};
use spectre_text::{Label, TextRenderer};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

use spectre_draw::Canvas;
use crate::clock::Clock;
use crate::layout::{Item, Placed};
use crate::readout::Readout;

use smithay_client_toolkit::reexports::client as wayland_client;

/// Left mouse button, from `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;

fn main() -> anyhow::Result<()> {
    init_tracing();

    let (config, error) = Config::load_active();
    if let Some(error) = error {
        tracing::error!(%error, "using built-in defaults");
    }
    let config = config.resolved();
    if !config.panel.enabled {
        tracing::info!("the panel is disabled in the configuration");
        return Ok(());
    }

    let conn = Connection::connect_to_env().context("no Wayland compositor to connect to")?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor is missing")?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .context("this compositor does not support wlr-layer-shell")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm is missing")?;

    let height = config.theme.metrics.panel_height.max(8) as i32;
    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Top, Some("spectre-panel"), None);
    configure_layer(&layer, &config, height);
    layer.commit();

    let pool = SlotPool::new((1920 * height * 4) as usize, &shm)
        .context("could not allocate the panel's shared memory")?;

    let mut event_loop: EventLoop<Panel> = EventLoop::try_new()?;
    let ipc = connect_ipc(&event_loop);

    let mut panel = Panel {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer,
        pointer: None,
        width: 0,
        height,
        scale: 1,
        configured: false,
        exit: false,
        dirty: true,
        canvas: Canvas::new(0, 0),
        mask: spectre_draw::PatternMask::new(),
        text: TextRenderer::new(),
        config,
        desktop: Desktop::default(),
        items: Vec::new(),
        pointer_position: None,
        clock: Clock::now(),
        readout: Readout::new(),
        ipc,
        started: Instant::now(),
    };

    WaylandSource::new(conn, event_queue).insert(event_loop.handle())?;

    // One tick a second is enough for a clock that shows minutes and a load
    // readout; anything faster would be the panel burning power to say nothing.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, panel: &mut Panel| {
            panel.tick();
            TimeoutAction::ToDuration(Duration::from_secs(1))
        })
        .map_err(|err| anyhow::anyhow!("could not start the panel clock: {err}"))?;

    // A moving pattern is paced here rather than off frame callbacks: the
    // panel is a thin strip, and repainting it faster than this only spends
    // CPU that a laptop on battery would rather keep.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(ANIMATION_INTERVAL), |_, _, panel: &mut Panel| {
            if panel.config.theme.panel_pattern.needs_continuous_redraw() {
                panel.dirty = true;
                panel.redraw_if_needed();
            }
            TimeoutAction::ToDuration(ANIMATION_INTERVAL)
        })
        .map_err(|err| anyhow::anyhow!("could not start the panel animation: {err}"))?;

    tracing::info!("Spectre panel ready");
    event_loop.run(None, &mut panel, |panel| {
        if panel.exit {
            panel.loop_stop();
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

/// Anchor the panel to its configured edge and reserve room for it.
fn configure_layer(layer: &LayerSurface, config: &Config, height: i32) {
    let (anchor, size) = match config.panel.position {
        PanelPosition::Top => (Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, (0, height as u32)),
        PanelPosition::Bottom => {
            (Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT, (0, height as u32))
        }
        PanelPosition::Left => (Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM, (height as u32, 0)),
        PanelPosition::Right => (Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM, (height as u32, 0)),
    };
    layer.set_anchor(anchor);
    layer.set_size(size.0, size.1);
    // Windows must not end up underneath the panel.
    layer.set_exclusive_zone(height);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
}

/// Connect to the compositor and subscribe, if a session is running.
fn connect_ipc(event_loop: &EventLoop<'static, Panel>) -> Option<Client> {
    let mut client = match Client::connect() {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(%err, "no control socket; the panel will show no windows");
            return None;
        }
    };
    if let Err(err) = client.send(&Request::Subscribe) {
        tracing::warn!(%err, "could not subscribe to desktop state");
        return None;
    }

    let Ok(socket) = client.as_raw().try_clone() else {
        tracing::warn!("could not watch the control socket");
        return Some(client);
    };
    if socket.set_nonblocking(true).is_err() {
        tracing::warn!("could not put the control socket in non-blocking mode");
        return Some(client);
    }

    let inserted = event_loop.handle().insert_source(
        Generic::new(socket, Interest::READ, Mode::Level),
        |_, socket, panel: &mut Panel| {
            // `Generic` guards the fd against being closed twice, so read
            // through a borrow of the inner socket rather than the wrapper.
            panel.read_ipc(&mut &**socket);
            Ok(PostAction::Continue)
        },
    );
    if let Err(err) = inserted {
        tracing::warn!(?err, "could not watch the control socket");
    }
    Some(client)
}

struct Panel {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    pointer: Option<wl_pointer::WlPointer>,

    width: i32,
    height: i32,
    scale: i32,
    configured: bool,
    exit: bool,
    dirty: bool,

    canvas: Canvas,
    mask: spectre_draw::PatternMask,
    text: TextRenderer,
    config: Config,

    desktop: Desktop,
    items: Vec<Placed>,
    pointer_position: Option<(i32, i32)>,
    clock: Clock,
    readout: Readout,
    ipc: Option<Client>,
    started: Instant,
}

impl Panel {
    fn loop_stop(&self) {}

    /// Once-a-second work: the clock and the load readout.
    fn tick(&mut self) {
        let clock = Clock::now();
        if clock != self.clock {
            self.clock = clock;
            self.dirty = true;
        }
        if self.readout.refresh() {
            self.dirty = true;
        }
        self.redraw_if_needed();
    }

    /// Drain the control socket.
    fn read_ipc(&mut self, socket: &mut impl Read) {
        let mut buf = [0u8; 8192];
        let mut pending = String::new();
        loop {
            match socket.read(&mut buf) {
                Ok(0) => {
                    tracing::info!("the compositor closed the control socket");
                    self.exit = true;
                    break;
                }
                Ok(n) => pending.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) => {
                    tracing::warn!(?err, "control socket read failed");
                    self.exit = true;
                    break;
                }
            }
        }

        for line in pending.lines().filter(|l| !l.trim().is_empty()) {
            match spectre_ipc::parse_line::<Event>(line) {
                Ok(Event::State(desktop)) => {
                    if desktop != self.desktop {
                        self.desktop = desktop;
                        self.dirty = true;
                    }
                }
                Ok(Event::ConfigChanged) => {
                    let (config, error) = spectre_config::Config::load_active();
                    if let Some(error) = error {
                        tracing::warn!(%error, "keeping the running configuration");
                    } else {
                        self.config = config;
                        self.dirty = true;
                    }
                }
                Ok(Event::Error { message }) => tracing::warn!(%message, "compositor reported"),
                Err(err) => tracing::debug!(%err, "ignoring an unreadable event"),
            }
        }
        self.redraw_if_needed();
    }

    fn send(&mut self, request: Request) {
        if let Some(client) = self.ipc.as_mut() {
            if let Err(err) = client.send(&request) {
                tracing::warn!(?err, "could not send a request");
            }
        }
    }

    /// Act on a click at a panel-local position.
    fn click(&mut self, x: i32, y: i32) {
        let Some(placed) = layout::item_at(&self.items, x, y) else {
            return;
        };
        match placed.item.clone() {
            Item::Workspace { index, .. } => self.send(Request::SwitchWorkspace { index }),
            Item::Task { id, focused, minimized, .. } => {
                // Clicking the window you are already in minimises it, the
                // behaviour every taskbar has had for thirty years.
                if focused && !minimized {
                    self.send(Request::MinimizeWindow { id });
                } else {
                    self.send(Request::ActivateWindow { id });
                }
            }
            Item::Session => self.send(Request::Quit),
            Item::Launcher => tracing::info!("the launcher is not implemented yet"),
            Item::Resources | Item::Clock => {}
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

        let scale = self.scale as f32;
        let time = self.clock.time.clone();
        let date = self.clock.date.clone();
        let resources = self.readout.label();
        let show_resources = width >= 900 * self.scale;

        let mut mono = |text: &str, size: f32| -> i32 {
            let label =
                Label::new(text).size(size * scale).family(spectre_text::FontFamily::Monospace);
            self.text.measure(&label).0 as i32
        };
        let measured = layout::Measured {
            clock: mono(&time, draw::LABEL_SIZE).max(mono(&date, draw::DATE_SIZE)),
            resources: mono(&resources, draw::DATE_SIZE + 1.0),
        };

        let text = &mut self.text;
        let items = layout::layout(
            width,
            height,
            &self.desktop,
            &measured,
            |title| text.measure(&Label::new(title).size(draw::LABEL_SIZE * scale)).0 as i32,
            show_resources,
        );

        let elapsed = self.started.elapsed().as_secs_f64();
        let pattern = draw::panel_pattern(&self.config.theme);
        self.mask.prepare(width, height, &pattern, pattern.phase(elapsed), scale);
        let frame = draw::Frame {
            theme: &self.config.theme,
            pointer: self.pointer_position.map(|(x, y)| (x * self.scale, y * self.scale)),
            time: &time,
            date: &date,
            resources: &resources,
            mask: &self.mask,
            color_phase: pattern.color_phase(elapsed),
        };
        draw::draw(&mut self.canvas, &mut self.text, &items, &frame);
        self.items = items;

        let stride = width * 4;
        let Ok((buffer, target)) =
            self.pool.create_buffer(width, height, stride, wl_shm::Format::Argb8888)
        else {
            tracing::warn!("could not get a buffer for the panel");
            return;
        };
        let bytes = self.canvas.as_bytes();
        let len = target.len().min(bytes.len());
        target[..len].copy_from_slice(&bytes[..len]);

        let surface = self.layer.wl_surface();
        surface.set_buffer_scale(self.scale);
        surface.damage_buffer(0, 0, width, height);
        if let Err(err) = buffer.attach_to(surface) {
            tracing::warn!(?err, "could not attach the panel buffer");
            return;
        }
        self.layer.commit();
    }
}

impl CompositorHandler for Panel {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        let scale = new_factor.max(1);
        if scale != self.scale {
            self.scale = scale;
            self.dirty = true;
            self.redraw_if_needed();
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        self.redraw_if_needed();
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for Panel {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (w, h) = configure.new_size;
        tracing::debug!(w, h, "layer surface configured");
        // A zero from the compositor means "you choose", which for a panel
        // spanning an edge only ever happens on the axis we did not fix.
        self.width = if w == 0 { self.width.max(1) } else { w as i32 };
        if h != 0 {
            self.height = h as i32;
        }
        self.configured = true;
        self.dirty = true;
        self.redraw_if_needed();
    }
}

impl OutputHandler for Panel {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: wl_output::WlOutput) {}
    fn update_output(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: wl_output::WlOutput) {}
}

impl SeatHandler for Panel {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _s: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => self.pointer = Some(pointer),
                Err(err) => tracing::warn!(?err, "no pointer for the panel"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
            self.pointer_position = None;
            self.dirty = true;
        }
    }

    fn remove_seat(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _s: wl_seat::WlSeat) {}
}

impl PointerHandler for Panel {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            let (x, y) = (event.position.0 as i32, event.position.1 as i32);
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    if self.pointer_position != Some((x, y)) {
                        self.pointer_position = Some((x, y));
                        self.dirty = true;
                    }
                }
                PointerEventKind::Leave { .. } => {
                    self.pointer_position = None;
                    self.dirty = true;
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    self.click(x * self.scale, y * self.scale);
                }
                _ => {}
            }
        }
        self.redraw_if_needed();
    }
}

impl ShmHandler for Panel {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Panel {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

delegate_registry!(Panel);
smithay_client_toolkit::delegate_dispatch2!(Panel);
