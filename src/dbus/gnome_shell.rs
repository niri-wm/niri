use std::borrow::Cow;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Context as _;
use futures_util::StreamExt as _;
use niri_config::{Action, Bind, BoundAction, Key};
use serde::Deserialize;
use smithay::backend::input::InputTime;
use smithay::wayland::xdg_activation::XdgActivationState;
use zbus::blocking::object_server::InterfaceRef;
use zbus::fdo::{self};
use zbus::interface;
use zbus::message::Header;
use zbus::names::{BusName, OwnedUniqueName, UniqueName};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{NoneValue as _, SerializeDict, Type, Value};

use super::Start;
use crate::dbus::request_name;

// It would be nice to make this a newtype to prevent accidental misuse, but we need to return a
// Vec<u32> from org.gnome.Shell, so newtyping would require an extra allocation or a transmute.
pub type ActionId = u32;

// The specific value seems to not be important at the time of writing, but GNOME Shell uses 0:
// - https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/7904014f5b1a104c003aa481d1e4d0b9c7fe7f79/js/ui/shellDBus.js#L321
// - https://gitlab.gnome.org/GNOME/mutter/-/blob/b48811767189fb5446bd1b831e98271a21866e09/src/meta/prefs.h#L325
const INVALID_ACTION: ActionId = 0;

#[derive(Clone)]
pub struct GlobalShortcutsPortalBinds {
    next_action: ActionId,
    binds: Vec<Bind>,
}

impl GlobalShortcutsPortalBinds {
    /// Creates a new instance of these shortcuts.
    pub fn new() -> Self {
        Self {
            // Start from 1 because 0 is INVALID_ACTION.
            next_action: 1,
            binds: vec![],
        }
    }

    /// Grabs the given accelerators, returning the [`Action`]s corresponding to each accelerator.
    pub fn grab_accelerators(&mut self, accelerators: &[AcceleratorGrab]) -> Vec<ActionId> {
        accelerators
            .iter()
            .map(|accelerator| self.bind_accelerator(accelerator))
            .collect()
    }

    /// Removes all given `actions` from previously grabbed accelerators.
    ///
    /// Returns true if any action was unbound, false otherwise.
    ///
    /// This is different from GNOME Shell which returns true only when all actions had been unbound
    /// successfully, but it's not really used for anything, and this way is more convenient.
    pub fn ungrab_accelerators(&mut self, actions: &[ActionId]) -> bool {
        let mut unbound = false;
        self.binds.retain(|bind| {
            let BoundAction::Both {
                press: Action::GlobalShortcutActivateAction(bound_action),
                ..
            } = bind.action
            else {
                // We only ever create this pattern of Bind in `binds`.
                unreachable!()
            };
            if actions.contains(&bound_action) {
                unbound = true;
                false
            } else {
                true
            }
        });
        unbound
    }

    /// Returns all bound global shortcuts.
    pub fn binds(&self) -> &[Bind] {
        &self.binds
    }

    /// Binds the given accelerator.
    ///
    /// This function may return [`INVALID_ACTION`] and **not** actually bind the accelerator.
    fn bind_accelerator(&mut self, accelerator: &AcceleratorGrab) -> ActionId {
        let converted_accelerator = match convert_gnome_modifiers(&accelerator.accelerator) {
            Ok(converted) => converted,
            Err(bad_modifier) => {
                warn!(
                    "failed to convert accelerator \"{}\" to key string due to invalid modifier \
                    \"{bad_modifier}\"",
                    accelerator.accelerator
                );
                return INVALID_ACTION;
            }
        };
        let Ok(key) = Key::from_str(converted_accelerator.as_ref()) else {
            warn!(
                "failed to parse key from acclerator \"{}\" (converted to canonical form \
                \"{converted_accelerator}\")",
                accelerator.accelerator
            );
            return INVALID_ACTION;
        };

        // We could check for conflicting keybinds here and return INVALID_ACTION, however that
        // doesn't seem useful. xdp-gnome currently doesn't do anything with this information so
        // it's not like the app can find out. In any case, if the conflicting bind is removed, this
        // one becomes activatable, so let's keep it.

        let action = self.get_next_action();
        if action == INVALID_ACTION {
            // There is no valid action here, so just don't bind the action at all and return the
            // invalid value. Unbinding this later won't do anything, and we won't invoke this
            // action.
            return action;
        }
        self.binds.push(Bind {
            key,
            action: BoundAction::Both {
                press: Action::GlobalShortcutActivateAction(action),
                release: Action::GlobalShortcutDeactivateAction(action),
            },
            repeat: false,
            pass_through: None,
            cooldown: None,
            allow_when_locked: false,
            allow_inhibiting: true,
            hotkey_overlay_title: None,
        });
        action
    }

