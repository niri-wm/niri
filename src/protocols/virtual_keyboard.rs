use smithay::backend::input::{
    Device, DeviceCapability, Event, InputBackend, InputEvent, KeyState, KeyboardKeyEvent, Keycode,
    UnusedEvent,
};
use smithay::delegate_virtual_keyboard_manager;
use smithay::input::keyboard::xkb::ModMask;
use smithay::input::keyboard::KeyboardHandle;
use smithay::wayland::virtual_keyboard::VirtualKeyboardHandler;

use crate::niri::State;

pub struct VirtualKeyboardInputBackend;

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

pub struct VirtualKeyboardKeyEvent {
    pub keycode: Keycode,
    pub state: KeyState,
    pub time: u32,
}

impl Event<VirtualKeyboardInputBackend> for VirtualKeyboardKeyEvent {
    fn time(&self) -> u64 {
        self.time as u64 * 1000 // millis to micros
    }

    fn device(&self) -> VirtualKeyboard {
        VirtualKeyboard
    }
}

impl KeyboardKeyEvent<VirtualKeyboardInputBackend> for VirtualKeyboardKeyEvent {
    fn key_code(&self) -> Keycode {
        self.keycode
    }

    fn state(&self) -> KeyState {
        self.state
    }

    fn count(&self) -> u32 {
        0 // Not used by niri
    }
}

impl InputBackend for VirtualKeyboardInputBackend {
    type Device = VirtualKeyboard;

    type KeyboardKeyEvent = VirtualKeyboardKeyEvent;
    type PointerAxisEvent = UnusedEvent;
    type PointerButtonEvent = UnusedEvent;
    type PointerMotionEvent = UnusedEvent;
    type PointerMotionAbsoluteEvent = UnusedEvent;

    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;

    type TouchDownEvent = UnusedEvent;
    type TouchUpEvent = UnusedEvent;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;

    type SwitchToggleEvent = UnusedEvent;

    type SpecialEvent = UnusedEvent;
}

impl VirtualKeyboardHandler for State {
    fn on_keyboard_event(
        &mut self,
        keycode: Keycode,
        state: KeyState,
        time: u32,
        _keyboard: KeyboardHandle<Self>,
    ) {
        // The virtual keyboard impl in Smithay changes the keymap, so we'll need to reset it on
        // the next real keyboard event.
        self.niri.reset_keymap = true;

        let event = VirtualKeyboardKeyEvent {
            keycode,
            state,
            time,
        };
        self.process_input_event(InputEvent::<VirtualKeyboardInputBackend>::Keyboard { event });
    }

    // We handle modifiers when the key event is sent.
    fn on_keyboard_modifiers(
        &mut self,
        _depressed_mods: ModMask,
        _latched_mods: ModMask,
        _locked_mods: ModMask,
        _keyboard: KeyboardHandle<Self>,
    ) {
    }
}
delegate_virtual_keyboard_manager!(State);
