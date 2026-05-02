//! Note: This backend has limited DMA-BUF support intended for screencopy.

use std::collections::HashMap;
use std::mem;
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context as _;
use niri_config::OutputName;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::format::FormatSet;
#[cfg(feature = "xdp-gnome-screencast")]
use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::allocator::Buffer;
#[cfg(feature = "xdp-gnome-screencast")]
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::drm::DrmNode;
use smithay::backend::egl::native::EGLSurfacelessDisplay;
use smithay::backend::egl::{EGLContext, EGLDevice, EGLDisplay};
use smithay::backend::libinput::LibinputInputBackend;
use smithay::backend::renderer::element::RenderElementStates;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{ImportDma, ImportEgl};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::input;
use smithay::reexports::input::Libinput;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
#[cfg(feature = "xdp-gnome-screencast")]
use smithay::utils::DeviceFd;
use smithay::utils::Size;
use smithay::wayland::dmabuf::{DmabufFeedbackBuilder, DmabufGlobal};
use smithay::wayland::presentation::Refresh;

use super::{virtual_output, IpcOutputMap, OutputId, RenderResult, VirtualOutputMarker};
use crate::niri::{Niri, RedrawState, State};
use crate::render_helpers::{resources, shaders};
use crate::utils::{get_monotonic_time, logical_output};

pub struct Headless {
    renderer: Option<GlesRenderer>,
    dmabuf_global: Option<DmabufGlobal>,
    /// DRM render node used by the EGL device backing the renderer (if detectable).
    ///
    /// This is used to provide linux-dmabuf feedback so clients can allocate buffers on the
    /// correct device/modifier set (important for dmabuf-based screencopy clients like Sunshine).
    render_node: Option<DrmNode>,
    /// GBM device backed by the detected DRM render node.
    ///
    /// This is required for PipeWire/portal screencasting (e.g. Discord/OBS PipeWire sources).
    #[cfg(feature = "xdp-gnome-screencast")]
    gbm: Option<GbmDevice<DrmDeviceFd>>,
    ipc_outputs: Arc<Mutex<IpcOutputMap>>,
    /// Seat name used for both libinput udev enumeration (`udev_assign_seat`) and the compositor
    /// `wl_seat` name.
    ///
    /// This defaults to `seat0` and can be overridden with `XDG_SEAT`.
    udev_seat: String,
    /// Counter for auto-naming headless outputs (HEADLESS-1, HEADLESS-2, etc.)
    output_counter: u32,
    /// Track outputs by name for removal, storing (Output, OutputId)
    outputs: HashMap<String, (Output, OutputId)>,
}

impl Headless {
    pub fn new(event_loop: LoopHandle<'static, State>) -> Self {
        let udev_seat = std::env::var("XDG_SEAT").unwrap_or_else(|_| "seat0".to_owned());
        init_headless_libinput(event_loop, &udev_seat);

        Self {
            renderer: None,
            dmabuf_global: None,
            render_node: None,
            #[cfg(feature = "xdp-gnome-screencast")]
            gbm: None,
            ipc_outputs: Default::default(),
            udev_seat,
            output_counter: 0,
            outputs: HashMap::new(),
        }
    }

    #[cfg(feature = "xdp-gnome-screencast")]
    pub fn gbm_device(&self) -> Option<GbmDevice<DrmDeviceFd>> {
        self.gbm.clone()
    }

    pub fn init(&mut self, niri: &mut Niri) {
        if let Err(err) = self.add_renderer() {
            warn!("failed to create headless renderer: {err:?}");
        } else if let Some(renderer) = self.renderer.as_mut() {
            if let Err(err) = renderer.bind_wl_display(&niri.display_handle) {
                warn!("error binding renderer wl_display: {err:?}");
            }

            let config = niri.config.borrow();
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

            // Advertise linux-dmabuf feedback when we can identify the EGL device's DRM render
            // node. This helps dmabuf-based screencopy clients (e.g. Sunshine) pick compatible
            // formats/modifiers. If we can't determine a node, fall back to a format-list-only
            // dmabuf global.
            if self.dmabuf_global.is_none() {
                // For screencopy, the client-provided dmabuf must be usable as a render target,
                // so only advertise formats that are both renderable by the EGL context and
                // importable by the renderer.
                let render_formats = renderer.egl_context().dmabuf_render_formats();
                let import_formats: FormatSet = renderer.dmabuf_formats().into_iter().collect();
                let formats: FormatSet = render_formats
                    .intersection(&import_formats)
                    .copied()
                    .collect();

                if formats.iter().next().is_none() {
                    warn!(
                        "headless: renderer reports no compatible dmabuf render formats; dmabuf screencopy will be unavailable"
                    );
                } else if let Some(render_node) = self.render_node {
                    match DmabufFeedbackBuilder::new(render_node.dev_id(), formats.clone())
                        .build()
                        .context("error building default dmabuf feedback")
                    {
                        Ok(default_feedback) => {
                            let global = niri
                                .dmabuf_state
                                .create_global_with_default_feedback::<State>(
                                    &niri.display_handle,
                                    &default_feedback,
                                );
                            self.dmabuf_global = Some(global);
                        }
                        Err(err) => {
                            warn!(
                                "headless: failed to build dmabuf feedback ({err:?}); falling back to format-list-only dmabuf global"
                            );
                            let formats_vec = formats.into_iter().collect::<Vec<_>>();
                            let global = niri
                                .dmabuf_state
                                .create_global::<State>(&niri.display_handle, formats_vec);
                            self.dmabuf_global = Some(global);
                        }
                    }
                } else {
                    let formats_vec = formats.into_iter().collect::<Vec<_>>();
                    let global = niri
                        .dmabuf_state
                        .create_global::<State>(&niri.display_handle, formats_vec);
                    self.dmabuf_global = Some(global);
                }
            }
        }

        // In real headless sessions we want a default output so clients have something to render
        // on. In tests, the harness explicitly creates predictable `headless-N` outputs; creating
        // an extra default `HEADLESS-1` here causes name collisions and snapshot churn.
        if self.outputs.is_empty() && !cfg!(test) {
            let _ = self.create_virtual_output(niri, 1920, 1080, 60, None);
        }
    }