    /// Gets the next available ID to assign to a global shortcut.
    ///
    /// This returns [`None`] if we exhaust all action IDs.
    fn get_next_action(&mut self) -> ActionId {
        if self.next_action == INVALID_ACTION {
            // In practice this will never happen, but it's better not to panic.
            return INVALID_ACTION;
        }
        let next = self.next_action;
        self.next_action = self.next_action.wrapping_add(1);
        next
    }
}

#[derive(Clone)]
pub struct Shell {
    to_niri: calloop::channel::Sender<ShellToNiri>,
    iface: Arc<OnceLock<InterfaceRef<Shell>>>,
    data: Arc<Mutex<Data>>,
}

#[derive(Default)]
pub struct Data {
    /// Clients and their currently bound actions.
    ///
    /// Typically this will be just one xdg-desktop-portal-gnome.
    clients: HashMap<OwnedUniqueName, Vec<ActionId>>,
}

pub enum ShellToNiri {
    GrabAccelerators {
        accelerators: Vec<AcceleratorGrab>,
        results: async_channel::Sender<Vec<ActionId>>,
    },
    UngrabAccelerators {
        actions: Vec<ActionId>,
        result: async_channel::Sender<bool>,
    },
}

#[derive(Debug, Clone, Deserialize, Type, Default)]
pub struct AcceleratorGrab {
    pub accelerator: String,

    // GNOME parameters, unused by us
    /// A bitflag of Gnome's
    /// [ShellActionMode](https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/9a753aa969709d351632dbefd23333681cc5b3c2/src/shell-action-modes.h#L22)
    /// enum.
    ///
    /// This requests where a given bind wants to run (e.g., in the lock screen, in the overview
    /// mode). These don't apply to niri.
    _mode_flags: u32,
    /// A bitflag of Gnome's
    /// [MetaKeyBindingFlags](https://gitlab.gnome.org/GNOME/mutter/-/blob/79b73fa28eabb81d3d8c67340f9e96b4503ab693/src/meta/prefs.h#L442)
    ///
    /// xdg-desktop-portal-gnome only ever sends the TRIGGER_RELEASE value for GlobalShortcuts.
    _grab_flags: u32,
}

#[derive(Debug, Clone, SerializeDict, Type, Value)]
#[zvariant(signature = "dict")]
pub struct ActivationParameters {
    // GNOME portal/shell use u32 -- despite GlobalShortcuts interface using `t` (u64)
    timestamp: u32,
    /// Specifies which "mode" the action was triggered in.
    ///
    /// This is the same enum used by [`AcceleratorGrab::_mode_flags`].
    #[zvariant(rename = "action-mode")]
    action_mode: u32,

    #[zvariant(rename = "activation-token")]
    activation_token: String,
}

