use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags};
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmEvent, DrmNode};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::input::InputEvent;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportDma;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev;
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::drm::control::{
    connector, crtc, Device as ControlDevice, Mode as DrmMode, ModeTypeFlags,
};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_server::Display;
use smithay::utils::DeviceFd;
use spectre_config::Config;

use smithay::backend::drm::compositor::PrimaryPlaneElement;

use crate::render::{output_elements, PatternShader, RenderCache, SpectreElement};
use crate::state::Spectre;

struct Surface {
    output: Output,
    connector: connector::Handle,
    compositor: DrmCompositor<
        GbmAllocator<DrmDeviceFd>,
        GbmFramebufferExporter<DrmDeviceFd>,
        (),
        DrmDeviceFd,
    >,
    awaiting_flip: bool,
    last_frame: Instant,
    frame_interval: Duration,
    last_scene: u64,
    last_cost: Duration,
    stats: FrameStats,
}

struct Udev {
    #[allow(dead_code)]
    session: LibSeatSession,
    renderer: GlesRenderer,
    gbm: GbmDevice<DrmDeviceFd>,
    drm: DrmDevice,
    surfaces: HashMap<crtc::Handle, Surface>,
    shader: Option<PatternShader>,
    cache: RenderCache,
    active: Arc<AtomicBool>,
}

type Shared = Rc<std::cell::RefCell<Udev>>;

pub fn run(config: Config) -> anyhow::Result<()> {
    let mut event_loop: EventLoop<Spectre> = EventLoop::try_new()?;
    let display: Display<Spectre> = Display::new()?;

    let (session, session_notifier) = LibSeatSession::new()
        .map_err(|err| anyhow::anyhow!("could not take a seat via libseat: {err}"))?;
    let seat_name = session.seat();

    let mut state = Spectre::new(
        display,
        event_loop.handle(),
        event_loop.get_signal(),
        config,
        &seat_name,
    )?;

    let (udev, drm_notifier) = open_primary_gpu(session.clone(), &seat_name)?;
    let shared: Shared = Rc::new(std::cell::RefCell::new(udev));

    scan_connectors(&mut state, &shared)?;
    init_dmabuf(&mut state, &shared);
    init_input(&mut state, session.clone(), &seat_name)?;
    init_drm_events(&mut state, &shared, drm_notifier)?;
    init_display_changes(&mut state, &seat_name)?;
    init_session_events(&mut state, session_notifier, &shared)?;

    state.start_ipc();
    if state.config.general.xwayland_at_login() {
        state.start_xwayland();
    }
    tracing::info!(socket = %state.socket_name, seat = %seat_name, "Spectre is up (native)");
    state.set_panel_running(state.config.panel.enabled);
    let startup = state.config.general.startup_commands(false);
    for command in startup {
        state.spawn(&command);
    }

    render_all(&mut state, &shared);

    event_loop.run(Some(Duration::from_millis(16)), &mut state, |state| {
        if crate::termination_requested() {
            tracing::info!("asked to quit; ending the session cleanly");
            state.running = false;
        }
        if !state.running {
            state.loop_signal.stop();
            return;
        }
        state.refresh();
        if state.take_display_dirty() {
            apply_display(state, &shared);
        }
        if state.take_dirty() {
            render_all(state, &shared);
        }
    })?;

    Ok(())
}

fn open_primary_gpu(
    mut session: LibSeatSession,
    seat_name: &str,
) -> anyhow::Result<(Udev, DrmDeviceNotifier)> {
    let path = udev::primary_gpu(seat_name)?
        .or_else(|| udev::all_gpus(seat_name).ok()?.into_iter().next())
        .ok_or_else(|| anyhow::anyhow!("no GPU found on seat `{seat_name}`"))?;
    tracing::info!(gpu = %path.display(), "opening the primary GPU");

    let fd = session
        .open(&path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)
        .map_err(|err| anyhow::anyhow!("cannot open {}: {err}", path.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));

    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd)?;

    let egl_display = unsafe { EGLDisplay::new(gbm.clone()) }?;
    let context = EGLContext::new(&egl_display)?;
    let mut renderer = unsafe { GlesRenderer::new(context) }?;
    let shader = PatternShader::compile(&mut renderer);
    if shader.is_none() {
        tracing::warn!("running without the Spectre Pattern");
    }

    Ok((
        Udev {
            session,
            renderer,
            gbm,
            drm,
            surfaces: HashMap::new(),
            shader,
            cache: RenderCache::default(),
            active: Arc::new(AtomicBool::new(true)),
        },
        drm_notifier,
    ))
}

