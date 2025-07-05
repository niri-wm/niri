//! Module containing the parts of the implementation of remote desktop that need global state,
//! including EIS (=emulated input server).

use std::collections::HashMap;

use calloop::RegistrationToken;
use enumflags2::BitFlags;
use reis::calloop::{EisRequestSource, EisRequestSourceEvent};
use reis::ei::device::DeviceType;
use reis::ei::keyboard::KeymapType;
use reis::eis;
use reis::event::{DeviceCapability, Region};
use reis::request::{Device as EiDevice, EisRequest, Seat as EiSeat};
use smithay::backend::input::{KeyState, Keycode};
use smithay::input::keyboard::{xkb, KeymapFile, Keysym, ModifiersState, SerializedMods};
use smithay::utils::{Logical, Size};

use crate::dbus::mutter_remote_desktop::{MutterXdpDeviceType, RemoteDesktopDBusToCalloop};
use crate::input::eis_backend::{
    AbsolutePositionEventExtra, EisEventAdapter, EisInputBackend, PressedCount, ScrollFrame,
    TouchFrame,
};
use crate::input::remote_desktop_backend::{RdEventAdapter, RdInputBackend, RdKeyboardKeyEvent};
use crate::niri::State;
use crate::utils::RemoteDesktopSessionId;

/// Processes an input event with the EIS event adapter.
macro_rules! process_event {
    ($global_state:expr, $context_state:expr, $inner:expr, $ident:ident) => {{
        process_event!($global_state, $context_state, $inner, $ident, ())
    }};
    ($global_state:expr, $context_state:expr, $inner:expr, $ident:ident, $extra:expr) => {{
        $global_state.process_input_event(InputEvent::$ident {
            event: EisEventAdapter {
                session_id: $context_state.session_id,
                inner: $inner,
                extra: $extra,
            },
        });
    }};
}

type InputEvent = smithay::backend::input::InputEvent<EisInputBackend>;

/// Child struct of the global [`State`] struct
#[derive(Default)]
pub struct RemoteDesktopState {
    /// Active EI sessions.
    active_ei_sessions: HashMap<RemoteDesktopSessionId, RegistrationToken>,

    /// Counts the number of remote desktop sessions requiring touch capability on the seat.
    ///
    /// Modified by both [`crate::dbus::mutter_remote_desktop`] (D-Bus) and EIS.
    pub touch_session_counter: usize,
}
impl RemoteDesktopState {
    /// Whether touch capability on the seat is needed.
    pub fn needs_touch_cap(&self) -> bool {
        self.touch_session_counter > 0
    }
}

/// The state for an EI connection.
struct ContextState {
    seat: Option<EiSeat>,
    /// The number of keys pressed on all devices in the seat.
    key_counter: u32,
    exposed_device_types: BitFlags<MutterXdpDeviceType>,
    session_id: RemoteDesktopSessionId,
    /// A scroll frame being filled in by the different scroll events. Assumes there is only
    /// one device that can emit scroll levents.
    scroll_frame: Option<ScrollFrame>,
    /// Whether to send a [`smithay::backend::input::TouchFrameEvent`] when an EIS frame
    /// request is received.
    next_frame_touch: bool,
    // TODO: these are stored for reload integration. e.g. keymap has to be reloaded by recreating
    // the ei_device completely
    keyboard_device: Option<EiDevice>,
    mouse_device: Option<EiDevice>,
    touch_device: Option<EiDevice>,
    regions: Vec<Region>,
    global_extent: Size<f64, Logical>,
}

