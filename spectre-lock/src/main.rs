mod auth;
mod ui;

use std::time::{Duration, Instant};

use anyhow::Context;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::{channel, Event as ChannelEvent, Sender};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client as wayland_client;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::session_lock::{
    SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
    SessionLockSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use spectre_config::Config;
use spectre_draw::{Canvas, PatternMask};
use spectre_text::TextRenderer;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

const MAX_PASSWORD: usize = 256;

fn main() -> anyhow::Result<()> {
    init_tracing();

    let (config, error) = Config::load_active();
    if let Some(error) = error {
        tracing::warn!(%error, "using the built-in look");
    }
    let config = config.resolved();

    let conn = Connection::connect_to_env().context("no Wayland compositor to connect to")?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor is missing")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm is missing")?;
    let lock_state = SessionLockState::new(&globals, &qh);
    let lock = lock_state
        .lock(&qh)
        .context("this compositor cannot lock the session")?;

    let pool = SlotPool::new(1920 * 1080 * 4, &shm)
        .context("could not allocate the lock screen's shared memory")?;

    let mut event_loop: EventLoop<Screen> = EventLoop::try_new()?;
    let (answers, results) = channel::<bool>();

    let mut screen = Screen {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        shm,
        pool,
        lock,
        surfaces: Vec::new(),
        keyboard: None,
        exit: false,
        user: auth::user(),
        password: String::new(),
        message: String::new(),
        working: false,
        answers,
        canvas: Canvas::new(0, 0),
        mask: PatternMask::new(),
        text: TextRenderer::new(),
        config,
        started: Instant::now(),
        drawn_at: None,
    };

    WaylandSource::new(conn, event_queue).insert(event_loop.handle())?;
    event_loop
        .handle()
        .insert_source(results, |event, _, screen: &mut Screen| {
            if let ChannelEvent::Msg(right) = event {
                screen.answer(right);
            }
        })
        .map_err(|err| anyhow::anyhow!("could not listen for the answer: {err}"))?;

    let signal = event_loop.get_signal();
    event_loop.run(Some(Duration::from_millis(500)), &mut screen, move |screen| {
        screen.tick();
        if screen.exit {
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

struct Cover {
    surface: SessionLockSurface,
    width: i32,
    height: i32,
    scale: i32,
}

struct Screen {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    shm: Shm,
    pool: SlotPool,
    lock: SessionLock,
    surfaces: Vec<Cover>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    exit: bool,

    user: String,
    password: String,
    message: String,
    working: bool,
    answers: Sender<bool>,

    canvas: Canvas,
    mask: PatternMask,
    text: TextRenderer,
    config: Config,
    started: Instant,
    drawn_at: Option<Instant>,
}

impl Screen {
    fn tick(&mut self) {
        let due = match self.drawn_at {
            Some(when) => when.elapsed() >= Duration::from_millis(900),
            None => true,
        };
        if due {
            self.draw_all();
        }
    }

    fn answer(&mut self, right: bool) {
        self.working = false;
        self.password.clear();
        if right {
            tracing::info!("the password was right, letting go of the screen");
            self.lock.unlock();
            self.exit = true;
            return;
        }
        self.message = String::from("Wrong password");
        self.draw_all();
    }

    fn try_the_password(&mut self) {
        if self.working || self.password.is_empty() {
            return;
        }
        self.working = true;
        self.message = String::from("Checking …");
        self.draw_all();

        let user = self.user.clone();
        let password = std::mem::take(&mut self.password);
        let answers = self.answers.clone();
        std::thread::spawn(move || {
            let right = auth::check(&user, &password);
            let _ = answers.send(right);
        });
    }

    fn typed(&mut self, letters: &str) {
        if self.working {
            return;
        }
        if self.password.chars().count() >= MAX_PASSWORD {
            return;
        }
        self.password.push_str(letters);
        self.message.clear();
        self.draw_all();
    }

    fn rub_out(&mut self) {
        if self.working {
            return;
        }
        self.password.pop();
        self.draw_all();
    }

    fn clear(&mut self) {
        if self.working {
            return;
        }
        self.password.clear();
        self.message.clear();
        self.draw_all();
    }

    fn draw_all(&mut self) {
        self.drawn_at = Some(Instant::now());
        let clock = now();
        for index in 0..self.surfaces.len() {
            self.draw_one(index, &clock);
        }
    }

    fn draw_one(&mut self, index: usize, clock: &(String, String)) {
        let Some(cover) = self.surfaces.get(index) else {
            return;
        };
        let (width, height, scale) = (cover.width * cover.scale, cover.height * cover.scale, cover.scale);
        if width <= 0 || height <= 0 {
            return;
        }
        self.canvas.resize(width, height);

        let pattern = self.config.theme.desktop_pattern.0;
        let elapsed = self.started.elapsed().as_secs_f64();
        self.mask.prepare_scrolling(width, height, &pattern, pattern.phase(elapsed), scale as f32);

        let typed = self.password.chars().count();
        let screen = ui::Screen {
            theme: &self.config.theme,
            mask: &self.mask,
            color_phase: pattern.color_phase(elapsed),
            time: &clock.0,
            date: &clock.1,
            user: &self.user,
            typed,
            message: &self.message,
            working: self.working,
        };
        ui::draw(&mut self.canvas, &mut self.text, &screen, scale);

        let stride = width * 4;
        let Ok((buffer, target)) =
            self.pool.create_buffer(width, height, stride, wl_shm::Format::Argb8888)
        else {
            tracing::warn!("could not get a buffer for the lock screen");
            return;
        };
        let bytes = self.canvas.as_bytes();
        let len = target.len().min(bytes.len());
        target[..len].copy_from_slice(&bytes[..len]);

        let Some(cover) = self.surfaces.get(index) else {
            return;
        };
        let surface = cover.surface.wl_surface();
        surface.set_buffer_scale(cover.scale);
        surface.damage_buffer(0, 0, width, height);
        if let Err(err) = buffer.attach_to(surface) {
            tracing::warn!(?err, "could not attach the lock screen buffer");
            return;
        }
        surface.commit();
    }
}

fn now() -> (String, String) {
    let clock = time::macros::format_description!("[hour]:[minute]");
    let day = time::macros::format_description!("[day].[month].[year]");
    let utc = time::OffsetDateTime::now_utc();
    let local = match time::UtcOffset::current_local_offset() {
        Ok(offset) => utc.to_offset(offset),
        Err(_) => utc,
    };
    (
        local.format(clock).unwrap_or_else(|_| String::from("--:--")),
        local.format(day).unwrap_or_else(|_| String::from("--.--.----")),
    )
}

impl SessionLockHandler for Screen {
    fn locked(&mut self, _c: &Connection, qh: &QueueHandle<Self>, lock: SessionLock) {
        tracing::info!("the screen is ours now");
        for output in self.output_state.outputs() {
            let surface = self.compositor.create_surface(qh);
            let cover = lock.create_lock_surface(surface, &output, qh);
            self.surfaces.push(Cover { surface: cover, width: 0, height: 0, scale: 1 });
        }
    }

    fn finished(&mut self, _c: &Connection, _qh: &QueueHandle<Self>, _lock: SessionLock) {
        tracing::warn!("the compositor took the lock away");
        self.exit = true;
    }

    fn configure(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
        surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _serial: u32,
    ) {
        let (width, height) = configure.new_size;
        let found = self
            .surfaces
            .iter()
            .position(|cover| cover.surface.wl_surface() == surface.wl_surface());
        let Some(index) = found else {
            return;
        };
        if let Some(cover) = self.surfaces.get_mut(index) {
            cover.width = width as i32;
            cover.height = height as i32;
        }
        let clock = now();
        self.draw_one(index, &clock);
    }
}

impl CompositorHandler for Screen {
    fn scale_factor_changed(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        let found = self.surfaces.iter().position(|cover| cover.surface.wl_surface() == surface);
        if let Some(index) = found {
            if let Some(cover) = self.surfaces.get_mut(index) {
                cover.scale = factor.max(1);
            }
            let clock = now();
            self.draw_one(index, &clock);
        }
    }

    fn transform_changed(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
        _s: &wl_surface::WlSurface,
        _t: wayland_client::protocol::wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
        _s: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
        _s: &wl_surface::WlSurface,
        _o: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
        _s: &wl_surface::WlSurface,
        _o: &wl_output::WlOutput,
    ) {
    }
}

impl KeyboardHandler for Screen {
    fn enter(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
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
        _qh: &QueueHandle<Self>,
        _k: &wl_keyboard::WlKeyboard,
        _s: &wl_surface::WlSurface,
        _serial: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
        _k: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        match event.keysym {
            Keysym::Return | Keysym::KP_Enter => self.try_the_password(),
            Keysym::BackSpace => self.rub_out(),
            Keysym::Escape => self.clear(),
            _ => {
                let Some(letters) = event.utf8.as_ref() else {
                    return;
                };
                let printable = !letters.is_empty() && letters.chars().all(|c| !c.is_control());
                if printable {
                    let letters = letters.clone();
                    self.typed(&letters);
                }
            }
        }
    }

    fn release_key(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
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
        _qh: &QueueHandle<Self>,
        _k: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _modifiers: Modifiers,
        _raw: RawModifiers,
        _layout: u32,
    ) {
    }
}

impl SeatHandler for Screen {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _c: &Connection, _qh: &QueueHandle<Self>, _s: wl_seat::WlSeat) {}

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
                Err(err) => tracing::warn!(?err, "no keyboard for the lock screen"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
        }
    }

    fn remove_seat(&mut self, _c: &Connection, _qh: &QueueHandle<Self>, _s: wl_seat::WlSeat) {}
}

impl OutputHandler for Screen {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _c: &Connection, _qh: &QueueHandle<Self>, _o: wl_output::WlOutput) {}

    fn update_output(&mut self, _c: &Connection, _qh: &QueueHandle<Self>, _o: wl_output::WlOutput) {}

    fn output_destroyed(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
        _o: wl_output::WlOutput,
    ) {
    }
}

impl ShmHandler for Screen {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Screen {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

delegate_registry!(Screen);
smithay_client_toolkit::delegate_dispatch2!(Screen);
