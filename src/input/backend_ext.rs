use ::input as libinput;
use smithay::backend::input;
use smithay::backend::winit::WinitVirtualDevice;
use smithay::output::Output;
use smithay::utils::{Logical, Point};

use crate::niri::State;
use crate::protocols::virtual_pointer::VirtualPointer;

pub trait NiriInputBackend: input::InputBackend<Device = Self::NiriDevice> {
    type NiriDevice: NiriInputDevice;
}
impl<T: input::InputBackend> NiriInputBackend for T
where
    Self::Device: NiriInputDevice,
{
    type NiriDevice = Self::Device;
}

pub trait NiriInputDevice: input::Device {
    // FIXME: this should maybe be per-event, not per-device,
    // but it's not clear that this matters in practice?
    // it might be more obvious once we implement it for libinput
    fn output(&self, state: &State) -> Option<Output>;

    /// If `Some`, absolute positions reported by this device are in the global logical
    /// coordinate space, offset by the returned origin.
    ///
    /// This maps a device directly onto the global output layout (rather than onto a single
    /// output), so the origin must be added to the raw absolute position.
    fn absolute_position_origin(&self, _state: &State) -> Option<Point<f64, Logical>> {
        None
    }
}

impl NiriInputDevice for libinput::Device {
    fn output(&self, _state: &State) -> Option<Output> {
        // FIXME: Allow specifying the output per-device?
        None
    }
}

impl NiriInputDevice for WinitVirtualDevice {
    fn output(&self, _state: &State) -> Option<Output> {
        // FIXME: we should be returning the single output that the winit backend creates,
        // but for now, that will cause issues because the output is normally upside down,
        // so we apply Transform::Flipped180 to it and that would also cause
        // the cursor position to be flipped, which is not what we want.
        //
        // instead, we just return None and rely on the fact that it has only one output.
        // doing so causes the cursor to be placed in *global* output coordinates,
        // which are not flipped, and happen to be what we want.
        None
    }
}

impl NiriInputDevice for VirtualPointer {
    fn output(&self, _: &State) -> Option<Output> {
        self.output().cloned()
    }
}

/// EI (remote desktop) devices get regions advertised relative to the global bounding
/// rectangle origin, and report absolute positions in that same offset coordinate space.
#[cfg(feature = "xdp-gnome-remote-desktop")]
impl NiriInputDevice for reis::request::Device {
    fn output(&self, _state: &State) -> Option<Output> {
        None
    }

    fn absolute_position_origin(&self, state: &State) -> Option<Point<f64, Logical>> {
        let ipc_outputs = state.backend.ipc_outputs();
        let ipc_outputs = ipc_outputs.lock().unwrap();
        let bounding_rect = crate::utils::global_bounding_rectangle_ipc(&ipc_outputs)?;
        Some(bounding_rect.loc.to_f64())
    }
}
