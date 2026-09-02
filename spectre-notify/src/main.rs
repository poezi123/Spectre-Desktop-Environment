//! The Spectre notification daemon.
//!
//! Implements `org.freedesktop.Notifications` and draws the popups as a
//! layer-shell surface in the top-right corner. Click a notification to dismiss
//! it; the rest expire on their own, except critical ones, which do not.

mod model;
mod service;
mod ui;

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::Event as ChannelEvent;
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopHandle, RegistrationToken};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client as wayland_client;
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
use spectre_config::Config;
use spectre_draw::Canvas;
use spectre_text::{Label, TextRenderer};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

use crate::model::{Id, IdAllocator, Stack};
use crate::service::{CloseReason, Message, Service};
use crate::ui::Card;

const BTN_LEFT: u32 = 0x110;
/// How many notifications are shown at once.
const STACK_CAPACITY: usize = 5;

fn main() -> anyhow::Result<()> {
    init_tracing();

    let (config, error) = Config::load_active();
    if let Some(error) = error {
        tracing::error!(%error, "using built-in defaults");
    }
    let config = config.resolved();

    let conn = Connection::connect_to_env().context("no Wayland compositor to connect to")?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor is missing")?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .context("this compositor does not support wlr-layer-shell")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm is missing")?;

    let surface = compositor.create_surface(&qh);
    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("spectre-notify"), None);
    layer.set_anchor(Anchor::TOP | Anchor::RIGHT);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    // Notifications float over everything; they must not reserve any room.
    layer.set_exclusive_zone(0);
    layer.set_size(ui::CARD_WIDTH as u32 + ui::SCREEN_MARGIN as u32 * 2, 1);
    layer.commit();

    let pool = SlotPool::new(512 * 512 * 4, &shm)
        .context("could not allocate the notification daemon's shared memory")?;

    let mut event_loop: EventLoop<Daemon> = EventLoop::try_new()?;
    let (tx, rx) = smithay_client_toolkit::reexports::calloop::channel::channel::<Message>();

    let ids = Arc::new(IdAllocator::new());
    let dbus = start_dbus(tx, Arc::clone(&ids))?;

    let mut daemon = Daemon {
        loop_handle: event_loop.handle(),
        expiry_timer: None,
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        compositor,
        layer,
        pointer: None,
        width: ui::CARD_WIDTH + ui::SCREEN_MARGIN * 2,
        height: 1,
        scale: 1,
        configured: false,
        dirty: false,
        canvas: Canvas::new(0, 0),
        text: TextRenderer::new(),
        config,
        stack: Stack::new(STACK_CAPACITY),
        cards: Vec::new(),
        hovered: None,
        dbus,
    };

    event_loop
        .handle()
        .insert_source(rx, |event, _, daemon: &mut Daemon| {
            if let ChannelEvent::Msg(message) = event {
                daemon.on_message(message);
            }
        })
        .map_err(|err| anyhow::anyhow!("could not listen for notifications: {err}"))?;

    WaylandSource::new(conn, event_queue).insert(event_loop.handle())?;
    tracing::info!("notification daemon ready");
    event_loop.run(None, &mut daemon, |_| {})?;
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("SPECTRE_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
}

/// Claim the notification bus name and serve the interface.
fn start_dbus(
    tx: smithay_client_toolkit::reexports::calloop::channel::Sender<Message>,
    ids: Arc<IdAllocator>,
) -> anyhow::Result<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::session()
        .context("no session bus")?
        .name(service::BUS_NAME)
        .context("another notification daemon already owns the bus name")?
        .serve_at(service::PATH, Service::new(tx, ids))
        .context("could not publish the notification interface")?
        .build()
        .context("could not connect to the session bus")
        .map_err(Into::into)
}

struct Daemon {
    loop_handle: LoopHandle<'static, Daemon>,
    /// The single armed expiry timer, if anything is waiting to expire.
    expiry_timer: Option<RegistrationToken>,
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    compositor: CompositorState,
    layer: LayerSurface,
    pointer: Option<wl_pointer::WlPointer>,

    width: i32,
    height: i32,
    scale: i32,
    configured: bool,
    dirty: bool,

    canvas: Canvas,
    text: TextRenderer,
    config: Config,

    stack: Stack,
    cards: Vec<Card>,
    hovered: Option<Id>,
    dbus: zbus::blocking::Connection,
}

impl Daemon {
    fn on_message(&mut self, message: Message) {
        match message {
            Message::Show { notification, replaces_id } => {
                tracing::debug!(
                    app = %notification.app_name,
                    summary = %notification.summary,
                    "notification"
                );
                self.stack.push(notification, replaces_id);
                self.dirty = true;
            }
            Message::Close(id) => {
                if self.stack.close(id) {
                    self.announce_closed(id, CloseReason::Requested);
                    self.dirty = true;
                }
            }
        }
        self.rearm_expiry();
        self.redraw_if_needed();
    }

    /// Arm a timer for the moment the next notification expires.
    ///
    /// A single timer is replaced rather than a repeating one polled, so a
    /// daemon with nothing on screen - which is nearly always - does not wake
    /// up at all. It has to be re-armed whenever the stack changes, because a
    /// timer already waiting on a distant deadline cannot be shortened.
    fn rearm_expiry(&mut self) {
        if let Some(token) = self.expiry_timer.take() {
            self.loop_handle.remove(token);
        }
        let Some(delay) = self.stack.next_deadline(Instant::now()) else {
            return;
        };
        // Never zero: a timer that fires instantly would spin the loop.
        let delay = delay.max(Duration::from_millis(10));

        match self.loop_handle.insert_source(
            Timer::from_duration(delay),
            |_, _, daemon: &mut Daemon| {
                daemon.expiry_timer = None;
                daemon.expire();
                TimeoutAction::Drop
            },
        ) {
            Ok(token) => self.expiry_timer = Some(token),
            Err(err) => tracing::warn!(?err, "could not arm the expiry timer"),
        }
    }

