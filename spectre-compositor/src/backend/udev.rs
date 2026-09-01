//! Native backend: KMS/DRM output, libinput devices, libseat session.
//!
//! This is the real Spectre session, the one a display manager starts. It is
//! deliberately single-GPU: the machines the project targets - old laptops,
//! VMs, low-power desktops - have exactly one, and supporting a second one
//! costs a buffer copy per frame that those machines cannot spare. Multi-GPU
//! belongs in a later phase, behind the same `Backend` enum.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

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
use smithay::reexports::drm::control::{connector, crtc, Device as ControlDevice, ModeTypeFlags};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_server::Display;
use smithay::utils::DeviceFd;
use spectre_config::Config;

use crate::render::{output_elements, PatternShader, SpectreElement};
use crate::state::Spectre;

/// One driven connector: its output, its DRM compositor and its damage state.
struct Surface {
    output: Output,
    compositor: DrmCompositor<
        GbmAllocator<DrmDeviceFd>,
        GbmFramebufferExporter<DrmDeviceFd>,
        (),
        DrmDeviceFd,
    >,
    /// Set while a page flip is in flight, so we do not queue two frames.
    awaiting_flip: bool,
}

/// Everything the native backend owns.
///
/// Kept behind an `Rc<RefCell<..>>` rather than inside [`Spectre`] so the
/// compositor state stays backend-agnostic: `Spectre` never has to know that
/// DRM exists.
struct Udev {
    /// Held for the lifetime of the backend: dropping the session hands the
    /// seat back to libseat and the compositor loses its devices.
    #[allow(dead_code)]
    session: LibSeatSession,
    renderer: GlesRenderer,
    gbm: GbmDevice<DrmDeviceFd>,
    drm: DrmDevice,
    surfaces: HashMap<crtc::Handle, Surface>,
    shader: Option<PatternShader>,
    /// libseat can take the session away when the user switches VT.
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
    init_session_events(&mut state, session_notifier, &shared)?;

    state.start_ipc();
    tracing::info!(socket = %state.socket_name, seat = %seat_name, "Spectre is up (native)");
    for command in state.config.general.autostart.clone() {
        state.spawn(&command);
    }

    // Kick the first frame; from here vblank events drive the loop.
    render_all(&mut state, &shared);

    event_loop.run(Some(Duration::from_millis(16)), &mut state, |state| {
        if !state.running {
            state.loop_signal.stop();
            return;
        }
        state.refresh();
        if state.take_dirty() {
            render_all(state, &shared);
        }
    })?;

    Ok(())
}

/// Open the primary GPU and build a renderer on it.
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
            active: Arc::new(AtomicBool::new(true)),
        },
        drm_notifier,
    ))
}

/// Create an output and a DRM compositor for every connected connector.
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
        // Prefer the connector's preferred mode, else the largest one.
        let Some(mode) = connector
            .modes()
            .iter()
            .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or_else(|| connector.modes().first())
            .copied()
        else {
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
        output.change_current_state(Some(output_mode), None, None, Some((x, 0).into()));
        output.set_preferred(output_mode);

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
        udev.surfaces.insert(crtc, Surface { output, compositor, awaiting_flip: false });
        x += w as i32;
    }

    if udev.surfaces.is_empty() {
        anyhow::bail!("no connector could be driven");
    }
    Ok(())
}

/// Find a CRTC that can drive `connector` and is not already taken.
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
                configure_device(device, &state.config);
            }
            state.handle_input(event);
        })
        .map_err(|err| anyhow::anyhow!("could not listen for input: {err}"))?;
    Ok(())
}

/// Apply the `[input.pointer]` settings to a freshly plugged device.
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
                // Only retire the flip here. Rendering the next frame straight
                // from the vblank handler turns into a busy loop on drivers
                // that complete a page flip immediately (vmwgfx does), so the
                // event loop's own tick decides when to draw again - and only
                // when something actually changed.
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

fn render_all(state: &mut Spectre, shared: &Shared) {
    let crtcs: Vec<crtc::Handle> = shared.borrow().surfaces.keys().copied().collect();
    for crtc in crtcs {
        render_crtc(state, shared, crtc);
    }
}

fn render_crtc(state: &mut Spectre, shared: &Shared, crtc: crtc::Handle) {
    let mut udev = shared.borrow_mut();
    if !udev.active.load(Ordering::SeqCst) {
        return;
    }

    // Resolve any dmabuf imports first; the renderer is right here.
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
        return;
    };
    if udev.surfaces.get(&crtc).is_some_and(|s| s.awaiting_flip) {
        tracing::trace!(?crtc, "skipping render: a page flip is still in flight");
        return;
    }

    let Udev { renderer, shader, surfaces, .. } = &mut *udev;
    let elements: Vec<SpectreElement> = output_elements(state, &output, renderer, shader.as_ref());

    let Some(surface) = surfaces.get_mut(&crtc) else {
        return;
    };

    match surface.compositor.render_frame(renderer, &elements, [0.0; 4], FrameFlags::DEFAULT) {
        Ok(frame) if !frame.is_empty => match surface.compositor.queue_frame(()) {
            Ok(()) => {
                surface.awaiting_flip = true;
                tracing::trace!(?crtc, elements = elements.len(), "frame queued");
            }
            Err(err) => tracing::warn!(?err, "could not queue a frame"),
        },
        Ok(_) => tracing::trace!(?crtc, elements = elements.len(), "nothing to redraw"),
        Err(err) => tracing::error!(?err, "frame failed"),
    }

    drop(udev);

    // Release clients to draw their next frame.
    let time = state.clock.now();
    for window in state.workspaces.active().elements() {
        window.send_frame(&output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    for layer in smithay::desktop::layer_map_for_output(&output).layers() {
        layer.send_frame(&output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
}