fn scan_connectors(state: &mut Spectre, shared: &Shared) -> anyhow::Result<()> {
    let mut udev = shared.borrow_mut();
    let resources = udev.drm.resource_handles()?;

    let connectors: Vec<connector::Info> = resources
        .connectors()
        .iter()
        .filter_map(|handle| udev.drm.get_connector(*handle, true).ok())
        .filter(|info| info.state() == connector::State::Connected)
        .collect();

    if connectors.is_empty() {
        anyhow::bail!("no display is connected");
    }

    let mut x = 0;
    let mut used_crtcs: Vec<crtc::Handle> = Vec::new();

    for connector in connectors {
        let Some(mode) = pick_mode(connector.modes(), &state.config.display) else {
            tracing::warn!(connector = ?connector.interface(), "connector reports no modes");
            continue;
        };

        let Some(crtc) = pick_crtc(&udev.drm, &resources, &connector, &used_crtcs) else {
            tracing::warn!(connector = ?connector.interface(), "no free CRTC");
            continue;
        };
        used_crtcs.push(crtc);

        let name = format!("{}-{}", connector.interface().as_str(), connector.interface_id());
        let (w, h) = mode.size();
        let output_mode = OutputMode {
            size: (w as i32, h as i32).into(),
            refresh: (mode.vrefresh() * 1000) as i32,
        };
        let (phys_w, phys_h) = connector.size().unwrap_or((0, 0));
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (phys_w as i32, phys_h as i32).into(),
                subpixel: Subpixel::Unknown,
                make: "Spectre".into(),
                model: name.clone(),
            },
        );
        let _global = output.create_global::<Spectre>(&state.display_handle);
        output.change_current_state(
            Some(output_mode),
            None,
            Some(smithay::output::Scale::Fractional(state.config.display.output_scale())),
            Some((x, 0).into()),
        );
        output.set_preferred(output_mode);
        for available in connector.modes() {
            let (mw, mh) = available.size();
            output.add_mode(OutputMode {
                size: (mw as i32, mh as i32).into(),
                refresh: (available.vrefresh() * 1000) as i32,
            });
        }

        let surface = udev.drm.create_surface(crtc, mode, &[connector.handle()])?;
        let allocator = GbmAllocator::new(
            udev.gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );
        let renderer_formats = udev.renderer.dmabuf_formats();
        let gbm = udev.gbm.clone();
        let render_node = DrmNode::from_file(udev.drm.device_fd()).ok();
        let exporter = GbmFramebufferExporter::new(gbm.clone(), render_node);
        let compositor = DrmCompositor::new(
            &output,
            surface,
            None,
            allocator,
            exporter,
            [
                smithay::backend::allocator::Fourcc::Argb8888,
                smithay::backend::allocator::Fourcc::Xrgb8888,
            ],
            renderer_formats,
            udev.drm.cursor_size(),
            Some(gbm),
        )?;

        tracing::info!(output = %name, mode = ?(w, h), refresh = mode.vrefresh(), "driving connector");
        state.workspaces.map_output(&output, (x, 0).into());
        state.refresh_wallpaper(w as i32, h as i32);
        let refresh = if mode.vrefresh() == 0 { 60 } else { mode.vrefresh() };
        udev.surfaces.insert(
            crtc,
            Surface {
                output,
                connector: connector.handle(),
                compositor,
                awaiting_flip: false,
                last_frame: Instant::now() - Duration::from_secs(1),
                frame_interval: Duration::from_secs_f64(1.0 / refresh as f64),
                last_scene: 0,
                last_cost: Duration::ZERO,
                stats: FrameStats::new(),
            },
        );
        x += w as i32;
    }

    if udev.surfaces.is_empty() {
        anyhow::bail!("no connector could be driven");
    }
    Ok(())
}