    fn expire(&mut self) {
        for id in self.stack.expire(Instant::now()) {
            self.announce_closed(id, CloseReason::Expired);
            self.dirty = true;
        }
        self.rearm_expiry();
        self.redraw_if_needed();
    }

    /// Tell the sender its notification is gone, as the spec requires.
    fn announce_closed(&self, id: Id, reason: CloseReason) {
        let result = self.dbus.emit_signal(
            None::<&str>,
            service::PATH,
            service::INTERFACE,
            "NotificationClosed",
            &(id, reason as u32),
        );
        if let Err(err) = result {
            tracing::debug!(?err, "could not announce a closed notification");
        }
    }

    fn redraw_if_needed(&mut self) {
        if self.dirty {
            self.draw();
        }
    }

    fn draw(&mut self) {
        self.dirty = false;
        let scale = self.scale;

        let notifications: Vec<&model::Notification> = self.stack.newest_first().collect();
        let text = &mut self.text;
        let inner_width = (ui::CARD_WIDTH - ui::ACCENT_WIDTH - ui::PADDING * 2).max(1) as u32;
        let cards = ui::layout(&notifications, |body| {
            let label = Label::new(body)
                .size(ui::BODY_SIZE)
                .max_width(inner_width)
                .max_lines(ui::BODY_LINES);
            text.measure(&label).1 as i32
        });

        let (width, height) = ui::surface_size(&cards);
        self.cards = cards;

        if height == 0 {
            // Nothing to show: unmap the surface so it stops covering the
            // corner of the screen entirely.
            self.layer.wl_surface().attach(None, 0, 0);
            self.layer.commit();
            self.height = 1;
            return;
        }

        if width != self.width || height != self.height {
            self.width = width;
            self.height = height;
            self.layer.set_size(width as u32, height as u32);
        }

        let (pixel_width, pixel_height) = (width * scale, height * scale);
        self.canvas.resize(pixel_width, pixel_height);

        // The cards are laid out in logical pixels; scale them for drawing.
        let scaled: Vec<Card> = self
            .cards
            .iter()
            .map(|card| Card {
                id: card.id,
                rect: spectre_draw::Rect::new(
                    card.rect.x * scale,
                    card.rect.y * scale,
                    card.rect.w * scale,
                    card.rect.h * scale,
                ),
            })
            .collect();

        ui::draw(
            &mut self.canvas,
            &mut self.text,
            &self.config.theme,
            &notifications,
            &scaled,
            self.hovered,
            scale as f32,
        );

        let stride = pixel_width * 4;
        let Ok((buffer, target)) =
            self.pool.create_buffer(pixel_width, pixel_height, stride, wl_shm::Format::Argb8888)
        else {
            tracing::warn!("could not get a buffer for the notifications");
            return;
        };
        let bytes = self.canvas.as_bytes();
        let len = target.len().min(bytes.len());
        target[..len].copy_from_slice(&bytes[..len]);

        let surface = self.layer.wl_surface();
        surface.set_buffer_scale(scale);

        // Only the cards take clicks; the transparent margin around them must
        // stay clickable through to whatever is underneath.
        if let Ok(region) = Region::new(&self.compositor) {
            for card in &self.cards {
                region.add(card.rect.x, card.rect.y, card.rect.w, card.rect.h);
            }
            surface.set_input_region(Some(region.wl_region()));
        }

        surface.damage_buffer(0, 0, pixel_width, pixel_height);
        if let Err(err) = buffer.attach_to(surface) {
            tracing::warn!(?err, "could not attach the notification buffer");
            return;
        }
        self.layer.commit();
    }
}

impl PointerHandler for Daemon {
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
            let (x, y) = (event.position.0 as i32, event.position.1 as i32);
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    let hovered = ui::card_at(&self.cards, x, y).map(|card| card.id);
                    if hovered != self.hovered {
                        self.hovered = hovered;
                        self.dirty = true;
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if self.hovered.is_some() {
                        self.hovered = None;
                        self.dirty = true;
                    }
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    if let Some(card) = ui::card_at(&self.cards, x, y) {
                        let id = card.id;
                        if self.stack.close(id) {
                            self.announce_closed(id, CloseReason::Dismissed);
                            self.hovered = None;
                            self.dirty = true;
                            self.rearm_expiry();
                        }
                    }
                }
                _ => {}
            }
        }
        self.redraw_if_needed();
    }
}

impl CompositorHandler for Daemon {
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

impl LayerShellHandler for Daemon {
    fn closed(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _l: &LayerSurface) {}

    fn configure(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _l: &LayerSurface,
        _configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        // The size is ours to choose: it follows the stack, not the output.
        self.configured = true;
        self.dirty = true;
        self.redraw_if_needed();
    }
}

impl OutputHandler for Daemon {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: wl_output::WlOutput) {}
    fn update_output(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: wl_output::WlOutput) {}
}

impl SeatHandler for Daemon {
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
                Err(err) => tracing::warn!(?err, "no pointer for the notification daemon"),
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
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
    }

    fn remove_seat(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _s: wl_seat::WlSeat) {}
}

impl ShmHandler for Daemon {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Daemon {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

delegate_registry!(Daemon);
smithay_client_toolkit::delegate_dispatch2!(Daemon);