#[interface(name = "org.gnome.Shell")]
impl Shell {
    async fn grab_accelerators(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        accelerators: Vec<AcceleratorGrab>,
    ) -> fdo::Result<Vec<ActionId>> {
        let Some(sender) = hdr.sender() else {
            return Err(fdo::Error::Failed("no sender".to_owned()));
        };
        let sender = OwnedUniqueName::from(sender.to_owned());

        // We want to avoid the following race:
        // - We call GrabAccelerators
        // - The client meanwhile disconnects and monitor_disappeared_clients removes it
        // - GrabAccelerators sends us the list of actions
        // - We add it to data.clients, but it will never get cleared out now
        //
        // So, we ensure the clients entry first, then afterwards check if it's gone, and undo the
        // binds then.
        {
            let mut data = self.data.lock().unwrap();
            let _ = data.clients.entry(sender.clone()).or_default();
        }

        let (tx, rx) = async_channel::bounded(1);

        if let Err(err) = self.to_niri.send(ShellToNiri::GrabAccelerators {
            accelerators,
            results: tx,
        }) {
            warn!("error sending grab accelerators message to niri: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        let actions = match rx.recv().await {
            Ok(actions) => actions,
            Err(err) => {
                warn!("error receiving message from niri: {err:?}");
                return Err(fdo::Error::Failed("internal error".to_owned()));
            }
        };

        let gone = {
            let mut data = self.data.lock().unwrap();
            match data.clients.get_mut(&sender) {
                Some(client) => {
                    client.extend(&actions);
                    false
                }
                None => true,
            }
        };
        if gone {
            // The client had disconnected. Remove the binds.
            let (tx, rx) = async_channel::bounded(1);
            let ungrab = ShellToNiri::UngrabAccelerators {
                actions,
                result: tx,
            };
            if self.to_niri.send(ungrab).is_ok() {
                let _ = rx.recv().await;
            }
            // Doesn't really matter now.
            return Err(fdo::Error::Failed("client disconnected".to_owned()));
        }

        Ok(actions)
    }

    async fn ungrab_accelerators(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        mut actions: Vec<ActionId>,
    ) -> fdo::Result<bool> {
        if actions.is_empty() {
            // No actions to unbind.
            return Ok(true);
        }

        let Some(sender) = hdr.sender() else {
            return Err(fdo::Error::Failed("no sender".to_owned()));
        };
        let sender = OwnedUniqueName::from(sender.to_owned());

        {
            let mut data = self.data.lock().unwrap();
            let Some(client) = data.clients.get_mut(&sender) else {
                // Client without bound shortcuts.
                return Ok(false);
            };

            // Don't allow clients to ungrab someone else's actions.
            actions.retain(|requested| client.contains(requested));

            // Clear out actions that we're about to ungrab.
            //
            // We do it here since if the send/recv below fails
            // then something is screwed up beyond repair anyway.
            client.retain(|bound| !actions.contains(bound));
        }

        let (tx, rx) = async_channel::bounded(1);

        if let Err(err) = self.to_niri.send(ShellToNiri::UngrabAccelerators {
            actions,
            result: tx,
        }) {
            warn!("error sending ungrab accelerators message to niri: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        rx.recv().await.map_err(|err| {
            warn!("error receiving message from niri: {err:?}");
            fdo::Error::Failed("internal error".to_owned())
        })
    }

    #[zbus(signal)]
    async fn accelerator_activated(
        ctxt: &SignalEmitter<'_>,
        action: u32,
        parameters: ActivationParameters,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn accelerator_deactivated(
        ctxt: &SignalEmitter<'_>,
        action: u32,
        parameters: ActivationParameters,
    ) -> zbus::Result<()>;
}

impl Shell {
    pub fn new(to_niri: calloop::channel::Sender<ShellToNiri>) -> Self {
        Self {
            to_niri,
            iface: Arc::new(OnceLock::new()),
            data: Arc::new(Mutex::new(Data::default())),
        }
    }

    /// Activates the given `action`, with the given `time` being the time the action was pressed.
    pub fn activate_action(
        &self,
        action: ActionId,
        time: InputTime,
        activation_state: &mut XdgActivationState,
    ) {
        let _span = tracy_client::span!("Shell::activate_action");

        let data = self.data.lock().unwrap();

        let Some(dst) = data
            .clients
            .iter()
            .find_map(|(name, actions)| actions.contains(&action).then_some(name))
        else {
            warn!("tried to activate an action {action} that no client currently owns");
            return;
        };

        // Unwrap is safe since we drop this Shell instance before activating any actions if the
        // iface fails to populate.
        let iface = self.iface.get().unwrap();

        let mut ctxt = iface.signal_emitter().clone();
        ctxt = ctxt.set_destination(BusName::Unique(dst.as_ref()));

        let (activation_token, _) = activation_state.create_external_token(None);

        async_io::block_on(async {
            if let Err(err) = Self::accelerator_activated(
                &ctxt,
                action,
                ActivationParameters {
                    timestamp: time.millis(),
                    action_mode: 1,
                    activation_token: activation_token.to_string(),
                },
            )
            .await
            {
                warn!("error emitting accelerator_activated: {err:?}");
            }
        });
    }

    /// Activates the given `action`, with the given `time` being the time the action was pressed.
    pub fn deactivate_action(
        &self,
        action: ActionId,
        time: InputTime,
        activation_state: &mut XdgActivationState,
    ) {
        let _span = tracy_client::span!("Shell::deactivate_action");

        let data = self.data.lock().unwrap();

        let Some(dst) = data
            .clients
            .iter()
            .find_map(|(name, actions)| actions.contains(&action).then_some(name))
        else {
            warn!("tried to deactivate an action {action} that no client currently owns");
            return;
        };

        // Unwrap is safe since we drop this Shell instance before activating any actions if the
        // iface fails to populate.
        let iface = self.iface.get().unwrap();

        let mut ctxt = iface.signal_emitter().clone();
        ctxt = ctxt.set_destination(BusName::Unique(dst.as_ref()));

        let (activation_token, _) = activation_state.create_external_token(None);

        async_io::block_on(async {
            if let Err(err) = Self::accelerator_deactivated(
                &ctxt,
                action,
                ActivationParameters {
                    timestamp: time.millis(),
                    action_mode: 1,
                    activation_token: activation_token.to_string(),
                },
            )
            .await
            {
                warn!("error emitting accelerator_deactivated: {err:?}");
            }
        });
    }
}

async fn monitor_disappeared_clients(conn: &zbus::Connection, shell: Shell) -> anyhow::Result<()> {
    let proxy = fdo::DBusProxy::new(conn)
        .await
        .context("error creating a DBusProxy")?;

    let mut stream = proxy
        .receive_name_owner_changed_with_args(&[(2, UniqueName::null_value())])
        .await
        .context("error creating a NameOwnerChanged stream")?;

    while let Some(signal) = stream.next().await {
        let args = signal
            .args()
            .context("error retrieving NameOwnerChanged args")?;

        let Some(name) = &**args.old_owner() else {
            continue;
        };

        if args.name() != name {
            // The owner has dropped some other well-known name, but it remains.
            continue;
        }

        if args.new_owner().is_none() {
            trace!("org.gnome.Shell client disconnected: {name}");

            // Ungrab any bound global shortcuts.
            let name = OwnedUniqueName::from(name.to_owned());
            let actions = {
                let mut data = shell.data.lock().unwrap();
                data.clients.remove(&name)
            };
            if let Some(actions) = actions {
                let (tx, rx) = async_channel::bounded(1);
                let ungrab = ShellToNiri::UngrabAccelerators {
                    actions,
                    result: tx,
                };
                if shell.to_niri.send(ungrab).is_ok() {
                    let _ = rx.recv().await;
                }
            }
        } else {
            error!("non-null new_owner should've been filtered out");
        }
    }

    Ok(())
}

impl Start for Shell {
    fn start(self, monitor: bool) -> anyhow::Result<zbus::blocking::Connection> {
        let shell = self.clone();

        let iface_arc = self.iface.clone();

        let conn = zbus::blocking::Connection::session()?;

        conn.object_server().at("/org/gnome/Shell", self)?;
        request_name(&conn, "org.gnome.Shell", monitor)?;

        // Cache the iface so we can use it later when emitting signals.
        let iface = conn.object_server().interface("/org/gnome/Shell")?;
        let _ = iface_arc.set(iface);

        // Slightly racy with request_name() above but oh well.
        let async_conn = conn.inner().clone();
        let future = async move {
            if let Err(err) = monitor_disappeared_clients(&async_conn, shell).await {
                warn!("error monitoring org.gnome.Shell clients: {err:?}");

                // The monitor breaking is kinda bad but not catastrophically so,
                // so let's not disable anything.
            }
        };
        let task = conn
            .inner()
            .executor()
            .spawn(future, "monitor disappearing org.gnome.Shell clients");
        task.detach();

        Ok(conn)
    }
}

// Gnome portal converts modifiers into this format and back again afterwards. Their settings
// provider seems to use a different format than everything else and their portal expects that.
//
// https://gitlab.gnome.org/GNOME/mutter/-/blob/b48811767189fb5446bd1b831e98271a21866e09/src/core/meta-accel-parse.c#L199-274
const MUTTER_KEY_MAP: &[(&str, &str)] = &[
    ("<control>", "Ctrl"),
    ("<ctrl>", "Ctrl"),
    ("<ctl>", "Ctrl"),
    ("<primary>", "Ctrl"),
    ("<shift>", "Shift"),
    ("<shft>", "Shift"),
    ("<alt>", "Alt"),
    ("<super>", "Super"),
    ("<mod1>", "Alt"),
    // mod2 is Num Lock but we don't support binding that as a modifier.
    ("<mod3>", "Mod3"),
    ("<mod4>", "Super"),
    ("<mod5>", "Mod5"),
];

/// Returns either the string with any Gnome modifiers replaced with their "canonical" form, or on
/// failure, the modifier that was unable to be converted.
fn convert_gnome_modifiers(input: &str) -> Result<Cow<'_, str>, &str> {
    let mut converted_string = String::new();
    let mut remaining = input;
    while let Some(start) = remaining.find('<') {
        let Some(end) = remaining[start..].find('>') else {
            break;
        };
        let token = &remaining[start..=start + end];
        let found_value = 'found: {
            for (key, value) in MUTTER_KEY_MAP {
                if token.eq_ignore_ascii_case(key) {
                    break 'found value;
                }
            }
            return Err(token);
        };
        if !converted_string.is_empty() {
            converted_string.push('+');
        }
        converted_string.push_str(found_value);
        remaining = &remaining[(start + end + 1)..];
    }

