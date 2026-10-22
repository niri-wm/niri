use std::sync::{Arc, Mutex};

use niri_config::Modifiers;
use smithay::input::keyboard::{keysyms, Keycode, Keysym, Xkb};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};
use xx_hotkey_manager_v1::XxHotkeyManagerV1;
use xx_hotkey_v1::XxHotkeyV1;

use super::raw::xx_hotkey::v1::server::{xx_hotkey_manager_v1, xx_hotkey_v1};

const VERSION: u32 = 1;

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;
pub const BTN_SIDE: u32 = 0x113;
pub const BTN_EXTRA: u32 = 0x114;
const BTN_TASK: u32 = 0x117;

pub struct XxHotkeyManagerState {
    hotkeys: Vec<Hotkey>,
}

pub struct XxHotkeyManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait XxHotkeyHandler {
    fn xx_hotkey_manager_state(&mut self) -> &mut XxHotkeyManagerState;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerInput {
    Key(Keysym),
    Button(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trigger {
    pub input: TriggerInput,
    pub modifiers: Modifiers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldInput {
    Key(Keycode),
    Button(u32),
}

pub struct HotkeyInfo {
    pub trigger: Trigger,
    pub app_id: String,
    pub description: String,
}

struct Hotkey {
    resource: XxHotkeyV1,
    trigger: Trigger,
    app_id: String,
    description: String,
    held: Option<HeldInput>,
}

#[derive(Default)]
struct ManagerInfo {
    app_id: Option<String>,
    committed: bool,
}

pub struct XxHotkeyManagerData {
    info: Arc<Mutex<ManagerInfo>>,
}

pub struct XxHotkeyData {
    info: Arc<Mutex<ManagerInfo>>,
    pending: Mutex<Pending>,
}

#[derive(Default)]
struct Pending {
    trigger: Option<Trigger>,
    description: String,
}

impl XxHotkeyManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<XxHotkeyManagerV1, XxHotkeyManagerGlobalData>,
        D: XxHotkeyHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = XxHotkeyManagerGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, XxHotkeyManagerV1, _>(VERSION, global_data);

        Self {
            hotkeys: Vec::new(),
        }
    }

    pub fn on_key_press(
        &mut self,
        xkb: &Xkb,
        keycode: Keycode,
        modifiers: Modifiers,
        serial: u32,
        time: u32,
    ) -> bool {
        let produces = |keysym: Keysym| {
            xkb.layouts().any(|layout| {
                xkb.raw_syms_for_key_in_layout(keycode, layout)
                    .iter()
                    .any(|sym| normalize_keypad(*sym) == normalize_keypad(keysym))
            })
        };

        self.press(
            HeldInput::Key(keycode),
            modifiers,
            serial,
            time,
            |input| matches!(input, TriggerInput::Key(keysym) if produces(keysym)),
        )
    }

    pub fn on_button_press(
        &mut self,
        button: u32,
        modifiers: Modifiers,
        serial: u32,
        time: u32,
    ) -> bool {
        self.press(
            HeldInput::Button(button),
            modifiers,
            serial,
            time,
            |input| input == TriggerInput::Button(button),
        )
    }

    pub fn on_release(&mut self, input: HeldInput, serial: u32, time: u32) -> bool {
        let mut fired = false;
        for hotkey in &mut self.hotkeys {
            if hotkey.held == Some(input) {
                hotkey.held = None;
                hotkey.resource.released(serial, time);
                fired = true;
            }
        }
        fired
    }

    pub fn hotkeys(&self) -> Vec<HotkeyInfo> {
        self.hotkeys
            .iter()
            .map(|hotkey| HotkeyInfo {
                trigger: hotkey.trigger,
                app_id: hotkey.app_id.clone(),
                description: hotkey.description.clone(),
            })
            .collect()
    }

    fn press(
        &mut self,
        input: HeldInput,
        modifiers: Modifiers,
        serial: u32,
        time: u32,
        matches: impl Fn(TriggerInput) -> bool,
    ) -> bool {
        let modifiers =
            modifiers & (Modifiers::CTRL | Modifiers::SHIFT | Modifiers::ALT | Modifiers::SUPER);

        let mut fired = false;
        for hotkey in &mut self.hotkeys {
            // A press of the input we think is held means we missed its release.
            if hotkey.held.is_none_or(|held| held == input)
                && hotkey.trigger.modifiers == modifiers
                && matches(hotkey.trigger.input)
            {
                hotkey.held = Some(input);
                hotkey.resource.triggered(serial, time);
                fired = true;
            }
        }
        fired
    }
}

impl<D> GlobalDispatch2<XxHotkeyManagerV1, D> for XxHotkeyManagerGlobalData
where
    D: Dispatch<XxHotkeyManagerV1, XxHotkeyManagerData>,
    D: XxHotkeyHandler,
    D: 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<XxHotkeyManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        let data = XxHotkeyManagerData {
            info: Arc::new(Mutex::new(ManagerInfo::default())),
        };
        data_init.init(manager, data);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl<D> Dispatch2<XxHotkeyManagerV1, D> for XxHotkeyManagerData
where
    D: Dispatch<XxHotkeyV1, XxHotkeyData>,
    D: XxHotkeyHandler,
    D: 'static,
{
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        resource: &XxHotkeyManagerV1,
        request: <XxHotkeyManagerV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            xx_hotkey_manager_v1::Request::SetAppId { app_id } => {
                let mut info = self.info.lock().unwrap();
                if app_id.is_empty() || info.app_id.is_some() || info.committed {
                    resource.post_error(
                        xx_hotkey_manager_v1::Error::InvalidAppId,
                        "app_id must be non-empty and set once before the first commit",
                    );
                    return;
                }
                info.app_id = Some(app_id);
            }
            xx_hotkey_manager_v1::Request::CreateHotkey { id } => {
                let data = XxHotkeyData {
                    info: self.info.clone(),
                    pending: Mutex::new(Pending::default()),
                };
                data_init.init(id, data);
            }
            xx_hotkey_manager_v1::Request::Destroy => (),
        }
    }
}