    pub fn add_renderer(&mut self) -> anyhow::Result<()> {
        if self.renderer.is_some() {
            error!("add_renderer: renderer must not already exist");
            return Ok(());
        }

        let mut renderer = unsafe {
            let display =
                EGLDisplay::new(EGLSurfacelessDisplay).context("error creating EGL display")?;

            self.render_node = EGLDevice::device_for_display(&display)
                .ok()
                .and_then(|egl_device| {
                    if egl_device.is_software() {
                        debug!(
                            "headless: EGL device is software; skipping dmabuf feedback device metadata"
                        );
                        return None;
                    }

                    egl_device.try_get_render_node().ok().flatten()
                });

            #[cfg(feature = "xdp-gnome-screencast")]
            {
                if self.gbm.is_none() {
                    if let Some(render_node) = self.render_node {
                        match try_init_headless_gbm_device(render_node) {
                            Ok(gbm) => self.gbm = Some(gbm),
                            Err(err) => {
                                warn!(
                                    "headless: failed to initialize GBM device from render node {render_node}: {err:?}"
                                );
                            }
                        }
                    } else {
                        debug!(
                            "headless: no DRM render node detected; portal/PipeWire screencasting will be unavailable"
                        );
                    }
                }
            }

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

        output
            .user_data()
            .insert_if_missing(VirtualOutputMarker::default);

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

    /// Create a virtual output with the given mode and add it to the compositor.
    /// Returns the name of the created output.
    pub fn create_virtual_output(
        &mut self,
        niri: &mut Niri,
        width: u16,
        height: u16,
        refresh_rate: u32,
        name: Option<String>,
    ) -> Result<String, String> {
        let mut width = width;
        let mut height = height;
        let mut refresh_millihz = refresh_rate.saturating_mul(1000);
        let mut should_connect = true;

        if let Some(name) = name.as_deref() {
            let lookup_name = OutputName {
                connector: name.to_string(),
                make: None,
                model: None,
                serial: None,
            };
            let config = niri.config.borrow();
            if let Some(output_config) = config.outputs.find(&lookup_name) {
                should_connect = !output_config.off;

                if let Some(mode_config) = output_config.mode {
                    width = mode_config.mode.width;
                    height = mode_config.mode.height;

                    let refresh_hz = mode_config.mode.refresh.unwrap_or(60.0);
                    refresh_millihz =
                        (refresh_hz * 1000.0).round().clamp(1.0, u32::MAX as f64) as u32;
                }
            }
        }

        if let Some(name) = name.as_deref() {
            if name.trim().is_empty() {
                return Err("virtual output name cannot be empty".to_string());
            }
            if self.outputs.contains_key(name)
                || niri.global_space.outputs().any(|o| o.name() == name)
            {
                return Err(format!("output '{name}' already exists"));
            }
        }

        let built = virtual_output::build_headless_virtual_output(
            &mut self.output_counter,
            width,
            height,
            refresh_millihz,
            name,
        );

        if self.outputs.contains_key(&built.name)
            || niri.global_space.outputs().any(|o| o.name() == built.name)
        {
            return Err(format!("output '{}' already exists", built.name));
        }

        self.ipc_outputs
            .lock()
            .unwrap()
            .insert(built.output_id, built.ipc_output);

        self.outputs
            .insert(built.name.clone(), (built.output.clone(), built.output_id));

        if should_connect {
            niri.add_output(built.output, Some(built.refresh_interval), false);
        }

        // Ensure IPC state matches the (potentially) configured mode and connection state.
        niri.ipc_outputs_changed = true;
        self.on_output_config_changed(niri);

        Ok(built.name)
    }

    /// Remove the virtual output with the given name.
    /// Returns an error if no such virtual output exists.
    pub fn remove_virtual_output(&mut self, niri: &mut Niri, name: &str) -> Result<(), String> {
        virtual_output::remove_virtual_output_from_map(
            niri,
            &self.ipc_outputs,
            &mut self.outputs,
            name,
            "output",
        )
    }

    pub fn seat_name(&self) -> String {
        self.udev_seat.clone()
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

        output_state.frame_clock.presented(now);
        output_state.frame_callback_sequence = output_state.frame_callback_sequence.wrapping_add(1);

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
                    data.niri
                        .send_frame_callbacks_for_virtual_output(&output_clone);
                }
                TimeoutAction::Drop
            })
            .unwrap();
        output_state.redraw_state = RedrawState::WaitingForEstimatedVBlank(token);