fn pick_crtc(
    drm: &DrmDevice,
    resources: &smithay::reexports::drm::control::ResourceHandles,
    connector: &connector::Info,
    used: &[crtc::Handle],
) -> Option<crtc::Handle> {
    connector
        .encoders()
        .iter()
        .filter_map(|handle| drm.get_encoder(*handle).ok())
        .flat_map(|encoder| resources.filter_crtcs(encoder.possible_crtcs()))
        .find(|crtc| !used.contains(crtc))
}

fn init_dmabuf(state: &mut Spectre, shared: &Shared) {
    use smithay::wayland::dmabuf::DmabufFeedbackBuilder;

    let udev = shared.borrow();
    let formats: Vec<_> = udev.renderer.dmabuf_formats().into_iter().collect();
    if formats.is_empty() {
        tracing::warn!("the renderer exposes no dmabuf formats");
        return;
    }

    let node = DrmNode::from_file(udev.drm.device_fd())
        .ok()
        .and_then(|node| node.node_with_type(smithay::backend::drm::NodeType::Render)?.ok())
        .unwrap_or_else(|| DrmNode::from_file(udev.drm.device_fd()).expect("the DRM fd is a node"));

    let global = match DmabufFeedbackBuilder::new(node.dev_id(), formats.clone()).build() {
        Ok(feedback) => state
            .dmabuf_state
            .create_global_with_default_feedback::<Spectre>(&state.display_handle, &feedback),
        Err(err) => {
            tracing::warn!(?err, "could not build dmabuf feedback; advertising formats only");
            state.dmabuf_state.create_global::<Spectre>(&state.display_handle, formats)
        }
    };
    drop(udev);
    state.dmabuf_global = Some(global);
}

fn init_input(
    state: &mut Spectre,
    session: LibSeatSession,
    seat_name: &str,
) -> anyhow::Result<()> {
    let mut context = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(
        session.into(),
    );
    context
        .udev_assign_seat(seat_name)
        .map_err(|_| anyhow::anyhow!("libinput refused seat `{seat_name}`"))?;

    let backend = LibinputInputBackend::new(context);
    state
        .loop_handle
        .insert_source(backend, move |event, _, state: &mut Spectre| {
            if let InputEvent::DeviceAdded { device } = &event {
                tracing::info!(device = device.name(), "input device added");
                configure_device(device, &state.config);
            }
            state.handle_input(event);
        })
        .map_err(|err| anyhow::anyhow!("could not listen for input: {err}"))?;
    Ok(())
}

fn apply_display(state: &mut Spectre, shared: &Shared) {
    let scale = state.config.display.output_scale();
    let wanted: Vec<(crtc::Handle, Option<DrmMode>)> = {
        let udev = shared.borrow();
        udev.surfaces
            .iter()
            .map(|(crtc, surface)| {
                let modes = udev
                    .drm
                    .get_connector(surface.connector, true)
                    .map(|info| info.modes().to_vec())
                    .unwrap_or_default();
                (*crtc, pick_mode(&modes, &state.config.display))
            })
            .collect()
    };

    let mut outputs = Vec::new();
    {
        let mut udev = shared.borrow_mut();
        for (crtc, mode) in wanted {
            let Some(mode) = mode else { continue };
            let Some(surface) = udev.surfaces.get_mut(&crtc) else { continue };

            let (w, h) = mode.size();
            let output_mode = OutputMode {
                size: (w as i32, h as i32).into(),
                refresh: (mode.vrefresh() * 1000) as i32,
            };
            let changed = surface.output.current_mode() != Some(output_mode);
            if changed {
                if let Err(err) = surface.compositor.use_mode(mode) {
                    tracing::warn!(?err, "the display refused the requested mode");
                    continue;
                }
                let refresh = if mode.vrefresh() == 0 { 60 } else { mode.vrefresh() };
                surface.frame_interval = Duration::from_secs_f64(1.0 / refresh as f64);
                tracing::info!(mode = ?(w, h), refresh, "display mode changed");
            }
            surface.output.change_current_state(
                Some(output_mode),
                None,
                Some(smithay::output::Scale::Fractional(scale)),
                None,
            );
            outputs.push((surface.output.clone(), (w as i32, h as i32)));
        }
    }

    for (output, (w, h)) in outputs {
        smithay::desktop::layer_map_for_output(&output).arrange();
        state.reflow_output(&output);
        state.refresh_wallpaper(w, h);
    }
    state.mark_dirty();
}

