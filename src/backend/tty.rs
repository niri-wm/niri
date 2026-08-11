use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::iter::zip;
use std::num::NonZeroU64;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use std::{io, mem};

use anyhow::{anyhow, bail, ensure, Context};
use bytemuck::cast_slice_mut;
use drm_ffi::drm_mode_modeinfo;
use libc::dev_t;
use niri_config::output::{MaxBpc, Modeline};
use niri_config::{Config, OutputName};
use niri_ipc::{HSyncPolarity, VSyncPolarity};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags, PrimaryPlaneElement};
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::{
    DrmDevice, DrmDeviceFd, DrmEvent, DrmEventMetadata, DrmEventTime, DrmNode, NodeType, VrrSupport,
};
use smithay::backend::egl::context::ContextPriority;
use smithay::backend::egl::{EGLDevice, EGLDisplay};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::multigpu::gbm::GbmGlesBackend;
use smithay::backend::renderer::multigpu::{GpuManager, MultiFrame, MultiRenderer};
use smithay::backend::renderer::{DebugFlags, ImportDma, ImportEgl, RendererSuper};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{self, UdevBackend, UdevEvent};
use smithay::desktop::utils::OutputPresentationFeedback;
use smithay::output::{Mode, Output, OutputModeSource, PhysicalProperties};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{Dispatcher, LoopHandle, RegistrationToken};
use smithay::reexports::drm::control::atomic::AtomicModeReq;
use smithay::reexports::drm::control::dumbbuffer::DumbBuffer;
use smithay::reexports::drm::control::{
    self, connector, crtc, plane, property, AtomicCommitFlags, Device, Mode as DrmMode, ModeFlags,
    ModeTypeFlags, PlaneType, ResourceHandle,
};
use smithay::reexports::gbm::Modifier;
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_protocols;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{DeviceFd, Transform};
use smithay::wayland::dmabuf::{DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal};
use smithay::wayland::drm_lease::{
    DrmLease, DrmLeaseBuilder, DrmLeaseRequest, DrmLeaseState, LeaseRejected,
};
use smithay::wayland::presentation::Refresh;
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};
use wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1::TrancheFlags;
use wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;

use super::{IpcOutputMap, RenderResult};
use crate::backend::OutputId;
use crate::frame_clock::FrameClock;
use crate::niri::{Niri, RedrawState, State};
use crate::render_helpers::debug::draw_damage;
use crate::render_helpers::renderer::AsGlesRenderer;
use crate::render_helpers::{resources, shaders, RenderCtx, RenderTarget};
use crate::utils::{get_monotonic_time, is_laptop_panel, logical_output, PanelOrientation};

// specific 10-bit formats for multigpu setup,
// i.e. copying from rendering Nvidia dGPU to target iGPU.
const SUPPORTED_COLOR_FORMATS: [Fourcc; 6] = [
    Fourcc::Xbgr2101010,
    Fourcc::Abgr2101010,
    Fourcc::Xrgb8888,
    Fourcc::Xbgr8888,
    Fourcc::Argb8888,
    Fourcc::Abgr8888,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemPmState {
    Running,
    Quiescing,
    Suspended,
    ResumePending,
}

struct GpuGateState {
    open_allowed: bool,
    active_leases: usize,
    generation: u64,
}

struct GpuGateShared {
    state: Mutex<GpuGateState>,
    cvar: Condvar,
}

#[derive(Clone)]
pub struct GpuGate {
    shared: Arc<GpuGateShared>,
}

pub struct GpuLease {
    shared: Arc<GpuGateShared>,
}

impl Drop for GpuLease {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());

        state.active_leases = state.active_leases.saturating_sub(1);
        if state.active_leases == 0 {
            self.shared.cvar.notify_all();
        }
    }
}

impl Default for GpuGate {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuGate {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(GpuGateShared {
                state: Mutex::new(GpuGateState {
                    open_allowed: true,
                    active_leases: 0,
                    generation: 0,
                }),
                cvar: Condvar::new(),
            }),
        }
    }

    pub fn acquire_lease(&self, expected_generation: u64) -> Option<GpuLease> {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());

        if !state.open_allowed || state.generation != expected_generation {
            return None;
        }

        state.active_leases = state.active_leases.saturating_add(1);

        Some(GpuLease {
            shared: self.shared.clone(),
        })
    }

    pub fn set_open_allowed(&self, allowed: bool, generation: u64) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());

        state.open_allowed = allowed;
        state.generation = generation;
    }

    pub fn active_lease_count(&self) -> usize {
        let state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active_leases
    }

    pub fn wait_for_drain(&self, timeout: Duration) -> bool {
        let state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());

        let (state, _) = self
            .shared
            .cvar
            .wait_timeout_while(state, timeout, |state| state.active_leases != 0)
            .unwrap_or_else(|e| e.into_inner());

        state.active_leases == 0
    }
}

fn is_pci_device_ready(node: DrmNode) -> bool {
    let Some(path) = node.dev_path() else {
        return true;
    };
    let Some(filename) = path.file_name().and_then(|s| s.to_str()) else {
        return true;
    };
    let sysfs_path = format!("/sys/class/drm/{filename}/device/power/runtime_status");
    if let Ok(content) = std::fs::read_to_string(&sysfs_path) {
        let status = content.trim();
        if status == "suspended" || status == "suspending" || status == "resuming" {
            return false;
        }
    }
    true
}

#[derive(Debug)]
enum WakeupState {
    Pending,
    Woken {
        // Held open so the kernel doesn't autosuspend the GPU mid-init.
        #[allow(dead_code)]
        file: std::fs::File,
        generation: u64,
    },
    WokenNoFd {
        generation: u64,
    },
    Failed,
    Cancelled,
}

struct PendingDeviceInit {
    attempts: usize,
    generation: u64,
    timer_token: Option<RegistrationToken>,
    wakeup_state: Arc<Mutex<WakeupState>>,
}

struct GpuPowerState {
    suspended: bool,
    waking_up: bool,
    wakeup_failed: bool,
    resume_attempts: usize,
    wakeup_generation: u64,
    wakeup_state: Arc<Mutex<WakeupState>>,
    suspend_timer: Option<RegistrationToken>,
    wakeup_timer: Option<RegistrationToken>,
    disable_suspend_due_to_nvidia_vram: bool,
}

impl GpuPowerState {
    fn new(disable_suspend_due_to_nvidia_vram: bool) -> Self {
        Self {
            suspended: false,
            waking_up: false,
            wakeup_failed: false,
            resume_attempts: 0,
            wakeup_generation: 0,
            wakeup_state: Arc::new(Mutex::new(WakeupState::Pending)),
            suspend_timer: None,
            wakeup_timer: None,
            disable_suspend_due_to_nvidia_vram,
        }
    }
}

pub struct Tty {
    config: Rc<RefCell<Config>>,
    session: LibSeatSession,
    udev_dispatcher: Dispatcher<'static, UdevBackend, State>,
    libinput: Libinput,
    gpu_manager: GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>,
    // DRM node corresponding to the primary GPU. May or may not be the same as
    // primary_render_node.
    primary_node: DrmNode,
    // DRM render node corresponding to the primary GPU.
    primary_render_node: DrmNode,
    // Ignored DRM nodes.
    ignored_nodes: HashSet<DrmNode>,
    // Devices indexed by DRM node (not necessarily the render node).
    devices: HashMap<DrmNode, OutputDevice>,
    // PCIe wakeup can take seconds; poll from a thread instead of blocking the event loop.
    pending_device_inits: HashMap<DrmNode, PendingDeviceInit>,
    next_pending_init_generation: u64,
    // Track suspend state across udev churn.
    gpu_power: HashMap<DrmNode, GpuPowerState>,
    system_pm: SystemPmState,
    gpu_gate: GpuGate,
    pm_generation: u64,
    inhibitor_fd: Option<OwnedFd>,
    // The dma-buf global corresponds to the output device (the primary GPU). It is only `Some()`
    // if we have a device corresponding to the primary GPU.
    dmabuf_global: Option<DmabufGlobal>,
    // The output config had changed, but the session is paused, so we need to update it on resume.
    update_output_config_on_resume: bool,
    // Whether the debug tinting is enabled.
    debug_tint: bool,
    ipc_outputs: Arc<Mutex<IpcOutputMap>>,
}

pub type TtyRenderer<'render> = MultiRenderer<
    'render,
    'render,
    GbmGlesBackend<GlesRenderer, DrmDeviceFd>,
    GbmGlesBackend<GlesRenderer, DrmDeviceFd>,
>;

pub type TtyFrame<'render, 'frame, 'buffer> = MultiFrame<
    'render,
    'render,
    'frame,
    'buffer,
    GbmGlesBackend<GlesRenderer, DrmDeviceFd>,
    GbmGlesBackend<GlesRenderer, DrmDeviceFd>,
>;

pub type TtyRendererError<'render> = <TtyRenderer<'render> as RendererSuper>::Error;

type GbmDrmCompositor = DrmCompositor<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    (OutputPresentationFeedback, Duration),
    DrmDeviceFd,
>;

pub struct OutputDevice {
    token: RegistrationToken,
    // Can be None for display-only devices such as DisplayLink.
    render_node: Option<DrmNode>,
    drm_scanner: DrmScanner,
    surfaces: HashMap<crtc::Handle, Surface>,
    known_crtcs: HashMap<crtc::Handle, CrtcInfo>,
    // SAFETY: drop after all the objects used with them are dropped.
    // See https://github.com/Smithay/smithay/issues/1102.
    drm: DrmDevice,
    gbm: GbmDevice<DrmDeviceFd>,
    // For display-only devices this will be the allocator from the primary device.
    allocator: GbmAllocator<DrmDeviceFd>,

    pub drm_lease_state: Option<DrmLeaseState>,
    non_desktop_connectors: HashSet<(connector::Handle, crtc::Handle)>,
    active_leases: Vec<DrmLease>,
}

// A connected, but not necessarily enabled, crtc.
#[derive(Debug, Clone)]
pub struct CrtcInfo {
    id: OutputId,
    name: OutputName,
}

impl OutputDevice {
    pub fn lease_request(
        &self,
        request: DrmLeaseRequest,
    ) -> Result<DrmLeaseBuilder, LeaseRejected> {
        let mut builder = DrmLeaseBuilder::new(&self.drm);
        for connector in request.connectors {
            let (_, crtc) = self
                .non_desktop_connectors
                .iter()
                .find(|(conn, _)| connector == *conn)
                .ok_or_else(|| {
                    warn!("Attempted to lease connector that is not non-desktop");
                    LeaseRejected::default()
                })?;
            builder.add_connector(connector);
            builder.add_crtc(*crtc);
            let planes = self.drm.planes(crtc).map_err(LeaseRejected::with_cause)?;
            let (primary_plane, primary_plane_claim) = planes
                .primary
                .iter()
                .find_map(|plane| {
                    self.drm
                        .claim_plane(plane.handle, *crtc)
                        .map(|claim| (plane, claim))
                })
                .ok_or_else(LeaseRejected::default)?;
            builder.add_plane(primary_plane.handle, primary_plane_claim);
        }
        Ok(builder)
    }

    pub fn new_lease(&mut self, lease: DrmLease) {
        self.active_leases.push(lease);
    }

    pub fn remove_lease(&mut self, lease_id: u32) {
        self.active_leases.retain(|l| l.id() != lease_id);
    }

    pub fn known_crtc_name(
        &self,
        crtc: &crtc::Handle,
        conn: &connector::Info,
        disable_monitor_names: bool,
    ) -> OutputName {
        if disable_monitor_names {
            let conn_name = format_connector_name(conn);
            return OutputName {
                connector: conn_name,
                make: None,
                model: None,
                serial: None,
            };
        }

        let Some(info) = self.known_crtcs.get(crtc) else {
            let conn_name = format_connector_name(conn);
            error!("crtc for connector {conn_name} missing from known");
            return OutputName {
                connector: conn_name,
                make: None,
                model: None,
                serial: None,
            };
        };
        info.name.clone()
    }

    fn cleanup_mismatching_resources(
        &self,
        should_be_off: &dyn Fn(crtc::Handle, &connector::Info) -> bool,
    ) -> anyhow::Result<()> {
        let _span = tracy_client::span!("OutputDevice::cleanup_disconnected_resources");

        let res_handles = self
            .drm
            .resource_handles()
            .context("error getting plane handles")?;
        let plane_handles = self
            .drm
            .plane_handles()
            .context("error getting plane handles")?;

        let mut req = AtomicModeReq::new();

        // We want to disable all CRTCs that do not correspond to a connector we're using.
        let mut cleanup = HashSet::<crtc::Handle>::new();
        cleanup.extend(res_handles.crtcs());

        for (conn, info) in self.drm_scanner.connectors() {
            // We only keep the connector if it has a CRTC and the output isn't off in niri.
            if let Some(crtc) = self.drm_scanner.crtc_for_connector(conn) {
                // Verify that the connector's current CRTC matches the CRTC we expect. If not,
                // clear the CRTC and the connector so that all connectors can get the expected
                // CRTCs afterwards. (We do this because we do not handle CRTC rotations across TTY
                // switches.)
                let mut has_different_crtc = false;
                if let Some(enc) = info.current_encoder() {
                    match self.drm.get_encoder(enc) {
                        Ok(enc) => {
                            if let Some(current_crtc) = enc.crtc() {
                                if current_crtc != crtc {
                                    has_different_crtc = true;
                                }
                            }
                        }
                        Err(err) => {
                            debug!("couldn't get encoder: {err:?}");
                            // Err on the safe side.
                            has_different_crtc = true;
                        }
                    }
                }

                if !has_different_crtc && !should_be_off(crtc, info) {
                    // Keep the corresponding CRTC.
                    cleanup.remove(&crtc);
                    continue;
                }
            }

            // Clear the connector.
            let Some((crtc_id, _, _)) = find_drm_property(&self.drm, *conn, "CRTC_ID") else {
                debug!("couldn't find connector CRTC_ID property");
                continue;
            };

            req.add_property(*conn, crtc_id, property::Value::CRTC(None));
        }

        // Legacy fallback.
        if !self.drm.is_atomic() {
            for crtc in res_handles.crtcs() {
                #[allow(deprecated)]
                let _ = self.drm.set_cursor(*crtc, Option::<&DumbBuffer>::None);
            }
            for crtc in cleanup {
                let _ = self.drm.set_crtc(crtc, None, (0, 0), &[], None);
            }
            return Ok(());
        }

        // Disable non-primary planes, and planes belonging to disabled CRTCs.
        let is_primary = |plane: plane::Handle| {
            if let Some((_, info, value)) = find_drm_property(&self.drm, plane, "type") {
                match info.value_type().convert_value(value) {
                    property::Value::Enum(Some(val)) => val.value() == PlaneType::Primary as u64,
                    _ => false,
                }
            } else {
                debug!("couldn't find plane type property");
                false
            }
        };

        for plane in plane_handles {
            let info = match self.drm.get_plane(plane) {
                Ok(x) => x,
                Err(err) => {
                    debug!("error getting plane: {err:?}");
                    continue;
                }
            };

            let Some(crtc) = info.crtc() else {
                continue;
            };

            if !cleanup.contains(&crtc) && is_primary(plane) {
                continue;
            }

            let Some((crtc_id, _, _)) = find_drm_property(&self.drm, plane, "CRTC_ID") else {
                debug!("couldn't find plane CRTC_ID property");
                continue;
            };

            let Some((fb_id, _, _)) = find_drm_property(&self.drm, plane, "FB_ID") else {
                debug!("couldn't find plane FB_ID property");
                continue;
            };

            req.add_property(plane, crtc_id, property::Value::CRTC(None));
            req.add_property(plane, fb_id, property::Value::Framebuffer(None));
        }

        // Disable the CRTCs.
        for crtc in cleanup {
            let Some((mode_id, _, _)) = find_drm_property(&self.drm, crtc, "MODE_ID") else {
                debug!("couldn't find CRTC MODE_ID property");
                continue;
            };

            let Some((active, _, _)) = find_drm_property(&self.drm, crtc, "ACTIVE") else {
                debug!("couldn't find CRTC ACTIVE property");
                continue;
            };

            req.add_property(crtc, mode_id, property::Value::Unknown(0));
            req.add_property(crtc, active, property::Value::Boolean(false));
        }

        self.drm
            .atomic_commit(AtomicCommitFlags::ALLOW_MODESET, req)
            .context("error doing atomic commit")?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct TtyOutputState {
    node: DrmNode,
    crtc: crtc::Handle,
}

struct Surface {
    name: OutputName,
    compositor: GbmDrmCompositor,
    connector: connector::Handle,
    dmabuf_feedback: Option<SurfaceDmabufFeedback>,
    gamma_props: Option<GammaProps>,
    /// Gamma change to apply upon session resume.
    pending_gamma_change: Option<Option<Vec<u16>>>,
    /// Tracy frame that goes from vblank to vblank.
    vblank_frame: Option<tracy_client::Frame>,
    /// Frame name for the VBlank frame.
    vblank_frame_name: tracy_client::FrameName,
    /// Plot name for the time since presentation plot.
    time_since_presentation_plot_name: tracy_client::PlotName,
    /// Plot name for the presentation misprediction plot.
    presentation_misprediction_plot_name: tracy_client::PlotName,
    sequence_delta_plot_name: tracy_client::PlotName,
}

pub struct SurfaceDmabufFeedback {
    pub render: DmabufFeedback,
    pub scanout: DmabufFeedback,
}

struct GammaProps {
    crtc: crtc::Handle,
    gamma_lut: property::Handle,
    gamma_lut_size: property::Handle,
    previous_blob: Option<NonZeroU64>,
}

struct ConnectorProperties<'a> {
    device: &'a DrmDevice,
    connector: connector::Handle,
    properties: Vec<(property::Info, property::RawValue)>,
    has_change: bool,
    requests: AtomicModeReq,
}

// Check if a secondary GPU is completely idle and has no active display attachments.
//
// NOTE on render offload (PRIME):
// Applications doing direct render offload (e.g., via DRI3/Vulkan) open the secondary
// GPU's render node directly. This access automatically wakes up the hardware at the
// kernel driver level, independent of the compositor.
// The compositor imports the resulting dmabufs on the primary GPU's renderer. Thus,
// removing the secondary render node from GpuManager does not break offloaded clients,
// as the compositor does not need the secondary renderer registered for presentation.
fn is_device_idle_and_last(tty: &Tty, node: DrmNode) -> bool {
    let Some(device) = tty.devices.get(&node) else {
        return false;
    };
    let Some(render_node) = device.render_node else {
        return false;
    };
    let key = tty.gpu_power_key(node);
    let Some(power) = tty.gpu_power.get(&key) else {
        return false;
    };

    if tty.config.borrow().debug.disable_idle_drm_device_suspend
        || power.disable_suspend_due_to_nvidia_vram
        || !device.surfaces.is_empty()
        || !device.active_leases.is_empty()
    {
        return false;
    }

    !tty.devices.iter().any(|(other_node, other)| {
        let other_is_suspended = tty
            .gpu_power
            .get(&tty.gpu_power_key(*other_node))
            .map(|power| power.suspended)
            .unwrap_or(false);

        *other_node != node
            && other.render_node == Some(render_node)
            && !other_is_suspended
            && (!other.surfaces.is_empty() || !other.active_leases.is_empty())
    })
}

// Checks if VRAM allocations preservation is enabled for NVIDIA GPUs.
// Returns Some(true) if active, Some(false) if disabled or unreadable, and None if not an NVIDIA
// card.
fn check_nvidia_preserve_memory(node: DrmNode) -> Option<bool> {
    let has_nvidia_driver = std::path::Path::new("/sys/module/nvidia").exists()
        || std::path::Path::new("/proc/driver/nvidia").exists();

    let try_get_driver_name = || -> Option<std::ffi::OsString> {
        let path = node.dev_path()?;
        let name = path.file_name()?;
        let sys_path = format!("/sys/class/drm/{}/device/driver", name.to_string_lossy());
        let driver_path = std::fs::read_link(sys_path).ok()?;
        let driver_name = driver_path.file_name()?;
        Some(driver_name.to_owned())
    };

    match try_get_driver_name() {
        Some(driver_name) => {
            if driver_name != "nvidia" {
                return None;
            }
        }
        None => {
            if has_nvidia_driver {
                return Some(false);
            } else {
                return None;
            }
        }
    }

    if let Ok(content) =
        std::fs::read_to_string("/sys/module/nvidia/parameters/PreserveVideoMemoryAllocations")
    {
        return Some(matches!(content.trim(), "Y" | "1"));
    }

    if let Ok(content) = std::fs::read_to_string("/proc/driver/nvidia/params") {
        let val = content
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.trim() == "PreserveVideoMemoryAllocations")
            .map(|(_, value)| value.trim() == "1")
            .unwrap_or(false);
        return Some(val);
    }

