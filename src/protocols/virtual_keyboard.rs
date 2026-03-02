use smithay::backend::input::{Device, DeviceCapability};
use smithay::delegate_virtual_keyboard_manager;

use crate::niri::State;

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct VirtualKeyboard;

impl Device for VirtualKeyboard {
    fn id(&self) -> String {
        String::from("virtual keyboard")
    }

    fn name(&self) -> String {
        String::from("virtual keyboard")
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        matches!(capability, DeviceCapability::Keyboard)
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<std::path::PathBuf> {
        None
    }
}
delegate_virtual_keyboard_manager!(State);