impl State {
    pub fn on_remote_desktop_msg_from_dbus(&mut self, msg: RemoteDesktopDBusToCalloop) {
        match msg {
            RemoteDesktopDBusToCalloop::RemoveEisHandler { session_id } => {
                if let Some(token) = self
                    .niri
                    .remote_desktop
                    .active_ei_sessions
                    .remove(&session_id)
                {
                    self.niri.event_loop.remove(token);
                } else {
                    warn!(
                        "RemoteDesktop RemoveEisHandler: Invalid session ID {}",
                        session_id
                    )
                }
            }
            RemoteDesktopDBusToCalloop::NewEisContext {
                session_id,
                ctx,
                exposed_device_types,
            } => {
                // This lives with the closure
                let mut context_state = ContextState {
                    seat: None,
                    key_counter: 0,
                    exposed_device_types,
                    session_id,
                    scroll_frame: None,
                    next_frame_touch: false,
                    keyboard_device: None,
                    mouse_device: None,
                    touch_device: None,
                    regions: Vec::new(),
                    global_extent: Size::new(0., 0.),
                };

                let token = self
                    .niri
                    .event_loop
                    .insert_source(
                        EisRequestSource::new(ctx, 1),
                        move |event, connection, state| {
                            match handle_eis_request_source_event(
                                event,
                                connection,
                                state,
                                &mut context_state,
                            ) {
                                Ok(post_action) => {
                                    if post_action != calloop::PostAction::Continue {
                                        debug!("EIS connection {post_action:?}");
                                    }
                                    Ok(post_action)
                                }
                                Err(err) => {
                                    // Always Ok because we never want to error out of the entire
                                    // event loop
                                    warn!("Error while flushing connection: {err}");
                                    Ok(calloop::PostAction::Remove)
                                }
                            }
                        },
                    )
                    .unwrap();
                self.niri
                    .remote_desktop
                    .active_ei_sessions
                    .insert(session_id, token);
            }
            RemoteDesktopDBusToCalloop::EmulateInput(event) => self.process_input_event(event),
            RemoteDesktopDBusToCalloop::EmulateKeysym {
                keysym,
                state,
                session_id,
                time,
            } => {
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

                self.process_input_event::<RdInputBackend>(
                    smithay::backend::input::InputEvent::Keyboard {
                        event: RdEventAdapter {
                            session_id,
                            time,
                            inner: RdKeyboardKeyEvent { keycode, state },
                        },
                    },
                );
            }
            RemoteDesktopDBusToCalloop::IncTouchSession => {
                self.niri.remote_desktop.touch_session_counter += 1;

                self.refresh_wayland_device_caps();
            }
            RemoteDesktopDBusToCalloop::DecTouchSession => {
                self.niri.remote_desktop.touch_session_counter = self
                    .niri
                    .remote_desktop
                    .touch_session_counter
                    .saturating_sub(1);

                self.refresh_wayland_device_caps();
            }
        }
    }
}

fn handle_eis_request_source_event(
    event: Result<EisRequestSourceEvent, reis::Error>,
    connection: &mut reis::request::Connection,
    global_state: &mut State,
    context_state: &mut ContextState,
) -> Result<calloop::PostAction, std::io::Error> {
    Ok(match event {
        Ok(event) => match event {
            EisRequestSourceEvent::Connected => {
                debug!("EIS connected!");
                if !connection.has_interface("ei_seat") || !connection.has_interface("ei_device") {
                    connection.disconnected(
                        eis::connection::DisconnectReason::Protocol,
                        Some("Need `ei_seat` and `ei_device`"),
                    );
                    connection.flush()?;
                    return Ok(calloop::PostAction::Remove);
                }

                let seat = connection.add_seat(
                    Some("default"),
                    MutterXdpDeviceType::to_reis_capabilities(context_state.exposed_device_types),
                );

                context_state.seat = Some(seat);

                connection.flush()?;
                calloop::PostAction::Continue
            }
            EisRequestSourceEvent::Request(request) => {
                debug!("EIS request! {:#?}", request);
                let post_action =
                    handle_eis_request(request, connection, global_state, context_state);
                connection.flush()?;
                post_action
            }
        },
        Err(err) => {
            warn!("EIS protocol error: {err}");
            connection.disconnected(
                eis::connection::DisconnectReason::Protocol,
                Some(&err.to_string()),
            );
            connection.flush()?;
            calloop::PostAction::Remove
        }
    })
}

// TODO: send ei_keyboard.modifiers when other keyboards change modifier state?
// TODO: recreate keyboard with new keymaps
// ^ Waiting for https://github.com/Smithay/smithay/issues/1776

/// Creates an EI keyboard if the capabilities match.
///
/// The device must be [`EiDevice::resumed`] for clients to request to
/// [`EisRequest::DeviceStartEmulating`] input.
fn create_ei_keyboard(
    seat: &EiSeat,
    capabilities: BitFlags<DeviceCapability>,
    connection: &mut reis::request::Connection,
    global_state: &mut State,
) -> Option<EiDevice> {
    (capabilities.contains(DeviceCapability::Keyboard) && connection.has_interface("ei_keyboard"))
        .then(|| {
            seat.add_device(
                Some("keyboard"),
                DeviceType::Virtual,
                DeviceCapability::Keyboard.into(),
                |device| {
                    let keyboard: reis::eis::Keyboard = device
                        .interface()
                        .expect("Should exist because it was just defined");

                    let file = global_state
                        .niri
                        .seat
                        .get_keyboard()
                        .unwrap()
                        .with_xkb_state(global_state, |context| {
                            let xkb = context.xkb().lock().unwrap();

                            // SAFETY: the keymap's ref count isn't increased
                            let keymap = unsafe { xkb.keymap() };
                            KeymapFile::new(keymap)
                        });

                    // > The fd must be mapped with MAP_PRIVATE by the recipient, as MAP_SHARED may fail.
                    //
                    // EI protocol allows us to use anonymous, sealed files.
                    file.with_fd(true, |fd, size| {
                        // Smithay also does this cast
                        keyboard.keymap(KeymapType::Xkb, size as u32, fd);
                    })
                    .unwrap();
                    debug!("Sent keymap file");
                },
            )
        })
}

