//! The `vicinae_hotkey_v1` protocol: client-managed global hotkeys. A client binds a key
//! combination and the compositor arbitrates (`bound`/`denied`/`revoked`);
//! For every bind that is accepted, the compositor fires `pressed`/ `released` regardless of
//! keyboard focus.

use niri_config::Modifiers;
use smithay::input::keyboard::Keysym;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};
pub use vicinae_hotkey_v1::{DenyReason, RevokeReason};

use super::raw::vicinae_hotkey::v1::server::vicinae_hotkey_manager_v1::{
    self, VicinaeHotkeyManagerV1,
};
use super::raw::vicinae_hotkey::v1::server::vicinae_hotkey_v1::{self, VicinaeHotkeyV1};

const VERSION: u32 = 1;

pub struct VicinaeHotkeyManagerState {
    hotkeys: Vec<BoundHotkey>,
    // Held hotkeys keyed by triggering keycode, so `released` fires on key release regardless of
    // the current modifier state.
    held: Vec<HeldHotkey>,
}

struct BoundHotkey {
    resource: VicinaeHotkeyV1,
    keysym: u32,
    // Semantic modifier bits only (CTRL, SHIFT, ALT, SUPER).
    modifiers: Modifiers,
    // Advisory (and spoofable) identity from the bind request, used only to build a human-readable
    // already_bound message so a clashing client can show what owns the combination.
    app_id: String,
    description: String,
}

struct HeldHotkey {
    keycode: u32,
    resource: VicinaeHotkeyV1,
}

pub struct VicinaeHotkeyManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait VicinaeHotkeyHandler {
    fn vicinae_hotkey_manager_state(&mut self) -> &mut VicinaeHotkeyManagerState;

    // Compositor policy: accept or deny a bind request, where
    // `message` is advisory text for the client's UI. `modifiers` holds only the semantic bits.
    // We try to forward the most helpful `message` we can so that clients can gracefully
    // communicate errors to the user.
    fn vicinae_hotkey_decide(
        &mut self,
        keysym: Keysym,
        modifiers: Modifiers,
    ) -> Result<(), (DenyReason, String)>;
}

impl VicinaeHotkeyManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<VicinaeHotkeyManagerV1, VicinaeHotkeyManagerGlobalData>,
        D: Dispatch<VicinaeHotkeyManagerV1, ()>,
        D: Dispatch<VicinaeHotkeyV1, ()>,
        D: VicinaeHotkeyHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = VicinaeHotkeyManagerGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, VicinaeHotkeyManagerV1, _>(VERSION, global_data);

        Self {
            hotkeys: Vec::new(),
            held: Vec::new(),
        }
    }

    // Fires `pressed` on every matching hotkey; returns true if any fired (so the caller
    // intercepts the key). `modifiers` must hold only the semantic bits.
    pub fn on_key_press(
        &mut self,
        keycode: u32,
        keysym: u32,
        modifiers: Modifiers,
        serial: u32,
        time: u32,
    ) -> bool {
        let mut fired = false;
        for hotkey in &self.hotkeys {
            if hotkey.keysym == keysym
                && hotkey.modifiers == modifiers
                && hotkey.resource.is_alive()
            {
                hotkey.resource.pressed(serial, time);
                self.held.push(HeldHotkey {
                    keycode,
                    resource: hotkey.resource.clone(),
                });
                fired = true;
            }
        }
        fired
    }

    pub fn on_key_release(&mut self, keycode: u32, serial: u32, time: u32) -> bool {
        let mut fired = false;
        self.held.retain(|held| {
            if held.keycode != keycode {
                return true;
            }
            if held.resource.is_alive() {
                held.resource.released(serial, time);
            }
            fired = true;
            false
        });
        fired
    }

    // revoke hotkeys that may be invalidated by a change in compositor policy or by a config
    // reload.
    pub fn revoke_if(
        &mut self,
        reason: RevokeReason,
        mut revoke_message: impl FnMut(Keysym, Modifiers) -> Option<String>,
    ) {
        self.hotkeys.retain(|hotkey| {
            let Some(message) = revoke_message(Keysym::new(hotkey.keysym), hotkey.modifiers) else {
                return true;
            };
            if hotkey.resource.is_alive() {
                hotkey.resource.revoked(reason, message);
            }
            false
        });
    }

    fn forget(&mut self, resource: &VicinaeHotkeyV1) {
        self.hotkeys.retain(|h| h.resource != *resource);
        self.held.retain(|h| h.resource != *resource);
    }
}

impl<D> GlobalDispatch<VicinaeHotkeyManagerV1, VicinaeHotkeyManagerGlobalData, D>
    for VicinaeHotkeyManagerState
