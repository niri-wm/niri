//! `zwp_virtual_keyboard_v1` handling.
//!
//! Virtual keyboards are handled like a real keyboard (binds, grabs, etc), but differ in a few
//! ways:
//!
//! - A virtual keyboard's keymap may differ from the seat's one, and if so, will be swapped before
//!   events from it are forwarded (restoring the saved layout and a subset of modifiers when
//!   swapping back to the original one).
//! - If the virtual keyboard uses `no_keymap` or the same one as the seat, it will use the seat's
//!   xkb state directly.
//! - If a client grabs the keyboard, then forwards events through a virtual keyboard (e.g., fcitx),
//!   they are then forwarded bypassing the grab to avoid loops (sway special-cases this too).
//! - Modifiers set through the `modifiers` request apply to the shared xkb state (see
//!   `DeviceRemoved`).

use std::sync::Arc;

use smithay::backend::input::{Event, InputEvent, KeyboardKeyEvent};
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::input_method::InputMethodSeat;
use smithay::wayland::virtual_keyboard::{
    VirtualKeyboardBackend, VirtualKeyboardDevice, VirtualKeyboardHandler, VirtualKeyboardKeyEvent,
    VirtualKeyboardSpecialEvent,
};

use crate::niri::{SavedKeyboardState, State};

/// Virtual keyboard keymap (which temporarily replaces the shared seat one).
#[derive(Debug)]
pub struct VirtualKeyboardKeymap {
    pub device: VirtualKeyboardDevice,
    pub keymap: Arc<str>,
    pub saved: SavedKeyboardState,
}

impl VirtualKeyboardHandler for State {
    fn process_virtual_keyboard_event(&mut self, event: InputEvent<VirtualKeyboardBackend>) {
        match event {
            InputEvent::Keyboard { event } => {
                let device = event.device();
                self.activate_virtual_keyboard_keymap(&device);
                if self.is_input_method_virtual_keyboard(&device) {
                    // Do not forward passed-through events from a grab back to
                    // the grab.
                    self.forward_input_method_key(&event);
                } else {
                    self.process_input_event(InputEvent::<VirtualKeyboardBackend>::Keyboard {
                        event,
                    });
                }
            }
            InputEvent::Special(VirtualKeyboardSpecialEvent::Modifiers {
                device,
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
            }) => {
                self.activate_virtual_keyboard_keymap(&device);

                let keyboard = self.niri.seat.get_keyboard().unwrap();
                keyboard.with_xkb_state(self, |mut context| {
                    context.set_modifier_mask(mods_depressed, mods_latched, mods_locked, group);
                });
            }
            InputEvent::Special(VirtualKeyboardSpecialEvent::KeymapChanged { .. }) => {
                // The new keymap will be activated by the next key or modifiers event.
            }
            InputEvent::DeviceAdded { device } => {
                self.niri.virtual_keyboard_sources.insert(device.source());
                self.process_input_event(InputEvent::<VirtualKeyboardBackend>::DeviceAdded {
                    device,
                });
            }
            InputEvent::DeviceRemoved { device } => {
                self.niri.virtual_keyboard_sources.remove(&device.source());

                // Restore the original keymap if this device is active. This is
                // checked based on the device rather than on the keymap, since
                // the client may have replaced its keymap after we activated it
                // and not sent any key/modifier events afterwards before
                // removing the device.
                let active = self
                    .niri
                    .virtual_keyboard_keymap
                    .as_ref()
                    .is_some_and(|active| active.device == device);
                if active {
                    self.restore_configured_keymap();
                }

                // The held keys have already been released by smithay.
                //
                // We don't reset the modifiers since there's no way to tell which ones were set by
                // this keyboard specifically since the masks are absolute. This only affects
                // virtual keyboards without a keymap, or with the same keymap as the seat. We could
                // track which bits were set by each virtual keyboard, but it's more complex, and we
                // still would need to rebuild the modifier state after to avoid potentially leaving
                // modifiers without a corresponding key stuck, and it's not really possible to
                // rebuild it cleanly for all possible layouts (e.g., `grp:alt_shift_toggle`,
                // layouts with multi-level modifier keys) without tracking the exact sequence the
                // modifiers were pressed in (see
                // https://github.com/niri-wm/niri/pull/4548#discussion_r4237056727).
                //
                // Not resetting the modifiers means that:
                //
                // - a locked modifier or layout group set by `modifiers` stays until the user
                //   toggles it (most layouts have clearLocks on Shift/Ctrl/Alt/Super, so those can
                //   be reset by pressing the key, but ones without a corresponding key can't be
                //   cleared without a keymap change or another modifiers request from a virtual
                //   keyboard)
                // - if the virtual keyboard sets the modifiers without taking into account the
                //   existing state and clears a locked modifier, the locked modifier will remain
                //   cleared
                // - a modifier set only through `modifiers` will stay set until the corresponding
                //   key is pressed
                //
                // The `reset-keyboard` action can be used to work around these issues.
                //
                // To handle this properly, we need support for per-device xkb state (and we can
                // merge the modifier state across the xkb states to keep shortcuts working across
                // keyboards) in smithay like sway and hyprland.
                self.process_input_event(InputEvent::<VirtualKeyboardBackend>::DeviceRemoved {
                    device,
                });
            }
            event => self.process_input_event(event),
        }
    }
}

