//! Module containing the parts of the implementation of remote desktop that need global state,
//! including EIS (=emulated input server).

use std::collections::HashMap;

use calloop::RegistrationToken;
use enumflags2::BitFlags;
use reis::calloop::{EisRequestSource, EisRequestSourceEvent};
use reis::ei::device::DeviceType;
use reis::ei::keyboard::KeymapType;
use reis::eis;
use reis::event::DeviceCapability;
use reis::request::{Device as EiDevice, EisRequest, Seat as EiSeat};
use smithay::backend::input::{KeyState, Keycode};
use smithay::input::keyboard::{xkb, KeymapFile, Keysym, ModifiersState, SerializedMods};
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::backend::IpcOutputMap;
use crate::dbus::mutter_remote_desktop::{MutterXdpDeviceType, RemoteDesktopDBusToCalloop};
use crate::input::dbus_remote_desktop_backend::{
    RdEventAdapter, RdInputBackend, RdKeyboardKeyEvent,
};
use crate::input::eis_backend::{
    AbsolutePositionEventExtra, EisEventAdapter, EisInputBackend, PressedCount, ScrollFrame,
    TouchFrame,
};
use crate::niri::State;
use crate::utils::{global_bounding_rectangle_ipc, RemoteDesktopSessionId};

/// Processes an input event with the EIS event adapter.
macro_rules! process_event {
    ($global_state:expr, $session_id:expr, $inner:expr, $variant:ident) => {{
        process_event!($global_state, $session_id, $inner, $variant, ())
    }};
    ($global_state:expr, $session_id:expr, $inner:expr, $variant:ident, $extra:expr) => {{
        $global_state.process_input_event(InputEvent::$variant {
            event: EisEventAdapter {
                session_id: $session_id,
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
    active_ei_sessions: HashMap<RemoteDesktopSessionId, ConnectionState>,

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
                .any(|sess| sess.needs_touch_cap())
    }
}

/// The state for an EI connection (not the `ei_connection` object but
/// [`Context`](reis::eis::Context)).
struct ConnectionState {
    last_capabilities: Option<BitFlags<DeviceCapability, u64>>,
    ei_connection: Option<reis::request::Connection>,
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

    // Stored for e.g. reload integration
    keyboard_device: Option<EiDevice>,
    mouse_device: Option<EiDevice>,
    touch_device: Option<EiDevice>,

    event_loop_token: RegistrationToken,
}
impl ConnectionState {
    /// Whether this session requires touch capabilities on the seat.
    fn needs_touch_cap(&self) -> bool {
        self.touch_device.is_some()
    }

    /// Returns the rectangle that covers all [EI regions](reis::event::Region) advertised to the EI
    /// client.
    ///
    /// Ignores location as we offset it to 0 in [`advertise_regions`].
    fn regions_extent(&self, backend: &crate::backend::Backend) -> Option<Rectangle<f64, Logical>> {
        let ipc_outputs = backend.ipc_outputs();
        let ipc_outputs = ipc_outputs.lock().unwrap();

        let global_extent = global_bounding_rectangle_ipc(&ipc_outputs)?;

        Some(Rectangle::new(
            // EI doesn't allow negative positions so this is offset to 0 when advertising
            // `ei_region`s.
            Point::default(),
            Size::new(global_extent.size.w as f64, global_extent.size.h as f64),
        ))
    }
}

impl State {
    pub fn on_ipc_outputs_changed_remote_desktop(&mut self) {
        // recreate EI devices
        let cloned: Vec<_> = self
            .niri
            .remote_desktop
            .active_ei_sessions
            .values()
            .filter_map(|sess| {
                Some((
                    sess.session_id,
                    sess.ei_connection.clone()?,
                    sess.seat.clone()?,
                    sess.last_capabilities?,
                ))
            })
            .collect();

        for (session_id, ei_conn, seat, last_caps) in cloned {
            create_ei_devices(&ei_conn, self, session_id, seat, last_caps);
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
                self.create_new_eis_context(session_id, ctx, exposed_device_types);
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

    fn create_new_eis_context(
        &mut self,
        session_id: RemoteDesktopSessionId,
        ctx: eis::Context,
        exposed_device_types: BitFlags<MutterXdpDeviceType, u32>,
    ) {
        let event_loop_token = self
            .niri
            .event_loop
            .insert_source(
                EisRequestSource::new(ctx, 1),
                move |event, connection, state| {
                    let mut post_action =
                        handle_eis_request_source_event(event, connection, state, session_id);
                    if post_action != calloop::PostAction::Continue {
                        debug!("EIS connection {post_action:?}");
                    }
                    if let Err(err) = connection.flush() {
                        warn!("Error while flushing connection: {err}");
                        post_action = calloop::PostAction::Remove
                    }

                    if matches!(
                        post_action,
                        calloop::PostAction::Remove | calloop::PostAction::Disable
                    ) {
                        state
                            .niri
                            .remote_desktop
                            .active_ei_sessions
                            .remove(&session_id);
                    }

                    // Always Ok because we never want to propagate the error
                    // out of the entire event loop
                    Ok(post_action)
                },
            )
            .unwrap();

        let conn_state = ConnectionState {
            last_capabilities: None,
            ei_connection: None,
            seat: None,
            key_counter: 0,
            exposed_device_types,
            session_id,
            scroll_frame: None,
            next_frame_touch: false,
            keyboard_device: None,
            mouse_device: None,
            touch_device: None,
            event_loop_token,
        };

        self.niri
            .remote_desktop
            .active_ei_sessions
            .insert(session_id, conn_state);
    }
}

fn handle_eis_request_source_event(
    event: Result<EisRequestSourceEvent, reis::Error>,
    connection: &mut reis::request::Connection,
    global_state: &mut State,
    session_id: RemoteDesktopSessionId,
) -> calloop::PostAction {
    match event {
        Ok(event) => match event {
            EisRequestSourceEvent::Connected => {
                debug!("EIS connected!");
                if !connection.has_interface("ei_seat") || !connection.has_interface("ei_device") {
                    connection.disconnected(
                        eis::connection::DisconnectReason::Protocol,
                        Some("Need `ei_seat` and `ei_device`"),
                    );
                    return calloop::PostAction::Remove;
                }

                let conn_state: &mut ConnectionState = global_state
                    .niri
                    .remote_desktop
                    .active_ei_sessions
                    .get_mut(&session_id)
                    .expect("remote desktop session being processed should exist");

                let seat = connection.add_seat(
                    Some("default"),
                    MutterXdpDeviceType::to_reis_capabilities(conn_state.exposed_device_types),
                );

                conn_state.seat = Some(seat);

                calloop::PostAction::Continue
            }
            EisRequestSourceEvent::Request(request) => {
                debug!("EIS request! {:#?}", request);
                handle_eis_request(request, connection, global_state, session_id)
            }
        },
        Err(err) => {
            warn!("EIS protocol error: {err}");
            connection.disconnected(
                eis::connection::DisconnectReason::Protocol,
                Some(&err.to_string()),
            );
            calloop::PostAction::Remove
        }
    }
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
    connection: &reis::request::Connection,
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

                    let ipc_outputs = global_state.backend.ipc_outputs();
                    let ipc_outputs = ipc_outputs.lock().unwrap();
                    advertise_regions(device, &ipc_outputs);
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
    connection: &reis::request::Connection,
    ipc_outputs: &IpcOutputMap,
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
            |device| {
                advertise_regions(device, ipc_outputs);
            },
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
    connection: &reis::request::Connection,
    ipc_outputs: &IpcOutputMap,
) -> Option<EiDevice> {
    (capabilities.contains(DeviceCapability::Touch) && connection.has_interface("ei_touchscreen"))
        .then(|| {
            seat.add_device(
                Some("touchscreen"),
                DeviceType::Virtual,
                DeviceCapability::Touch.into(),
                |device| {
                    advertise_regions(device, ipc_outputs);
                },
            )
        })
}

/// (Re)creates EI devices.
fn create_ei_devices(
    connection: &reis::request::Connection,
    global_state: &mut State,
    session_id: RemoteDesktopSessionId,
    seat: EiSeat,
    capabilities: BitFlags<DeviceCapability, u64>,
) {
    macro_rules! get_conn_state {
        // global_state is explicitly specified as a reminder for the lifetime stuff.
        ($global_state: ident) => {
            $global_state
                .niri
                .remote_desktop
                .active_ei_sessions
                .get_mut(&session_id)
                .expect("remote desktop session being processed should exist")
        };
    }

    {
        let conn_state = get_conn_state!(global_state);

        // Remove "old" devices
        for old_device_slot in [
            &mut conn_state.keyboard_device,
            &mut conn_state.mouse_device,
            &mut conn_state.touch_device,
        ]
        .into_iter()
        {
            if let Some(old_device) = old_device_slot {
                old_device.remove();
                *old_device_slot = None;
            }
        }
    }

    // This is funnily separated like this because of the mutable aliasing of conn_state and
    // global_state
    let keyboard_device = create_ei_keyboard(&seat, capabilities, connection, global_state);

    {
        let conn_state = get_conn_state!(global_state);

        let ipc_outputs = global_state.backend.ipc_outputs();
        let ipc_outputs = ipc_outputs.lock().unwrap();

        if let Some(device) = keyboard_device {
            device.resumed();
            conn_state.keyboard_device = Some(device);
        }

        if let Some(device) = create_ei_mouse(&seat, capabilities, connection, &ipc_outputs) {
            device.resumed();
            conn_state.mouse_device = Some(device);
        }

        if let Some(device) = create_ei_touchscreen(&seat, capabilities, connection, &ipc_outputs) {
            device.resumed();
            conn_state.touch_device = Some(device);
        }
    }

    // Update the Wayland devices based on the stored data
    global_state.refresh_wayland_device_caps();
}

/// Advertises regions on EI devices.
fn advertise_regions(device: &EiDevice, ipc_outputs: &IpcOutputMap) {
    let Some(bounding_rect) = global_bounding_rectangle_ipc(ipc_outputs) else {
        return;
    };

    for output in ipc_outputs.values() {
        let Some(l) = output.logical else { continue };

        device.device().region_mapping_id(&output.name);

        device.device().region(
            // EI doesn't allow negative positions
            (l.x - bounding_rect.loc.x) as u32,
            (l.y - bounding_rect.loc.y) as u32,
            l.width,
            l.height,
            l.scale as f32,
        );
    }
}

fn handle_eis_request(
    request: reis::request::EisRequest,
    connection: &mut reis::request::Connection,
    global_state: &mut State,
    session_id: RemoteDesktopSessionId,
) -> calloop::PostAction {
    macro_rules! get_conn_state {
        // global_state is explicitly specified as a reminder for the lifetime stuff.
        ($global_state: ident) => {
            $global_state
                .niri
                .remote_desktop
                .active_ei_sessions
                .get_mut(&session_id)
                .expect("remote desktop session being processed should exist")
        };
    }

    match request {
        EisRequest::Disconnect => {
            return calloop::PostAction::Remove;
        }
        EisRequest::Bind(reis::request::Bind { seat, capabilities }) => {
            let conn_state = get_conn_state!(global_state);
            if capabilities
                & MutterXdpDeviceType::to_reis_capabilities(conn_state.exposed_device_types)
                != capabilities
            {
                connection.disconnected(
                    eis::connection::DisconnectReason::Value,
                    Some("Binding to invalid capabilities"),
                );
                return calloop::PostAction::Remove;
            }

            conn_state.ei_connection = Some(connection.clone());
            conn_state.last_capabilities = Some(capabilities);

            // TODO: Why not combine everything into a single device?

            create_ei_devices(connection, global_state, session_id, seat, capabilities);
        }

        EisRequest::DeviceStartEmulating(inner) => {}
        EisRequest::DeviceStopEmulating(inner) => {}

        EisRequest::PointerMotion(inner) => {
            process_event!(global_state, session_id, inner, PointerMotion)
        }

        EisRequest::PointerMotionAbsolute(inner) => {
            if let Some(regions_extent) =
                get_conn_state!(global_state).regions_extent(&global_state.backend)
            {
                process_event!(
                    global_state,
                    session_id,
                    inner,
                    PointerMotionAbsolute,
                    AbsolutePositionEventExtra { regions_extent }
                )
            };
        }

        EisRequest::Button(inner) => {
            process_event!(global_state, session_id, inner, PointerButton)
        }

        EisRequest::ScrollDelta(inner) => {
            let scroll_frame = get_conn_state!(global_state)
                .scroll_frame
                .get_or_insert_default();
            scroll_frame.delta = Some((inner.dx, inner.dy));
        }

        EisRequest::ScrollStop(inner) => {
            let scroll_frame = get_conn_state!(global_state)
                .scroll_frame
                .get_or_insert_default();
            scroll_frame.stop = Some(((inner.x, inner.y), false));
        }

        EisRequest::ScrollCancel(inner) => {
            let scroll_frame = get_conn_state!(global_state)
                .scroll_frame
                .get_or_insert_default();
            scroll_frame.stop = Some(((inner.x, inner.y), true));
        }

        EisRequest::ScrollDiscrete(inner) => {
            let scroll_frame = get_conn_state!(global_state)
                .scroll_frame
                .get_or_insert_default();
            scroll_frame.discrete = Some((inner.discrete_dx, inner.discrete_dy));
        }

        EisRequest::Frame(inner) => {
            let conn_state = get_conn_state!(global_state);
            let next_frame_touch = conn_state.next_frame_touch;
            conn_state.next_frame_touch = false;

            if let Some(scroll_frame) = conn_state.scroll_frame.take() {
                process_event!(
                    global_state,
                    session_id,
                    inner.clone(),
                    PointerAxis,
                    scroll_frame
                )
            }

            if next_frame_touch {
                process_event!(
                    global_state,
                    session_id,
                    inner.clone(),
                    TouchFrame,
                    TouchFrame
                );
            }
        }

        EisRequest::KeyboardKey(inner) => {
            let conn_state = get_conn_state!(global_state);

            // This is super naive but Smithay's winit and x11 input adapters do this.
            match inner.state {
                reis::ei::keyboard::KeyState::Released => {
                    conn_state.key_counter = conn_state.key_counter.saturating_sub(1)
                }
                reis::ei::keyboard::KeyState::Press => conn_state.key_counter += 1,
            }
            let pressed_count = PressedCount(conn_state.key_counter);

            process_event!(global_state, session_id, inner, Keyboard, pressed_count);
        }

        EisRequest::TouchDown(inner) => {
            let conn_state = get_conn_state!(global_state);
            if let Some(regions_extent) = conn_state.regions_extent(&global_state.backend) {
                conn_state.next_frame_touch = true;
                process_event!(
                    global_state,
                    session_id,
                    inner,
                    TouchDown,
                    AbsolutePositionEventExtra { regions_extent }
                );
            }
        }
        EisRequest::TouchMotion(inner) => {
            let conn_state = get_conn_state!(global_state);
            if let Some(regions_extent) = conn_state.regions_extent(&global_state.backend) {
                conn_state.next_frame_touch = true;

                process_event!(
                    global_state,
                    session_id,
                    inner,
                    TouchMotion,
                    AbsolutePositionEventExtra { regions_extent }
                );
            }
        }
        EisRequest::TouchUp(inner) => {
            process_event!(global_state, session_id, inner, TouchUp);
            get_conn_state!(global_state).next_frame_touch = true;
        }
        EisRequest::TouchCancel(inner) => {
            process_event!(global_state, session_id, inner, TouchCancel);
            get_conn_state!(global_state).next_frame_touch = true;
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