where
    D: GlobalDispatch<VicinaeHotkeyManagerV1, VicinaeHotkeyManagerGlobalData>,
    D: Dispatch<VicinaeHotkeyManagerV1, ()>,
    D: Dispatch<VicinaeHotkeyV1, ()>,
    D: VicinaeHotkeyHandler,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<VicinaeHotkeyManagerV1>,
        _manager_state: &VicinaeHotkeyManagerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ());
    }

    fn can_view(client: Client, global_data: &VicinaeHotkeyManagerGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<VicinaeHotkeyManagerV1, (), D> for VicinaeHotkeyManagerState
where
    D: Dispatch<VicinaeHotkeyManagerV1, ()>,
    D: Dispatch<VicinaeHotkeyV1, ()>,
    D: VicinaeHotkeyHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &VicinaeHotkeyManagerV1,
        request: <VicinaeHotkeyManagerV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            vicinae_hotkey_manager_v1::Request::Bind {
                id,
                keysym,
                modifiers,
                seat: _,
                app_id,
                description,
            } => {
                let hotkey = data_init.init(id, ());

                let keysym = Keysym::new(keysym);
                let modifiers = parse_modifiers(modifiers);

                // Compositor policy (combination validity, conflicts with configured binds).
                if let Err((reason, message)) = state.vicinae_hotkey_decide(keysym, modifiers) {
                    hotkey.denied(reason, message);
                    return;
                }

                // Exclusivity: the combination is owned by at most one hotkey. If another already
                // holds it, deny and name the owner so the client can show what owns it.
                let mgr = state.vicinae_hotkey_manager_state();
                let owner = mgr
                    .hotkeys
                    .iter()
                    .find(|h| {
                        h.keysym == keysym.raw()
                            && h.modifiers == modifiers
                            && h.resource.is_alive()
                    })
                    .map(|h| already_bound_message(&h.app_id, &h.description));
                if let Some(message) = owner {
                    hotkey.denied(DenyReason::AlreadyBound, message);
                    return;
                }

                hotkey.bound();
                mgr.hotkeys.push(BoundHotkey {
                    resource: hotkey,
                    keysym: keysym.raw(),
                    modifiers,
                    app_id,
                    description,
                });
            }
            vicinae_hotkey_manager_v1::Request::Destroy => (),
        }
    }
}

impl<D> Dispatch<VicinaeHotkeyV1, (), D> for VicinaeHotkeyManagerState
where
    D: Dispatch<VicinaeHotkeyV1, ()>,
    D: VicinaeHotkeyHandler,
    D: 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &VicinaeHotkeyV1,
        request: <VicinaeHotkeyV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            vicinae_hotkey_v1::Request::Destroy => (),
        }
    }

    fn destroyed(state: &mut D, _client: ClientId, resource: &VicinaeHotkeyV1, _data: &()) {
        state.vicinae_hotkey_manager_state().forget(resource);
    }
}

fn already_bound_message(app_id: &str, description: &str) -> String {
    match (app_id.is_empty(), description.is_empty()) {
        (false, false) => format!("Already bound by {app_id} ({description})"),
        (false, true) => format!("Already bound by {app_id}"),
        (true, false) => format!("Already bound ({description})"),
        (true, true) => String::from("Already bound by another application"),
    }
}

// protocol mods to niri mods
fn parse_modifiers(modifiers: WEnum<vicinae_hotkey_manager_v1::Modifiers>) -> Modifiers {
    let bits = match modifiers {
        WEnum::Value(m) => m.bits(),
        WEnum::Unknown(bits) => bits,
    };

    let mut out = Modifiers::empty();
    if bits & 1 != 0 {
        out |= Modifiers::SHIFT;
    }
    if bits & 2 != 0 {
        out |= Modifiers::CTRL;
    }
    if bits & 4 != 0 {
        out |= Modifiers::ALT;
    }
    if bits & 8 != 0 {
        out |= Modifiers::SUPER;
    }
    out
}

#[macro_export]
macro_rules! delegate_vicinae_hotkey {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::vicinae_hotkey::v1::server::vicinae_hotkey_manager_v1::VicinaeHotkeyManagerV1: $crate::protocols::vicinae_hotkey::VicinaeHotkeyManagerGlobalData
        ] => $crate::protocols::vicinae_hotkey::VicinaeHotkeyManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::vicinae_hotkey::v1::server::vicinae_hotkey_manager_v1::VicinaeHotkeyManagerV1: ()
        ] => $crate::protocols::vicinae_hotkey::VicinaeHotkeyManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::vicinae_hotkey::v1::server::vicinae_hotkey_v1::VicinaeHotkeyV1: ()
        ] => $crate::protocols::vicinae_hotkey::VicinaeHotkeyManagerState);
    };
}