    if converted_string.is_empty() {
        return Ok(Cow::Borrowed(remaining));
    }

    if !remaining.is_empty() {
        converted_string.push('+');
        converted_string.push_str(remaining);
    }

    Ok(Cow::Owned(converted_string))
}

#[cfg(test)]
mod tests {
    use niri_config::{ModKey, Modifiers, Trigger};
    use smithay::input::keyboard::Keysym;

    use super::*;

    #[test]
    fn converts_without_gnome_modifiers() {
        // Use match so we can check that we borrow the original input for no-modifier values.
        match convert_gnome_modifiers("KP_Multiply").unwrap() {
            Cow::Borrowed(value) => {
                assert_eq!(value, "KP_Multiply");
            }
            Cow::Owned(_) => panic!("expected output to be borrowed, not owned"),
        }
        match convert_gnome_modifiers("ABC").unwrap() {
            Cow::Borrowed(value) => {
                assert_eq!(value, "ABC");
            }
            Cow::Owned(_) => panic!("expected output to be borrowed, not owned"),
        }
    }

    #[test]
    fn converts_with_gnome_modifiers() {
        assert_eq!(
            convert_gnome_modifiers("<Control>KP_Multiply"),
            Ok(Cow::Owned("Ctrl+KP_Multiply".into()))
        );
        assert_eq!(
            convert_gnome_modifiers("<shift>KP_Multiply"),
            Ok(Cow::Owned("Shift+KP_Multiply".into()))
        );
        assert_eq!(
            convert_gnome_modifiers("<ALT>KP_Multiply"),
            Ok(Cow::Owned("Alt+KP_Multiply".into()))
        );
        assert_eq!(
            convert_gnome_modifiers("<SuPeR>KP_Multiply"),
            Ok(Cow::Owned("Super+KP_Multiply".into()))
        );
        assert_eq!(
            convert_gnome_modifiers("<Control><Control>KP_Multiply"),
            Ok(Cow::Owned("Ctrl+Ctrl+KP_Multiply".into()))
        );
    }

