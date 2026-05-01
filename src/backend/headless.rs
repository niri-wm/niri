//! Headless backend for virtual outputs and testing.
//!
//! This backend can be used for:
//! - Running niri without physical displays (for VNC, screensharing, etc.)
//! - Testing purposes
//!
//! Note: This is missing some parts like dmabufs.

use std::collections::HashMap;
use std::mem;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context as _;
use niri_config::OutputName;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::backend::egl::native::EGLSurfacelessDisplay;
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::element::RenderElementStates;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::utils::Size;
use smithay::wayland::presentation::Refresh;

use super::{IpcOutputMap, OutputId, RenderResult};
use crate::niri::{Niri, RedrawState};
use crate::render_helpers::{resources, shaders};
use crate::utils::{get_monotonic_time, logical_output};

pub struct Headless {
    renderer: Option<GlesRenderer>,
    ipc_outputs: Arc<Mutex<IpcOutputMap>>,
    /// Counter for auto-naming headless outputs (HEADLESS-1, HEADLESS-2, etc.)
    output_counter: u32,
    /// Track outputs by name for removal, storing (Output, OutputId)
    outputs: HashMap<String, (Output, OutputId)>,
}

impl Headless {
    pub fn new() -> Self {
        Self {
            renderer: None,
            ipc_outputs: Default::default(),
            output_counter: 0,
            outputs: HashMap::new(),
        }
    }

    pub fn init(&mut self, niri: &mut Niri) {
        // Create a default output on startup
        self.create_virtual_output(niri, 1920, 1080, 60);
    }

    pub fn add_renderer(&mut self) -> anyhow::Result<()> {
        if self.renderer.is_some() {
            error!("add_renderer: renderer must not already exist");
            return Ok(());
        }

        let mut renderer = unsafe {
            let display =
                EGLDisplay::new(EGLSurfacelessDisplay).context("error creating EGL display")?;
            let context = EGLContext::new(&display).context("error creating EGL context")?;
            GlesRenderer::new(context).context("error creating renderer")?
        };

        resources::init(&mut renderer);
        shaders::init(&mut renderer);

        self.renderer = Some(renderer);
        Ok(())
    }

    /// Add an output for testing (uses lowercase naming like `headless-1`).
    /// This is kept for backwards compatibility with tests.
    pub fn add_output(&mut self, niri: &mut Niri, n: u8, size: (u16, u16)) {
        let connector = format!("headless-{n}");
        let make = "niri".to_string();
        let model = "headless".to_string();
        let serial = n.to_string();

        let output = Output::new(
            connector.clone(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: make.clone(),
                model: model.clone(),
                serial_number: serial.clone(),
            },
        );

        let mode = Mode {
            size: Size::from((i32::from(size.0), i32::from(size.1))),
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), None, None, None);
        output.set_preferred(mode);

        output.user_data().insert_if_missing(|| OutputName {
            connector,
            make: Some(make),
            model: Some(model),
            serial: Some(serial),
        });

        let physical_properties = output.physical_properties();
        self.ipc_outputs.lock().unwrap().insert(
            OutputId::next(),
            niri_ipc::Output {
                name: output.name(),
                make: physical_properties.make,
                model: physical_properties.model,
                serial: None,
                physical_size: None,
                modes: vec![niri_ipc::Mode {
                    width: size.0,
                    height: size.1,
                    refresh_rate: 60_000,
                    is_preferred: true,
                }],
                current_mode: Some(0),
                is_custom_mode: true,
                vrr_supported: false,
                vrr_enabled: false,
                logical: Some(logical_output(&output)),
            },
        );

