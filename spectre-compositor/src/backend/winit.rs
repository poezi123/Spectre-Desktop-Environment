//! Nested backend.
//!
//! Runs the whole compositor inside a window of an existing Wayland or X11
//! session. This is the development loop and the VM smoke test: it exercises
//! the same state, handlers and renderer as the real session, minus KMS.

use std::time::Duration;

use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use smithay::utils::{Rectangle, Transform};
use spectre_config::Config;

use crate::render::{output_elements, PatternShader};
use crate::state::Spectre;

/// Nominal frame interval. The nested window has no vblank to follow, so the
/// compositor paces itself; 60 Hz is plenty for an ambient pattern and keeps
/// idle CPU low on the low-end machines Spectre targets.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

pub fn run(config: Config) -> anyhow::Result<()> {
    let mut event_loop: EventLoop<Spectre> = EventLoop::try_new()?;
    let display: Display<Spectre> = Display::new()?;

    let mut state = Spectre::new(
        display,
        event_loop.handle(),
        event_loop.get_signal(),
        config,
        "winit",
    )?;

    let (mut backend, winit_events) = winit::init::<GlesRenderer>()
        .map_err(|err| anyhow::anyhow!("failed to open a nested window: {err}"))?;

    let size = backend.window_size();
    let mode = Mode { size, refresh: 60_000 };
    let output = Output::new(
        "spectre-nested".to_owned(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Spectre".into(),
            model: "Nested".into(),
        },
    );
    let _global = output.create_global::<Spectre>(&state.display_handle);
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    output.set_preferred(mode);
    state.workspaces.map_output(&output, (0, 0).into());

    init_dmabuf(&mut state, &mut backend);

    let shader = PatternShader::compile(backend.renderer());
    if shader.is_none() {
        tracing::warn!("running without the Spectre Pattern");
    }
    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    tracing::info!(socket = %state.socket_name, "Spectre is up (nested)");
    for command in state.config.general.autostart.clone() {
        state.spawn(&command);
    }

    // A timer drives redraws; input and Wayland traffic wake the loop on their
    // own, and `mark_dirty` short-circuits the "nothing changed" case.
    let mut winit_events = winit_events;
    let timer = Timer::immediate();
    event_loop
        .handle()
        .insert_source(timer, move |_, _, state: &mut Spectre| {
            let status = winit_events.dispatch_new_events(|event| match event {
                WinitEvent::Resized { size, scale_factor } => {
                    let mode = Mode { size, refresh: 60_000 };
                    output.change_current_state(
                        Some(mode),
                        None,
                        Some(smithay::output::Scale::Fractional(scale_factor)),
                        None,
                    );
                    output.set_preferred(mode);
                    state.workspaces.map_output(&output, (0, 0).into());
                    state.reflow_output(&output);
                }
                WinitEvent::Input(event) => state.handle_input(event),
                WinitEvent::CloseRequested => state.stop(),
                WinitEvent::Redraw => state.mark_dirty(),
                WinitEvent::Focus(_) => {}
            });

            // The window closing arrives as `CloseRequested`, which already
            // stops the loop; anything else here would just duplicate it.
            let _ = status;

            // Resolve dmabuf imports now that the renderer is reachable.
            if !state.pending_dmabufs.is_empty() {
                use smithay::backend::renderer::ImportDma;
                let renderer = backend.renderer();
                for (dmabuf, notifier) in std::mem::take(&mut state.pending_dmabufs) {
                    match renderer.import_dmabuf(&dmabuf, None) {
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

            if state.take_dirty() {
                if let Err(err) = draw(state, &mut backend, &output, &mut damage_tracker, shader.as_ref())
                {
                    tracing::error!(?err, "frame failed");
                }
            }

            state.refresh();
            TimeoutAction::ToDuration(FRAME_INTERVAL)
        })
        .map_err(|err| anyhow::anyhow!("failed to install the frame timer: {err}"))?;

    event_loop.run(None, &mut state, |state| {
        if !state.running {
            state.loop_signal.stop();
        }
    })?;

    Ok(())
}

/// Publish the `zwp_linux_dmabuf_v1` global.
///
/// Clients need more than a format list: Mesa asks the feedback for the DRM
/// node it should allocate on, and without one it gives up with
/// `failed to get driver name for fd -1`. So the render node is looked up from
/// the EGL display and advertised as the main device, with a plain format-only
/// global as the fallback for drivers that cannot report a node.
fn init_dmabuf(
    state: &mut Spectre,
    backend: &mut smithay::backend::winit::WinitGraphicsBackend<GlesRenderer>,
) {
    use smithay::backend::egl::EGLDevice;
    use smithay::backend::renderer::ImportDma;
    use smithay::wayland::dmabuf::DmabufFeedbackBuilder;

    let formats: Vec<_> = backend.renderer().dmabuf_formats().into_iter().collect();
    if formats.is_empty() {
        tracing::warn!("the renderer exposes no dmabuf formats; GPU clients will fall back to shared memory");
        return;
    }

    let render_node = EGLDevice::device_for_display(backend.renderer().egl_context().display())
        .ok()
        .and_then(|device| device.try_get_render_node().ok().flatten());

    let global = match render_node {
        Some(node) => {
            let feedback = DmabufFeedbackBuilder::new(node.dev_id(), formats.clone()).build();
            match feedback {
                Ok(feedback) => state
                    .dmabuf_state
                    .create_global_with_default_feedback::<Spectre>(&state.display_handle, &feedback),
                Err(err) => {
                    tracing::warn!(?err, "could not build dmabuf feedback; advertising formats only");
                    state.dmabuf_state.create_global::<Spectre>(&state.display_handle, formats)
                }
            }
        }
        None => {
            tracing::warn!("no DRM render node behind the EGL display; advertising formats only");
            state.dmabuf_state.create_global::<Spectre>(&state.display_handle, formats)
        }
    };

    state.dmabuf_global = Some(global);
}

fn draw(
    state: &mut Spectre,
    backend: &mut smithay::backend::winit::WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    shader: Option<&PatternShader>,
) -> anyhow::Result<()> {
    let elements = {
        let renderer = backend.renderer();
        output_elements(state, output, renderer, shader)
    };

    // Bind first: querying the buffer age before the surface is current makes
    // EGL complain about a bad surface on the very first frame.
    let (renderer, mut framebuffer) = backend.bind()?;
    let age = 0;
    let result = damage_tracker.render_output(renderer, &mut framebuffer, age, &elements, [0.0; 4])?;
    drop(framebuffer);

    if let Some(damage) = result.damage {
        let damage: Vec<Rectangle<i32, smithay::utils::Physical>> = damage.to_vec();
        backend.submit(Some(&damage))?;
    } else {
        backend.submit(None)?;
    }

    // Tell clients their frame made it to the screen so they can draw the next.
    let time = state.clock.now();
    state.workspaces.active().elements().for_each(|window| {
        window.send_frame(output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    });
    for layer in smithay::desktop::layer_map_for_output(output).layers() {
        layer.send_frame(output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }

    Ok(())
}
