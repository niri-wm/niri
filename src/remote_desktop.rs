//! Module containing the parts of the implementation of remote desktop that need global state,
//! including EIS (=emulated input server).

use std::collections::HashMap;

use calloop::RegistrationToken;
use enumflags2::BitFlags;
use smithay::backend::input::{InputTime, KeyState, Keycode};
use smithay::backend::libei::{EiInput, EiInputConnection, EiInputEvent, EiInputSeat, EiRegion};
use smithay::input::keyboard::{xkb, Keysym, ModifiersState, SerializedMods};
use smithay::utils::{Point, Rectangle, Size};

use crate::backend::IpcOutputMap;
use crate::dbus::mutter_remote_desktop::{MutterXdpDeviceType, RemoteDesktopDBusToCalloop};
use crate::input::dbus_remote_desktop_backend::{
    RdEventAdapter, RdInputBackend, RdKeyboardKeyEvent,
};
use crate::niri::State;
use crate::utils::{global_bounding_rectangle_ipc, RemoteDesktopSessionId};

/// Child struct of the global [`State`] struct
#[derive(Default)]
pub struct RemoteDesktopState {
    /// Active EI sessions.
    active_ei_sessions: HashMap<RemoteDesktopSessionId, EiSessionState>,

    /// Counts the number of remote desktop sessions requiring touch capability on the seat.
    ///
    /// Modified by [`crate::dbus::mutter_remote_desktop`] (D-Bus).
    pub dbus_touch_session_counter: usize,
}
impl RemoteDesktopState {
    /// Whether touch capability on the seat is needed.
    pub fn needs_touch_cap(&self) -> bool {
        self.dbus_touch_session_counter > 0
            || self
                .active_ei_sessions
                .values()
                .any(|sess| sess.touch_devices > 0)
    }
}

/// The state for an EI session (not the `ei_connection` object but
/// [`Context`](reis::eis::Context)).
struct EiSessionState {
    /// The EI seat, available once the client has finished the handshake.
    seat: Option<EiInputSeat>,
    /// The device types exposed to the EI client by the portal.
    exposed_device_types: BitFlags<MutterXdpDeviceType>,
    /// The number of EI touch devices created for this session.
    touch_devices: usize,
    event_loop_token: RegistrationToken,
}

/// Returns the [EI regions](EiRegion) to advertise for the current outputs.
fn ei_regions(ipc_outputs: &IpcOutputMap) -> Vec<EiRegion> {
    let Some(bounding_rect) = global_bounding_rectangle_ipc(ipc_outputs) else {
        return Vec::new();
    };

    ipc_outputs
        .values()
        .filter_map(|output| {
            let logical = output.logical?;

            Some(EiRegion {
                // EI doesn't allow negative positions, so the regions are offset by the
                // bounding rectangle origin. Absolute positions get the origin added back
                // in `NiriInputDevice::absolute_position_origin`.
                rect: Rectangle::new(
                    Point::from((
                        logical.x - bounding_rect.loc.x,
                        logical.y - bounding_rect.loc.y,
                    )),
                    Size::from((logical.width as i32, logical.height as i32)),
                ),
                scale: logical.scale as f32,
                mapping_id: Some(output.name.clone()),
            })
        })
        .collect()
}

impl State {
    pub fn on_ipc_outputs_changed_remote_desktop(&mut self) {
        let regions = {
            let ipc_outputs = self.backend.ipc_outputs();
            let ipc_outputs = ipc_outputs.lock().unwrap();
            ei_regions(&ipc_outputs)
        };

        // Update the advertised regions on all EI seats. Smithay recreates the affected
        // absolute pointer and touch devices internally.
        for sess in self.niri.remote_desktop.active_ei_sessions.values() {
            if let Some(seat) = &sess.seat {
                seat.update_regions(&regions);
            }
        }
    }