impl<D> Dispatch2<XxHotkeyV1, D> for XxHotkeyData
where
    D: XxHotkeyHandler,
    D: 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        resource: &XxHotkeyV1,
        request: <XxHotkeyV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let mut pending = self.pending.lock().unwrap();

        match request {
            xx_hotkey_v1::Request::SetDescription { description } => {
                pending.description = description;
            }
            // We only have one seat.
            xx_hotkey_v1::Request::SetSeat { .. } => (),
            xx_hotkey_v1::Request::SetKeyTrigger { keysym, modifiers } => {
                let modifiers = parse_modifiers(modifiers.into());
                let Some(modifiers) = modifiers.filter(|_| keysym != keysyms::KEY_NoSymbol) else {
                    resource.post_error(xx_hotkey_v1::Error::InvalidTrigger, "invalid trigger");
                    return;
                };

                pending.trigger = Some(Trigger {
                    input: TriggerInput::Key(Keysym::new(keysym)),
                    modifiers,
                });
            }
            xx_hotkey_v1::Request::SetButtonTrigger { button, modifiers } => {
                let Some(modifiers) = parse_modifiers(modifiers.into()) else {
                    resource.post_error(xx_hotkey_v1::Error::InvalidTrigger, "invalid trigger");
                    return;
                };

                pending.trigger = Some(Trigger {
                    input: TriggerInput::Button(button),
                    modifiers,
                });
            }
            xx_hotkey_v1::Request::Commit => {
                let Some(trigger) = pending.trigger else {
                    resource.post_error(xx_hotkey_v1::Error::NoTrigger, "no trigger was set");
                    return;
                };

                let app_id = {
                    let mut info = self.info.lock().unwrap();
                    info.committed = true;
                    info.app_id.clone().unwrap_or_default()
                };

                if let Err(message) = check_trigger(trigger) {
                    resource.denied(xx_hotkey_v1::DenyReason::NotPermitted, message.to_owned());
                    return;
                }

                let description = pending.description.clone();
                let hotkeys = &mut state.xx_hotkey_manager_state().hotkeys;
                if let Some(hotkey) = hotkeys.iter_mut().find(|h| h.resource == *resource) {
                    hotkey.trigger = trigger;
                    hotkey.description = description;
                } else {
                    hotkeys.push(Hotkey {
                        resource: resource.clone(),
                        trigger,
                        app_id,
                        description,
                        held: None,
                    });
                }

                resource.bound();
            }
            xx_hotkey_v1::Request::Destroy => (),
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, resource: &XxHotkeyV1) {
        let hotkeys = &mut state.xx_hotkey_manager_state().hotkeys;
        hotkeys.retain(|hotkey| hotkey.resource != *resource);
    }
}

