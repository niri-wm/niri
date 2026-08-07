use std::collections::HashMap;

use reis::enumflags2::BitFlags;
use reis::request::DeviceCapability;
use zbus::fdo;
use zbus::zvariant::Value;

pub const DEVICE_TYPE_KEYBOARD: u32 = 1;
pub const DEVICE_TYPE_POINTER: u32 = 2;
pub const DEVICE_TYPE_TOUCHSCREEN: u32 = 4;

const SUPPORTED_DEVICE_TYPES: u32 =
    DEVICE_TYPE_KEYBOARD | DEVICE_TYPE_POINTER | DEVICE_TYPE_TOUCHSCREEN;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteDesktopDeviceTypes {
    bits: u32,
}

impl Default for RemoteDesktopDeviceTypes {
    fn default() -> Self {
        Self {
            bits: SUPPORTED_DEVICE_TYPES,
        }
    }
}

impl RemoteDesktopDeviceTypes {
    pub fn from_options(options: &HashMap<&str, Value<'_>>) -> fdo::Result<Self> {
        let bits = match options.get("device-types") {
            Some(value) => value
                .downcast_ref::<u32>()
                .map_err(|_| fdo::Error::InvalidArgs("device-types must be a uint32".to_owned()))?,
            None => SUPPORTED_DEVICE_TYPES,
        };

        if bits & !SUPPORTED_DEVICE_TYPES != 0 {
            return Err(fdo::Error::InvalidArgs(format!(
                "unsupported remote desktop device types: 0x{:x}",
                bits & !SUPPORTED_DEVICE_TYPES
            )));
        }

        Ok(Self {
            bits: bits & SUPPORTED_DEVICE_TYPES,
        })
    }

    pub fn keyboard(self) -> bool {
        self.bits & DEVICE_TYPE_KEYBOARD != 0
    }

    pub fn pointer(self) -> bool {
        self.bits & DEVICE_TYPE_POINTER != 0
    }

    pub fn touchscreen(self) -> bool {
        self.bits & DEVICE_TYPE_TOUCHSCREEN != 0
    }

    pub fn capabilities(self) -> BitFlags<DeviceCapability> {
        let mut capabilities = BitFlags::empty();

        if self.pointer() {
            capabilities |= DeviceCapability::Pointer;
            capabilities |= DeviceCapability::PointerAbsolute;
            capabilities |= DeviceCapability::Button;
            capabilities |= DeviceCapability::Scroll;
        }

        if self.keyboard() {
            capabilities |= DeviceCapability::Keyboard;
        }

        if self.touchscreen() {
            capabilities |= DeviceCapability::Touch;
        }

        capabilities
    }
}

pub fn supported_device_types() -> u32 {
    SUPPORTED_DEVICE_TYPES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_types_default_to_every_supported_type() {
        let types = RemoteDesktopDeviceTypes::from_options(&HashMap::new()).unwrap();

        assert!(types.keyboard());
        assert!(types.pointer());
        assert!(types.touchscreen());
    }

    #[test]
    fn device_types_reject_unknown_bits() {
        let options = HashMap::from([("device-types", Value::new(8_u32))]);
        let err = RemoteDesktopDeviceTypes::from_options(&options).unwrap_err();

        assert!(format!("{err:?}").contains("unsupported"));
    }
}