    pub fn on_remote_desktop_msg_from_dbus(&mut self, msg: RemoteDesktopDBusToCalloop) {
        match msg {
            RemoteDesktopDBusToCalloop::RemoveEisHandler { session_id } => {
                if let Some(sess) = self
                    .niri
                    .remote_desktop
                    .active_ei_sessions
                    .remove(&session_id)
                {
                    self.niri.event_loop.remove(sess.event_loop_token);
                    self.refresh_wayland_device_caps();
                } else {
                    // Normal when the EI client disconnected first.
                    debug!("RemoteDesktop RemoveEisHandler: no session {}", session_id);
                }
            }
            RemoteDesktopDBusToCalloop::NewEisContext {
                session_id,
                ctx,
                exposed_device_types,
            } => {
                self.create_new_eis_context(session_id, ctx, exposed_device_types);
            }

            RemoteDesktopDBusToCalloop::EmulateInput(event) => self.process_input_event(event),
            RemoteDesktopDBusToCalloop::EmulateKeysym {
                keysym,
                state,
                session_id,
                time,
            } => self.emulate_keysym(keysym, state, session_id, time),

            RemoteDesktopDBusToCalloop::IncTouchSession => {
                self.niri.remote_desktop.dbus_touch_session_counter += 1;

                self.refresh_wayland_device_caps();
            }
            RemoteDesktopDBusToCalloop::DecTouchSession => {
                self.niri.remote_desktop.dbus_touch_session_counter = self
                    .niri
                    .remote_desktop
                    .dbus_touch_session_counter
                    .saturating_sub(1);

                self.refresh_wayland_device_caps();
            }
        }
    }

    /// Emulates a keysym by looking up a keycode and modifiers in the current keymap.
    pub fn emulate_keysym(
        &mut self,
        keysym: u32,
        state: KeyState,
        session_id: RemoteDesktopSessionId,
        time: InputTime,
    ) {
        let keysym = Keysym::from(keysym);

        let keyboard_handle = self.niri.seat.get_keyboard().unwrap();

        let prev_mods_state = keyboard_handle.modifier_state();

        let Some((keycode, mod_mask, mods_state)) =
            keyboard_handle.with_xkb_state(self, |context| {
                let xkb = context.xkb().lock().unwrap();

                // SAFETY: the state's ref count isn't increased
                let xkb_state = unsafe { xkb.state() };

                // SAFETY: the keymap's ref count isn't increased
                let keymap = unsafe { xkb.keymap() };

                let (keycode, mod_mask) = keysym_to_keycode(xkb_state, keymap, keysym)?;

                let mut new_serialized = prev_mods_state.serialized;
                match state {
                    KeyState::Pressed => new_serialized.depressed |= mod_mask,
                    KeyState::Released => new_serialized.depressed &= !mod_mask,
                }

                // Turn into `ModifiersState`
                let mods_state = deserialize_mods(new_serialized, keymap);

                Some((keycode, mod_mask, mods_state))
            })
        else {
            // TODO: update when multi keyboard layouts/groups is supported in search
            warn!(
                "Couldn't find keycode for keysym {} (raw {}) in the current keyboard layout",
                keysym.name().unwrap_or_default(),
                keysym.raw()
            );
            return;
        };

        debug!(
            "{} Emulating keysym={:12} X11 keycode={: <3} depressed={:#04b}, latched={:#04b}, locked={:#04b}, mod_mask={mod_mask:#04b}, prev mod mask={:#04b}",
            match state {
                KeyState::Pressed => "╭",
                KeyState::Released => "╰"
            },
            keysym.name().unwrap_or_default(), // Used only for debug
            keycode.raw(),
            mods_state.serialized.depressed,
            mods_state.serialized.latched,
            mods_state.serialized.locked,
            prev_mods_state.serialized.depressed
              | prev_mods_state.serialized.latched
              | prev_mods_state.serialized.locked,
        );

        let modifiers_changed = keyboard_handle.set_modifier_state(mods_state);
        if modifiers_changed != 0 {
            keyboard_handle.advertise_modifier_state(self);
        }

        self.process_input_event::<RdInputBackend>(smithay::backend::input::InputEvent::Keyboard {
            event: RdEventAdapter {
                session_id,
                time,
                inner: RdKeyboardKeyEvent { keycode, state },
            },
        });
    }

