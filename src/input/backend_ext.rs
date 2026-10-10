use ::input as libinput;
use smithay::backend::input;
use smithay::backend::winit::WinitVirtualDevice;
use smithay::output::Output;
use smithay::wayland::virtual_keyboard::VirtualKeyboardDevice;
use smithay::wayland::virtual_pointer::VirtualPointerDevice;

use crate::niri::State;

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

    /// Whether absolute positions from this device are already in the output's logical
    /// (transformed) coordinate space.
    ///
    /// Physical devices like touchscreens and tablets report positions relative to the panel, so
    /// the output transform needs to be applied to them. Virtual pointers give positions relative
    /// to the output as it appears in the layout (like wlroots treats them).
    fn absolute_position_is_logical(&self) -> bool {
        false
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

impl NiriInputDevice for VirtualPointerDevice {
    fn output(&self, _: &State) -> Option<Output> {
        self.output()
    }

    fn absolute_position_is_logical(&self) -> bool {
        true
    }
}

impl NiriInputDevice for VirtualKeyboardDevice {
    fn output(&self, _: &State) -> Option<Output> {
        None
    }
}