        RenderResult::Submitted
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        let renderer = match self.renderer.as_mut() {
            Some(r) => r,
            None => {
                debug!("import_dmabuf: no renderer available");
                return false;
            }
        };

        let render_formats = renderer.egl_context().dmabuf_render_formats();
        let import_formats: FormatSet = renderer.dmabuf_formats().into_iter().collect();
        let format = dmabuf.format();
        if !render_formats.contains(&format) || !import_formats.contains(&format) {
            debug!(
                "import_dmabuf: unsupported format code={:?} modifier={:?}",
                format.code, format.modifier
            );
            return false;
        }

        match renderer.import_dmabuf(dmabuf, None) {
            Ok(_texture) => {
                dmabuf.set_node(self.render_node);
                true
            }
            Err(err) => {
                debug!("error importing dmabuf: {err:?}");
                false
            }
        }
    }

    pub fn on_output_config_changed(&mut self, niri: &mut Niri) {
        let config = niri.config.clone();
        virtual_output::apply_config_to_managed_virtual_outputs(
            niri,
            &mut self.outputs,
            Some(&self.ipc_outputs),
            &config,
        );
    }

    pub fn ipc_outputs(&self) -> Arc<Mutex<IpcOutputMap>> {
        self.ipc_outputs.clone()
    }
}

impl Default for Headless {
    fn default() -> Self {
        // This is only used by tests / callers that don't care about input.
        // Headless libinput requires an event loop handle, so default disables it.
        Self {
            renderer: None,
            dmabuf_global: None,
            render_node: None,
            #[cfg(feature = "xdp-gnome-screencast")]
            gbm: None,
            ipc_outputs: Default::default(),
            udev_seat: "seat0".to_string(),
            output_counter: 0,
            outputs: HashMap::new(),
        }
    }
}

#[cfg(feature = "xdp-gnome-screencast")]
fn try_init_headless_gbm_device(render_node: DrmNode) -> anyhow::Result<GbmDevice<DrmDeviceFd>> {
    use std::fs::OpenOptions;
    use std::os::fd::OwnedFd;
    use std::os::unix::fs::OpenOptionsExt;

    let path = render_node
        .dev_path()
        .context("render node has no dev_path")?;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOCTTY)
        .open(&path)
        .with_context(|| format!("error opening render node at {path:?}"))?;

    let owned_fd = OwnedFd::from(file);
    let device_fd = DrmDeviceFd::new(DeviceFd::from(owned_fd));
    let gbm = GbmDevice::new(device_fd).context("error creating GBM device")?;
    Ok(gbm)
}

#[derive(Clone, Default)]
struct HeadlessLibinputInterface;

impl input::LibinputInterface for HeadlessLibinputInterface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| libc::EINVAL)?;
        // Keep libinput's requested access mode (read-only vs read-write), but add a few
        // safety/behavior flags (mirrors libseat's noop backend).
        let flags = flags | libc::O_CLOEXEC | libc::O_NOCTTY | libc::O_NOFOLLOW | libc::O_NONBLOCK;
        let fd = unsafe { libc::open(c_path.as_ptr(), flags) };
        if fd < 0 {
            let errno = unsafe { *libc::__errno_location() };

            if errno == libc::ENOENT || errno == libc::ENODEV {
                trace!("headless: libinput open_restricted failed for {path:?}: errno={errno}");
            } else {
                debug!("headless: libinput open_restricted failed for {path:?}: errno={errno}");
            }
            return Err(errno);
        }

        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(fd);
    }
}

fn init_headless_libinput(event_loop: LoopHandle<'static, State>, seat: &str) {
    let mut libinput = Libinput::new_with_udev(HeadlessLibinputInterface::default());

    unsafe { super::libinput_plugins::init_libinput_plugin_system(&libinput) };

    if libinput.udev_assign_seat(seat).is_err() {
        debug!("headless: failed to assign libinput seat {seat:?}; input will be unavailable");
        return;
    }

    let input_backend = LibinputInputBackend::new(libinput);
    if event_loop
        .insert_source(input_backend, |mut event, _, state| {
            state.process_libinput_event(&mut event);
            state.process_input_event(event);
        })
        .is_err()
    {
        debug!("headless: failed to insert libinput backend; input will be unavailable");
        return;
    }
}
