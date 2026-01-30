use std::cell::RefCell;
use std::collections::HashMap;
use std::mem;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use niri_config::{Config, OutputName};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{DebugFlags, ImportDma, ImportEgl, Renderer};
use smithay::backend::winit::{self, WinitEvent, WinitGraphicsBackend};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::reexports::winit::dpi::LogicalSize;
use smithay::reexports::winit::window::Window;
use smithay::wayland::presentation::Refresh;

use super::{IpcOutputMap, OutputId, RenderResult};
use crate::niri::{Niri, RedrawState, State};
use crate::render_helpers::debug::draw_damage;
use crate::render_helpers::{resources, shaders, RenderCtx, RenderTarget};
use crate::utils::{get_monotonic_time, logical_output};

pub struct Winit {
    config: Rc<RefCell<Config>>,
    output: Output,
    backend: WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: OutputDamageTracker,
    ipc_outputs: Arc<Mutex<IpcOutputMap>>,
}

impl Winit {
    pub fn new(
        config: Rc<RefCell<Config>>,
        event_loop: LoopHandle<State>,
    ) -> Result<Self, winit::Error> {
        let _span = tracy_client::span!("Winit::new");

        let builder = Window::default_attributes()
            .with_inner_size(LogicalSize::new(1280.0, 800.0))
            // .with_resizable(false)
            .with_title("niri");
        let (backend, winit) = winit::init_from_attributes(builder)?;

        let output = Output::new(
            "winit".to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Smithay".into(),
                model: "Winit".into(),
                serial_number: "Unknown".into(),
            },
        );