    Some(false)
}

fn return_device_fd(session: &mut LibSeatSession, device_fd: DrmDeviceFd, raw_device_fd: DeviceFd) {
    drop(device_fd);
    match TryInto::<OwnedFd>::try_into(raw_device_fd) {
        Ok(fd) => {
            if let Err(err) = session.close(fd) {
                warn!("error closing DRM device fd: {err:?}");
            }
        }
        Err(_) => warn!("could not close DRM device fd: unexpected Arc references"),
    }
}

fn spawn_gpu_wakeup_thread(
    render_path: std::path::PathBuf,
    expected_dev_id: u64,
    generation: u64,
    pm_generation: u64,
    wakeup_state: Arc<Mutex<WakeupState>>,
    gpu_gate: GpuGate,
) {
    std::thread::spawn(move || {
        for _ in 0..WAKEUP_ATTEMPTS {
            {
                let lock = wakeup_state.lock().unwrap_or_else(|e| e.into_inner());
                if let WakeupState::Cancelled = *lock {
                    return;
                }
            }

            let file_opt = {
                let Some(_lease) = gpu_gate.acquire_lease(pm_generation) else {
                    debug!(
                        "gpu gate is closed or stale generation for {render_path:?}, cancelling wakeup"
                    );
                    let mut lock = wakeup_state.lock().unwrap_or_else(|e| e.into_inner());
                    if let WakeupState::Pending = *lock {
                        *lock = WakeupState::Cancelled;
                    }
                    return;
                };

                std::fs::OpenOptions::new().read(true).open(&render_path)
            };

            if let Ok(file) = file_opt {
                match file.metadata() {
                    Ok(meta) => {
                        if meta.rdev() == expected_dev_id {
                            let mut lock = wakeup_state.lock().unwrap_or_else(|e| e.into_inner());
                            if let WakeupState::Pending = *lock {
                                *lock = WakeupState::Woken { file, generation };
                            }
                            return;
                        } else {
                            let mut lock = wakeup_state.lock().unwrap_or_else(|e| e.into_inner());
                            if let WakeupState::Pending = *lock {
                                *lock = WakeupState::Failed;
                            }
                            return;
                        }
                    }
                    Err(err) => {
                        warn!("failed to read metadata for {render_path:?}: {err:?}");
                    }
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(WAKEUP_DELAY_MS));
        }

        let mut lock = wakeup_state.lock().unwrap_or_else(|e| e.into_inner());
        if let WakeupState::Pending = *lock {
            warn!(
                "failed to open GPU render node at {render_path:?} for wakeup after \
                 {WAKEUP_ATTEMPTS} attempts"
            );
            *lock = WakeupState::Failed;
        }
    });
}

// Avoid power-cycling the GPU during brief connector churn.
pub(crate) const SUSPEND_TIMEOUT_SECS: u64 = 10;
// Slow eGPU links need time before the render node comes back.
pub(crate) const WAKEUP_ATTEMPTS: usize = 40;
pub(crate) const WAKEUP_DELAY_MS: u64 = 250;

fn wakeup_retry_delay() -> std::time::Duration {
    std::time::Duration::from_millis(WAKEUP_DELAY_MS)
}

impl Tty {
    pub(crate) fn update_dmabuf_feedbacks(&mut self, niri: &mut Niri) {
        if let Ok(primary_renderer) = self.gpu_manager.single_renderer(&self.primary_render_node) {
            let primary_formats = primary_renderer.dmabuf_formats();

            if let Some(dmabuf_global) = self.dmabuf_global.as_mut() {
                if let Ok(default_feedback) = DmabufFeedbackBuilder::new(
                    self.primary_render_node.dev_id(),
                    primary_formats.clone(),
                )
                .build()
                {
                    niri.dmabuf_state
                        .set_default_feedback(dmabuf_global, &default_feedback);
                }
            }

            for (node, device) in self.devices.iter_mut() {
                for surface in device.surfaces.values_mut() {
                    match surface_dmabuf_feedback(
                        &surface.compositor,
                        primary_formats.clone(),
                        self.primary_render_node,
                        device.render_node,
                        *node,
                    ) {
                        Ok(feedback) => {
                            surface.dmabuf_feedback = Some(feedback);
                        }
                        Err(err) => warn!("error building dmabuf feedback: {err:?}"),
                    }
                }
            }

            niri.queue_redraw_all();
        }
    }

    fn gpu_power_key(&self, node: DrmNode) -> DrmNode {
        if let Some(device) = self.devices.get(&node) {
            return device.render_node.unwrap_or(node);
        }
        if node.ty() == NodeType::Render {
            node
        } else {
            node.node_with_type(NodeType::Render)
                .and_then(Result::ok)
                .unwrap_or(node)
        }
    }

    fn find_active_primary_for_key(&self, key: DrmNode) -> Option<DrmNode> {
        self.devices.iter().find_map(|(node, device)| {
            if device.render_node == Some(key) || (device.render_node.is_none() && *node == key) {
                Some(*node)
            } else {
                None
            }
        })
    }

    fn sync_gpu_manager_devices(&mut self) {
        // Smithay may keep old Arc refs around until devices() runs.
        // Touch it so stale GPU state gets dropped before fd cleanup.
        // Otherwise closing or reusing the fd can trip on extra refs.
        let _ = self.gpu_manager.devices();
    }

    fn prime_gpu_power_state(&mut self, node: DrmNode, render_node: Option<DrmNode>) {
        let key = render_node.unwrap_or(node);
        if self.gpu_power.contains_key(&key) {
            return;
        }

        let disable_suspend_due_to_nvidia_vram =
            node != self.primary_node && check_nvidia_preserve_memory(node) == Some(false);

        if disable_suspend_due_to_nvidia_vram {
            warn!(
                "NVIDIA GPU {node:?}: PreserveVideoMemoryAllocations off or unreadable, not suspending. \
                 Boot with nvidia.NVreg_PreserveVideoMemoryAllocations=1 to fix."
            );
        }

        self.gpu_power
            .insert(key, GpuPowerState::new(disable_suspend_due_to_nvidia_vram));
    }
}

fn should_start_gpu_wakeup(power: &GpuPowerState, is_retry: bool) -> bool {
    let block = (power.waking_up || power.wakeup_failed) && !is_retry;
    !block
}

fn migrate_gpu_power_state_map<K: std::hash::Hash + std::cmp::Eq + Copy + std::fmt::Debug>(
    gpu_power: &mut std::collections::HashMap<K, GpuPowerState>,
    early_key: K,
    actual_key: K,
) -> Vec<calloop::RegistrationToken> {
    let mut tokens_to_cancel = Vec::new();
    if early_key != actual_key {
        if let Some(power) = gpu_power.get_mut(&early_key) {
            if let Some(token) = power.suspend_timer.take() {
                tokens_to_cancel.push(token);
            }
            if let Some(token) = power.wakeup_timer.take() {
                tokens_to_cancel.push(token);
            }
        }

        if gpu_power.contains_key(&actual_key) {
            if let Some(power) = gpu_power.remove(&early_key) {
                *power.wakeup_state.lock().unwrap_or_else(|e| e.into_inner()) =
                    WakeupState::Cancelled;
            }
        } else if let Some(power) = gpu_power.remove(&early_key) {
            gpu_power.insert(actual_key, power);
            debug!(
                "migrated GPU power state from early key {early_key:?} to actual key {actual_key:?}"
            );
        }
    }
    tokens_to_cancel
}

impl Tty {
    fn migrate_and_prepare_gpu_power_state(
        &mut self,
        niri: &mut Niri,
        node: DrmNode,
        render_node: Option<DrmNode>,
    ) -> &mut GpuPowerState {
        let early_key = node
            .node_with_type(NodeType::Render)
            .and_then(Result::ok)
            .unwrap_or(node);
        let actual_key = render_node.unwrap_or(node);

        let tokens = migrate_gpu_power_state_map(&mut self.gpu_power, early_key, actual_key);
        for token in tokens {
            niri.event_loop.remove(token);
        }

        self.prime_gpu_power_state(node, render_node);
        self.cancel_gpu_power_timer_by_key(niri, actual_key);
        let power = self
            .gpu_power
            .entry(actual_key)
            .or_insert_with(|| GpuPowerState::new(false));
        *power.wakeup_state.lock().unwrap_or_else(|e| e.into_inner()) = WakeupState::Cancelled;
        power.suspended = false;
        power.waking_up = false;
        power.wakeup_failed = false;
        power.resume_attempts = 0;
        power
    }

    fn gpu_power_state_mut(&mut self, node: DrmNode) -> &mut GpuPowerState {
        let key = self.gpu_power_key(node);
        if !self.gpu_power.contains_key(&key) {
            let render_node = if node.ty() == NodeType::Render {
                Some(node)
            } else {
                node.node_with_type(NodeType::Render).and_then(Result::ok)
            };
            self.prime_gpu_power_state(node, render_node);
        }
        self.gpu_power
            .get_mut(&key)
            .expect("gpu_power state must exist after priming")
    }

    fn cancel_gpu_power_timer_by_key(&mut self, niri: &mut Niri, key: DrmNode) {
        if let Some(power) = self.gpu_power.get_mut(&key) {
            if let Some(token) = power.suspend_timer.take() {
                niri.event_loop.remove(token);
            }

            if let Some(token) = power.wakeup_timer.take() {
                niri.event_loop.remove(token);
            }
        }
    }

    fn reset_wakeup_state_by_key(&mut self, niri: &mut Niri, key: DrmNode, failed: bool) {
        self.cancel_gpu_power_timer_by_key(niri, key);

        if let Some(power) = self.gpu_power.get_mut(&key) {
            power.waking_up = false;
            power.resume_attempts = 0;
            if failed {
                power.wakeup_failed = true;
            }
            *power.wakeup_state.lock().unwrap_or_else(|e| e.into_inner()) = WakeupState::Cancelled;
        }
    }

    fn reset_wakeup_state(&mut self, niri: &mut Niri, node: DrmNode) {
        let key = self.gpu_power_key(node);
        self.reset_wakeup_state_by_key(niri, key, false);
    }

    fn remove_pending_device_init(&mut self, niri: &mut Niri, node: &DrmNode) {
        if let Some(pending) = self.pending_device_inits.remove(node) {
            *pending
                .wakeup_state
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = WakeupState::Cancelled;
            if let Some(token) = pending.timer_token {
                niri.event_loop.remove(token);
            }
        }
    }

    fn start_gpu_wakeup(&mut self, niri: &mut Niri, node: DrmNode) -> Result<(), ()> {
        if !self.gpu_access_allowed() {
            return Err(());
        }

        let render_node = self
            .devices
            .get(&node)
            .and_then(|device| device.render_node);
        let render_target =
            render_node.and_then(|rn| rn.dev_path().map(|path| (path, rn.dev_id())));

        let key = self.gpu_power_key(node);
        self.cancel_gpu_power_timer_by_key(niri, key);

        let wakeup_state = Arc::new(Mutex::new(WakeupState::Pending));
        let generation = {
            let power = self.gpu_power_state_mut(node);
            // Keep generation > 0 so Cancelled (generation 0) is never matched.
            power.wakeup_generation = power.wakeup_generation.wrapping_add(1).max(1);
            *power.wakeup_state.lock().unwrap_or_else(|e| e.into_inner()) = WakeupState::Cancelled;

            power.wakeup_state = wakeup_state.clone();
            power.waking_up = true;
            power.resume_attempts = 1;

            power.wakeup_generation
        };

        if let Some((render_path, expected_dev_id)) = render_target {
            debug!("waking GPU {node:?} in background thread");
            spawn_gpu_wakeup_thread(
                render_path,
                expected_dev_id,
                generation,
                self.pm_generation,
                wakeup_state,
                self.gpu_gate.clone(),
            );
        } else {
            *wakeup_state.lock().unwrap_or_else(|e| e.into_inner()) =
                WakeupState::WokenNoFd { generation };
        }

        if self.schedule_wakeup_retry(niri, key).is_err() {
            self.reset_wakeup_state(niri, node);
            return Err(());
        }
        Ok(())
    }

    fn schedule_wakeup_retry(&mut self, niri: &mut Niri, key: DrmNode) -> Result<(), ()> {
        let token = niri
            .event_loop
            .insert_source(
                Timer::from_duration(wakeup_retry_delay()),
                move |_, _, state| {
                    if let Some(tty) = state.backend.tty_checked() {
                        tty.check_async_device_resume(key, &mut state.niri);
                    }

                    TimeoutAction::Drop
                },
            )
            .map_err(|err| {
                error!("couldn't set wakeup check timer for GPU {key:?}: {err:?}");
            })?;

        if let Some(power) = self.gpu_power.get_mut(&key) {
            if let Some(old_token) = power.wakeup_timer.take() {
                niri.event_loop.remove(old_token);
            }
            power.wakeup_timer = Some(token);
        }

        Ok(())
    }

    fn check_async_device_init(&mut self, device_id: dev_t, path: &Path, niri: &mut Niri) {
        if !self.session.is_active() {
            let Ok(node) = DrmNode::from_dev_id(device_id) else {
                return;
            };
            self.remove_pending_device_init(niri, &node);
            return;
        }

        let Ok(node) = DrmNode::from_dev_id(device_id) else {
            return;
        };

        let Some(pending) = self.pending_device_inits.get_mut(&node) else {
            debug!("pending GPU init check cancelled because device was removed: {node:?}");
            return;
        };

        let status = {
            let lock = pending
                .wakeup_state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            match &*lock {
                WakeupState::Woken { generation, .. } if *generation == pending.generation => {
                    Some(Ok(()))
                }
                WakeupState::WokenNoFd { generation } if *generation == pending.generation => {
                    Some(Ok(()))
                }
                WakeupState::Failed => Some(Err(())),
                _ => None,
            }
        };

        if let Some(result) = status {
            if result.is_ok() {
                debug!("GPU {node:?} is back, adding device");
                let add_res = self.device_added(device_id, path, niri, true);
                self.remove_pending_device_init(niri, &node);
                if let Err(err) = add_res {
                    error!("async GPU init failed: {err:?}");
                }
            } else {
                self.remove_pending_device_init(niri, &node);
                error!("GPU {node:?} failed to wake up, giving up");
            }
            return;
        }

        pending.attempts += 1;
        if pending.attempts >= WAKEUP_ATTEMPTS {
            error!("gave up waking GPU {node:?}, marking it failed");
            self.remove_pending_device_init(niri, &node);
            return;
        }

        let dev_id = node.dev_id();
        let path = path.to_owned();

        match niri.event_loop.insert_source(
            Timer::from_duration(wakeup_retry_delay()),
            move |_, _, state| {
                if let Some(tty) = state.backend.tty_checked() {
                    tty.check_async_device_init(dev_id, &path, &mut state.niri);
                }

                TimeoutAction::Drop
            },
        ) {
            Ok(token) => {
                if let Some(pending) = self.pending_device_inits.get_mut(&node) {
                    pending.timer_token = Some(token);
                }
            }
            Err(err) => {
                error!("failed to schedule async GPU init check for {node:?}: {err:?}");
                self.remove_pending_device_init(niri, &node);
            }
        }
    }

    fn check_async_device_resume(&mut self, key: DrmNode, niri: &mut Niri) {
        if !self.session.is_active() {
            self.reset_wakeup_state_by_key(niri, key, false);
            return;
        }

        let Some(power) = self.gpu_power.get_mut(&key) else {
            return;
        };

        if !power.waking_up {
            return;
        }

        power.wakeup_timer = None;

        let status = {
            let lock = power.wakeup_state.lock().unwrap_or_else(|e| e.into_inner());
            match &*lock {
                WakeupState::Woken { generation, .. } if *generation == power.wakeup_generation => {
                    Some(Ok(()))
                }
                WakeupState::WokenNoFd { generation } if *generation == power.wakeup_generation => {
                    Some(Ok(()))
                }
                WakeupState::Failed => Some(Err(())),
                _ => None,
            }
        };

        if let Some(result) = status {
            if result.is_ok() {
                debug!("GPU {key:?} is back, resuming");
                if let Some(node) = self.find_active_primary_for_key(key) {
                    match self.resume_device(niri, node, true) {
                        Ok(gpu_topology_changed) => {
                            if gpu_topology_changed {
                                self.update_dmabuf_feedbacks(niri);
                            }
                            self.device_changed(node.dev_id(), niri, true);
                        }
                        Err(()) => {
                            error!("GPU {key:?} failed to resume after wakeup");
                            self.reset_wakeup_state_by_key(niri, key, true);
                        }
                    }
                } else {
                    debug!("GPU {key:?} woke up but no active primary devices share this render node; resetting state");
                    self.reset_wakeup_state_by_key(niri, key, false);
                }
            } else {
                error!("GPU {key:?} failed to resume: wakeup failed");
                self.reset_wakeup_state_by_key(niri, key, true);
            }
            return;
        }

        let attempts = power.resume_attempts;
        if attempts >= WAKEUP_ATTEMPTS {
            error!("gave up waking GPU {key:?}, marking it failed");
            self.reset_wakeup_state_by_key(niri, key, true);
            return;
        }

        power.resume_attempts += 1;

        if self.schedule_wakeup_retry(niri, key).is_err() {
            self.reset_wakeup_state_by_key(niri, key, true);
        }
    }

    pub fn new(
        config: Rc<RefCell<Config>>,
        event_loop: LoopHandle<'static, State>,
    ) -> anyhow::Result<Self> {
        let _span = tracy_client::span!("Tty::new");

        let (session, notifier) = LibSeatSession::new().context(
            "Error creating a session. This might mean that you're trying to run niri on a TTY \
             that is already busy, for example if you're running this inside tmux that had been \
             originally started on a different TTY",
        )?;
        let seat_name = session.seat();

        let udev_backend =
            UdevBackend::new(session.seat()).context("error creating a udev backend")?;
        let udev_dispatcher = Dispatcher::new(udev_backend, move |event, _, state: &mut State| {
            state.backend.tty().on_udev_event(&mut state.niri, event);
        });
        event_loop
            .register_dispatcher(udev_dispatcher.clone())
            .unwrap();

        let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
        unsafe { init_libinput_plugin_system(&libinput) };
        {
            let _span = tracy_client::span!("Libinput::udev_assign_seat");
            libinput.udev_assign_seat(&seat_name)
        }
        .map_err(|()| anyhow!("error assigning the seat to libinput"))?;

        // If the session is not active at startup (e.g. niri was launched from a different TTY),
        // suspend libinput now so that when ActivateSession fires, libinput.resume() performs a
        // full re-enumeration of input devices instead of being a no-op.
        if !session.is_active() {
            debug!("session is not active, starting libinput in paused state");
            libinput.suspend();
        }

        let input_backend = LibinputInputBackend::new(libinput.clone());
        event_loop
            .insert_source(input_backend, |mut event, _, state| {
                state.process_libinput_event(&mut event);
                state.process_input_event(event);
            })
            .unwrap();

        event_loop
            .insert_source(notifier, move |event, _, state| {
                state.backend.tty().on_session_event(&mut state.niri, event);
            })
            .unwrap();

        let api = GbmGlesBackend::with_context_priority(ContextPriority::High);
        let gpu_manager = GpuManager::new(api).context("error creating the GPU manager")?;

        let (primary_node, primary_render_node) = primary_node_from_config(&config.borrow())
            .ok_or(())
            .or_else(|()| {
                let primary_gpu_path = udev::primary_gpu(&seat_name)
                    .context("error getting the primary GPU")?
                    .context("couldn't find a GPU")?;
                let primary_node = DrmNode::from_path(primary_gpu_path)
                    .context("error opening the primary GPU DRM node")?;
                let primary_render_node = primary_node
                    .node_with_type(NodeType::Render)
                    .and_then(Result::ok)
                    .unwrap_or_else(|| {
                        warn!(
                            "error getting the render node for the primary GPU; proceeding anyway"
                        );
                        primary_node
                    });

                Ok::<_, anyhow::Error>((primary_node, primary_render_node))
            })?;

        let mut node_path = String::new();
        if let Some(path) = primary_render_node.dev_path() {
            write!(node_path, "{path:?}").unwrap();
        } else {
            write!(node_path, "{primary_render_node}").unwrap();
        }
        info!("using as the render node: {node_path}");

        Ok(Self {
            config,
            session,
            udev_dispatcher,
            libinput,
            gpu_manager,
            primary_node,
            primary_render_node,
            ignored_nodes: HashSet::new(),
            devices: HashMap::new(),
            pending_device_inits: HashMap::new(),
            next_pending_init_generation: 0,
            gpu_power: HashMap::new(),
            system_pm: SystemPmState::Running,
            gpu_gate: GpuGate::new(),
            pm_generation: 0,
            inhibitor_fd: None,
            dmabuf_global: None,
            update_output_config_on_resume: false,
            debug_tint: false,
            ipc_outputs: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn gpu_access_allowed(&self) -> bool {
        self.system_pm == SystemPmState::Running && self.session.is_active()
    }

    pub fn on_prepare_for_sleep(&mut self, niri: &mut Niri, start: bool) {
        if start {
            info!("system PM: prepare for sleep (start=true)");
            self.system_pm = SystemPmState::Quiescing;
            self.pm_generation = self.pm_generation.wrapping_add(1);

            // Disallow new leases from being acquired and record new generation.
            self.gpu_gate.set_open_allowed(false, self.pm_generation);

            // Cancel all pending timers and background inits.
            for power in self.gpu_power.values_mut() {
                if let Some(token) = power.suspend_timer.take() {
                    niri.event_loop.remove(token);
                }
                if let Some(token) = power.wakeup_timer.take() {
                    niri.event_loop.remove(token);
                }
                power.waking_up = false;
                *power.wakeup_state.lock().unwrap_or_else(|e| e.into_inner()) =
                    WakeupState::Cancelled;
            }

            for pending in self.pending_device_inits.drain().map(|(_, v)| v) {
                if let Some(token) = pending.timer_token {
                    niri.event_loop.remove(token);
                }
                *pending
                    .wakeup_state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = WakeupState::Cancelled;
            }

            let gate = self.gpu_gate.clone();
            let inhibitor = self.inhibitor_fd.take();

            self.system_pm = SystemPmState::Quiescing;

            // Keep the event loop responsive while pending GPU work stops.
            if let Err(e) = std::thread::Builder::new()
                .name("niri-gpu-quiesce".into())
                .spawn(move || {
                    if gate.wait_for_drain(Duration::from_secs(4)) {
                        debug!("dGPU leases drained successfully before sleep");
                    } else {
                        warn!("dGPU lease drain timed out during quiesce");
                    }

                    // logind may sleep after GPU access has stopped or timed out.
                    drop(inhibitor);
                })
            {
                warn!("failed to spawn GPU quiesce thread: {e}");
            }
        } else {
            info!("system PM: prepare for sleep (start=false, resume)");
            self.system_pm = SystemPmState::ResumePending;
            self.pm_generation = self.pm_generation.wrapping_add(1);

            // Clear failed wakeup state on secondary GPUs so they retry on resume.
            let primary_key = self.gpu_power_key(self.primary_node);
            let nodes: Vec<_> = self.devices.keys().copied().collect();
            for node in nodes {
                if self.gpu_power_key(node) != primary_key && !is_pci_device_ready(node) {
                    debug!("secondary GPU {node:?} is not active in sysfs yet on resume");
                }
            }
            for power in self.gpu_power.values_mut() {
                power.wakeup_failed = false;
            }

            // Re-acquire the sleep inhibitor for the next sleep cycle.
            #[cfg(feature = "dbus")]
            {
                self.inhibitor_fd = crate::dbus::freedesktop_login1::take_sleep_inhibitor_system();
            }

            // Allow GPU leases again and go back to Running.
            self.gpu_gate.set_open_allowed(true, self.pm_generation);
            self.system_pm = SystemPmState::Running;

            if self.update_output_config_on_resume {
                self.update_output_config_on_resume = false;
                self.on_output_config_changed(niri);
            }
        }
    }

    pub fn init(&mut self, niri: &mut Niri) {
        #[cfg(feature = "dbus")]
        {
            self.inhibitor_fd = crate::dbus::freedesktop_login1::take_sleep_inhibitor_system();
        }

        // If the session is inactive, skip initialization because we won't be able to do much with
        // the devices anyway. We'll get ActivateSession and add the devices there instead.
        //
        // This can happen when starting niri while having a different TTY active (e.g. via tmux).
        if !self.session.is_active() {
            return;
        }

        // Initialize the ignored nodes.
        self.ignored_nodes = self.compute_ignored_nodes();

        let udev = self.udev_dispatcher.clone();
        let udev = udev.as_source_ref();

        // Initialize the primary node first as later nodes might depend on the primary render node
        // being available.
        if let Some((primary_device_id, primary_device_path)) = udev
            .device_list()
            .find(|&(device_id, _)| device_id == self.primary_node.dev_id())
        {
            if let Err(err) = self.device_added(primary_device_id, primary_device_path, niri, false)
            {
                warn!(
                    "error adding primary node device, display-only devices may not work: {err:?}"
                );
            }
        } else {
            warn!("primary node is missing, display-only devices may not work");
        };

        for (device_id, path) in udev.device_list() {
            if device_id == self.primary_node.dev_id() {
                continue;
            }

            if let Err(err) = self.device_added(device_id, path, niri, false) {
                warn!("error adding device: {err:?}");
            }
        }
    }

    fn on_udev_event(&mut self, niri: &mut Niri, event: UdevEvent) {
        let _span = tracy_client::span!("Tty::on_udev_event");

        match event {
            UdevEvent::Added { device_id, path } => {
                if !self.session.is_active() {
                    debug!("skipping UdevEvent::Added as session is inactive");
                    return;
                }

                // Recompute ignored nodes to resolve symlinks (like /dev/dri/by-path/...) to their
                // new underlying device IDs.
                self.ignored_nodes = self.compute_ignored_nodes();

                if let Err(err) = self.device_added(device_id, &path, niri, false) {
                    warn!("error adding device: {err:?}");
                }
            }
            UdevEvent::Changed { device_id } => {
                if !self.session.is_active() {
                    debug!("skipping UdevEvent::Changed as session is inactive");
                    return;
                }

                self.device_changed(device_id, niri, false)
            }
            UdevEvent::Removed { device_id } => {
                if !self.session.is_active() {
                    debug!("skipping UdevEvent::Removed as session is inactive");
                    return;
                }

                self.device_removed(device_id, niri)
            }
        }
    }

    fn on_session_event(&mut self, niri: &mut Niri, event: SessionEvent) {
        let _span = tracy_client::span!("Tty::on_session_event");

        match event {
            SessionEvent::PauseSession => {
                debug!("pausing session");

                self.gpu_gate.set_open_allowed(false, self.pm_generation);
                self.libinput.suspend();

                for device in self.devices.values_mut() {
                    device.drm.pause();

                    if let Some(lease_state) = &mut device.drm_lease_state {
                        lease_state.suspend();
                    }
                }
            }
            SessionEvent::ActivateSession => {
                debug!("resuming session");

                if self.system_pm == SystemPmState::Running {
                    self.gpu_gate.set_open_allowed(true, self.pm_generation);
                }

                if self.libinput.resume().is_err() {
                    warn!("error resuming libinput");
                }

                // While the session was suspended, GPUs could have been added, so
                // /dev/dri/by-path/... symlinks need to be re-resolved.
                self.ignored_nodes = self.compute_ignored_nodes();

                let mut device_list = self
                    .udev_dispatcher
                    .as_source_ref()
                    .device_list()
                    .map(|(device_id, path)| (device_id, path.to_owned()))
                    .collect::<HashMap<_, _>>();

                let removed_devices = self
                    .devices
                    .keys()
                    .filter(|node| {
                        !device_list.contains_key(&node.dev_id())
                            || self.ignored_nodes.contains(node)
                    })
                    .copied()
                    .collect::<Vec<_>>();

                let remained_devices = self
                    .devices
                    .keys()
                    .filter(|node| {
                        device_list.contains_key(&node.dev_id())
                            && !self.ignored_nodes.contains(node)
                    })
                    .copied()
                    .collect::<Vec<_>>();

                // Remove removed devices.
                for node in removed_devices {
                    device_list.remove(&node.dev_id());
                    self.device_removed(node.dev_id(), niri);
                }

                // Update remained devices.
                for node in remained_devices {
                    device_list.remove(&node.dev_id());

                    let key = self.gpu_power_key(node);
                    if let Some(power) = self.gpu_power.get_mut(&key) {
                        power.wakeup_failed = false;
                    }

                    // It hasn't been removed, update its state as usual.
                    let device = self.devices.get_mut(&node).unwrap();

                    // Someone on an old device hit what seems to be a driver bug without this:
                    // https://github.com/niri-wm/niri/issues/3048
                    let force_disable = self
                        .config
                        .borrow()
                        .debug
                        .force_disable_connectors_on_resume;

                    if let Err(err) = device.drm.activate(force_disable) {
                        warn!("error activating DRM device: {err:?}");
                    }
                    if let Some(lease_state) = &mut device.drm_lease_state {
                        lease_state.resume::<State>();
                    }

                    // Refresh the connectors.
                    self.device_changed(node.dev_id(), niri, true);

                    // Apply pending gamma changes and restore our existing gamma.
                    let device = self.devices.get_mut(&node).unwrap();
                    for (crtc, surface) in device.surfaces.iter_mut() {
                        if let Ok(mut props) =
                            ConnectorProperties::try_new(&device.drm, surface.connector)
                        {
                            let max_bpc = self
                                .config
                                .borrow()
                                .outputs
                                .find(&surface.name)
                                .and_then(|o| o.max_bpc);
                            set_connector_properties(&mut props, max_bpc, true);
                        } else {
                            warn!("failed to get connector properties");
                        }

                        if let Some(ramp) = surface.pending_gamma_change.take() {
                            let ramp = ramp.as_deref();
                            let res = if let Some(gamma_props) = &mut surface.gamma_props {
                                gamma_props.set_gamma(&device.drm, ramp)
                            } else {
                                set_gamma_for_crtc(&device.drm, *crtc, ramp)
                            };
                            if let Err(err) = res {
                                warn!("error applying pending gamma change: {err:?}");
                            }
                        } else if let Some(gamma_props) = &surface.gamma_props {
                            if let Err(err) = gamma_props.restore_gamma(&device.drm) {
                                warn!("error restoring gamma: {err:?}");
                            }
                        }
                    }
                }

                // Add new devices.
                //
                // Add the primary node first as later nodes might depend on the primary render
                // node being available.
                let primary_device_id = self.primary_node.dev_id();
                let primary_device_path = device_list.remove(&primary_device_id);
                let primary = primary_device_path.map(|path| (primary_device_id, path));

                for (device_id, path) in primary.into_iter().chain(device_list) {
                    if let Err(err) = self.device_added(device_id, &path, niri, false) {
                        warn!("error adding device: {err:?}");
                    }
                }

                if self.update_output_config_on_resume {
                    self.on_output_config_changed(niri);
                }

                self.refresh_ipc_outputs(niri);

                niri.notify_activity();
                niri.monitors_active = true;
                self.set_monitors_active(true);
                niri.queue_redraw_all();
            }
        }
    }

    fn device_added(
        &mut self,
        device_id: dev_t,
        path: &Path,
        niri: &mut Niri,
        is_retry: bool,
    ) -> anyhow::Result<()> {
        debug!("adding device: {device_id} {path:?}");

        let node = DrmNode::from_dev_id(device_id)?;

        if node.ty() != NodeType::Primary {
            debug!("not a primary node, skipping");
            return Ok(());
        }

        if self.ignored_nodes.contains(&node) {
            debug!("node is ignored, skipping");
            return Ok(());
        }

        if self.devices.contains_key(&node) {
            debug!("device {node:?} already exists, skipping duplicate add");
            self.remove_pending_device_init(niri, &node);
            return Ok(());
        }

        let early_render_node = node.node_with_type(NodeType::Render).and_then(Result::ok);
        self.prime_gpu_power_state(node, early_render_node);

        // PCIe resume can block, so wake secondary GPUs off the main loop.
        if !is_retry && node != self.primary_node {
            if self.pending_device_inits.contains_key(&node) {
                debug!("GPU {node:?} is already in the process of initialization, skipping duplicate add event");
                return Ok(());
            }

            let is_waking_up = self
                .gpu_power
                .get(&self.gpu_power_key(node))
                .map(|p| p.waking_up)
                .unwrap_or(false);
            if is_waking_up {
                debug!(
                    "GPU {node:?} is already waking up from suspend, skipping duplicate add event"
                );
                return Ok(());
            }

            let render_target =
                early_render_node.and_then(|rn| rn.dev_path().map(|path| (path, rn.dev_id())));

            let wakeup_state = Arc::new(Mutex::new(WakeupState::Pending));
            // Keep generation > 0 so Cancelled (generation 0) is never matched.
            self.next_pending_init_generation =
                self.next_pending_init_generation.wrapping_add(1).max(1);
            let generation = self.next_pending_init_generation;

            self.pending_device_inits.insert(
                node,
                PendingDeviceInit {
                    attempts: 0,
                    generation,
                    timer_token: None,
                    wakeup_state: wakeup_state.clone(),
                },
            );

            if let Some((render_path, expected_dev_id)) = render_target {
                spawn_gpu_wakeup_thread(
                    render_path,
                    expected_dev_id,
                    generation,
                    self.pm_generation,
                    wakeup_state,
                    self.gpu_gate.clone(),
                );
            } else {
                // Display-only devices without a render node do not require slow PCIe dGPU wakeups,
                // so we transition to WokenNoFd immediately. Opening card nodes for display-only
                // output will not block the event loop.
                *wakeup_state.lock().unwrap_or_else(|e| e.into_inner()) =
                    WakeupState::WokenNoFd { generation };
            }

            let dev_id = node.dev_id();
            let path_buf = path.to_owned();
            match niri.event_loop.insert_source(
                Timer::from_duration(wakeup_retry_delay()),
                move |_, _, state| {
                    if let Some(tty) = state.backend.tty_checked() {
                        tty.check_async_device_init(dev_id, &path_buf, &mut state.niri);
                    }
                    TimeoutAction::Drop
                },
            ) {
                Ok(token) => {
                    if let Some(pending) = self.pending_device_inits.get_mut(&node) {
                        pending.timer_token = Some(token);
                    }
                }
                Err(e) => {
                    error!("failed to schedule async GPU init check for {node:?}: {e:?}");
                    self.remove_pending_device_init(niri, &node);
                    return Err(anyhow::anyhow!(
                        "failed to schedule async GPU init check: {e:?}"
                    ));
                }
            }
            return Ok(());
        }

        let _span = tracy_client::span!("Tty::device_added");

        let open_flags = OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK;
        let fd = {
            let _span = tracy_client::span!("LibSeatSession::open");
            self.session.open(path, open_flags)
        }?;
        let raw_device_fd = DeviceFd::from(fd);
        let device_fd = DrmDeviceFd::new(raw_device_fd.clone());

        let (drm, drm_notifier) = {
            let _span = tracy_client::span!("DrmDevice::new");
            DrmDevice::new(device_fd.clone(), false)
        }?;

        let gbm = {
            let _span = tracy_client::span!("GbmDevice::new");
            GbmDevice::new(device_fd.clone())
        }?;

        let mut try_initialize_gpu = || -> Result<Option<DrmNode>, anyhow::Error> {
            let display = unsafe { EGLDisplay::new(gbm.clone())? };
            let egl_device = EGLDevice::device_for_display(&display)?;

            // Software renderers segfault on dmabuf import and mess up DRM leasing,
            // so skip render integration but keep the device for scanout.
            if egl_device.is_software() {
                return Ok(None);
            }

            let render_node = egl_device
                .try_get_render_node()
                .ok()
                .flatten()
                .unwrap_or(node);
            self.gpu_manager
                .as_mut()
                .add_node(render_node, gbm.clone())
                .context("error adding render node to GPU manager")?;

            Ok(Some(render_node))
        };

        let gpu_init = try_initialize_gpu();

        let render_node = match gpu_init {
            Ok(Some(render_node)) => {
                debug!("got render node: {render_node}");
                self.remove_pending_device_init(niri, &node);
                Some(render_node)
            }
            Ok(None) => {
                debug!("GPU {node:?} is a software renderer, skipping rendering integration");
                self.remove_pending_device_init(niri, &node);
                None
            }
            Err(err) => {
                self.remove_pending_device_init(niri, &node);
                debug!("failed to initialize renderer for {node:?}: {err:?}");
                None
            }
        };

        if render_node == Some(self.primary_render_node) && self.dmabuf_global.is_none() {
            let render_node = self.primary_render_node;
            debug!("initializing the primary renderer");

            let mut renderer = self
                .gpu_manager
                .single_renderer(&render_node)
                .context("error creating renderer")?;

            if let Err(err) = renderer.bind_wl_display(&niri.display_handle) {
                // wl_drm is on its way out so this is expected on most modern distros.
                trace!("error binding legacy EGL to wl_display: {err}");
            } else {
                debug!("bound legacy EGL to wl_display");
            }

            let gles_renderer = renderer.as_gles_renderer();
            resources::init(gles_renderer);
            shaders::init(gles_renderer);

            let config = self.config.borrow();
            if let Some(src) = config.animations.window_resize.custom_shader.as_deref() {
                shaders::set_custom_resize_program(gles_renderer, Some(src));
            }
            if let Some(src) = config.animations.window_close.custom_shader.as_deref() {
                shaders::set_custom_close_program(gles_renderer, Some(src));
            }
            if let Some(src) = config.animations.window_open.custom_shader.as_deref() {
                shaders::set_custom_open_program(gles_renderer, Some(src));
            }
            drop(config);

            niri.update_shaders();

            // Create the dmabuf global.
            let primary_formats = renderer.dmabuf_formats();
            let default_feedback =
                match DmabufFeedbackBuilder::new(render_node.dev_id(), primary_formats.clone())
                    .build()
                    .context("error building default dmabuf feedback")
                {
                    Ok(fb) => fb,
                    Err(err) => {
                        drop(renderer);
                        self.gpu_manager.as_mut().remove_node(&render_node);
                        drop(drm_notifier);
                        drop(drm);
                        drop(gbm);
                        self.sync_gpu_manager_devices();
                        return_device_fd(&mut self.session, device_fd, raw_device_fd);
                        return Err(err);
                    }
                };
            let dmabuf_global = niri
                .dmabuf_state
                .create_global_with_default_feedback::<State>(
                    &niri.display_handle,
                    &default_feedback,
                );
            assert!(self.dmabuf_global.replace(dmabuf_global).is_none());

            // Update the dmabuf feedbacks for all surfaces.
            for (node, device) in self.devices.iter_mut() {
                for surface in device.surfaces.values_mut() {
                    match surface_dmabuf_feedback(
                        &surface.compositor,
                        primary_formats.clone(),
                        self.primary_render_node,
                        device.render_node,
                        *node,
                    ) {
                        Ok(feedback) => {
                            surface.dmabuf_feedback = Some(feedback);
                        }
                        Err(err) => {
                            warn!("error building dmabuf feedback: {err:?}");
                        }
                    }
                }
            }
        }

        let allocator_gbm = if render_node.is_some() {
            gbm.clone()
        } else if let Some(primary_device) = self.devices.get(&self.primary_node) {
            primary_device.gbm.clone()
        } else {
            bail!("no allocator available for device");
        };
        let gbm_flags = GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT;
        let allocator = GbmAllocator::new(allocator_gbm, gbm_flags);

        let token = niri
            .event_loop
            .insert_source(drm_notifier, move |event, meta, state| {
                let tty = state.backend.tty();
                match event {
                    DrmEvent::VBlank(crtc) => {
                        let meta = meta.expect("VBlank events must have metadata");
                        tty.on_vblank(&mut state.niri, node, crtc, meta);
                    }
                    DrmEvent::Error(error) => warn!("DRM error: {error}"),
                };
            })
            .unwrap();

        let drm_lease_state = DrmLeaseState::new::<State>(&niri.display_handle, &node)
            .map_err(|err| warn!("error initializing DRM leasing for {node}: {err:?}"))
            .ok();

        // Reset stale suspend state after a fresh device init.
        let _power = self.migrate_and_prepare_gpu_power_state(niri, node, render_node);

        let device = OutputDevice {
            token,
            render_node,
            drm,
            gbm,
            allocator,
            drm_scanner: DrmScanner::new(),
            surfaces: HashMap::new(),
            known_crtcs: HashMap::new(),
            drm_lease_state,
            active_leases: Vec::new(),
            non_desktop_connectors: HashSet::new(),
        };
        match self.devices.entry(node) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(device);
            }
            std::collections::hash_map::Entry::Occupied(_) => {
                if let Some(render_node) = render_node {
                    self.gpu_manager.as_mut().remove_node(&render_node);
                }
                niri.event_loop.remove(device.token);
                drop(device); // drm and gbm live inside, must go before closing the fd
                self.sync_gpu_manager_devices();
                return_device_fd(&mut self.session, device_fd, raw_device_fd);
                bail!("device {node:?} was unexpectedly already present in self.devices");
            }
        }

        self.device_changed(device_id, niri, true);

        Ok(())
    }

    pub(crate) fn check_suspend_device(&mut self, niri: &mut Niri, node: DrmNode) {
        if !self.gpu_access_allowed() {
            return;
        }

        let key = self.gpu_power_key(node);
        let should_suspend = node != self.primary_node
            && self
                .gpu_power
                .get(&key)
                .map(|power| !power.suspended && !power.waking_up)
                .unwrap_or(false)
            && is_device_idle_and_last(self, node);

        if !should_suspend {
            if let Some(power) = self.gpu_power.get_mut(&key) {
                if let Some(token) = power.suspend_timer.take() {
                    niri.event_loop.remove(token);
                }
            }
            return;
        }

        let Some(device) = self.devices.get(&node) else {
            return;
        };

        if device.render_node.is_none() {
            return;
        }

        if self
            .gpu_power
            .get(&key)
            .and_then(|power| power.suspend_timer.as_ref())
            .is_some()
        {
            return;
        }

        debug!("secondary GPU {node:?} idle, suspending in {SUSPEND_TIMEOUT_SECS}s");

        let token = match niri.event_loop.insert_source(
            Timer::from_duration(std::time::Duration::from_secs(SUSPEND_TIMEOUT_SECS)),
            move |_, _, state| {
                let Some(tty) = state.backend.tty_checked() else {
                    return TimeoutAction::Drop;
                };

                let Some(node) = tty.find_active_primary_for_key(key) else {
                    debug!("secondary GPU timer fired for key {key:?}, but no active primary devices share it; ignoring suspend");
                    return TimeoutAction::Drop;
                };

                if let Some(power) = tty.gpu_power.get_mut(&key) {
                    power.suspend_timer = None;
                }

                if !tty.gpu_access_allowed() {
                    return TimeoutAction::Drop;
                }

                if !is_device_idle_and_last(tty, node) {
                    return TimeoutAction::Drop;
                }

                let Some(render_node) =
                    tty.devices.get(&node).and_then(|device| device.render_node)
                else {
                    return TimeoutAction::Drop;
                };

                if render_node == tty.primary_render_node {
                    warn!("attempted to suspend primary render node; skipping suspend");
                    return TimeoutAction::Drop;
                }

                tty.cancel_gpu_power_timer_by_key(&mut state.niri, key);

                if let Some(power) = tty.gpu_power.get_mut(&key) {
                    power.suspended = true;
                    power.waking_up = false;
                    power.resume_attempts = 0;
                    *power.wakeup_state.lock().unwrap_or_else(|e| e.into_inner()) =
                        WakeupState::Cancelled;
                }

                debug!("secondary GPU {node:?} timer up, suspending");
                tty.gpu_manager.as_mut().remove_node(&render_node);
                tty.sync_gpu_manager_devices();
                tty.update_dmabuf_feedbacks(&mut state.niri);

                TimeoutAction::Drop
            },
        ) {
            Ok(token) => token,
            Err(err) => {
                error!("couldn't set suspend timer for GPU {node:?}: {err:?}");
                return;
            }
        };

        self.gpu_power_state_mut(node).suspend_timer = Some(token);
    }

    pub(crate) fn resume_device(
        &mut self,
        niri: &mut Niri,
        node: DrmNode,
        is_retry: bool,
    ) -> Result<bool, ()> {
        let Some(device) = self.devices.get(&node) else {
            return Ok(false);
        };

        let key = self.gpu_power_key(node);
        let render_node = device.render_node;
        let gbm = device.gbm.clone();

        if let Some(power) = self.gpu_power.get_mut(&key) {
            if !should_start_gpu_wakeup(power, is_retry) {
                if power.wakeup_failed && !is_retry {
                    debug!("GPU {key:?} failed to wake before, not touching it again");
                }
                return Err(());
            }
            power.wakeup_failed = false;
            if let Some(token) = power.suspend_timer.take() {
                niri.event_loop.remove(token);
            }
        }

        let was_suspended = self
            .gpu_power
            .get(&key)
            .map(|power| power.suspended)
            .unwrap_or(false);

        if !was_suspended {
            if let Some(power) = self.gpu_power.get_mut(&key) {
                power.waking_up = false;
                power.resume_attempts = 0;
            }
            return Ok(false);
        }

        let Some(render_node) = render_node else {
            if let Some(power) = self.gpu_power.get_mut(&key) {
                power.suspended = false;
                power.waking_up = false;
                power.resume_attempts = 0;
            }
            return Ok(false);
        };

        let shared_active = self.devices.iter().any(|(other_node, other)| {
            *other_node != node
                && other.render_node == Some(render_node)
                && !self
                    .gpu_power
                    .get(&self.gpu_power_key(*other_node))
                    .map(|power| power.suspended)
                    .unwrap_or(false)
        });

        if shared_active {
            if let Some(power) = self.gpu_power.get_mut(&key) {
                power.suspended = false;
                power.waking_up = false;
                power.resume_attempts = 0;
            }
            return Ok(false);
        }

        if !is_retry {
            self.start_gpu_wakeup(niri, node)?;
            return Err(());
        }

        self.gpu_manager
            .as_mut()
            .add_node(render_node, gbm)
            .map_err(|err| {
                warn!("error re-adding render node {render_node:?}: {err:?}");
            })?;

        self.sync_gpu_manager_devices();

        {
            let power = self.gpu_power_state_mut(node);
            *power.wakeup_state.lock().unwrap_or_else(|e| e.into_inner()) = WakeupState::Cancelled;
            power.suspended = false;
            power.waking_up = false;
            power.resume_attempts = 0;
        };

        Ok(true)
    }

    pub(crate) fn device_changed(&mut self, device_id: dev_t, niri: &mut Niri, cleanup: bool) {
        debug!("device changed: {device_id}");

        let Ok(node) = DrmNode::from_dev_id(device_id) else {
            warn!("error creating DrmNode");
            return;
        };

        if node.ty() != NodeType::Primary {
            debug!("not a primary node, skipping");
            return;
        }

        let key = self.gpu_power_key(node);
        let state = self
            .gpu_power
            .get(&key)
            .map(|power| (power.waking_up, power.suspended));

        if let Some((waking_up, suspended)) = state {
            if waking_up {
                // Duplicate udev events get dropped during wakeup: on success we replay one,
                // on failure the device is marked failed and ignored, so nothing is lost either way.
                debug!("holding udev event until wakeup is done");
                return;
            }
            if suspended {
                debug!(
                    "secondary GPU {device_id} is suspended, triggering async wakeup instead of scanning connectors"
                );
                if let Err(err) = self.start_gpu_wakeup(niri, node) {
                    warn!("failed to start async GPU wakeup for device {device_id}: {err:?}");
                }
                return;
            }
        }

        if let Some(power) = self.gpu_power.get_mut(&key) {
            power.resume_attempts = 0;
        }

        if self.ignored_nodes.contains(&node) {
            debug!("node is ignored, skipping");
            return;
        }

        let Some(device) = self.devices.get_mut(&node) else {
            if let Some(path) = node.dev_path() {
                warn!("unknown device; trying to add");

                if let Err(err) = self.device_added(device_id, &path, niri, false) {
                    warn!("error adding device: {err:?}");
                }
            } else {
                warn!("unknown device");
            }

            return;
        };

        // DrmScanner will preserve any existing connector-CRTC mapping.
        let scan_result = match device.drm_scanner.scan_connectors(&device.drm) {
            Ok(x) => x,
            Err(err) => {
                warn!("error scanning connectors: {err:?}");
                return;
            }
        };

        let mut added = Vec::new();
        let mut removed = Vec::new();
        for event in scan_result {
            match event {
                DrmScanEvent::Connected {
                    connector,
                    crtc: Some(crtc),
                } => {
                    let connector_name = format_connector_name(&connector);
                    let name = make_output_name(&device.drm, connector.handle(), connector_name);
                    debug!(
                        "new connector: {} \"{}\"",
                        &name.connector,
                        name.format_make_model_serial(),
                    );

                    // Assign an id to this crtc.
                    let id = OutputId::next();
                    added.push((crtc, CrtcInfo { id, name }));
                }
                DrmScanEvent::Disconnected {
                    crtc: Some(crtc), ..
                } => {
                    removed.push(crtc);
                }
                // Emitted when the list of connector modes changes at runtime.
                //
                // Some devices, notably USB-C docks with DP-MST/alt-mode, report Connected before
                // the EDID has been read, with an empty mode list. Then, at a later point, the
                // modes will be populated, at which point we'll get this Changed event.
                DrmScanEvent::Changed {
                    connector,
                    crtc: Some(crtc),
                } => {
                    let connector_name = format_connector_name(&connector);
                    let name = make_output_name(&device.drm, connector.handle(), connector_name);
                    debug!(
                        "connector changed: {} \"{}\"",
                        &name.connector,
                        name.format_make_model_serial(),
                    );

                    if !device.known_crtcs.contains_key(&crtc) {
                        // I guess this can happen if the connector initially wasn't mapped to a
                        // CRTC but then got mapped before being changed.
                        warn!("changed connector missing from known crtcs");
                    }

                    // We don't actually need to do anything here; on_output_config_changed() will
                    // take care of picking a new mode if needed.
                }
                _ => (),
            }
        }

        for crtc in &removed {
            self.connector_disconnected(niri, node, *crtc);
        }

        let Some(device) = self.devices.get_mut(&node) else {
            error!("device disappeared");
            return;
        };

        for crtc in removed {
            if device.known_crtcs.remove(&crtc).is_none() {
                error!("output ID missing for disconnected crtc: {crtc:?}");
            }
        }

        for (crtc, mut info) in added {
            // Make/model/serial can match exactly between different physical monitors. This doesn't
            // happen often, but our Layout does not support such duplicates and will panic.
            //
            // As a workaround, search for duplicates, and unname the new connectors if one is
            // found. Connector names are always unique.
            let name = &mut info.name;
            let formatted = name.format_make_model_serial_or_connector();
            for info in self.devices.values().flat_map(|d| d.known_crtcs.values()) {
                if info.name.matches(&formatted) {
                    let connector = mem::take(&mut name.connector);
                    warn!(
                        "new connector {connector} duplicates make/model/serial \
                         of existing connector {}, unnaming",
                        info.name.connector,
                    );
                    *name = OutputName {
                        connector,
                        make: None,
                        model: None,
                        serial: None,
                    };
                    break;
                }
            }

            // Insert it right away so next added connector will check against this one too.
            let device = self.devices.get_mut(&node).unwrap();
            device.known_crtcs.insert(crtc, info);
        }

        // If the device was just added or resumed, we need to cleanup any disconnected connectors
        // and planes.
        if cleanup {
            let device = self.devices.get(&node).unwrap();

            // Follow the logic in on_output_config_changed().
            let disable_laptop_panels = self.should_disable_laptop_panels(niri.is_lid_closed);
            let should_disable = |conn: &str| disable_laptop_panels && is_laptop_panel(conn);

            let config = self.config.borrow();
            let disable_monitor_names = config.debug.disable_monitor_names;

            let should_be_off = |crtc, conn: &connector::Info| {
                let output_name = device.known_crtc_name(&crtc, conn, disable_monitor_names);

                let config = config
                    .outputs
                    .find(&output_name)
                    .cloned()
                    .unwrap_or_default();

                config.off || should_disable(&output_name.connector)
            };

            if let Err(err) = device.cleanup_mismatching_resources(&should_be_off) {
                warn!("error cleaning up connectors: {err:?}");
            }

            let device = self.devices.get_mut(&node).unwrap();
            for surface in device.surfaces.values_mut() {
                // We aren't force-clearing the CRTCs, so we need to make the surfaces read the
                // updated state after a session resume. This also causes a full damage for the
                // next redraw.
                if let Err(err) = surface.compositor.reset_state() {
                    warn!("error resetting DrmCompositor state: {err:?}");
                }
                surface.compositor.reset_buffers();
            }
        }

        // This will connect any new connectors if needed, and apply other changes, such as
        // connecting back the internal laptop monitor once it becomes the only monitor left.
        //
        // It will also call refresh_ipc_outputs(), which will catch the disconnected connectors
        // above.
        self.on_output_config_changed(niri);
    }

    fn device_removed(&mut self, device_id: dev_t, niri: &mut Niri) {
        debug!("removing device: {device_id}");

        let Ok(node) = DrmNode::from_dev_id(device_id) else {
            warn!("error creating DrmNode");
            return;
        };

        if node.ty() != NodeType::Primary {
            debug!("not a primary node, skipping");
            return;
        }

        let key = self.gpu_power_key(node);

        let Some(device) = self.devices.get_mut(&node) else {
            // Stop the wakeup thread if udev removed the device first.
            if self.pending_device_inits.contains_key(&node) {
                self.remove_pending_device_init(niri, &node);
                debug!("cancelled pending async GPU init for removed device {node:?}");
            } else {
                warn!("unknown device");
            }
            return;
        };

        let crtcs: Vec<_> = device
            .drm_scanner
            .crtcs()
            .map(|(_info, crtc)| crtc)
            .collect();

        for crtc in crtcs {
            self.connector_disconnected(niri, node, crtc);
        }

        let mut device = self.devices.remove(&node).unwrap();
        let device_fd = device.drm.device_fd().device_fd();

        let was_last_primary_user = if let Some(render_node) = device.render_node {
            !self
                .devices
                .values()
                .any(|device| device.render_node == Some(render_node))
        } else {
            true
        };

        if was_last_primary_user {
            self.cancel_gpu_power_timer_by_key(niri, key);
            if let Some(power) = self.gpu_power.get(&key) {
                *power.wakeup_state.lock().unwrap_or_else(|e| e.into_inner()) =
                    WakeupState::Cancelled;
            }
            self.gpu_power.remove(&key);
        }

        if let Some(lease_state) = &mut device.drm_lease_state {
            lease_state.disable_global::<State>();
        }

        if let Some(render_node) = device.render_node {
            // Sometimes (Asahi DisplayLink), multiple primary nodes will correspond to the same
            // render node. In this case, we want to keep the render node active until the last
            // primary node that uses it is gone.
            let was_last = !self.devices.iter().any(|(other_node, other)| {
                other.render_node == Some(render_node)
                    && !self
                        .gpu_power
                        .get(&self.gpu_power_key(*other_node))
                        .map(|p| p.suspended)
                        .unwrap_or(false)
            });

            if was_last && render_node == self.primary_render_node {
                debug!("destroying the primary renderer");

                match self.gpu_manager.single_renderer(&self.primary_render_node) {
                    Ok(mut renderer) => renderer.unbind_wl_display(),
                    Err(err) => {
                        warn!("error creating renderer during device removal: {err}");
                    }
                }

                // Disable and destroy the dmabuf global.
                if let Some(global) = self.dmabuf_global.take() {
                    niri.dmabuf_state
                        .disable_global::<State>(&niri.display_handle, &global);
                    niri.event_loop
                        .insert_source(
                            Timer::from_duration(Duration::from_secs(10)),
                            move |_, _, state| {
                                state
                                    .niri
                                    .dmabuf_state
                                    .destroy_global::<State>(&state.niri.display_handle, global);
                                TimeoutAction::Drop
                            },
                        )
                        .unwrap();

                    // Clear the dmabuf feedbacks for all surfaces.
                    for device in self.devices.values_mut() {
                        for surface in device.surfaces.values_mut() {
                            surface.dmabuf_feedback = None;
                        }
                    }
                } else {
                    error!("dmabuf global was already missing");
                }
            }

            if was_last {
                self.gpu_manager.as_mut().remove_node(&render_node);
                // Nudge gpu_manager to forget the removed render node.
                self.sync_gpu_manager_devices();
                self.update_dmabuf_feedbacks(niri);
            }
        }

        niri.event_loop.remove(device.token);

        self.refresh_ipc_outputs(niri);

        drop(device);

        match TryInto::<OwnedFd>::try_into(device_fd) {
            Ok(fd) => {
                if let Err(err) = self.session.close(fd) {
                    warn!("error closing DRM device fd: {err:?}");
                }
            }
            Err(_) => {
                error!("unable to close DRM device cleanly: fd has unexpected references");
            }
        }
    }

    fn connector_connected(
        &mut self,
        niri: &mut Niri,
        node: DrmNode,
        connector: connector::Info,
        crtc: crtc::Handle,
    ) -> anyhow::Result<()> {
        let connector_name = format_connector_name(&connector);
        debug!("connecting connector: {connector_name}");

        let device = self.devices.get_mut(&node).context("missing device")?;

        let disable_monitor_names = self.config.borrow().debug.disable_monitor_names;
        let output_name = device.known_crtc_name(&crtc, &connector, disable_monitor_names);

        let non_desktop = find_drm_property(&device.drm, connector.handle(), "non-desktop")
            .and_then(|(_, info, value)| info.value_type().convert_value(value).as_boolean())
            .unwrap_or(false);

        if non_desktop {
            debug!("output is non desktop");
            let description = output_name.format_description();
            if let Some(lease_state) = &mut device.drm_lease_state {
                lease_state.add_connector::<State>(connector.handle(), connector_name, description);
            }
            device
                .non_desktop_connectors
                .insert((connector.handle(), crtc));
            return Ok(());
        }

        let config = self
            .config
            .borrow()
            .outputs
            .find(&output_name)
            .cloned()
            .unwrap_or_default();

        for m in connector.modes() {
            trace!("{m:?}");
        }

        let mut mode = None;
        if let Some(modeline) = &config.modeline {
            match calculate_drm_mode_from_modeline(modeline) {
                Ok(x) => mode = Some(x),
                Err(err) => {
                    warn!("invalid custom modeline; falling back to advertised modes: {err:?}");
                }
            }
        }

        let (mode, fallback) = match mode {
            Some(x) => (x, false),
            None => pick_mode(&connector, config.mode).ok_or_else(|| anyhow!("no mode"))?,
        };

        if fallback {
            let target = config.mode.unwrap();
            warn!(
                "configured mode {}x{}{} could not be found, falling back to preferred",
                target.mode.width,
                target.mode.height,
                if let Some(refresh) = target.mode.refresh {
                    format!("@{refresh}")
                } else {
                    String::new()
                },
            );
        }

        debug!("picking mode: {mode:?}");

        let mut orientation = None;
        if let Ok(mut props) = ConnectorProperties::try_new(&device.drm, connector.handle()) {
            set_connector_properties(&mut props, config.max_bpc, true);

            match props.get_panel_orientation() {
                Ok(x) => orientation = Some(x),
                Err(err) => {
                    trace!("couldn't get panel orientation: {err:?}");
                }
            }
        } else {
            warn!("failed to get connector properties");
        }

        let mut gamma_props = GammaProps::new(&device.drm, crtc)
            .map_err(|err| debug!("couldn't get gamma properties: {err:?}"))
            .ok();

        // Reset gamma in case it was set before.
        let res = if let Some(gamma_props) = &mut gamma_props {
            gamma_props.set_gamma(&device.drm, None)
        } else {
            set_gamma_for_crtc(&device.drm, crtc, None)
        };
        if let Err(err) = res {
            debug!("couldn't reset gamma: {err:?}");
        }

        let surface = device
            .drm
            .create_surface(crtc, mode, &[connector.handle()])?;

        // Try to enable VRR if requested.
        match surface.vrr_supported(connector.handle()) {
            Ok(VrrSupport::Supported | VrrSupport::RequiresModeset) => {
                // Even if on-demand, we still disable it until later checks.
                let vrr = config.is_vrr_always_on();
                let word = if vrr { "enabling" } else { "disabling" };

                if let Err(err) = surface.use_vrr(vrr) {
                    warn!("error {} VRR: {err:?}", word);
                }
            }
            Ok(VrrSupport::NotSupported) => {
                if !config.is_vrr_always_off() {
                    warn!("cannot enable VRR because connector does not support it");
                }

                // Try to disable it anyway to work around a bug where resetting DRM state causes
                // vrr_capable to be reset to 0, potentially leaving VRR_ENABLED at 1.
                let _ = surface.use_vrr(false);
            }
            Err(err) => {
                warn!("error querying for VRR support: {err:?}");
            }
        }

        // Update the output mode.
        let (physical_width, physical_height) = connector.size().unwrap_or((0, 0));

        let output = Output::new(
            connector_name.clone(),
            PhysicalProperties {
                size: (physical_width as i32, physical_height as i32).into(),
                subpixel: connector.subpixel().into(),
                model: output_name.model.as_deref().unwrap_or("Unknown").to_owned(),
                make: output_name.make.as_deref().unwrap_or("Unknown").to_owned(),
                serial_number: output_name
                    .serial
                    .as_deref()
                    .unwrap_or("Unknown")
                    .to_owned(),
            },
        );

        let wl_mode = Mode::from(mode);
        output.change_current_state(Some(wl_mode), None, None, None);
        output.set_preferred(wl_mode);

        output
            .user_data()
            .insert_if_missing(|| TtyOutputState { node, crtc });
        output.user_data().insert_if_missing(|| output_name.clone());
        if let Some(x) = orientation {
            output.user_data().insert_if_missing(|| PanelOrientation(x));
        }

        let render_node = device.render_node.unwrap_or(self.primary_render_node);
        let renderer = self.gpu_manager.single_renderer(&render_node)?;
        let egl_context = renderer.as_ref().egl_context();
        let render_formats = egl_context.dmabuf_render_formats();

        // Filter out the CCS modifiers as they have increased bandwidth, causing some monitor
        // configurations to stop working.
        //
        // For display only devices, restrict to linear buffers for best compatibility.
        //
        // The invalid modifier attempt below should make this unnecessary in some cases, but it
        // would still be a bad idea to remove this until Smithay has some kind of full-device
        // modesetting test that is able to "downgrade" existing connector modifiers to get enough
        // bandwidth for a newly connected one.
        let render_formats = render_formats
            .iter()
            .copied()
            .filter(|format| {
                if device.render_node.is_none() {
                    return format.modifier == Modifier::Linear;
                }

                let is_ccs = matches!(
                    format.modifier,
                    Modifier::I915_y_tiled_ccs
                    // I915_FORMAT_MOD_Yf_TILED_CCS
                    | Modifier::Unrecognized(0x100000000000005)
                    | Modifier::I915_y_tiled_gen12_rc_ccs
                    | Modifier::I915_y_tiled_gen12_mc_ccs
                    // I915_FORMAT_MOD_Y_TILED_GEN12_RC_CCS_CC
                    | Modifier::Unrecognized(0x100000000000008)
                    // I915_FORMAT_MOD_4_TILED_DG2_RC_CCS
                    | Modifier::Unrecognized(0x10000000000000a)
                    // I915_FORMAT_MOD_4_TILED_DG2_MC_CCS
                    | Modifier::Unrecognized(0x10000000000000b)
                    // I915_FORMAT_MOD_4_TILED_DG2_RC_CCS_CC
                    | Modifier::Unrecognized(0x10000000000000c)
                );

                !is_ccs
            })
            .collect::<FormatSet>();

        // Create the compositor.
        let res = DrmCompositor::new(
            OutputModeSource::Auto(output.downgrade()),
            surface,
            None,
            device.allocator.clone(),
            GbmFramebufferExporter::new(device.gbm.clone(), device.render_node.into()),
            SUPPORTED_COLOR_FORMATS,
            // This is only used to pick a good internal format, so it can use the surface's render
            // formats, even though we only ever render on the primary GPU.
            render_formats.clone(),
            device.drm.cursor_size(),
            Some(device.gbm.clone()),
        );

        let mut compositor = match res {
            Ok(x) => x,
            Err(err) => {
                warn!("error creating DRM compositor, will try with invalid modifier: {err:?}");

                let render_formats = render_formats
                    .iter()
                    .copied()
                    .filter(|format| format.modifier == Modifier::Invalid)
                    .collect::<FormatSet>();

                // DrmCompositor::new() consumed the surface...
                let surface = device
                    .drm
                    .create_surface(crtc, mode, &[connector.handle()])?;

                DrmCompositor::new(
                    OutputModeSource::Auto(output.downgrade()),
                    surface,
                    None,
                    device.allocator.clone(),
                    GbmFramebufferExporter::new(device.gbm.clone(), device.render_node.into()),
                    SUPPORTED_COLOR_FORMATS,
                    render_formats,
                    device.drm.cursor_size(),
                    Some(device.gbm.clone()),
                )
                .context("error creating DRM compositor")?
            }
        };

        if self.debug_tint {
            compositor.set_debug_flags(DebugFlags::TINT);
        }

        let mut dmabuf_feedback = None;
        if let Ok(primary_renderer) = self.gpu_manager.single_renderer(&self.primary_render_node) {
            let primary_formats = primary_renderer.dmabuf_formats();

            match surface_dmabuf_feedback(
                &compositor,
                primary_formats,
                self.primary_render_node,
                device.render_node,
                node,
            ) {
                Ok(feedback) => {
                    dmabuf_feedback = Some(feedback);
                }
                Err(err) => {
                    warn!("error building dmabuf feedback: {err:?}");
                }
            }
        }

        // Some buggy monitors replug upon powering off, so powering on here would prevent such
        // monitors from powering off. Therefore, we avoid unconditionally powering on.
        if !niri.monitors_active {
            if let Err(err) = compositor.clear() {
                warn!("error clearing drm surface: {err:?}");
            }
        }

        let vrr_enabled = compositor.vrr_enabled();

        let vblank_frame_name =
            tracy_client::FrameName::new_leak(format!("vblank on {connector_name}"));
        let time_since_presentation_plot_name = tracy_client::PlotName::new_leak(format!(
            "{connector_name} time since presentation, ms"
        ));
        let presentation_misprediction_plot_name = tracy_client::PlotName::new_leak(format!(
            "{connector_name} presentation misprediction, ms"
        ));
        let sequence_delta_plot_name =
            tracy_client::PlotName::new_leak(format!("{connector_name} sequence delta"));

        let surface = Surface {
            name: output_name,
            connector: connector.handle(),
            compositor,
            dmabuf_feedback,
            gamma_props,
            pending_gamma_change: None,
            vblank_frame: None,
            vblank_frame_name,
            time_since_presentation_plot_name,
            presentation_misprediction_plot_name,
            sequence_delta_plot_name,
        };

        let res = device.surfaces.insert(crtc, surface);
        assert!(res.is_none(), "crtc must not have already existed");

        niri.add_output(output.clone(), Some(refresh_interval(mode)), vrr_enabled);

        if niri.monitors_active {
            // Redraw the new monitor.
            niri.event_loop.insert_idle(move |state| {
                // Guard against output disconnecting before the idle has a chance to run.
                if state.niri.output_state.contains_key(&output) {
                    state.niri.queue_redraw(&output);
                }
            });
        }

        Ok(())
    }

    fn connector_disconnected(&mut self, niri: &mut Niri, node: DrmNode, crtc: crtc::Handle) {
        let Some(device) = self.devices.get_mut(&node) else {
            debug!("disconnecting connector for crtc: {crtc:?}");
            error!("missing device");
            return;
        };

        let Some(surface) = device.surfaces.remove(&crtc) else {
            debug!("disconnecting connector for crtc: {crtc:?}");

            if let Some((conn, _)) = device
                .non_desktop_connectors
                .iter()
                .find(|(_, crtc_)| *crtc_ == crtc)
            {
                debug!("withdrawing non-desktop connector from DRM leasing");

                let conn = *conn;
                device.non_desktop_connectors.remove(&(conn, crtc));

                if let Some(lease_state) = &mut device.drm_lease_state {
                    lease_state.withdraw_connector(conn);
                }
            } else {
                debug!("crtc wasn't enabled");
            }

            return;
        };

        debug!("disconnecting connector: {:?}", surface.name.connector);

        let output = niri
            .global_space
            .outputs()
            .find(|output| {
                let tty_state: &TtyOutputState = output.user_data().get().unwrap();
                tty_state.node == node && tty_state.crtc == crtc
            })
            .cloned();
        if let Some(output) = output {
            niri.remove_output(&output);
        } else {
            error!("missing output for crtc {crtc:?}");
        };
    }

    fn on_vblank(
        &mut self,
        niri: &mut Niri,
        node: DrmNode,
        crtc: crtc::Handle,
        meta: DrmEventMetadata,
    ) {
        let span = tracy_client::span!("Tty::on_vblank");

        let now = get_monotonic_time();

        let Some(device) = self.devices.get_mut(&node) else {
            // I've seen it happen.
            error!("missing device in vblank callback for crtc {crtc:?}");
            return;
        };

        let Some(surface) = device.surfaces.get_mut(&crtc) else {
            error!("missing surface in vblank callback for crtc {crtc:?}");
            return;
        };

        // Finish the Tracy frame, if any.
        drop(surface.vblank_frame.take());

        let name = &surface.name.connector;
        trace!("vblank on {name} {meta:?}");
        span.emit_text(name);

        let presentation_time = match meta.time {
            DrmEventTime::Monotonic(time) => time,
            DrmEventTime::Realtime(_) => {
                // Not supported.

                // This value will be ignored in the frame clock code.
                Duration::ZERO
            }
        };
        let presentation_time = if niri.config.borrow().debug.emulate_zero_presentation_time {
            Duration::ZERO
        } else {
            presentation_time
        };

        let message = if presentation_time.is_zero() {
            format!("vblank on {name}, presentation time unknown")
        } else if presentation_time > now {
            let diff = presentation_time - now;
            tracy_client::Client::running().unwrap().plot(
                surface.time_since_presentation_plot_name,
                -diff.as_secs_f64() * 1000.,
            );
            format!("vblank on {name}, presentation is {diff:?} later")
        } else {
            let diff = now - presentation_time;
            tracy_client::Client::running().unwrap().plot(
                surface.time_since_presentation_plot_name,
                diff.as_secs_f64() * 1000.,
            );
            format!("vblank on {name}, presentation was {diff:?} ago")
        };
        tracy_client::Client::running()
            .unwrap()
            .message(&message, 0);

        let Some(output) = niri
            .global_space
            .outputs()
            .find(|output| {
                let tty_state: &TtyOutputState = output.user_data().get().unwrap();
                tty_state.node == node && tty_state.crtc == crtc
            })
            .cloned()
        else {
            error!("missing output in global space for {name}");
            return;
        };

        let Some(output_state) = niri.output_state.get_mut(&output) else {
            error!("missing output state for {name}");
            return;
        };

        let refresh_interval = output_state.frame_clock.refresh_interval();

        let time = if presentation_time.is_zero() {
            now
        } else {
            presentation_time
        };

        if output_state
            .vblank_throttle
            .throttle(refresh_interval, time, move |state| {
                let meta = DrmEventMetadata {
                    sequence: meta.sequence,
                    time: DrmEventTime::Monotonic(Duration::ZERO),
                };

                let tty = state.backend.tty();
                tty.on_vblank(&mut state.niri, node, crtc, meta);
            })
        {
            // Throttled.
            return;
        }

        let redraw_needed = match mem::replace(&mut output_state.redraw_state, RedrawState::Idle) {
            RedrawState::WaitingForVBlank { redraw_needed } => redraw_needed,
            state @ (RedrawState::Idle
            | RedrawState::Queued
            | RedrawState::WaitingForEstimatedVBlank(_)
            | RedrawState::WaitingForEstimatedVBlankAndQueued(_)) => {
                // This is an error!() because it shouldn't happen, but on some systems it somehow
                // does. Kernel sending rogue vblank events?
                //
                // https://github.com/niri-wm/niri/issues/556
                // https://github.com/niri-wm/niri/issues/615
                error!(
                    "unexpected redraw state for output {name} (should be WaitingForVBlank); \
                     can happen when resuming from sleep or powering on monitors: {state:?}"
                );
                true
            }
        };

        // Mark the last frame as submitted.
        match surface.compositor.frame_submitted() {
            Ok(Some((mut feedback, target_presentation_time))) => {
                let refresh = match refresh_interval {
                    Some(refresh) => {
                        if output_state.frame_clock.vrr() {
                            Refresh::Variable(refresh)
                        } else {
                            Refresh::Fixed(refresh)
                        }
                    }
                    None => Refresh::Unknown,
                };

                // FIXME: ideally should be monotonically increasing for a surface.
                let seq = meta.sequence as u64;
                let mut flags = wp_presentation_feedback::Kind::Vsync
                    | wp_presentation_feedback::Kind::HwCompletion;

                if !presentation_time.is_zero() {
                    flags.insert(wp_presentation_feedback::Kind::HwClock);
                }

                feedback.presented::<_, smithay::utils::Monotonic>(time, refresh, seq, flags);

                if !presentation_time.is_zero() {
                    let misprediction_s =
                        presentation_time.as_secs_f64() - target_presentation_time.as_secs_f64();
                    tracy_client::Client::running().unwrap().plot(
                        surface.presentation_misprediction_plot_name,
                        misprediction_s * 1000.,
                    );
                }
            }
            Ok(None) => (),
            Err(err) => {
                warn!("error marking frame as submitted: {err}");
            }
        }

        if let Some(last_sequence) = output_state.last_drm_sequence {
            let delta = meta.sequence as f64 - last_sequence as f64;
            tracy_client::Client::running()
                .unwrap()
                .plot(surface.sequence_delta_plot_name, delta);
        }
        output_state.last_drm_sequence = Some(meta.sequence);

        output_state.frame_clock.presented(presentation_time);

        if redraw_needed || output_state.unfinished_animations_remain {
            let vblank_frame = tracy_client::Client::running()
                .unwrap()
                .non_continuous_frame(surface.vblank_frame_name);
            surface.vblank_frame = Some(vblank_frame);

            niri.queue_redraw(&output);
        } else {
            niri.send_frame_callbacks(&output);
        }
    }

    fn on_estimated_vblank_timer(&self, niri: &mut Niri, output: Output) {
        let span = tracy_client::span!("Tty::on_estimated_vblank_timer");

        let name = output.name();
        span.emit_text(&name);

        let Some(output_state) = niri.output_state.get_mut(&output) else {
            error!("missing output state for {name}");
            return;
        };

        // We waited for the timer, now we can send frame callbacks again.
        output_state.frame_callback_sequence = output_state.frame_callback_sequence.wrapping_add(1);

        match mem::replace(&mut output_state.redraw_state, RedrawState::Idle) {
            RedrawState::Idle => unreachable!(),
            RedrawState::Queued => unreachable!(),
            RedrawState::WaitingForVBlank { .. } => unreachable!(),
            RedrawState::WaitingForEstimatedVBlank(_) => (),
            // The timer fired just in front of a redraw.
            RedrawState::WaitingForEstimatedVBlankAndQueued(_) => {
                output_state.redraw_state = RedrawState::Queued;
                return;
            }
        }

        if output_state.unfinished_animations_remain {
            niri.queue_redraw(&output);
        } else {
            niri.send_frame_callbacks(&output);
        }
    }

    pub fn seat_name(&self) -> String {
        self.session.seat()
    }

    pub fn with_primary_renderer<T>(
        &mut self,
        f: impl FnOnce(&mut GlesRenderer) -> T,
    ) -> Option<T> {
        let mut renderer = self
            .gpu_manager
            .single_renderer(&self.primary_render_node)
            .ok()?;
        Some(f(renderer.as_gles_renderer()))
    }

    pub fn render(
        &mut self,
        niri: &mut Niri,
        output: &Output,
        target_presentation_time: Duration,
    ) -> RenderResult {
        let span = tracy_client::span!("Tty::render");

        let mut rv = RenderResult::Skipped;

        let tty_state: &TtyOutputState = output.user_data().get().unwrap();
        let Some(device) = self.devices.get_mut(&tty_state.node) else {
            error!("missing output device");
            return rv;
        };

        let Some(surface) = device.surfaces.get_mut(&tty_state.crtc) else {
            error!("missing surface");
            return rv;
        };

        span.emit_text(&surface.name.connector);

        if !device.drm.is_active() {
            // This branch hits any time we try to render while the user had switched to a
            // different VT, so don't print anything here.
            return rv;
        }

        let mut renderer = match self.gpu_manager.renderer(
            &self.primary_render_node,
            &device.render_node.unwrap_or(self.primary_render_node),
            surface.compositor.format(),
        ) {
            Ok(renderer) => renderer,
            Err(err) => {
                warn!("error creating renderer for primary GPU: {err:?}");
                return rv;
            }
        };

        // Render the elements.
        let ctx = RenderCtx {
            renderer: &mut renderer,
            target: RenderTarget::Output,
            xray: None,
        };
        let mut elements = niri.render_to_vec(ctx, output, true);

        // Visualize the damage, if enabled.
        if niri.debug_draw_damage {
            let output_state = niri.output_state.get_mut(output).unwrap();
            draw_damage(&mut output_state.debug_damage_tracker, &mut elements);
        }

        // Overlay planes are disabled by default as they cause weird performance issues on my
        // system.
        let flags = {
            let debug = &self.config.borrow().debug;

            let primary_scanout_flag = if debug.restrict_primary_scanout_to_matching_format {
                FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT
            } else {
                FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY
            };
            let mut flags = primary_scanout_flag | FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT;

            if debug.enable_overlay_planes {
                flags.insert(FrameFlags::ALLOW_OVERLAY_PLANE_SCANOUT);
            }
            if debug.disable_direct_scanout {
                flags.remove(primary_scanout_flag);
                flags.remove(FrameFlags::ALLOW_OVERLAY_PLANE_SCANOUT);
            }
            if debug.disable_cursor_plane {
                flags.remove(FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT);
            }
            if debug.skip_cursor_only_updates_during_vrr {
                let output_state = niri.output_state.get(output).unwrap();
                if output_state.frame_clock.vrr() {
                    flags.insert(FrameFlags::SKIP_CURSOR_ONLY_UPDATES);
                }
            }

            flags
        };

        // Hand them over to the DRM.
        let drm_compositor = &mut surface.compositor;
        match drm_compositor.render_frame::<_, _>(&mut renderer, &elements, [0.; 4], flags) {
            Ok(res) => {
                let needs_sync = res.needs_sync()
                    || self
                        .config
                        .borrow()
                        .debug
                        .wait_for_frame_completion_before_queueing;
                if needs_sync {
                    if let PrimaryPlaneElement::Swapchain(element) = res.primary_element {
                        let _span = tracy_client::span!("wait for completion");
                        if let Err(err) = element.sync.wait() {
                            warn!("error waiting for frame completion: {err:?}");
                        }
                    }
                }

                niri.update_primary_scanout_output(output, &res.states);
                if let Some(dmabuf_feedback) = surface.dmabuf_feedback.as_ref() {
                    niri.send_dmabuf_feedbacks(output, dmabuf_feedback, &res.states);
                }

                if !res.is_empty {
                    let presentation_feedbacks =
                        niri.take_presentation_feedbacks(output, &res.states);
                    let data = (presentation_feedbacks, target_presentation_time);

                    match drm_compositor.queue_frame(data) {
                        Ok(()) => {
                            let output_state = niri.output_state.get_mut(output).unwrap();
                            let new_state = RedrawState::WaitingForVBlank {
                                redraw_needed: false,
                            };
                            match mem::replace(&mut output_state.redraw_state, new_state) {
                                RedrawState::Idle => unreachable!(),
                                RedrawState::Queued => (),
                                RedrawState::WaitingForVBlank { .. } => unreachable!(),
                                RedrawState::WaitingForEstimatedVBlank(_) => unreachable!(),
                                RedrawState::WaitingForEstimatedVBlankAndQueued(token) => {
                                    niri.event_loop.remove(token);
                                }
                            };

                            // We queued this frame successfully, so the current client buffers were
                            // latched. We can send frame callbacks now, since a new client commit
                            // will no longer overwrite this frame and will wait for a VBlank.
                            output_state.frame_callback_sequence =
                                output_state.frame_callback_sequence.wrapping_add(1);

                            return RenderResult::Submitted;
                        }
                        Err(err) => {
                            warn!("error queueing frame: {err}");
                        }
                    }
                } else {
                    rv = RenderResult::NoDamage;
                }
            }
            Err(err) => {
                // Can fail if we switched to a different TTY.
                warn!("error rendering frame: {err}");
            }
        }

        // We're not expecting a vblank right after this.
        drop(surface.vblank_frame.take());

        // Queue a timer to fire at the predicted vblank time.
        queue_estimated_vblank_timer(niri, output.clone(), target_presentation_time);

        rv
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Err(err) = self.session.change_vt(vt) {
            warn!("error changing VT: {err}");
        }
    }

    pub fn suspend(&self) {
        #[cfg(feature = "dbus")]
        if let Err(err) = suspend() {
            warn!("error suspending: {err:?}");
        }
    }

    pub fn toggle_debug_tint(&mut self) {
        self.debug_tint = !self.debug_tint;

        for device in self.devices.values_mut() {
            for surface in device.surfaces.values_mut() {
                let compositor = &mut surface.compositor;

                let mut flags = compositor.debug_flags();
                flags.set(DebugFlags::TINT, self.debug_tint);
                compositor.set_debug_flags(flags);
            }
        }
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        let mut renderer = match self.gpu_manager.single_renderer(&self.primary_render_node) {
            Ok(renderer) => renderer,
            Err(err) => {
                debug!("error creating renderer for primary GPU: {err:?}");
                return false;
            }
        };

        match renderer.import_dmabuf(dmabuf, None) {
            Ok(_texture) => {
                if dmabuf.node().is_none() {
                    dmabuf.set_node(Some(self.primary_render_node));
                }
                true
            }
            Err(err) => {
                debug!("error importing dmabuf: {err:?}");
                false
            }
        }
    }

    pub fn early_import(&mut self, surface: &WlSurface) {
        if let Err(err) = self.gpu_manager.early_import(
            // We always render on the primary GPU.
            self.primary_render_node,
            surface,
        ) {
            warn!("error doing early import: {err:?}");
        }
    }

    pub fn get_gamma_size(&self, output: &Output) -> anyhow::Result<u32> {
        let tty_state = output.user_data().get::<TtyOutputState>().unwrap();
        let crtc = tty_state.crtc;

        let device = self
            .devices
            .get(&tty_state.node)
            .context("missing device")?;

        let surface = device.surfaces.get(&crtc).context("missing surface")?;
        if let Some(gamma_props) = &surface.gamma_props {
            gamma_props.gamma_size(&device.drm)
        } else {
            let info = device
                .drm
                .get_crtc(crtc)
                .context("error getting crtc info")?;
            Ok(info.gamma_length())
        }
    }

    pub fn set_gamma(&mut self, output: &Output, ramp: Option<Vec<u16>>) -> anyhow::Result<()> {
        let tty_state = output.user_data().get::<TtyOutputState>().unwrap();
        let crtc = tty_state.crtc;

        let device = self
            .devices
            .get_mut(&tty_state.node)
            .context("missing device")?;
        let surface = device.surfaces.get_mut(&crtc).context("missing surface")?;

        // Cannot change properties while the device is inactive.
        if !self.session.is_active() {
            surface.pending_gamma_change = Some(ramp);
            return Ok(());
        }

        let ramp = ramp.as_deref();
        if let Some(gamma_props) = &mut surface.gamma_props {
            gamma_props.set_gamma(&device.drm, ramp)
        } else {
            set_gamma_for_crtc(&device.drm, crtc, ramp)
        }
    }

    fn refresh_ipc_outputs(&self, niri: &mut Niri) {
        let _span = tracy_client::span!("Tty::refresh_ipc_outputs");

        let mut ipc_outputs = HashMap::new();
        let disable_monitor_names = self.config.borrow().debug.disable_monitor_names;

        for (node, device) in &self.devices {
            for (connector, crtc) in device.drm_scanner.crtcs() {
                let connector_name = format_connector_name(connector);
                let physical_size = connector.size();
                let output_name = device.known_crtc_name(&crtc, connector, disable_monitor_names);

                let surface = device.surfaces.get(&crtc);
                let current_crtc_mode = surface.map(|surface| surface.compositor.pending_mode());
                let mut current_mode = None;
                let mut is_custom_mode = false;

                let mut modes: Vec<niri_ipc::Mode> = connector
                    .modes()
                    .iter()
                    .filter(|m| !m.flags().contains(ModeFlags::INTERLACE))
                    .enumerate()
                    .map(|(idx, m)| {
                        if Some(*m) == current_crtc_mode {
                            current_mode = Some(idx);
                        }

                        niri_ipc::Mode {
                            width: m.size().0,
                            height: m.size().1,
                            refresh_rate: Mode::from(*m).refresh as u32,
                            is_preferred: m.mode_type().contains(ModeTypeFlags::PREFERRED),
                        }
                    })
                    .collect();

                if let Some(crtc_mode) = current_crtc_mode {
                    // Custom mode
                    if crtc_mode.mode_type().contains(ModeTypeFlags::USERDEF) {
                        modes.insert(
                            0,
                            niri_ipc::Mode {
                                width: crtc_mode.size().0,
                                height: crtc_mode.size().1,
                                refresh_rate: Mode::from(crtc_mode).refresh as u32,
                                is_preferred: false,
                            },
                        );
                        current_mode = Some(0);
                        is_custom_mode = true;
                    }

                    if current_mode.is_none() {
                        if crtc_mode.flags().contains(ModeFlags::INTERLACE) {
                            warn!("connector mode list missing current mode (interlaced)");
                        } else {
                            error!("connector mode list missing current mode");
                        }
                    }
                }

                let vrr_supported = surface
                    .map(|surface| {
                        matches!(
                            surface.compositor.vrr_supported(connector.handle()),
                            Ok(VrrSupport::Supported | VrrSupport::RequiresModeset)
                        )
                    })
                    .unwrap_or_else(|| {
                        is_vrr_capable(&device.drm, connector.handle()) == Some(true)
                    });
                let vrr_enabled = surface.is_some_and(|surface| surface.compositor.vrr_enabled());

                let logical = niri
                    .global_space
                    .outputs()
                    .find(|output| {
                        let tty_state: &TtyOutputState = output.user_data().get().unwrap();
                        tty_state.node == *node && tty_state.crtc == crtc
                    })
                    .map(logical_output);

                let id = device.known_crtcs.get(&crtc).map(|info| info.id);
                let id = id.unwrap_or_else(|| {
                    error!("crtc for connector {connector_name} missing from known");
                    OutputId::next()
                });

                let props = ConnectorProperties::try_new(&device.drm, connector.handle()).ok();
                let max_bpc = props.as_ref().and_then(|p| p.find(c"max bpc").ok());
                let max_bpc = max_bpc.and_then(|(info, value)| {
                    info.value_type()
                        .convert_value(*value)
                        .as_unsigned_range()
                        .map(|v| v as u8)
                });

                let ipc_output = niri_ipc::Output {
                    name: connector_name,
                    make: output_name.make.unwrap_or_else(|| "Unknown".into()),
                    model: output_name.model.unwrap_or_else(|| "Unknown".into()),
                    serial: output_name.serial,
                    physical_size,
                    modes,
                    current_mode,
                    is_custom_mode,
                    vrr_supported,
                    vrr_enabled,
                    logical,
                    max_bpc,
                };

                ipc_outputs.insert(id, ipc_output);
            }
        }

        let mut guard = self.ipc_outputs.lock().unwrap();
        *guard = ipc_outputs;
        niri.ipc_outputs_changed = true;
    }

    pub fn ipc_outputs(&self) -> Arc<Mutex<IpcOutputMap>> {
        self.ipc_outputs.clone()
    }

    #[cfg(feature = "xdp-gnome-screencast")]
    pub fn primary_gbm_device(&self) -> Option<GbmDevice<DrmDeviceFd>> {
        // Try to find a device corresponding to the primary render node.
        let device = self
            .devices
            .values()
            .find(|d| d.render_node == Some(self.primary_render_node));
        // Otherwise, try to get the device corresponding to the primary node.
        let device = device.or_else(|| self.devices.get(&self.primary_node));

        Some(device?.gbm.clone())
    }

    pub fn set_monitors_active(&mut self, active: bool) {
        // We only disable the CRTC here, this will also reset the
        // surface state so that the next call to `render_frame` will
        // always produce a new frame and `queue_frame` will change
        // the CRTC to active. This makes sure we always enable a CRTC
        // within an atomic operation.
        if active {
            return;
        }

        for device in self.devices.values_mut() {
            for surface in device.surfaces.values_mut() {
                if let Err(err) = surface.compositor.clear() {
                    warn!("error clearing drm surface: {err:?}");
                }
            }
        }
    }

    pub fn set_output_on_demand_vrr(&mut self, niri: &mut Niri, output: &Output, enable_vrr: bool) {
        let _span = tracy_client::span!("Tty::set_output_on_demand_vrr");

        let output_state = niri.output_state.get_mut(output).unwrap();
        output_state.on_demand_vrr_enabled = enable_vrr;
        if output_state.frame_clock.vrr() == enable_vrr {
            return;
        }
        for (&node, device) in self.devices.iter_mut() {
            for (&crtc, surface) in device.surfaces.iter_mut() {
                let tty_state: &TtyOutputState = output.user_data().get().unwrap();
                if tty_state.node == node && tty_state.crtc == crtc {
                    let word = if enable_vrr { "enabling" } else { "disabling" };
                    if let Err(err) = surface.compositor.use_vrr(enable_vrr) {
                        warn!(
                            "output {:?}: error {} VRR: {err:?}",
                            surface.name.connector, word
                        );
                    }
                    output_state
                        .frame_clock
                        .set_vrr(surface.compositor.vrr_enabled());

                    self.refresh_ipc_outputs(niri);
                    return;
                }
            }
        }
    }

    fn compute_ignored_nodes(&self) -> HashSet<DrmNode> {
        let mut ignored_nodes = ignored_nodes_from_config(&self.config.borrow());
        if ignored_nodes.remove(&self.primary_node)
            || ignored_nodes.remove(&self.primary_render_node)
        {
            warn!("ignoring the primary node or render node is not allowed");
        }
        ignored_nodes
    }

    pub fn update_ignored_nodes_config(&mut self, niri: &mut Niri) {
        let _span = tracy_client::span!("Tty::update_ignored_nodes_config");

        // If we're inactive, we can't do anything, but we'll recompute in ActivateSession.
        if !self.session.is_active() {
            return;
        }

        let ignored_nodes = self.compute_ignored_nodes();
        if ignored_nodes == self.ignored_nodes {
            return;
        }
        self.ignored_nodes = ignored_nodes;

        let mut device_list = self
            .udev_dispatcher
            .as_source_ref()
            .device_list()
            .map(|(device_id, path)| (device_id, path.to_owned()))
            .collect::<HashMap<_, _>>();

        let removed_devices = self
            .devices
            .keys()
            .filter(|node| {
                self.ignored_nodes.contains(node) || !device_list.contains_key(&node.dev_id())
            })
            .copied()
            .collect::<Vec<_>>();

        for node in removed_devices {
            device_list.remove(&node.dev_id());
            self.device_removed(node.dev_id(), niri);
        }

        for node in self.devices.keys() {
            device_list.remove(&node.dev_id());
        }

        for (device_id, path) in device_list {
            if let Err(err) = self.device_added(device_id, &path, niri, false) {
                warn!("error adding device {path:?}: {err:?}");
            }
        }
    }

    fn should_disable_laptop_panels(&self, is_lid_closed: bool) -> bool {
        if !is_lid_closed {
            return false;
        }

        let config = self.config.borrow();
        if !config.debug.keep_laptop_panel_on_when_lid_is_closed {
            // Check if any external monitor is connected.
            for device in self.devices.values() {
                for (connector, _crtc) in device.drm_scanner.crtcs() {
                    if !is_laptop_panel(&format_connector_name(connector)) {
                        return true;
                    }
                }
            }
        }

        false
    }

    pub fn on_output_config_changed(&mut self, niri: &mut Niri) {
        let _span = tracy_client::span!("Tty::on_output_config_changed");

        // If we're inactive, we can't do anything, so just set a flag for later.
        if !self.session.is_active() {
            self.update_output_config_on_resume = true;
            return;
        }
        self.update_output_config_on_resume = false;

        // Figure out if we should disable laptop panels.
        let disable_laptop_panels = self.should_disable_laptop_panels(niri.is_lid_closed);
        let should_disable = |connector: &str| disable_laptop_panels && is_laptop_panel(connector);

        let mut to_disconnect = vec![];
        let mut to_connect = vec![];

        for (&node, device) in &mut self.devices {
            for (&crtc, surface) in device.surfaces.iter_mut() {
                let config = self
                    .config
                    .borrow()
                    .outputs
                    .find(&surface.name)
                    .cloned()
                    .unwrap_or_default();
                if config.off || should_disable(&surface.name.connector) {
                    to_disconnect.push((node, crtc));
                    continue;
                }

                // Check if we need to change the mode.
                let Some(connector) = device.drm_scanner.connectors().get(&surface.connector)
                else {
                    error!("missing enabled connector in drm_scanner");
                    continue;
                };

                let mut mode = None;
                if let Some(modeline) = &config.modeline {
                    match calculate_drm_mode_from_modeline(modeline) {
                        Ok(x) => mode = Some(x),
                        Err(err) => {
                            warn!(
                                "output {:?}: invalid custom modeline; \
                                 falling back to advertised modes: {err:?}",
                                surface.name.connector
                            );
                        }
                    }
                }

                let (mode, fallback) = match mode {
                    Some(x) => (x, false),
                    None => match pick_mode(connector, config.mode) {
                        Some(result) => result,
                        None => {
                            warn!("couldn't pick mode for enabled connector");
                            continue;
                        }
                    },
                };

                if let Ok(mut props) = ConnectorProperties::try_new(&device.drm, surface.connector)
                {
                    set_connector_properties(&mut props, config.max_bpc, false);
                } else {
                    warn!("failed to get connector properties");
                }

                let change_mode = surface.compositor.pending_mode() != mode;

                let vrr_enabled = surface.compositor.vrr_enabled();
                let change_always_vrr = vrr_enabled != config.is_vrr_always_on();
                let is_on_demand_vrr = config.is_vrr_on_demand();

                if !change_mode && !change_always_vrr && !is_on_demand_vrr {
                    continue;
                }

                let output = niri
                    .global_space
                    .outputs()
                    .find(|output| {
                        let tty_state: &TtyOutputState = output.user_data().get().unwrap();
                        tty_state.node == node && tty_state.crtc == crtc
                    })
                    .cloned();
                let Some(output) = output else {
                    error!("missing output for crtc: {crtc:?}");
                    continue;
                };
                let Some(output_state) = niri.output_state.get_mut(&output) else {
                    error!("missing state for output {:?}", surface.name.connector);
                    continue;
                };

                if (is_on_demand_vrr && vrr_enabled != output_state.on_demand_vrr_enabled)
                    || (!is_on_demand_vrr && change_always_vrr)
                {
                    let vrr = !vrr_enabled;
                    let word = if vrr { "enabling" } else { "disabling" };
                    if let Err(err) = surface.compositor.use_vrr(vrr) {
                        warn!(
                            "output {:?}: error {} VRR: {err:?}",
                            surface.name.connector, word
                        );
                    }
                    output_state
                        .frame_clock
                        .set_vrr(surface.compositor.vrr_enabled());
                }

                if change_mode {
                    if fallback {
                        let target = config.mode.unwrap();
                        warn!(
                            "output {:?}: configured mode {}x{}{} could not be found, \
                             falling back to preferred",
                            surface.name.connector,
                            target.mode.width,
                            target.mode.height,
                            if let Some(refresh) = target.mode.refresh {
                                format!("@{refresh}")
                            } else {
                                String::new()
                            },
                        );
                    }

                    debug!(
                        "output {:?}: picking mode: {mode:?}",
                        surface.name.connector
                    );
                    if let Err(err) = surface.compositor.use_mode(mode) {
                        warn!("error changing mode: {err:?}");
                        continue;
                    }

                    let wl_mode = Mode::from(mode);
                    output.change_current_state(Some(wl_mode), None, None, None);
                    output.set_preferred(wl_mode);
                    output_state.frame_clock = FrameClock::new(
                        Some(refresh_interval(mode)),
                        surface.compositor.vrr_enabled(),
                    );
                    niri.output_resized(&output);
                }
            }

            let config = self.config.borrow();
            let disable_monitor_names = config.debug.disable_monitor_names;

            for (connector, crtc) in device.drm_scanner.crtcs() {
                // Check if connected.
                if connector.state() != connector::State::Connected {
                    continue;
                }

                // Check if already enabled.
                if device.surfaces.contains_key(&crtc)
                    || device
                        .non_desktop_connectors
                        .contains(&(connector.handle(), crtc))
                {
                    continue;
                }

                let output_name = device.known_crtc_name(&crtc, connector, disable_monitor_names);

                let config = config
                    .outputs
                    .find(&output_name)
                    .cloned()
                    .unwrap_or_default();

                if !(config.off || should_disable(&output_name.connector)) {
                    to_connect.push((node, connector.clone(), crtc, output_name));
                }
            }
        }

        for (node, crtc) in to_disconnect {
            self.connector_disconnected(niri, node, crtc);
        }

        // Sort by output name to get more predictable first focused output at initial compositor
        // startup, when multiple connectors appear at once.
        to_connect.sort_unstable_by(|a, b| a.3.compare(&b.3));

        let mut gpu_topology_changed = false;
        let mut failed_resume_nodes = HashSet::new();

        for (node, connector, crtc, _name) in to_connect {
            if failed_resume_nodes.contains(&node) {
                continue;
            }

            // Smithay wants the renderer back before we touch connectors.
            match self.resume_device(niri, node, false) {
                Ok(true) => gpu_topology_changed = true,
                Ok(false) => {}
                Err(_) => {
                    failed_resume_nodes.insert(node);
                    continue;
                }
            }

            if let Err(err) = self.connector_connected(niri, node, connector, crtc) {
                warn!("error connecting connector: {err:?}");
            }
        }

        self.refresh_ipc_outputs(niri);

        let nodes: Vec<_> = self.devices.keys().copied().collect();
        for node in nodes {
            let key = self.gpu_power_key(node);
            let disable_suspend = self.config.borrow().debug.disable_idle_drm_device_suspend
                || self
                    .gpu_power
                    .get(&key)
                    .map(|p| p.disable_suspend_due_to_nvidia_vram)
                    .unwrap_or(false);
            if disable_suspend {
                if let Some(power) = self.gpu_power.get_mut(&key) {
                    power.wakeup_failed = false;
                }
                if !failed_resume_nodes.contains(&node) {
                    match self.resume_device(niri, node, false) {
                        Ok(true) => gpu_topology_changed = true,
                        Ok(false) => {}
                        Err(_) => {}
                    }
                }
            } else {
                self.check_suspend_device(niri, node);
            }
        }

        if gpu_topology_changed {
            self.update_dmabuf_feedbacks(niri);
        }
    }

    pub fn get_device_from_node(&mut self, node: DrmNode) -> Option<&mut OutputDevice> {
        self.devices.get_mut(&node)
    }

    pub fn disconnected_connector_name_by_name_match(&self, target: &str) -> Option<OutputName> {
        let disable_monitor_names = self.config.borrow().debug.disable_monitor_names;
        for device in self.devices.values() {
            for (connector, crtc) in device.drm_scanner.crtcs() {
                // Check if connected.
                if connector.state() != connector::State::Connected {
                    continue;
                }

                // Check if already enabled.
                if device.surfaces.contains_key(&crtc)
                    || device
                        .non_desktop_connectors
                        .contains(&(connector.handle(), crtc))
                {
                    continue;
                }

                let output_name = device.known_crtc_name(&crtc, connector, disable_monitor_names);
                if output_name.matches(target) {
                    return Some(output_name);
                }
            }
        }

        None
    }
}

impl GammaProps {
    fn new(device: &DrmDevice, crtc: crtc::Handle) -> anyhow::Result<Self> {
        let mut gamma_lut = None;
        let mut gamma_lut_size = None;

        let props = device
            .get_properties(crtc)
            .context("error getting properties")?;
        for (prop, _) in props {
            let Ok(info) = device.get_property(prop) else {
                continue;
            };

            let Ok(name) = info.name().to_str() else {
                continue;
            };

            match name {
                "GAMMA_LUT" => {
                    ensure!(
                        matches!(info.value_type(), property::ValueType::Blob),
                        "wrong GAMMA_LUT value type"
                    );
                    gamma_lut = Some(prop);
                }
                "GAMMA_LUT_SIZE" => {
                    ensure!(
                        matches!(info.value_type(), property::ValueType::UnsignedRange(_, _)),
                        "wrong GAMMA_LUT_SIZE value type"
                    );
                    gamma_lut_size = Some(prop);
                }
                _ => (),
            }
        }

        let gamma_lut = gamma_lut.context("missing GAMMA_LUT property")?;
        let gamma_lut_size = gamma_lut_size.context("missing GAMMA_LUT_SIZE property")?;

        Ok(Self {
            crtc,
            gamma_lut,
            gamma_lut_size,
            previous_blob: None,
        })
    }

    fn gamma_size(&self, device: &DrmDevice) -> anyhow::Result<u32> {
        let value = get_drm_property(device, self.crtc, self.gamma_lut_size)
            .context("missing GAMMA_LUT_SIZE property")?;
        Ok(value as u32)
    }

    fn set_gamma(&mut self, device: &DrmDevice, gamma: Option<&[u16]>) -> anyhow::Result<()> {
        let _span = tracy_client::span!("GammaProps::set_gamma");

        let blob = if let Some(gamma) = gamma {
            let gamma_size = self
                .gamma_size(device)
                .context("error getting gamma size")? as usize;

            ensure!(gamma.len() == gamma_size * 3, "wrong gamma length");

            #[allow(non_camel_case_types)]
            #[repr(C)]
            #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
            pub struct drm_color_lut {
                pub red: u16,
                pub green: u16,
                pub blue: u16,
                pub reserved: u16,
            }

            let (red, rest) = gamma.split_at(gamma_size);
            let (blue, green) = rest.split_at(gamma_size);
            let mut data = zip(zip(red, blue), green)
                .map(|((&red, &green), &blue)| drm_color_lut {
                    red,
                    green,
                    blue,
                    reserved: 0,
                })
                .collect::<Vec<_>>();
            let data = cast_slice_mut(&mut data);

            let blob = drm_ffi::mode::create_property_blob(device.as_fd(), data)
                .context("error creating property blob")?;
            NonZeroU64::new(u64::from(blob.blob_id))
        } else {
            None
        };

        {
            let _span = tracy_client::span!("set_property");

            let blob = blob.map(NonZeroU64::get).unwrap_or(0);
            device
                .set_property(
                    self.crtc,
                    self.gamma_lut,
                    property::Value::Blob(blob).into(),
                )
                .context("error setting GAMMA_LUT")
                .inspect_err(|_| {
                    if blob != 0 {
                        // Destroy the blob we just allocated.
                        if let Err(err) = device.destroy_property_blob(blob) {
                            warn!("error destroying GAMMA_LUT property blob: {err:?}");
                        }
                    }
                })?;
        }

        if let Some(blob) = mem::replace(&mut self.previous_blob, blob) {
            if let Err(err) = device.destroy_property_blob(blob.get()) {
                warn!("error destroying previous GAMMA_LUT blob: {err:?}");
            }
        }

        Ok(())
    }

    fn restore_gamma(&self, device: &DrmDevice) -> anyhow::Result<()> {
        let _span = tracy_client::span!("GammaProps::restore_gamma");

        let blob = self.previous_blob.map(NonZeroU64::get).unwrap_or(0);
        device
            .set_property(
                self.crtc,
                self.gamma_lut,
                property::Value::Blob(blob).into(),
            )
            .context("error setting GAMMA_LUT")?;

        Ok(())
    }
}

fn primary_node_from_render_node(path: &Path) -> Option<(DrmNode, DrmNode)> {
    match DrmNode::from_path(path) {
        Ok(node) => {
            if node.ty() == NodeType::Render {
                match node.node_with_type(NodeType::Primary) {
                    Some(Ok(primary_node)) => {
                        return Some((primary_node, node));
                    }
                    Some(Err(err)) => {
                        warn!("error opening primary node for render node {path:?}: {err:?}");
                    }
                    None => {
                        warn!("error opening primary node for render node {path:?}");
                    }
                }
            } else {
                warn!("DRM node {path:?} is not a render node");

                // Gracefully handle misconfiguration on regular desktop systems.
                if let Some(Ok(render_node)) = node.node_with_type(NodeType::Render) {
                    return Some((node, render_node));
                }

                warn!("could not get render node for DRM node {path:?}; proceeding anyway");
                return Some((node, node));
            }
        }
        Err(err) => {
            warn!("error opening {path:?} as DRM node: {err:?}");
        }
    }

    None
}

fn primary_node_from_config(config: &Config) -> Option<(DrmNode, DrmNode)> {
    let path = config.debug.render_drm_device.as_ref()?;
    debug!("attempting to use render node from config: {path:?}");

    primary_node_from_render_node(path)
}

fn ignored_nodes_from_config(config: &Config) -> HashSet<DrmNode> {
    let mut disabled_nodes = HashSet::new();

    for path in &config.debug.ignored_drm_devices {
        if let Some((primary_node, render_node)) = primary_node_from_render_node(path) {
            disabled_nodes.insert(primary_node);
            disabled_nodes.insert(render_node);
        }
    }

    disabled_nodes
}

fn surface_dmabuf_feedback(
    compositor: &GbmDrmCompositor,
    primary_formats: FormatSet,
    primary_render_node: DrmNode,
    surface_render_node: Option<DrmNode>,
    surface_scanout_node: DrmNode,
) -> Result<SurfaceDmabufFeedback, io::Error> {
    let surface = compositor.surface();
    let planes = surface.planes();

    let primary_plane_formats = surface.plane_info().formats.clone();
    let primary_or_overlay_plane_formats = primary_plane_formats
        .iter()
        .chain(planes.overlay.iter().flat_map(|p| p.formats.iter()))
        .copied()
        .collect::<FormatSet>();

    // We limit the scan-out trache to formats we can also render from so that there is always a
    // fallback render path available in case the supplied buffer can not be scanned out directly.
    let mut primary_scanout_formats = primary_plane_formats
        .intersection(&primary_formats)
        .copied()
        .collect::<Vec<_>>();
    let mut primary_or_overlay_scanout_formats = primary_or_overlay_plane_formats
        .intersection(&primary_formats)
        .copied()
        .collect::<Vec<_>>();

    // HACK: AMD iGPU + dGPU systems share some modifiers between the two, and yet cross-device
    // buffers produce a glitched scanout if the modifier is not Linear...
    //
    // Also limit scan-out formats to Linear if we have a device without a render node (i.e.
    // we're rendering on a different device).
    if surface_render_node != Some(primary_render_node) {
        primary_scanout_formats.retain(|f| f.modifier == Modifier::Linear);
        primary_or_overlay_scanout_formats.retain(|f| f.modifier == Modifier::Linear);
    }

    let builder = DmabufFeedbackBuilder::new(primary_render_node.dev_id(), primary_formats);

    trace!(
        "primary scanout formats: {}, overlay adds: {}",
        primary_scanout_formats.len(),
        primary_or_overlay_scanout_formats.len() - primary_scanout_formats.len(),
    );

    // Prefer the primary-plane-only formats, then primary-or-overlay-plane formats. This will
    // increase the chance of scanning out a client even with our disabled-by-default overlay
    // planes.
    let scanout = builder
        .clone()
        .add_preference_tranche(
            surface_scanout_node.dev_id(),
            TrancheFlags::Scanout,
            primary_scanout_formats,
            4..=6,
        )
        .add_preference_tranche(
            surface_scanout_node.dev_id(),
            TrancheFlags::Scanout,
            primary_or_overlay_scanout_formats,
            4..=6,
        )
        .build()?;

    // If this is the primary node surface, send scanout formats in both tranches to avoid
    // duplication.
    let render = if surface_render_node == Some(primary_render_node) {
        scanout.clone()
    } else {
        builder.build()?
    };

    Ok(SurfaceDmabufFeedback { render, scanout })
}

fn find_drm_property(
    drm: &DrmDevice,
    resource: impl ResourceHandle,
    name: &str,
) -> Option<(property::Handle, property::Info, property::RawValue)> {
    let props = match drm.get_properties(resource) {
        Ok(props) => props,
        Err(err) => {
            warn!("error getting properties: {err:?}");
            return None;
        }
    };

    props.into_iter().find_map(|(handle, value)| {
        let info = drm.get_property(handle).ok()?;
        let n = info.name().to_str().ok()?;

        (n == name).then_some((handle, info, value))
    })
}

fn get_drm_property(
    drm: &DrmDevice,
    resource: impl ResourceHandle,
    prop: property::Handle,
) -> Option<property::RawValue> {
    let props = match drm.get_properties(resource) {
        Ok(props) => props,
        Err(err) => {
            warn!("error getting properties: {err:?}");
            return None;
        }
    };

    props
        .into_iter()
        .find_map(|(handle, value)| (handle == prop).then_some(value))
}

fn refresh_interval(mode: DrmMode) -> Duration {
    let clock = mode.clock() as u64;
    let htotal = mode.hsync().2 as u64;
    let vtotal = mode.vsync().2 as u64;

    let mut numerator = htotal * vtotal * 1_000_000;
    let mut denominator = clock;

    if mode.flags().contains(ModeFlags::INTERLACE) {
        denominator *= 2;
    }

    if mode.flags().contains(ModeFlags::DBLSCAN) {
        numerator *= 2;
    }

    if mode.vscan() > 1 {
        numerator *= mode.vscan() as u64;
    }

    let refresh_interval = (numerator + denominator / 2) / denominator;
    Duration::from_nanos(refresh_interval)
}

#[cfg(feature = "dbus")]
fn suspend() -> anyhow::Result<()> {
    let conn = zbus::blocking::Connection::system().context("error connecting to system bus")?;

    conn.call_method(
        Some("org.freedesktop.login1"),
        "/org/freedesktop/login1",
        Some("org.freedesktop.login1.Manager"),
        "Suspend",
        &(true),
    )
    .context("error suspending")?;

    Ok(())
}

fn queue_estimated_vblank_timer(
    niri: &mut Niri,
    output: Output,
    target_presentation_time: Duration,
) {
    let output_state = niri.output_state.get_mut(&output).unwrap();
    match mem::take(&mut output_state.redraw_state) {
        RedrawState::Idle => unreachable!(),
        RedrawState::Queued => (),
        RedrawState::WaitingForVBlank { .. } => unreachable!(),
        RedrawState::WaitingForEstimatedVBlank(token)
        | RedrawState::WaitingForEstimatedVBlankAndQueued(token) => {
            output_state.redraw_state = RedrawState::WaitingForEstimatedVBlank(token);
            return;
        }
    }

    let now = get_monotonic_time();
    let mut duration = target_presentation_time.saturating_sub(now);

    // No use setting a zero timer, since we'll send frame callbacks anyway right after the call to
    // render(). This can happen for example with unknown presentation time from DRM.
    if duration.is_zero() {
        duration += output_state
            .frame_clock
            .refresh_interval()
            // Unknown refresh interval, i.e. winit backend. Would be good to estimate it somehow
            // but it's not that important for this code path.
            .unwrap_or(Duration::from_micros(16_667));
    }

    trace!("queueing estimated vblank timer to fire in {duration:?}");

    let timer = Timer::from_duration(duration);
    let token = niri
        .event_loop
        .insert_source(timer, move |_, _, data| {
            if let Some(tty) = data.backend.tty_checked() {
                tty.on_estimated_vblank_timer(&mut data.niri, output.clone());
            }
            TimeoutAction::Drop
        })
        .unwrap();
    output_state.redraw_state = RedrawState::WaitingForEstimatedVBlank(token);
}

pub fn calculate_drm_mode_from_modeline(modeline: &Modeline) -> anyhow::Result<DrmMode> {
    ensure!(
        modeline.hdisplay < modeline.hsync_start,
        "hdisplay {} must be < hsync_start {}",
        modeline.hdisplay,
        modeline.hsync_start
    );
    ensure!(
        modeline.hsync_start < modeline.hsync_end,
        "hsync_start {} must be < hsync_end {}",
        modeline.hsync_start,
        modeline.hsync_end
    );
    ensure!(
        modeline.hsync_end < modeline.htotal,
        "hsync_end {} must be < htotal {}",
        modeline.hsync_end,
        modeline.htotal
    );
    ensure!(
        modeline.vdisplay < modeline.vsync_start,
        "vdisplay {} must be < vsync_start {}",
        modeline.vdisplay,
        modeline.vsync_start
    );
    ensure!(
        modeline.vsync_start < modeline.vsync_end,
        "vsync_start {} must be < vsync_end {}",
        modeline.vsync_start,
        modeline.vsync_end
    );
    ensure!(
        modeline.vsync_end < modeline.vtotal,
        "vsync_end {} must be < vtotal {}",
        modeline.vsync_end,
        modeline.vtotal
    );

    let pixel_clock_kilo_hertz = modeline.clock * 1000.0;
    // Calculated as documented in the CVT 1.2 standard:
    // https://app.box.com/s/vcocw3z73ta09txiskj7cnk6289j356b/file/93518784646
    let vrefresh_hertz = (pixel_clock_kilo_hertz * 1000.0)
        / (modeline.htotal as u64 * modeline.vtotal as u64) as f64;
    ensure!(
        vrefresh_hertz.is_finite(),
        "calculated refresh rate is not finite"
    );
    let vrefresh_rounded = vrefresh_hertz.round() as u32;

    let flags = match modeline.hsync_polarity {
        HSyncPolarity::PHSync => ModeFlags::PHSYNC,
        HSyncPolarity::NHSync => ModeFlags::NHSYNC,
    } | match modeline.vsync_polarity {
        VSyncPolarity::PVSync => ModeFlags::PVSYNC,
        VSyncPolarity::NVSync => ModeFlags::NVSYNC,
    };

    let mode_name = format!(
        "{}x{}@{:.2}",
        modeline.hdisplay, modeline.vdisplay, vrefresh_hertz
    );
    let name = modeinfo_name_slice_from_string(&mode_name);

    // https://www.kernel.org/doc/html/v6.17/gpu/drm-uapi.html#c.drm_mode_modeinfo
    Ok(DrmMode::from(drm_mode_modeinfo {
        clock: pixel_clock_kilo_hertz.round() as u32,
        hdisplay: modeline.hdisplay,
        hsync_start: modeline.hsync_start,
        hsync_end: modeline.hsync_end,
        htotal: modeline.htotal,
        vdisplay: modeline.vdisplay,
        vsync_start: modeline.vsync_start,
        vsync_end: modeline.vsync_end,
        vtotal: modeline.vtotal,
        vrefresh: vrefresh_rounded,
        flags: flags.bits(),
        name,
        // Defaults
        type_: drm_ffi::DRM_MODE_TYPE_USERDEF,
        hskew: 0,
        vscan: 0,
    }))
}

pub fn calculate_mode_cvt(width: u16, height: u16, refresh: f64) -> DrmMode {
    // Cross-checked with sway's implementation:
    // https://gitlab.freedesktop.org/wlroots/wlroots/-/blob/22528542970687720556035790212df8d9bb30bb/backend/drm/util.c#L251

    let options = libdisplay_info::cvt::Options {
        red_blank_ver: libdisplay_info::cvt::ReducedBlankingVersion::None,
        h_pixels: width as i32,
        v_lines: height as i32,
        ip_freq_rqd: refresh,

        // Defaults
        video_opt: false,
        vblank: 0f64,
        additional_hblank: 0,
        early_vsync_rqd: false,
        int_rqd: false,
        margins_rqd: false,
    };
    let cvt_timing = libdisplay_info::cvt::Timing::compute(options);

    let hsync_start = width + cvt_timing.h_front_porch as u16;
    let vsync_start = (cvt_timing.v_lines_rnd + cvt_timing.v_front_porch) as u16;
    let hsync_end = hsync_start + cvt_timing.h_sync as u16;
    let vsync_end = vsync_start + cvt_timing.v_sync as u16;

    let htotal = hsync_end + cvt_timing.h_back_porch as u16;
    let vtotal = vsync_end + cvt_timing.v_back_porch as u16;

    let clock = f64::round(cvt_timing.act_pixel_freq * 1000f64) as u32;
    let vrefresh = f64::round(cvt_timing.act_frame_rate) as u32;

    let flags = drm_ffi::DRM_MODE_FLAG_NHSYNC | drm_ffi::DRM_MODE_FLAG_PVSYNC;

    let mode_name = format!("{width}x{height}@{:.2}", cvt_timing.act_frame_rate);
    let name = modeinfo_name_slice_from_string(&mode_name);

    let drm_ffi_mode = drm_ffi::drm_sys::drm_mode_modeinfo {
        clock,

        hdisplay: width,
        hsync_start,
        hsync_end,
        htotal,

        vdisplay: height,
        vsync_start,
        vsync_end,
        vtotal,

        vrefresh,

        flags,
        type_: drm_ffi::DRM_MODE_TYPE_USERDEF,
        name,

        // Defaults
        hskew: 0,
        vscan: 0,
    };

    DrmMode::from(drm_ffi_mode)
}

// Returns a c-string of maximally 31 Rust string chars + null terminator. Excess characters are
// dropped.
fn modeinfo_name_slice_from_string(mode_name: &str) -> [core::ffi::c_char; 32] {
    let mut name: [core::ffi::c_char; 32] = [0; 32];

    for (a, b) in zip(&mut name[..31], mode_name.as_bytes()) {
        // Can be u8 on aarch64 and i8 on x86_64.
        *a = *b as _;
    }

    name
}

fn pick_mode(
    connector: &connector::Info,
    target: Option<niri_config::output::Mode>,
) -> Option<(control::Mode, bool)> {
    let mut mode = None;
    let mut fallback = false;

    if let Some(target) = target {
        let target_mode = target.mode;

        if target.custom {
            if let Some(refresh) = target_mode.refresh {
                let custom_mode =
                    calculate_mode_cvt(target_mode.width, target_mode.height, refresh);
                return Some((custom_mode, false));
            } else {
                warn!("ignoring custom mode without refresh rate");
            }
        }

        let refresh = target_mode.refresh.map(|r| (r * 1000.).round() as i32);
        for m in connector.modes() {
            if m.size() != (target.mode.width, target.mode.height) {
                continue;
            }

            // Interlaced modes don't appear to work.
            if m.flags().contains(ModeFlags::INTERLACE) {
                continue;
            }

            if let Some(refresh) = refresh {
                // If refresh is set, only pick modes with matching refresh.
                let wl_mode = Mode::from(*m);
                if wl_mode.refresh == refresh {
                    mode = Some(m);
                }
            } else if let Some(curr) = mode {
                // If refresh isn't set, pick the mode with the highest refresh.
                if curr.vrefresh() < m.vrefresh() {
                    mode = Some(m);
                }
            } else {
                mode = Some(m);
            }
        }

        if mode.is_none() {
            fallback = true;
        }
    }

    if mode.is_none() {
        // Pick a preferred mode.
        for m in connector.modes() {
            if !m.mode_type().contains(ModeTypeFlags::PREFERRED) {
                continue;
            }

            if let Some(curr) = mode {
                if curr.vrefresh() < m.vrefresh() {
                    mode = Some(m);
                }
            } else {
                mode = Some(m);
            }
        }
    }

    if mode.is_none() {
        // Last attempt.
        mode = connector.modes().first();
    }

    mode.map(|m| (*m, fallback))
}

fn get_edid_info(
    device: &DrmDevice,
    connector: connector::Handle,
) -> anyhow::Result<libdisplay_info::info::Info> {
    let (_, info, value) =
        find_drm_property(device, connector, "EDID").context("no EDID property")?;
    let blob = info
        .value_type()
        .convert_value(value)
        .as_blob()
        .context("EDID was not blob type")?;
    let data = device
        .get_property_blob(blob)
        .context("error getting EDID blob value")?;
    libdisplay_info::info::Info::parse_edid(&data).context("error parsing EDID")
}

impl<'a> ConnectorProperties<'a> {
    fn try_new(device: &'a DrmDevice, connector: connector::Handle) -> anyhow::Result<Self> {
        let prop_vals = device
            .get_properties(connector)
            .context("error getting properties")?;

        let mut properties = Vec::new();

        for (prop, value) in prop_vals {
            let info = device
                .get_property(prop)
                .context("error getting property")?;

            properties.push((info, value));
        }

        Ok(Self {
            device,
            connector,
            properties,
            has_change: false,
            requests: AtomicModeReq::new(),
        })
    }

    fn find(&self, name: &std::ffi::CStr) -> anyhow::Result<&(property::Info, property::RawValue)> {
        for prop in &self.properties {
            if prop.0.name() == name {
                return Ok(prop);
            }
        }

        Err(anyhow!("couldn't find property: {name:?}"))
    }

    fn get_panel_orientation(&self) -> anyhow::Result<Transform> {
        let (info, value) = self.find(c"panel orientation")?;
        match info.value_type().convert_value(*value) {
            property::Value::Enum(Some(val)) => match val.value() {
                // "Normal"
                0 => Ok(Transform::Normal),
                // "Upside Down"
                1 => Ok(Transform::_180),
                // "Left Side Up"
                2 => Ok(Transform::_90),
                // "Right Side Up"
                3 => Ok(Transform::_270),
                _ => bail!("panel orientation has invalid value: {:?}", val),
            },
            _ => bail!("panel orientation has wrong value type"),
        }
    }

    fn reset_hdr(&mut self) -> anyhow::Result<()> {
        const DRM_MODE_COLORIMETRY_DEFAULT: u64 = 0;

        let (info, value) = self.find(c"HDR_OUTPUT_METADATA")?;

        let property::ValueType::Blob = info.value_type() else {
            bail!("wrong property type")
        };
        if *value != 0 {
            self.requests
                .add_raw_property(self.connector.into(), info.handle(), 0);
            self.has_change = true;
        }

        let (info, value) = self.find(c"Colorspace")?;
        let property::ValueType::Enum(_) = info.value_type() else {
            bail!("wrong property type")
        };
        if *value != DRM_MODE_COLORIMETRY_DEFAULT {
            self.requests.add_raw_property(
                self.connector.into(),
                info.handle(),
                DRM_MODE_COLORIMETRY_DEFAULT,
            );
            self.has_change = true;
        }

        Ok(())
    }

    fn set_max_bpc(&mut self, max_bpc: MaxBpc) -> anyhow::Result<u64> {
        let (info, value) = self.find(c"max bpc")?;

        let property::ValueType::UnsignedRange(min, max) = info.value_type() else {
            bail!("wrong property type")
        };

        let max_bpc = max_bpc.0 as u64;
        if !(min..=max).contains(&max_bpc) {
            bail!("max-bpc {max_bpc} outside valid range of [{min}, {max}]");
        }

        let property::Value::UnsignedRange(value) = info.value_type().convert_value(*value) else {
            bail!("wrong property type")
        };

        if value != max_bpc {
            self.requests.add_raw_property(
                self.connector.into(),
                info.handle(),
                property::Value::UnsignedRange(max_bpc).into(),
            );
            self.has_change = true;
        }

        Ok(max_bpc)
    }

    fn commit(&mut self) -> anyhow::Result<()> {
        if self.has_change {
            self.device.atomic_commit(
                AtomicCommitFlags::ALLOW_MODESET,
                std::mem::take(&mut self.requests),
            )?;
        }

        Ok(())
    }
}

fn set_connector_properties(
    props: &mut ConnectorProperties,
    max_bpc: Option<MaxBpc>,
    reset_hdr: bool,
) {
    if let Some(max_bpc) = max_bpc {
        if let Err(err) = props.set_max_bpc(max_bpc) {
            debug!("failed to set `max bpc` property: {err}");
        }
    }

    if reset_hdr {
        if let Err(err) = props.reset_hdr() {
            debug!("failed to set HDR properties: {err}");
        }
    }

    if let Err(err) = props.commit() {
        warn!("failed to atomically commit properties: {err}");
    }
}

fn is_vrr_capable(device: &DrmDevice, connector: connector::Handle) -> Option<bool> {
    let (_, info, value) = find_drm_property(device, connector, "vrr_capable")?;
    info.value_type().convert_value(value).as_boolean()
}

pub fn set_gamma_for_crtc(
    device: &DrmDevice,
    crtc: crtc::Handle,
    ramp: Option<&[u16]>,
) -> anyhow::Result<()> {
    let _span = tracy_client::span!("set_gamma_for_crtc");

    let info = device.get_crtc(crtc).context("error getting crtc info")?;
    let gamma_length = info.gamma_length() as usize;

    ensure!(gamma_length != 0, "setting gamma is not supported");

    let mut temp;
    let ramp = if let Some(ramp) = ramp {
        ensure!(ramp.len() == gamma_length * 3, "wrong gamma length");
        ramp
    } else {
        let _span = tracy_client::span!("generate linear gamma");

        // The legacy API provides no way to reset the gamma, so set a linear one manually.
        temp = vec![0u16; gamma_length * 3];

        let (red, rest) = temp.split_at_mut(gamma_length);
        let (green, blue) = rest.split_at_mut(gamma_length);
        let denom = gamma_length as u64 - 1;
        for (i, ((r, g), b)) in zip(zip(red, green), blue).enumerate() {
            let value = (0xFFFFu64 * i as u64 / denom) as u16;
            *r = value;
            *g = value;
            *b = value;
        }

        &temp
    };

    let (red, ramp) = ramp.split_at(gamma_length);
    let (green, blue) = ramp.split_at(gamma_length);

    device
        .set_gamma(crtc, red, green, blue)
        .context("error setting gamma")?;

    Ok(())
}

fn format_connector_name(connector: &connector::Info) -> String {
    format!(
        "{}-{}",
        connector.interface().as_str(),
        connector.interface_id(),
    )
}

fn make_output_name(
    device: &DrmDevice,
    connector: connector::Handle,
    connector_name: String,
) -> OutputName {
    let info = get_edid_info(device, connector)
        .map_err(|err| warn!("error getting EDID info for {connector_name}: {err:?}"))
        .ok();
    OutputName {
        connector: connector_name,
        make: info.as_ref().and_then(|info| info.make()),
        model: info.as_ref().and_then(|info| info.model()),
        serial: info.as_ref().and_then(|info| info.serial()),
    }
}

/// Initializes the libinput plugin system.
///
/// # Safety
///
/// This function must be called before libinput iterates through the devices, i.e. before
/// libinput_udev_assign_seat() or the first call to libinput_path_add_device().
unsafe fn init_libinput_plugin_system(libinput: &Libinput) {
    #[cfg(have_libinput_plugin_system)]
    unsafe {
        use std::ffi::{c_char, c_int, CString};
        use std::os::unix::ffi::OsStringExt;

        use directories::BaseDirs;
        use input::ffi::libinput;
        use input::AsRaw as _;

        extern "C" {
            fn libinput_plugin_system_append_path(libinput: *const libinput, path: *const c_char);
            fn libinput_plugin_system_append_default_paths(libinput: *const libinput);
            fn libinput_plugin_system_load_plugins(
                libinput: *const libinput,
                flags: c_int,
            ) -> c_int;
        }
        const LIBINPUT_PLUGIN_SYSTEM_FLAG_NONE: c_int = 0;
        let libinput = libinput.as_raw();

        // Also load plugins from $XDG_CONFIG_HOME/libinput/plugins.
        if let Some(dirs) = BaseDirs::new() {
            let mut plugins_dir = dirs.config_dir().to_path_buf();
            plugins_dir.push("libinput");
            plugins_dir.push("plugins");
            if let Ok(plugins_dir) = CString::new(plugins_dir.into_os_string().into_vec()) {
                libinput_plugin_system_append_path(libinput, plugins_dir.as_ptr());
            }
        }

        libinput_plugin_system_append_default_paths(libinput);
        libinput_plugin_system_load_plugins(libinput, LIBINPUT_PLUGIN_SYSTEM_FLAG_NONE);
    }
    #[cfg(not(have_libinput_plugin_system))]
    let _ = libinput;
}

#[cfg(test)]
mod tests {
    use insta::assert_debug_snapshot;
    use niri_config::output::Modeline;
    use niri_ipc::{HSyncPolarity, VSyncPolarity};

    use crate::backend::tty::{calculate_drm_mode_from_modeline, calculate_mode_cvt};

    #[test]
    fn test_calculate_drmmode_from_modeline() {
        let modeline1 = Modeline {
            clock: 173.0,
            hdisplay: 1920,
            vdisplay: 1080,
            hsync_start: 2048,
            hsync_end: 2248,
            htotal: 2576,
            vsync_start: 1083,
            vsync_end: 1088,
            vtotal: 1120,
            hsync_polarity: HSyncPolarity::NHSync,
            vsync_polarity: VSyncPolarity::PVSync,
        };
        assert_debug_snapshot!(calculate_drm_mode_from_modeline(&modeline1).unwrap(), @r#"
        Mode {
            name: "1920x1080@59.96",
            clock: 173000,
            size: (
                1920,
                1080,
            ),
            hsync: (
                2048,
                2248,
                2576,
            ),
            vsync: (
                1083,
                1088,
                1120,
            ),
            hskew: 0,
            vscan: 0,
            vrefresh: 60,
            mode_type: ModeTypeFlags(
                USERDEF,
            ),
        }
        "#);
        let modeline2 = Modeline {
            clock: 452.5,
            hdisplay: 1920,
            vdisplay: 1080,
            hsync_start: 2088,
            hsync_end: 2296,
            htotal: 2672,
            vsync_start: 1083,
            vsync_end: 1088,
            vtotal: 1177,
            hsync_polarity: HSyncPolarity::NHSync,
            vsync_polarity: VSyncPolarity::PVSync,
        };
        assert_debug_snapshot!(calculate_drm_mode_from_modeline(&modeline2).unwrap(), @r#"
        Mode {
            name: "1920x1080@143.88",
            clock: 452500,
            size: (
                1920,
                1080,
            ),
            hsync: (
                2088,
                2296,
                2672,
            ),
            vsync: (
                1083,
                1088,
                1177,
            ),
            hskew: 0,
            vscan: 0,
            vrefresh: 144,
            mode_type: ModeTypeFlags(
                USERDEF,
            ),
        }
        "#);
    }

    #[test]
    fn test_calc_cvt() {
        // Crosschecked with other calculators like the cvt commandline utility.
        assert_debug_snapshot!(calculate_mode_cvt(1920, 1080, 60.0), @r#"
        Mode {
            name: "1920x1080@59.96",
            clock: 173000,
            size: (
                1920,
                1080,
            ),
            hsync: (
                2048,
                2248,
                2576,
            ),
            vsync: (
                1083,
                1088,
                1120,
            ),
            hskew: 0,
            vscan: 0,
            vrefresh: 60,
            mode_type: ModeTypeFlags(
                USERDEF,
            ),
        }
        "#);
        assert_debug_snapshot!(calculate_mode_cvt(1920, 1080, 144.0), @r#"
        Mode {
            name: "1920x1080@143.88",
            clock: 452500,
            size: (
                1920,
                1080,
            ),
            hsync: (
                2088,
                2296,
                2672,
            ),
            vsync: (
                1083,
                1088,
                1177,
            ),
            hskew: 0,
            vscan: 0,
            vrefresh: 144,
            mode_type: ModeTypeFlags(
                USERDEF,
            ),
        }
        "#);
    }

    #[test]
    fn test_gpu_power_state_migration() {
        use crate::backend::tty::{migrate_gpu_power_state_map, GpuPowerState, WakeupState};
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        let event_loop = calloop::EventLoop::<()>::try_new().unwrap();
        let handle = event_loop.handle();
        let suspend_token = handle
            .insert_source(
                calloop::timer::Timer::from_duration(std::time::Duration::from_millis(100)),
                |_, _, _| calloop::timer::TimeoutAction::Drop,
            )
            .unwrap();
        let wakeup_token = handle
            .insert_source(
                calloop::timer::Timer::from_duration(std::time::Duration::from_millis(100)),
                |_, _, _| calloop::timer::TimeoutAction::Drop,
            )
            .unwrap();

        let early_key = 1u32;
        let actual_key = 2u32;

        let mut gpu_power = HashMap::new();

        // 1. Setup early state with active timers and pending wakeup
        let mut early_state = GpuPowerState::new(false);
        early_state.waking_up = true;
        early_state.suspend_timer = Some(suspend_token);
        early_state.wakeup_timer = Some(wakeup_token);
        let early_wakeup = Arc::new(Mutex::new(WakeupState::Pending));
        early_state.wakeup_state = early_wakeup.clone();
        gpu_power.insert(early_key, early_state);

        // 2. Scenario A: Migrate to a non-existent actual_key
        let tokens = migrate_gpu_power_state_map(&mut gpu_power, early_key, actual_key);
        // Verify both tokens are returned for cancellation
        assert_eq!(tokens.len(), 2);
        assert!(tokens.contains(&suspend_token));
        assert!(tokens.contains(&wakeup_token));

        assert!(!gpu_power.contains_key(&early_key));
        assert!(gpu_power.contains_key(&actual_key));

        // 3. Scenario B: Try migrating when actual_key already exists (shared node collision)
        let mut shared_early = GpuPowerState::new(false);
        shared_early.waking_up = true;
        let shared_wakeup = Arc::new(Mutex::new(WakeupState::Pending));
        shared_early.wakeup_state = shared_wakeup.clone();
        gpu_power.insert(early_key, shared_early);

        let tokens = migrate_gpu_power_state_map(&mut gpu_power, early_key, actual_key);
        assert!(tokens.is_empty());

        assert!(!gpu_power.contains_key(&early_key));
        assert!(gpu_power.contains_key(&actual_key));
        assert!(matches!(
            *shared_wakeup.lock().unwrap(),
            WakeupState::Cancelled
        ));

        // 4. Scenario C: Migrate when early_key == actual_key (no-op)
        let early_key_c = 3u32;
        let mut state_c = GpuPowerState::new(false);
        state_c.waking_up = true;
        gpu_power.insert(early_key_c, state_c);

        let tokens = migrate_gpu_power_state_map(&mut gpu_power, early_key_c, early_key_c);
        assert!(tokens.is_empty());
        assert!(gpu_power.contains_key(&early_key_c));
        assert!(gpu_power.get(&early_key_c).unwrap().waking_up);
    }

    #[test]
    fn test_gpu_power_state_transitions() {
        use crate::backend::tty::{should_start_gpu_wakeup, GpuPowerState};

        let mut state = GpuPowerState::new(false);

        // 1. Fresh state: should allow wakeup
        assert!(should_start_gpu_wakeup(&state, false));
        assert!(should_start_gpu_wakeup(&state, true));

        // 2. Waking up state: should block normal, but allow retry
        state.waking_up = true;
        assert!(!should_start_gpu_wakeup(&state, false));
        assert!(should_start_gpu_wakeup(&state, true));

        // 3. Reset waking up, set wakeup failed: should block normal, but allow retry
        state.waking_up = false;
        state.wakeup_failed = true;
        assert!(!should_start_gpu_wakeup(&state, false));
        assert!(should_start_gpu_wakeup(&state, true));

        // 4. Reset wakeup failed: should allow wakeup again
        state.wakeup_failed = false;
        assert!(should_start_gpu_wakeup(&state, false));
        assert!(should_start_gpu_wakeup(&state, true));
    }

    #[test]
    fn test_wakeup_failed_reset_and_retry() {
        use crate::backend::tty::{should_start_gpu_wakeup, GpuPowerState};

        let mut state = GpuPowerState::new(false);
        // clean slate, nothing blocking either path
        assert!(should_start_gpu_wakeup(&state, false));
        assert!(should_start_gpu_wakeup(&state, true));

        // wakeup_failed stops the normal path, retries still go through (same deal as waking_up)
        state.wakeup_failed = true;
        assert!(!should_start_gpu_wakeup(&state, false));
        assert!(should_start_gpu_wakeup(&state, true));

        // resume clears the flag so we can talk to the GPU again
        state.wakeup_failed = false;
        assert!(should_start_gpu_wakeup(&state, false));

        // waking_up and wakeup_failed should look the same to is_retry
        state.waking_up = true;
        assert!(!should_start_gpu_wakeup(&state, false));
        assert!(should_start_gpu_wakeup(&state, true));
    }
}