fn pick_mode(modes: &[DrmMode], display: &spectre_config::Display) -> Option<DrmMode> {
    if let Some(wanted) = display.wanted_mode() {
        let matching: Vec<&DrmMode> = modes
            .iter()
            .filter(|m| {
                let (w, h) = m.size();
                w as i32 == wanted.width && h as i32 == wanted.height
            })
            .collect();
        let chosen = match wanted.refresh {
            Some(hz) => matching.iter().find(|m| m.vrefresh() == hz).or(matching.first()),
            None => matching.first(),
        };
        if let Some(mode) = chosen {
            return Some(**mode);
        }
        let resolution = display.resolution.clone();
        tracing::warn!(
            %resolution,
            "the configured resolution is not offered by this display; using the preferred one"
        );
    }

    modes
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or_else(|| modes.first())
        .copied()
}

fn configure_device(device: &smithay::reexports::input::Device, config: &Config) {
    use smithay::reexports::input::{AccelProfile, ClickMethod, ScrollMethod};

    let pointer = &config.input.pointer;
    let mut device = device.clone();

    if device.config_tap_finger_count() > 0 {
        let _ = device.config_tap_set_enabled(pointer.tap_to_click);
        let _ = device.config_tap_set_drag_enabled(pointer.tap_and_drag);
        let _ = device.config_click_set_method(ClickMethod::Clickfinger);
        let _ = device.config_scroll_set_method(ScrollMethod::TwoFinger);
    }
    if device.config_scroll_has_natural_scroll() {
        let _ = device.config_scroll_set_natural_scroll_enabled(pointer.natural_scroll);
    }
    if device.config_left_handed_is_available() {
        let _ = device.config_left_handed_set(pointer.left_handed);
    }
    if device.config_accel_is_available() {
        let profile = match pointer.accel_profile {
            spectre_config::input::AccelProfile::Flat => AccelProfile::Flat,
            spectre_config::input::AccelProfile::Adaptive => AccelProfile::Adaptive,
        };
        let _ = device.config_accel_set_profile(profile);
        let _ = device.config_accel_set_speed(pointer.sane_accel());
    }
    if device.config_dwt_is_available() {
        let _ = device.config_dwt_set_enabled(pointer.disable_while_typing);
    }
}

fn init_drm_events(
    state: &mut Spectre,
    shared: &Shared,
    notifier: DrmDeviceNotifier,
) -> anyhow::Result<()> {
    let shared = shared.clone();
    state
        .loop_handle
        .insert_source(notifier, move |event, _, state: &mut Spectre| match event {
            DrmEvent::VBlank(crtc) => {
                let mut udev = shared.borrow_mut();
                if let Some(surface) = udev.surfaces.get_mut(&crtc) {
                    surface.awaiting_flip = false;
                    if let Err(err) = surface.compositor.frame_submitted() {
                        tracing::warn!(?err, "frame submission reported an error");
                    }
                }
                let _ = state;
            }
            DrmEvent::Error(err) => tracing::error!(?err, "DRM device error"),
        })
        .map_err(|err| anyhow::anyhow!("could not listen for vblank: {err}"))?;
    Ok(())
}

fn init_display_changes(state: &mut Spectre, seat_name: &str) -> anyhow::Result<()> {
    let watcher = udev::UdevBackend::new(seat_name)
        .map_err(|err| anyhow::anyhow!("cannot watch for display changes: {err}"))?;
    state
        .loop_handle
        .insert_source(watcher, move |event, _, state: &mut Spectre| {
            if let udev::UdevEvent::Changed { .. } = event {
                tracing::info!("the screen told us something changed, looking at it again");
                state.mark_display_dirty();
            }
        })
        .map_err(|err| anyhow::anyhow!("could not listen for display changes: {err}"))?;
    Ok(())
}