    #[test]
    fn converts_many_gnome_modifiers() {
        assert_eq!(
            convert_gnome_modifiers("<Control><ALT><shift>KP_Multiply"),
            Ok(Cow::Owned("Ctrl+Alt+Shift+KP_Multiply".into()))
        );
    }

    #[test]
    fn converts_only_gnome_modifiers() {
        assert_eq!(
            convert_gnome_modifiers("<Control><ALT><shift>"),
            Ok(Cow::Owned("Ctrl+Alt+Shift".into()))
        );
    }

    #[test]
    fn err_on_invalid_gnome_modifier() {
        assert_eq!(convert_gnome_modifiers("<fake>KP_Multiply"), Err("<fake>"));
    }

    #[test]
    fn creates_valid_keys() {
        let converted = convert_gnome_modifiers("<Control>KP_Multiply").unwrap();
        assert_eq!(
            Key::from_str(converted.as_ref()).unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::KP_Multiply),
                modifiers: Modifiers::CTRL,
            }
        );
        let converted = convert_gnome_modifiers("<Control><ALT><shift>KP_Multiply").unwrap();
        assert_eq!(
            Key::from_str(converted.as_ref()).unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::KP_Multiply),
                modifiers: Modifiers::CTRL | Modifiers::ALT | Modifiers::SHIFT,
            }
        );
        let converted = convert_gnome_modifiers("<Control><ALT><shift>").unwrap();
        assert_eq!(
            Key::from_str(converted.as_ref()).unwrap(),
            Key {
                trigger: Trigger::Modifier(ModKey::Shift),
                modifiers: Modifiers::CTRL | Modifiers::ALT | Modifiers::SHIFT,
            }
        );
    }
}