// The policy suggested by the protocol, minus lone modifier taps which we don't support.
fn check_trigger(trigger: Trigger) -> Result<(), &'static str> {
    if let TriggerInput::Key(keysym) = trigger.input {
        if keysym.is_modifier_key() {
            return Err("Modifier-only hotkeys are not supported");
        }
    }

    if trigger
        .modifiers
        .intersects(Modifiers::CTRL | Modifiers::ALT | Modifiers::SUPER)
    {
        return Ok(());
    }

    match trigger.input {
        TriggerInput::Button(button) if (BTN_SIDE..=BTN_TASK).contains(&button) => Ok(()),
        TriggerInput::Button(_) => {
            Err("Ctrl, Alt or Super is required for non-auxiliary mouse buttons")
        }
        TriggerInput::Key(keysym)
            if (keysyms::KEY_KP_Space..=keysyms::KEY_KP_Equal).contains(&keysym.raw()) =>
        {
            Ok(())
        }
        TriggerInput::Key(keysym)
            if (keysyms::KEY_dead_grave..=keysyms::KEY_dead_longsolidusoverlay)
                .contains(&keysym.raw())
                || keysym.key_char().is_some() =>
        {
            Err("Ctrl, Alt or Super is required for keys that produce text")
        }
        TriggerInput::Key(_) => Ok(()),
    }
}

// Num Lock selects one of two keysyms per numeric keypad key; fold each pair.
fn normalize_keypad(keysym: Keysym) -> u32 {
    match keysym.raw() {
        keysyms::KEY_KP_Insert => keysyms::KEY_KP_0,
        keysyms::KEY_KP_End => keysyms::KEY_KP_1,
        keysyms::KEY_KP_Down => keysyms::KEY_KP_2,
        keysyms::KEY_KP_Next => keysyms::KEY_KP_3,
        keysyms::KEY_KP_Left => keysyms::KEY_KP_4,
        keysyms::KEY_KP_Begin => keysyms::KEY_KP_5,
        keysyms::KEY_KP_Right => keysyms::KEY_KP_6,
        keysyms::KEY_KP_Home => keysyms::KEY_KP_7,
        keysyms::KEY_KP_Up => keysyms::KEY_KP_8,
        keysyms::KEY_KP_Prior => keysyms::KEY_KP_9,
        keysyms::KEY_KP_Delete => keysyms::KEY_KP_Decimal,
        raw => raw,
    }
}

fn parse_modifiers(bits: u32) -> Option<Modifiers> {
    use xx_hotkey_manager_v1::Modifiers as M;

    let mods = M::from_bits(bits)?;
    let mut out = Modifiers::empty();
    if mods.contains(M::Shift) {
        out |= Modifiers::SHIFT;
    }
    if mods.contains(M::Ctrl) {
        out |= Modifiers::CTRL;
    }
    if mods.contains(M::Alt) {
        out |= Modifiers::ALT;
    }
    if mods.contains(M::Super) {
        out |= Modifiers::SUPER;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(raw: u32, modifiers: Modifiers) -> Trigger {
        Trigger {
            input: TriggerInput::Key(Keysym::new(raw)),
            modifiers,
        }
    }

    fn button(button: u32, modifiers: Modifiers) -> Trigger {
        Trigger {
            input: TriggerInput::Button(button),
            modifiers,
        }
    }

    #[test]
    fn trigger_policy() {
        assert!(check_trigger(key(keysyms::KEY_k, Modifiers::SUPER)).is_ok());
        assert!(check_trigger(key(keysyms::KEY_space, Modifiers::CTRL)).is_ok());
        assert!(check_trigger(key(keysyms::KEY_F5, Modifiers::empty())).is_ok());
        assert!(check_trigger(key(keysyms::KEY_XF86AudioPlay, Modifiers::empty())).is_ok());
        assert!(check_trigger(key(keysyms::KEY_KP_1, Modifiers::empty())).is_ok());
        assert!(check_trigger(key(keysyms::KEY_KP_End, Modifiers::empty())).is_ok());

        assert!(check_trigger(key(keysyms::KEY_k, Modifiers::empty())).is_err());
        assert!(check_trigger(key(keysyms::KEY_k, Modifiers::SHIFT)).is_err());
        assert!(check_trigger(key(keysyms::KEY_Return, Modifiers::empty())).is_err());
        assert!(check_trigger(key(keysyms::KEY_dead_grave, Modifiers::empty())).is_err());
        assert!(check_trigger(key(keysyms::KEY_Super_L, Modifiers::empty())).is_err());
        assert!(check_trigger(key(keysyms::KEY_Shift_L, Modifiers::CTRL)).is_err());

        assert!(check_trigger(button(BTN_SIDE, Modifiers::empty())).is_ok());
        assert!(check_trigger(button(BTN_LEFT, Modifiers::SUPER)).is_ok());
        assert!(check_trigger(button(BTN_LEFT, Modifiers::empty())).is_err());
        assert!(check_trigger(button(BTN_RIGHT, Modifiers::SHIFT)).is_err());
    }
}
