//! The Spectre application launcher.
//!
//! A keyboard-driven overlay: type to filter, arrows to move, Enter to launch,
//! Escape to dismiss. It is a plain layer-shell client, so it can be replaced
//! by any other launcher without touching the compositor.

mod entry;
mod matcher;
mod ui;

use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::Context;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client as wayland_client;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::wlr_layer::{
    KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use spectre_config::Config;
use spectre_draw::Canvas;
use spectre_text::TextRenderer;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

use crate::entry::Entry;

const BTN_LEFT: u32 = 0x110;

fn main() -> anyhow::Result<()> {
    init_tracing();

    let (config, error) = Config::load();
    if let Some(error) = error {
        tracing::error!(%error, "using built-in defaults");
    }
    let config = config.resolved();

    let entries = Entry::load_all();
    tracing::info!(applications = entries.len(), "launcher starting");

    let conn = Connection::connect_to_env().context("no Wayland compositor to connect to")?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor is missing")?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .context("this compositor does not support wlr-layer-shell")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm is missing")?;

    let surface = compositor.create_surface(&qh);
    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("spectre-launcher"), None);
    // Unanchored, so the compositor centres it. Exclusive keyboard: a launcher
    // that does not get the keystrokes is not a launcher.
    let (width, height) = ui::window_size(1280, 800, 8);
    layer.set_size(width as u32, height as u32);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.commit();

    let pool = SlotPool::new((width * height * 4) as usize, &shm)
        .context("could not allocate the launcher's shared memory")?;

    let mut event_loop: EventLoop<Launcher> = EventLoop::try_new()?;
    let mut launcher = Launcher {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer,
        keyboard: None,
        pointer: None,
        width,
        height,
        scale: 1,
        configured: false,
        exit: false,
        dirty: true,
        canvas: Canvas::new(0, 0),
        text: TextRenderer::new(),
        config,
        entries,
        query: String::new(),
        results: Vec::new(),
        selected: 0,
        offset: 0,
        started: Instant::now(),
    };
    launcher.refilter();

    WaylandSource::new(conn, event_queue).insert(event_loop.handle())?;

    // The launcher is a one-shot: it dismisses itself once something has been
    // launched, Escape is pressed, or the keyboard is taken away.
    let signal = event_loop.get_signal();
    event_loop.run(None, &mut launcher, move |launcher| {
        if launcher.exit {
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

struct Launcher {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,

    width: i32,
    height: i32,
    scale: i32,
    configured: bool,
    exit: bool,
    dirty: bool,

    canvas: Canvas,
    text: TextRenderer,
    config: Config,

    entries: Vec<Entry>,
    query: String,
    /// Indices into `entries`, ranked. Indices rather than references so the
    /// borrow checker does not have to reason about a self-referential struct.
    results: Vec<usize>,
    selected: usize,
    offset: usize,
    started: Instant,
}

impl Launcher {
    /// Re-rank after the query changed.
    fn refilter(&mut self) {
        let ranked = matcher::rank(&self.query, &self.entries);
        self.results = ranked
            .into_iter()
            .map(|entry| {
                self.entries.iter().position(|e| std::ptr::eq(e, entry)).unwrap_or(0)
            })
            .collect();
        self.selected = 0;
        self.offset = 0;
        self.dirty = true;
    }

    fn move_selection(&mut self, delta: isize) {
        if self.results.is_empty() {
            return;
        }
        let count = self.results.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(count) as usize;
        self.offset = ui::scroll_offset(self.selected, ui::visible_rows(self.height), self.offset);
        self.dirty = true;
    }

    /// Launch the selected entry and quit.
    fn activate(&mut self) {
        let Some(&index) = self.results.get(self.selected) else {
            return;
        };
        let entry = self.entries[index].clone();
        self.launch(&entry);
        self.exit = true;
    }

    fn launch(&self, entry: &Entry) {
        let command = if entry.terminal {
            // A terminal application needs one; the configured spawn command is
            // the desktop's own terminal binding, which is what the user
            // already told us they want.
            format!("{} -e {}", self.terminal_command(), entry.exec)
        } else {
            entry.exec.clone()
        };

        let Some(argv) = shell_split(&command) else {
            tracing::warn!(%command, "unbalanced quotes in Exec");
            return;
        };
        let Some((program, args)) = argv.split_first() else {
            return;
        };

        let result = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match result {
            Ok(_) => tracing::info!(%command, "launched"),
            Err(err) => tracing::warn!(?err, %program, "could not launch"),
        }
    }

    /// The terminal to run console applications in.
    fn terminal_command(&self) -> String {
        use spectre_config::{Action, Keybind, Modifiers as Mods};

        // Reuse whatever `Mod+Return` is bound to: that is the terminal the
        // user actually uses, rather than a hard-coded guess.
        let binding = Keybind::new(Mods::logo(), "return");
        match self.config.keybinds.get(&binding) {
            Some(Action::Spawn { command }) => command.clone(),
            _ => std::env::var("TERMINAL").unwrap_or_else(|_| String::from("xterm")),
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

        let results: Vec<&Entry> = self.results.iter().map(|&i| &self.entries[i]).collect();
        let phase = self
            .config
            .theme
            .window_pattern
            .phase(self.started.elapsed().as_secs_f64());
        let frame = ui::Frame {
            theme: &self.config.theme,
            query: &self.query,
            results: &results,
            selected: self.selected,
            offset: self.offset,
            pattern_phase: phase,
            scale: self.scale as f32,
        };
        ui::draw(&mut self.canvas, &mut self.text, &frame);

        let stride = width * 4;
        let Ok((buffer, target)) =
            self.pool.create_buffer(width, height, stride, wl_shm::Format::Argb8888)
        else {
            tracing::warn!("could not get a buffer for the launcher");
            return;
        };
        let bytes = self.canvas.as_bytes();
        let len = target.len().min(bytes.len());
        target[..len].copy_from_slice(&bytes[..len]);

        let surface = self.layer.wl_surface();
        surface.set_buffer_scale(self.scale);
        surface.damage_buffer(0, 0, width, height);
        if let Err(err) = buffer.attach_to(surface) {
            tracing::warn!(?err, "could not attach the launcher buffer");
            return;
        }
        self.layer.commit();
    }
}

/// Split a command line on whitespace, honouring quotes.
fn shell_split(input: &str) -> Option<Vec<String>> {
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

impl KeyboardHandler for Launcher {
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
        // Losing keyboard focus means something else took over; a launcher that
        // lingers invisible-but-alive is a bug report waiting to happen.
        self.exit = true;
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
            Keysym::Escape => self.exit = true,
            Keysym::Return | Keysym::KP_Enter => self.activate(),
            Keysym::Down | Keysym::Tab => self.move_selection(1),
            Keysym::Up | Keysym::ISO_Left_Tab => self.move_selection(-1),
            Keysym::BackSpace => {
                if self.query.pop().is_some() {
                    self.refilter();
                }
            }
            _ => {
                if let Some(utf8) = event.utf8.filter(|s| !s.chars().any(char::is_control)) {
                    self.query.push_str(&utf8);
                    self.refilter();
                }
            }
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

    /// Held keys repeat, so holding Backspace clears the query and holding an
    /// arrow scrolls the list, exactly as a text field would.
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

impl PointerHandler for Launcher {
    fn pointer_frame(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _p: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            let (x, y) = (event.position.0 as i32 * self.scale, event.position.1 as i32 * self.scale);
            match event.kind {
                PointerEventKind::Motion { .. } => {
                    if let Some(row) = ui::row_at(self.width * self.scale, self.height * self.scale, x, y)
                    {
                        let index = self.offset + row;
                        if index < self.results.len() && index != self.selected {
                            self.selected = index;
                            self.dirty = true;
                        }
                    }
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    if let Some(row) = ui::row_at(self.width * self.scale, self.height * self.scale, x, y)
                    {
                        let index = self.offset + row;
                        if index < self.results.len() {
                            self.selected = index;
                            self.activate();
                        }
                    }
                }
                _ => {}
            }
        }
        self.redraw_if_needed();
    }
}

impl CompositorHandler for Launcher {
    fn scale_factor_changed(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: &wl_surface::WlSurface,
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

impl LayerShellHandler for Launcher {
    fn closed(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _l: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _l: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (w, h) = configure.new_size;
        if w != 0 {
            self.width = w as i32;
        }
        if h != 0 {
            self.height = h as i32;
        }
        self.configured = true;
        self.dirty = true;
        self.redraw_if_needed();
    }
}

impl OutputHandler for Launcher {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: wl_output::WlOutput) {}
    fn update_output(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: wl_output::WlOutput) {}
}

impl SeatHandler for Launcher {
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
        match capability {
            Capability::Keyboard if self.keyboard.is_none() => {
                match self.seat_state.get_keyboard(qh, &seat, None) {
                    Ok(keyboard) => self.keyboard = Some(keyboard),
                    Err(err) => tracing::warn!(?err, "no keyboard for the launcher"),
                }
            }
            Capability::Pointer if self.pointer.is_none() => {
                match self.seat_state.get_pointer(qh, &seat) {
                    Ok(pointer) => self.pointer = Some(pointer),
                    Err(err) => tracing::warn!(?err, "no pointer for the launcher"),
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

impl ShmHandler for Launcher {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Launcher {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

delegate_registry!(Launcher);
smithay_client_toolkit::delegate_dispatch2!(Launcher);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lines_split_on_whitespace_and_quotes() {
        assert_eq!(shell_split("firefox --new-window").unwrap(), ["firefox", "--new-window"]);
        assert_eq!(shell_split(r#"sh -c "echo hi""#).unwrap(), ["sh", "-c", "echo hi"]);
    }

    #[test]
    fn an_unbalanced_quote_is_rejected() {
        assert!(shell_split(r#"sh -c "echo"#).is_none());
    }

    #[test]
    fn an_empty_command_yields_nothing() {
        assert_eq!(shell_split("  ").unwrap(), Vec::<String>::new());
    }
}