/// Creates an EI mouse if the capabilities match.
///
/// The device must be [`EiDevice::resumed`] for clients to request to
/// [`EisRequest::DeviceStartEmulating`] input.
fn create_ei_mouse(
    seat: &EiSeat,
    capabilities: BitFlags<DeviceCapability>,
    connection: &mut reis::request::Connection,
) -> Option<EiDevice> {
    let mut mouse_capabilities = BitFlags::empty();

    let mut check_mouse_cap = |capability, interface| {
        // We check for the interfaces' existence because the client may send
        // a 0xffffffffffffffff and then any events we send to the sub-interfaces will be
        // protocol violations.
        if capabilities.contains(capability) && connection.has_interface(interface) {
            mouse_capabilities |= capability;
        }
    };

    check_mouse_cap(DeviceCapability::Pointer, "ei_pointer");
    check_mouse_cap(DeviceCapability::Scroll, "ei_scroll");
    check_mouse_cap(DeviceCapability::Button, "ei_button");
    check_mouse_cap(DeviceCapability::PointerAbsolute, "ei_pointer_absolute");

    (!mouse_capabilities.is_empty()).then(|| {
        seat.add_device(
            Some("mouse"),
            DeviceType::Virtual,
            mouse_capabilities,
            |_| {},
        )
    })
}

/// Creates an EI keyboard if the capabilities match.
///
/// The device must be [`EiDevice::resumed`] for clients to request to
/// [`EisRequest::DeviceStartEmulating`] input.
fn create_ei_touchscreen(
    seat: &EiSeat,
    capabilities: BitFlags<DeviceCapability>,
    connection: &mut reis::request::Connection,
) -> Option<EiDevice> {
    (capabilities.contains(DeviceCapability::Touch) && connection.has_interface("ei_touchscreen"))
        .then(|| {
            seat.add_device(
                Some("touchscreen"),
                DeviceType::Virtual,
                DeviceCapability::Touch.into(),
                |_| {},
            )
        })
}

/// (Re)creates EI devices.
fn create_ei_devices(
    connection: &mut reis::request::Connection,
    global_state: &mut State,
    context_state: &mut ContextState,
    seat: EiSeat,
    capabilities: BitFlags<DeviceCapability, u64>,
) {
    let mut touch_device_count_delta = if context_state.touch_device.is_some() {
        -1
    } else {
        0
    };

    for old_device_slot in [
        &mut context_state.keyboard_device,
        &mut context_state.mouse_device,
        &mut context_state.touch_device,
    ]
    .into_iter()
    {
        if let Some(old_device) = old_device_slot {
            old_device.remove();
            *old_device_slot = None;
        }
    }

    if let Some(device) = create_ei_keyboard(&seat, capabilities, connection, global_state) {
        advertise_regions(&device, &context_state.regions);
        device.resumed();
        context_state.keyboard_device = Some(device);
    }

    if let Some(device) = create_ei_mouse(&seat, capabilities, connection) {
        advertise_regions(&device, &context_state.regions);
        device.resumed();
        context_state.mouse_device = Some(device);
    }

    if let Some(device) = create_ei_touchscreen(&seat, capabilities, connection) {
        advertise_regions(&device, &context_state.regions);
        device.resumed();
        context_state.touch_device = Some(device);

        touch_device_count_delta += 1;
    }

    global_state.niri.remote_desktop.touch_session_counter = global_state
        .niri
        .remote_desktop
        .touch_session_counter
        .saturating_add_signed(touch_device_count_delta);
    if touch_device_count_delta != 0 {
        global_state.refresh_wayland_device_caps();
    }
}