    fn create_new_eis_context(
        &mut self,
        session_id: RemoteDesktopSessionId,
        ctx: reis::eis::Context,
        exposed_device_types: BitFlags<MutterXdpDeviceType, u32>,
    ) {
        let event_loop_token = self
            .niri
            .event_loop
            .insert_source(EiInput::new(ctx), move |event, connection, state| {
                handle_ei_input_event(state, session_id, event, connection);
            })
            .unwrap();

        let conn_state = EiSessionState {
            seat: None,
            exposed_device_types,
            touch_devices: 0,
            event_loop_token,
        };

        self.niri
            .remote_desktop
            .active_ei_sessions
            .insert(session_id, conn_state);
    }
}

fn handle_ei_input_event(
    global_state: &mut State,
    session_id: RemoteDesktopSessionId,
    event: EiInputEvent,
    connection: &mut EiInputConnection,
) {
    match event {
        EiInputEvent::Connected => {
            debug!("EIS connected!");

            let exposed_device_types = global_state
                .niri
                .remote_desktop
                .active_ei_sessions
                .get(&session_id)
                .expect("remote desktop session being processed should exist")
                .exposed_device_types;

            let regions = {
                let ipc_outputs = global_state.backend.ipc_outputs();
                let ipc_outputs = ipc_outputs.lock().unwrap();
                ei_regions(&ipc_outputs)
            };

            let seat = connection.add_seat("default");

            if exposed_device_types.contains(MutterXdpDeviceType::Keyboard) {
                let config = global_state.niri.config.borrow();
                let xkb = config.input.keyboard.xkb.to_xkb_config();
                if let Err(err) = seat.add_keyboard("keyboard", xkb) {
                    warn!("error adding EI keyboard: {err:?}");
                }
                // TODO: do we need to call EiInputEvent::keyboard_modifiers()?

                // TODO: implement UTF-8 text input? EiInputSeat::add_text()
            }

            if exposed_device_types.contains(MutterXdpDeviceType::Pointer) {
                seat.add_pointer("mouse");
                seat.add_pointer_absolute("mouse", &regions);
            }

            if exposed_device_types.contains(MutterXdpDeviceType::Touchscreen) {
                seat.add_touch("touchscreen", &regions);
            }

            global_state
                .niri
                .remote_desktop
                .active_ei_sessions
                .get_mut(&session_id)
                .expect("remote desktop session being processed should exist")
                .seat = Some(seat);
        }
        EiInputEvent::Disconnected => {
            debug!("EIS disconnected");

            // The EI connection can end independently of the D-Bus session (the client may
            // just close the socket), so clean up here. If the D-Bus session is dropped
            // later, its `RemoveEisHandler` will find nothing.
            //
            // Note: a client that fails the handshake never emits this event (reis removes
            // the source silently), so its session entry lingers until the D-Bus session is
            // dropped. That's harmless because no devices were created so `touch_devices` is 0.
            global_state
                .niri
                .remote_desktop
                .active_ei_sessions
                .remove(&session_id);
            global_state.refresh_wayland_device_caps();
        }
        EiInputEvent::Event(event) => {
            let Some(sess) = global_state
                .niri
                .remote_desktop
                .active_ei_sessions
                .get_mut(&session_id)
            else {
                return;
            };

            match &event {
                smithay::backend::input::InputEvent::DeviceAdded { device }
                    if device.has_capability(reis::request::DeviceCapability::Touch) =>
                {
                    sess.touch_devices += 1;
                }
                smithay::backend::input::InputEvent::DeviceRemoved { device }
                    if device.has_capability(reis::request::DeviceCapability::Touch) =>
                {
                    sess.touch_devices = sess.touch_devices.saturating_sub(1);
                }
                _ => (),
            }

            // This also refreshes the Wayland device capabilities (e.g. wl_touch), which
            // should happen after updating the touch device count above.
            global_state.process_input_event(event);
        }
        EiInputEvent::TextKeysym { keysym, state } => {
            global_state.emulate_keysym(keysym, state, session_id, InputTime::now());
        }
        EiInputEvent::TextUtf8 { text } => {
            debug!("ignoring text {text:?} from EI client");
        }
    }
}