impl State {
    /// Whether the virtual keyboard belongs to the input method that has grabbed the keyboard.
    pub fn is_input_method_virtual_keyboard(&self, device: &VirtualKeyboardDevice) -> bool {
        let grab_client = self.niri.seat.input_method().keyboard_grab_client();
        grab_client.is_some() && grab_client == device.client()
    }

    /// Forwards a key that the input method is passing through directly to the focused surface.
    ///
    /// While an input method holds the keyboard grab, keys go through the binds and then to the
    /// input method, which sends back unhandled ones through its own virtual keyboard. If we
    /// handled these normally, they would get sent back to the grab, looping forever.
    ///
    /// Note that sway also special-cases this in the same way.
    pub fn forward_input_method_key(&mut self, event: &VirtualKeyboardKeyEvent) {
        let serial = SERIAL_COUNTER.next_serial();

        // The modifiers associated with the physical key went to the input method
        // rather than to the surface, so always send the current ones.
        let keyboard = self.niri.seat.get_keyboard().unwrap();
        keyboard.input_forward_bypassing_grab(
            self,
            event.key_code(),
            event.state(),
            serial,
            event.time(),
            true,
        );
    }

    /// Activates the virtual keyboard's keymap on the current seat, unless it already is.
    pub fn activate_virtual_keyboard_keymap(&mut self, device: &VirtualKeyboardDevice) {
        let Some(keymap) = device.keymap() else {
            // The keycodes are relative to the configured keymap, so make sure
            // it's the active one.
            self.restore_configured_keymap();
            return;
        };

        if let Some(active) = &mut self.niri.virtual_keyboard_keymap {
            if active.keymap == keymap {
                // Another virtual keyboard may have installed an identical
                // keymap. Take ownership of it anyway, so that removing that
                // one doesn't take the keymap out from under us.
                active.device = device.clone();
                return;
            }
        }

        // A keymap identical to the configured one (e.g. from an input method
        // forwarding keys with the keymap it received from us) doesn't need to
        // be swapped in, which would needlessly reset the keyboard state.
        if *self.configured_keymap() == *keymap {
            self.restore_configured_keymap();
            return;
        }

        // Save the state from before the first virtual keyboard keymap, since
        // swapping keymaps resets it, and a virtual keyboard should neither
        // affect nor be affected by it.
        let saved = match &self.niri.virtual_keyboard_keymap {
            Some(active) => active.saved,
            None => self.current_keyboard_state(),
        };

        let keyboard = self.niri.seat.get_keyboard().unwrap();
        if let Err(err) = keyboard.set_keymap_from_string(self, keymap.to_string()) {
            warn!("error setting virtual keyboard keymap: {err:?}");
            return;
        }

        self.niri.virtual_keyboard_keymap = Some(VirtualKeyboardKeymap {
            device: device.clone(),
            keymap,
            saved,
        });
        self.ipc_keyboard_layouts_changed();
    }

    /// Switches the seat back from a virtual keyboard's keymap to the configured one, if needed.
    ///
    /// The configured keymap was saved as text before the virtual keyboard's replaced it, so this
    /// doesn't re-read the xkb file or recompile the xkb config unnecessarily.
    pub fn restore_configured_keymap(&mut self) {
        let Some(active) = self.niri.virtual_keyboard_keymap.take() else {
            return;
        };

        let keyboard = self.niri.seat.get_keyboard().unwrap();
        let restored = self.niri.configured_keymap.clone().is_some_and(|keymap| {
            keyboard
                .set_keymap_from_string(self, keymap.to_string())
                .inspect_err(|err| warn!("error restoring configured keymap: {err:?}"))
                .is_ok()
        });

        if !restored {
            // Shouldn't happen unless there's an xkbcommon bug or configured_keymap somehow ends
            // up none while a virtual keymap is active, but fall back to loading it from the
            // config.
            error!("failed to restore the configured keymap, reloading it from the config");
            self.niri.virtual_keyboard_keymap = Some(active);
            self.niri.configured_keymap = None;
            self.load_configured_keymap();
            return;
        }

        self.restore_keyboard_state(active.saved);
        self.ipc_keyboard_layouts_changed();
    }

    /// Returns the configured keymap as XKB_KEYMAP_FORMAT_TEXT_V1.
    fn configured_keymap(&mut self) -> Arc<str> {
        if let Some(keymap) = &self.niri.configured_keymap {
            return keymap.clone();
        }

        // The seat has the configured keymap while no virtual keyboard keymap is active.
        if self.niri.virtual_keyboard_keymap.is_some() {
            error!("virtual_keyboard_keymap must be None when resetting configured_keymap");
        }

        let keyboard = self.niri.seat.get_keyboard().unwrap();
        let keymap: Arc<str> = keyboard
            .with_xkb_state(self, |context| {
                context.xkb().lock().unwrap().keymap_as_string()
            })
            .into();
        self.niri.configured_keymap = Some(keymap.clone());
        keymap
    }
}