/// Advertises regions on EI devices.
fn advertise_regions(device: &EiDevice, regions: &[Region]) {
    for region in regions {
        device.device().region(
            region.x,
            region.y,
            region.width,
            region.height,
            region.scale,
        );
        if let Some(mapping_id) = &region.mapping_id {
            device.device().region_mapping_id(mapping_id);
        }
    }
}

fn handle_eis_request(
    request: reis::request::EisRequest,
    connection: &mut reis::request::Connection,
    global_state: &mut State,
    context_state: &mut ContextState,
) -> calloop::PostAction {
    match request {
        EisRequest::Disconnect => {
            return calloop::PostAction::Remove;
        }
        EisRequest::Bind(reis::request::Bind { seat, capabilities }) => {
            if capabilities
                & MutterXdpDeviceType::to_reis_capabilities(context_state.exposed_device_types)
                != capabilities
            {
                connection.disconnected(
                    eis::connection::DisconnectReason::Value,
                    Some("Binding to invalid capabilities"),
                );
                return calloop::PostAction::Remove;
            }

            // TODO: Why not combine everything into a single device?

            create_ei_devices(connection, global_state, context_state, seat, capabilities);
        }

        EisRequest::DeviceStartEmulating(inner) => {}
        EisRequest::DeviceStopEmulating(inner) => {}

        EisRequest::PointerMotion(inner) => {
            process_event!(global_state, context_state, inner, PointerMotion)
        }

        EisRequest::PointerMotionAbsolute(inner) => {
            process_event!(
                global_state,
                context_state,
                inner,
                PointerMotionAbsolute,
                AbsolutePositionEventExtra {
                    global_extent: context_state.global_extent
                }
            )
        }

        EisRequest::Button(inner) => {
            process_event!(global_state, context_state, inner, PointerButton)
        }

        EisRequest::ScrollDelta(inner) => {
            let scroll_frame = context_state
                .scroll_frame
                .get_or_insert_with(Default::default);
            scroll_frame.delta = Some((inner.dx, inner.dy));
        }

        EisRequest::ScrollStop(inner) => {
            let scroll_frame = context_state
                .scroll_frame
                .get_or_insert_with(Default::default);
            scroll_frame.stop = Some(((inner.x, inner.y), false));
        }

        EisRequest::ScrollCancel(inner) => {
            let scroll_frame = context_state
                .scroll_frame
                .get_or_insert_with(Default::default);
            scroll_frame.stop = Some(((inner.x, inner.y), true));
        }

        EisRequest::ScrollDiscrete(inner) => {
            let scroll_frame = context_state
                .scroll_frame
                .get_or_insert_with(Default::default);
            scroll_frame.discrete = Some((inner.discrete_dx, inner.discrete_dy));
        }

        EisRequest::Frame(inner) => {
            if let Some(scroll_frame) = context_state.scroll_frame.take() {
                process_event!(
                    global_state,
                    context_state,
                    inner.clone(),
                    PointerAxis,
                    scroll_frame
                )
            }

            if context_state.next_frame_touch {
                process_event!(
                    global_state,
                    context_state,
                    inner.clone(),
                    TouchFrame,
                    TouchFrame
                );
                context_state.next_frame_touch = false;
            }
        }

        EisRequest::KeyboardKey(inner) => {
            // This is super naive but Smithay's winit and x11 input adapters do this.
            match inner.state {
                reis::ei::keyboard::KeyState::Released => {
                    context_state.key_counter = context_state.key_counter.saturating_sub(1)
                }
                reis::ei::keyboard::KeyState::Press => context_state.key_counter += 1,
            }

            process_event!(
                global_state,
                context_state,
                inner,
                Keyboard,
                PressedCount(context_state.key_counter)
            );
        }

        EisRequest::TouchDown(inner) => {
            process_event!(
                global_state,
                context_state,
                inner,
                TouchDown,
                AbsolutePositionEventExtra {
                    global_extent: context_state.global_extent
                }
            );
            context_state.next_frame_touch = true;
        }
        EisRequest::TouchMotion(inner) => {
            process_event!(
                global_state,
                context_state,
                inner,
                TouchMotion,
                AbsolutePositionEventExtra {
                    global_extent: context_state.global_extent
                }
            );
            context_state.next_frame_touch = true;
        }
        EisRequest::TouchUp(inner) => {
            process_event!(global_state, context_state, inner, TouchUp);
            context_state.next_frame_touch = true;
        }
        EisRequest::TouchCancel(inner) => {
            process_event!(global_state, context_state, inner, TouchCancel);
            context_state.next_frame_touch = true;
        }
    }

    calloop::PostAction::Continue
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