/// Reconstructs symbolic meanings of modifiers ([`ModifiersState`]) from serialized modifiers.
///
/// The modifiers are active when they're present in any of the masks (depressed, latched or
/// locked).
///
/// This is the inverse of [`ModifiersState::serialize_back`].
fn deserialize_mods(serialized: SerializedMods, keymap: &xkb::Keymap) -> ModifiersState {
    let mod_mask = serialized.depressed | serialized.latched | serialized.locked;

    let is_index_active = |index| (mod_mask & (1u32 << index)) != 0;
    let is_mod_active = |name| {
        let index = keymap.mod_get_index(name);
        if index == xkb::MOD_INVALID {
            false
        } else {
            is_index_active(index)
        }
    };

    ModifiersState {
        caps_lock: is_mod_active(xkb::MOD_NAME_CAPS),
        num_lock: is_mod_active(xkb::MOD_NAME_NUM),
        ctrl: is_mod_active(xkb::MOD_NAME_CTRL),
        alt: is_mod_active(xkb::MOD_NAME_ALT),
        shift: is_mod_active(xkb::MOD_NAME_SHIFT),
        logo: is_mod_active(xkb::MOD_NAME_LOGO),
        iso_level3_shift: is_mod_active(xkb::MOD_NAME_ISO_LEVEL3_SHIFT),
        iso_level5_shift: is_mod_active(xkb::MOD_NAME_MOD3),
        serialized,
    }
}

/// Scans the given `keymap` and returns the first keycode (as u32) that produces `keysym` in any
/// level. If none found, returns `None`.
// TODO: Try other groups too, because it's basically trivial to switch groups with
// wl_keyboard.modifiers
fn keysym_to_keycode(
    state: &xkb::State,
    keymap: &xkb::Keymap,
    target_keysym: Keysym,
) -> Option<(Keycode, xkb::ModMask)> {
    let min = keymap.min_keycode().raw();
    let max = keymap.max_keycode().raw();

    let layout_index = state.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE);

    for keycode in min..=max {
        let keycode = Keycode::new(keycode);

        // Skip unused keycodes
        if keymap.key_get_name(keycode).is_none() {
            continue;
        }

        let num_levels = keymap.num_levels_for_key(keycode, layout_index);
        for level_index in 0..num_levels {
            let syms = keymap.key_get_syms_by_level(keycode, layout_index, level_index);

            if syms != [target_keysym] {
                // Inequal or nonzero count
                continue;
            };

            let mut mod_mask = xkb::ModMask::default();
            let num_masks = keymap.key_get_mods_for_level(
                keycode,
                layout_index,
                level_index,
                std::array::from_mut(&mut mod_mask),
            );

            if num_masks == 0 {
                error!(
                    "Couldn't retrieve modifiers for keycode {} and level {}",
                    keycode.raw(),
                    level_index + 1
                );
                return None;
            }

            return Some((keycode, mod_mask));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // Evdev keycode constants from `input-event-codes.h`
    const KEY_SPACE: u32 = 57;
    const KEY_Q: u32 = 16;
    const KEY_A: u32 = 30;

    #[test]
    fn space_to_keycode() {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap =
            xkb::Keymap::new_from_names(&ctx, "", "", "us", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)
                .expect("Failed to compile keymap");
        let state = xkb::State::new(&keymap);

        let keysym = Keysym::space;
        let (keycode, mod_mask) =
            keysym_to_keycode(&state, &keymap, keysym).expect("Could not find keycodepace");

        assert_eq!(keycode.raw(), KEY_SPACE + 8);
        assert_eq!(mod_mask, 0);
    }

    #[test]
    fn keysym_to_keycode_multilayout() {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(
            &ctx,
            "",
            "",
            "us,fr",
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("Failed to compile keymap");

        let mut state = xkb::State::new(&keymap);

        // Test the Q key on QWERTY and AZERTY layouts
        let keysym = Keysym::q;

        let (keycode, mod_mask) =
            keysym_to_keycode(&state, &keymap, keysym).expect("Could not find keycode");

        assert_eq!(keycode.raw(), KEY_Q + 8);
        assert_eq!(mod_mask, 0);

        // Wayland clients insert the `group` field of `wl_keyboard.modifiers` into `locked_layout`
        state.update_mask(0, 0, 0, 0, 0, 1);

        let (keycode, mod_mask) =
            keysym_to_keycode(&state, &keymap, keysym).expect("Could not find keycode");

        assert_eq!(keycode.raw(), KEY_A + 8);
        assert_eq!(mod_mask, 0);
    }
}