fn init_session_events(
    state: &mut Spectre,
    notifier: smithay::backend::session::libseat::LibSeatSessionNotifier,
    shared: &Shared,
) -> anyhow::Result<()> {
    let shared = shared.clone();
    state
        .loop_handle
        .insert_source(notifier, move |event, _, state: &mut Spectre| match event {
            SessionEvent::PauseSession => {
                tracing::info!("session paused (VT switch)");
                let mut udev = shared.borrow_mut();
                udev.active.store(false, Ordering::SeqCst);
                udev.drm.pause();
            }
            SessionEvent::ActivateSession => {
                tracing::info!("session resumed");
                {
                    let mut udev = shared.borrow_mut();
                    udev.active.store(true, Ordering::SeqCst);
                    if let Err(err) = udev.drm.activate(true) {
                        tracing::error!(?err, "failed to reactivate the DRM device");
                    }
                    for surface in udev.surfaces.values_mut() {
                        surface.awaiting_flip = false;
                        if let Err(err) = surface.compositor.reset_state() {
                            tracing::warn!(?err, "failed to reset a DRM surface");
                        }
                    }
                }
                state.mark_dirty();
                render_all(state, &shared);
            }
        })
        .map_err(|err| anyhow::anyhow!("could not listen for session events: {err}"))?;
    Ok(())
}

const SCANOUT: FrameFlags = FrameFlags::empty();

const FLIP_TIMEOUT: Duration = Duration::from_millis(500);

fn scene_hash(elements: &[SpectreElement], skip: usize) -> u64 {
    use smithay::backend::renderer::element::Element;
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for element in elements.iter().skip(skip) {
        element.id().hash(&mut hasher);
    }
    hasher.finish()
}

const STATS_INTERVAL: Duration = Duration::from_secs(5);

const SLOW_SYNC_MS: u64 = 50;

#[derive(Debug, Clone, Copy, Default)]
struct FramePhases {
    built: Duration,
    rendered: Duration,
    waited: Duration,
    queued: Duration,
}

#[derive(Debug)]
struct FrameStats {
    since: Instant,
    frames: u32,
    total: Duration,
    slowest: Duration,
    phases: FramePhases,
    full_redraws: u32,
    told_about_the_wait: bool,
}

impl FrameStats {
    fn new() -> Self {
        Self {
            since: Instant::now(),
            frames: 0,
            total: Duration::ZERO,
            slowest: Duration::ZERO,
            phases: FramePhases::default(),
            full_redraws: 0,
            told_about_the_wait: false,
        }
    }

    fn record(&mut self, cost: Duration, phases: FramePhases, full_redraw: bool) {
        self.frames += 1;
        self.total += cost;
        self.slowest = self.slowest.max(cost);
        self.phases.built += phases.built;
        self.phases.rendered += phases.rendered;
        self.phases.waited += phases.waited;
        self.phases.queued += phases.queued;
        if full_redraw {
            self.full_redraws += 1;
        }
        if self.since.elapsed() < STATS_INTERVAL {
            return;
        }
        let frames = self.frames.max(1);
        let average = |total: Duration| (total / frames).as_millis() as u64;
        tracing::info!(
            frames = self.frames,
            average_ms = average(self.total),
            slowest_ms = self.slowest.as_millis() as u64,
            build_ms = average(self.phases.built),
            render_ms = average(self.phases.rendered),
            wait_ms = average(self.phases.waited),
            queue_ms = average(self.phases.queued),
            full_redraws = self.full_redraws,
            "render stats"
        );

        let waited = average(self.phases.waited);
        let mut told = self.told_about_the_wait;
        if waited >= SLOW_SYNC_MS && !told {
            told = true;
            tracing::warn!(
                wait_ms = waited,
                "the graphics driver hands each frame back very slowly, which is what makes the \
                 desktop feel sluggish; inside a virtual machine a restart usually clears it"
            );
        }
        *self = Self { told_about_the_wait: told, ..Self::new() };
    }
}

fn render_all(state: &mut Spectre, shared: &Shared) {
    let crtcs: Vec<crtc::Handle> = shared.borrow().surfaces.keys().copied().collect();
    for crtc in crtcs {
        if !render_crtc(state, shared, crtc) {
            state.mark_dirty();
        }
    }
}

