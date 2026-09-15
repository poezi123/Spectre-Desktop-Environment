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

use crate::render::{output_elements, PatternShader, RenderCache};
use crate::state::Spectre;

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
    state.refresh_wallpaper(mode.size.w, mode.size.h);

    init_dmabuf(&mut state, &mut backend);

    let shader = PatternShader::compile(backend.renderer());
    if shader.is_none() {
        tracing::warn!("running without the Spectre Pattern");
    }
    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let mut cache = RenderCache::default();

    state.start_ipc();
    state.start_xwayland();
    tracing::info!(socket = %state.socket_name, "Spectre is up (nested)");
    state.set_panel_running(state.config.panel.enabled);
    let startup = state.config.general.startup_commands(false);
    for command in startup {
        state.spawn(&command);
    }

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
                    state.refresh_wallpaper(mode.size.w, mode.size.h);
                    state.reflow_output(&output);
                    state.mark_dirty();
                }
                WinitEvent::Input(event) => state.handle_input(event),
                WinitEvent::CloseRequested => state.stop(),
                WinitEvent::Redraw => state.mark_dirty(),
                WinitEvent::Focus(_) => {}
            });

            let _ = status;

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
                count_frame();
                if let Err(err) = draw(
                    state,
                    &mut backend,
                    &output,
                    &mut damage_tracker,
                    shader.as_ref(),
                    &mut cache,
                ) {
                    tracing::error!(?err, "frame failed");
                }
            }

            state.refresh();
            TimeoutAction::ToDuration(FRAME_INTERVAL)
        })
        .map_err(|err| anyhow::anyhow!("failed to install the frame timer: {err}"))?;

    event_loop.run(None, &mut state, |state| {
        if crate::termination_requested() {
            tracing::info!("asked to quit; ending the session cleanly");
            state.running = false;
        }
        if !state.running {
            state.loop_signal.stop();
        }
    })?;

    Ok(())
}

fn count_frame() {
    use std::cell::Cell;
    thread_local! {
        static COUNT: Cell<u32> = const { Cell::new(0) };
        static SINCE: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
    }
    COUNT.with(|c| c.set(c.get() + 1));
    SINCE.with(|since| {
        let start = since.get().unwrap_or_else(std::time::Instant::now);
        since.set(Some(start));
        if start.elapsed() >= Duration::from_secs(1) {
            tracing::trace!(fps = COUNT.with(|c| c.replace(0)), "frames drawn in the last second");
            since.set(Some(std::time::Instant::now()));
        }
    });
}

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
    cache: &mut RenderCache,
) -> anyhow::Result<()> {
    let elements = {
        let renderer = backend.renderer();
        let elements = output_elements(state, output, renderer, shader, cache);
        crate::render::dump_scene(&elements, output.current_scale().fractional_scale());
        elements
    };

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

    let time = state.clock.now();
    state.workspaces.active().elements().for_each(|window| {
        window.send_frame(output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    });
    for layer in smithay::desktop::layer_map_for_output(output).layers() {
        layer.send_frame(output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }

    Ok(())
}
