use std::path::PathBuf;

use smithay::backend::input::{Device, DeviceCapability};
use smithay::output::Output;

use crate::input::backend_ext::NiriInputDevice;
use crate::niri::State;

#[derive(Debug)]
pub(super) struct RemoteDesktopInputBackend;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct RemoteDesktopDevice {
    kind: RemoteDesktopDeviceKind,
    output_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RemoteDesktopDeviceKind {
    Keyboard,
    Pointer,
    Touch,
}

#[derive(Debug, Clone)]
pub(super) struct RemoteDesktopEventBase {
    pub(super) device: RemoteDesktopDevice,
    pub(super) time_msec: u32,
}

impl RemoteDesktopEventBase {
    pub(super) fn keyboard(time_msec: u32) -> Self {
        Self {
            device: RemoteDesktopDevice::keyboard(),
            time_msec,
        }
    }

    pub(super) fn pointer(time_msec: u32, output_name: Option<String>) -> Self {
        Self {
            device: RemoteDesktopDevice::pointer(output_name),
            time_msec,
        }
    }

    pub(super) fn touch(time_msec: u32, output_name: Option<String>) -> Self {
        Self {
            device: RemoteDesktopDevice::touch(output_name),
            time_msec,
        }
    }
}

impl RemoteDesktopDevice {
    fn keyboard() -> Self {
        Self {
            kind: RemoteDesktopDeviceKind::Keyboard,
            output_name: None,
        }
    }

    fn pointer(output_name: Option<String>) -> Self {
        Self {
            kind: RemoteDesktopDeviceKind::Pointer,
            output_name,
        }
    }

    fn touch(output_name: Option<String>) -> Self {
        Self {
            kind: RemoteDesktopDeviceKind::Touch,
            output_name,
        }
    }
}

impl Device for RemoteDesktopDevice {
    fn id(&self) -> String {
        format!("niri-remote-desktop-{:?}", self.kind)
    }

    fn name(&self) -> String {
        "niri remote desktop".to_owned()
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        matches!(
            (self.kind, capability),
            (
                RemoteDesktopDeviceKind::Keyboard,
                DeviceCapability::Keyboard
            ) | (RemoteDesktopDeviceKind::Pointer, DeviceCapability::Pointer)
                | (RemoteDesktopDeviceKind::Touch, DeviceCapability::Touch)
        )
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<PathBuf> {
        None
    }
}

impl NiriInputDevice for RemoteDesktopDevice {
    fn output(&self, state: &State) -> Option<Output> {
        self.output_name
            .as_deref()
            .and_then(|name| state.niri.output_by_name_match(name))
            .cloned()
    }
}