        niri.add_output(output, None, false);
    }

    /// Create a virtual headless output with the given dimensions.
    /// Returns the name of the created output (e.g., "HEADLESS-1").
    pub fn create_virtual_output(
        &mut self,
        niri: &mut Niri,
        width: u16,
        height: u16,
        refresh_rate: u32,
    ) -> String {
        self.output_counter += 1;
        let n = self.output_counter;

        let connector = format!("HEADLESS-{n}");
        let make = "niri".to_string();
        let model = "virtual".to_string();
        let serial = n.to_string();

        let refresh = i32::try_from(refresh_rate * 1000).unwrap_or(60_000);

        let output = Output::new(
            connector.clone(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: make.clone(),
                model: model.clone(),
                serial_number: serial.clone(),
            },
        );

        let mode = Mode {
            size: Size::from((i32::from(width), i32::from(height))),
            refresh,
        };
        output.change_current_state(Some(mode), None, None, None);
        output.set_preferred(mode);

        output.user_data().insert_if_missing(|| OutputName {
            connector: connector.clone(),
            make: Some(make),
            model: Some(model),
            serial: Some(serial),
        });

        let output_id = OutputId::next();
        let physical_properties = output.physical_properties();
        self.ipc_outputs.lock().unwrap().insert(
            output_id,
            niri_ipc::Output {
                name: output.name(),
                make: physical_properties.make,
                model: physical_properties.model,
                serial: None,
                physical_size: None,
                modes: vec![niri_ipc::Mode {
                    width,
                    height,
                    refresh_rate: refresh_rate * 1000,
                    is_preferred: true,
                }],
                current_mode: Some(0),
                is_custom_mode: true,
                vrr_supported: false,
                vrr_enabled: false,
                logical: Some(logical_output(&output)),
            },
        );

        // Track the output for potential removal
        self.outputs
            .insert(connector.clone(), (output.clone(), output_id));

        let refresh_interval = Duration::from_nanos(1_000_000_000 / u64::from(refresh_rate));
        niri.add_output(output, Some(refresh_interval), false);

        connector
    }

    /// Remove a virtual headless output by name.
    /// Returns Ok(()) if successful, Err with message if not found or failed.
    pub fn remove_virtual_output(&mut self, niri: &mut Niri, name: &str) -> Result<(), String> {
        let (output, output_id) = self
            .outputs
            .remove(name)
            .ok_or_else(|| format!("output '{}' not found", name))?;

        // Remove from IPC outputs
        self.ipc_outputs.lock().unwrap().remove(&output_id);

        // Remove from niri
        niri.remove_output(&output);

        Ok(())
    }

    pub fn seat_name(&self) -> String {
        "headless".to_owned()
    }

    pub fn with_primary_renderer<T>(
        &mut self,
        f: impl FnOnce(&mut GlesRenderer) -> T,
    ) -> Option<T> {
        self.renderer.as_mut().map(f)
    }

    pub fn render(&mut self, niri: &mut Niri, output: &Output) -> RenderResult {
        let now = get_monotonic_time();

        let states = RenderElementStates::default();
        let mut presentation_feedbacks = niri.take_presentation_feedbacks(output, &states);
        presentation_feedbacks.presented::<_, smithay::utils::Monotonic>(
            now,
            Refresh::Unknown,
            0,
            wp_presentation_feedback::Kind::empty(),
        );

        let output_state = niri.output_state.get_mut(output).unwrap();
        match mem::replace(&mut output_state.redraw_state, RedrawState::Idle) {
            RedrawState::Idle => unreachable!(),
            RedrawState::Queued => (),
            RedrawState::WaitingForVBlank { .. } => unreachable!(),
            RedrawState::WaitingForEstimatedVBlank(token)
            | RedrawState::WaitingForEstimatedVBlankAndQueued(token) => {
                niri.event_loop.remove(token);
            }
        }

        // Update the frame clock so animation timing works correctly.
        output_state.frame_clock.presented(now);
        output_state.frame_callback_sequence = output_state.frame_callback_sequence.wrapping_add(1);

        // Use a timer to pace redraws, simulating vblank for headless outputs.
        let refresh_interval = output_state
            .frame_clock
            .refresh_interval()
            .unwrap_or(Duration::from_micros(16_667));

        let output_clone = output.clone();
        let timer = Timer::from_duration(refresh_interval);
        let token = niri
            .event_loop
            .insert_source(timer, move |_, _, data| {
                let output_state = data.niri.output_state.get_mut(&output_clone).unwrap();
                output_state.frame_callback_sequence =
                    output_state.frame_callback_sequence.wrapping_add(1);

                match mem::replace(&mut output_state.redraw_state, RedrawState::Idle) {
                    RedrawState::WaitingForEstimatedVBlank(_) => (),
                    RedrawState::WaitingForEstimatedVBlankAndQueued(_) => {
                        output_state.redraw_state = RedrawState::Queued;
                        return TimeoutAction::Drop;
                    }
                    _ => unreachable!(),
                }

                if output_state.unfinished_animations_remain {
                    data.niri.queue_redraw(&output_clone);
                } else {
                    data.niri.send_frame_callbacks_for_virtual_output(&output_clone);
                }
                TimeoutAction::Drop
            })
            .unwrap();
        output_state.redraw_state = RedrawState::WaitingForEstimatedVBlank(token);

        RenderResult::Submitted
    }

    pub fn import_dmabuf(&mut self, _dmabuf: &Dmabuf) -> bool {
        unimplemented!()
    }

    pub fn ipc_outputs(&self) -> Arc<Mutex<IpcOutputMap>> {
        self.ipc_outputs.clone()
    }
}

impl Default for Headless {
    fn default() -> Self {
        Self::new()
    }
}