        let mode = Mode {
            size: backend.window_size(),
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), None, None, None);
        output.set_preferred(mode);

        output.user_data().insert_if_missing(|| OutputName {
            connector: "winit".to_string(),
            make: Some("Smithay".to_string()),
            model: Some("Winit".to_string()),
            serial: None,
        });

        let physical_properties = output.physical_properties();
        let ipc_outputs = Arc::new(Mutex::new(HashMap::from([(
            OutputId::next(),
            niri_ipc::Output {
                name: output.name(),
                make: physical_properties.make,
                model: physical_properties.model,
                serial: None,
                physical_size: None,
                modes: vec![niri_ipc::Mode {
                    width: backend.window_size().w.clamp(0, u16::MAX as i32) as u16,
                    height: backend.window_size().h.clamp(0, u16::MAX as i32) as u16,
                    refresh_rate: 60_000,
                    is_preferred: true,
                }],
                current_mode: Some(0),
                is_custom_mode: true,
                vrr_supported: false,
                vrr_enabled: false,
                logical: Some(logical_output(&output)),
            },
        )])));

        let damage_tracker = OutputDamageTracker::from_output(&output);

        event_loop
            .insert_source(winit, move |event, _, state| match event {
                WinitEvent::Resized { size, .. } => {
                    let winit = state.backend.winit();
                    winit.output.change_current_state(
                        Some(Mode {
                            size,
                            refresh: 60_000,
                        }),
                        None,
                        None,
                        None,
                    );

                    {
                        let mut ipc_outputs = winit.ipc_outputs.lock().unwrap();
                        let output = ipc_outputs.values_mut().next().unwrap();
                        let mode = &mut output.modes[0];
                        mode.width = size.w.clamp(0, u16::MAX as i32) as u16;
                        mode.height = size.h.clamp(0, u16::MAX as i32) as u16;
                        if let Some(logical) = output.logical.as_mut() {
                            logical.width = size.w as u32;
                            logical.height = size.h as u32;
                        }
                        state.niri.ipc_outputs_changed = true;
                    }

                    state.niri.output_resized(&winit.output);
                }
                WinitEvent::Input(event) => state.process_input_event(event),
                WinitEvent::Focus(_) => (),
                WinitEvent::Redraw => state.niri.queue_redraw(&state.backend.winit().output),
                WinitEvent::CloseRequested => state.niri.stop_signal.stop(),
            })
            .unwrap();

        Ok(Self {
            config,
            output,
            backend,
            damage_tracker,
            ipc_outputs,
        })
    }

    pub fn init(&mut self, niri: &mut Niri) {
        let renderer = self.backend.renderer();
        if let Err(err) = renderer.bind_wl_display(&niri.display_handle) {
            warn!("error binding renderer wl_display: {err}");
        }

        resources::init(renderer);
        shaders::init(renderer);

        let config = self.config.borrow();
        if let Some(src) = config.animations.window_resize.custom_shader.as_deref() {
            shaders::set_custom_resize_program(renderer, Some(src));
        }
        if let Some(src) = config.animations.window_close.custom_shader.as_deref() {
            shaders::set_custom_close_program(renderer, Some(src));
        }
        if let Some(src) = config.animations.window_open.custom_shader.as_deref() {
            shaders::set_custom_open_program(renderer, Some(src));
        }
        drop(config);

        niri.update_shaders();

        niri.add_output(self.output.clone(), None, false);
    }

    pub fn seat_name(&self) -> String {
        "winit".to_owned()
    }

    pub fn with_primary_renderer<T>(
        &mut self,
        f: impl FnOnce(&mut GlesRenderer) -> T,
    ) -> Option<T> {
        Some(f(self.backend.renderer()))
    }

    pub fn render(&mut self, niri: &mut Niri, output: &Output) -> RenderResult {
        let _span = tracy_client::span!("Winit::render");

        // Render the elements.
        let ctx = RenderCtx {
            renderer: self.backend.renderer(),
            target: RenderTarget::Output,
        };
        let mut elements = niri.render(ctx, output, true);

        // Visualize the damage, if enabled.
        if niri.debug_draw_damage {
            let output_state = niri.output_state.get_mut(output).unwrap();
            draw_damage(&mut output_state.debug_damage_tracker, &mut elements);
        }

        // Hand them over to winit.
        let res = {
            let (renderer, mut framebuffer) = self.backend.bind().unwrap();
            // FIXME: currently impossible to call due to a mutable borrow.
            //
            // let age = self.backend.buffer_age().unwrap();
            let age = 0;
            self.damage_tracker
                .render_output(renderer, &mut framebuffer, age, &elements, [0.; 4])
                .unwrap()
        };

        niri.update_primary_scanout_output(output, &res.states);

        let rv;
        if let Some(damage) = res.damage {
            if self
                .config
                .borrow()
                .debug
                .wait_for_frame_completion_before_queueing
            {
                let _span = tracy_client::span!("wait for completion");
                if let Err(err) = res.sync.wait() {
                    warn!("error waiting for frame completion: {err:?}");
                }
            }

            self.backend.submit(Some(damage)).unwrap();

            let mut presentation_feedbacks = niri.take_presentation_feedbacks(output, &res.states);
            presentation_feedbacks.presented::<_, smithay::utils::Monotonic>(
                get_monotonic_time(),
                Refresh::Unknown,
                0,
                wp_presentation_feedback::Kind::empty(),
            );

            rv = RenderResult::Submitted;
        } else {
            rv = RenderResult::NoDamage;
        }

        let output_state = niri.output_state.get_mut(output).unwrap();
        match mem::replace(&mut output_state.redraw_state, RedrawState::Idle) {
            RedrawState::Idle => unreachable!(),
            RedrawState::Queued => (),
            RedrawState::WaitingForVBlank { .. } => unreachable!(),
            RedrawState::WaitingForEstimatedVBlank(_) => unreachable!(),
            RedrawState::WaitingForEstimatedVBlankAndQueued(_) => unreachable!(),
        }

        output_state.frame_callback_sequence = output_state.frame_callback_sequence.wrapping_add(1);

        // FIXME: this should wait until a frame callback from the host compositor, but it redraws
        // right away instead.
        if output_state.unfinished_animations_remain {
            self.backend.window().request_redraw();
        }

        rv
    }

    pub fn toggle_debug_tint(&mut self) {
        let renderer = self.backend.renderer();
        renderer.set_debug_flags(renderer.debug_flags() ^ DebugFlags::TINT);
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        match self.backend.renderer().import_dmabuf(dmabuf, None) {
            Ok(_texture) => true,
            Err(err) => {
                debug!("error importing dmabuf: {err:?}");
                false
            }
        }
    }

    pub fn ipc_outputs(&self) -> Arc<Mutex<IpcOutputMap>> {
        self.ipc_outputs.clone()
    }
}