fn render_crtc(state: &mut Spectre, shared: &Shared, crtc: crtc::Handle) -> bool {
    let mut udev = shared.borrow_mut();
    if !udev.active.load(Ordering::SeqCst) {
        return true;
    }

    if !state.pending_dmabufs.is_empty() {
        for (dmabuf, notifier) in std::mem::take(&mut state.pending_dmabufs) {
            match udev.renderer.import_dmabuf(&dmabuf, None) {
                Ok(_) => {
                    let _ = notifier.successful::<Spectre>();
                }
                Err(err) => {
                    tracing::debug!(?err, "rejected a client dmabuf");
                    notifier.failed();
                }
            }
        }
    }

    let Some(output) = udev.surfaces.get(&crtc).map(|s| s.output.clone()) else {
        return true;
    };
    let now = Instant::now();
    if let Some(surface) = udev.surfaces.get_mut(&crtc) {
        if surface.awaiting_flip {
            if now.duration_since(surface.last_frame) < FLIP_TIMEOUT {
                tracing::trace!(?crtc, "skipping render: a page flip is still in flight");
                return false;
            }
            tracing::warn!(?crtc, "no vblank for the queued frame; carrying on without it");
            surface.awaiting_flip = false;
            if let Err(err) = surface.compositor.reset_state() {
                tracing::warn!(?err, "could not reset the surface after a lost flip");
            }
        }
        let slowest_pace = surface.frame_interval * 2;
        let pace = surface.frame_interval.max(surface.last_cost.min(slowest_pace));
        if now.duration_since(surface.last_frame) < pace {
            return false;
        }
    }

    let started = Instant::now();
    let Udev { renderer, shader, surfaces, cache, .. } = &mut *udev;
    let elements: Vec<SpectreElement> =
        output_elements(state, &output, renderer, shader.as_ref(), cache);
    let built = started.elapsed();
    if let Some(mode) = output.current_mode() {
        let scale = output.current_scale().fractional_scale();
        let without_cursor = elements.get(cache.cursor_elements()..).unwrap_or(&elements);
        state.serve_screenshot(renderer, without_cursor, mode.size, scale);
    }

    let Some(surface) = surfaces.get_mut(&crtc) else {
        return true;
    };

    crate::render::dump_scene(&elements, output.current_scale().fractional_scale());

    let scene = scene_hash(&elements, cache.cursor_elements());
    let mut full_redraw = false;
    if scene != surface.last_scene {
        surface.last_scene = scene;
        surface.compositor.reset_buffer_ages();
        full_redraw = true;
    }

    match surface.compositor.render_frame(renderer, &elements, [0.0; 4], SCANOUT) {
        Ok(frame) if !frame.is_empty => {
            let rendered = started.elapsed();
            if frame.needs_sync() {
                if let PrimaryPlaneElement::Swapchain(element) = &frame.primary_element {
                    if let Err(err) = element.sync.wait() {
                        tracing::warn!(?err, "could not wait for the frame to finish");
                    }
                }
            }
            let waited = started.elapsed();
            match surface.compositor.queue_frame(()) {
                Ok(()) => {
                    if let Some(locker) = state.pending_lock.take() {
                        locker.lock();
                    }
                    surface.awaiting_flip = true;
                    surface.last_frame = now;
                    surface.last_cost = started.elapsed();
                    let phases = FramePhases { built, rendered: rendered - built, waited: waited - rendered, queued: surface.last_cost - waited };
                    surface.stats.record(surface.last_cost, phases, full_redraw);
                    tracing::trace!(?crtc, elements = elements.len(), "frame queued");
                }
                Err(err) => tracing::warn!(?err, "could not queue a frame"),
            }
        }
        Ok(_) => tracing::trace!(?crtc, elements = elements.len(), "nothing to redraw"),
        Err(err) => tracing::error!(?err, "frame failed"),
    }

    drop(udev);

    let time = state.clock.now();
    for window in state.workspaces.active().elements() {
        window.send_frame(&output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    for layer in smithay::desktop::layer_map_for_output(&output).layers() {
        layer.send_frame(&output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    true
}
