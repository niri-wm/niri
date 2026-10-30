use std::borrow::Cow;
use std::str::FromStr;
use std::sync::{Arc, PoisonError, RwLock};

use niri_config::{Action, Bind, BoundAction, Key};
use serde::Deserialize;
use smithay::wayland::xdg_activation::XdgActivationState;
use zbus::blocking::object_server::InterfaceRef;
use zbus::fdo::{self};
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{SerializeDict, Type, Value};

use super::Start;
use crate::dbus::request_name;

// It would be nice to make this a newtype to prevent accidental misuse, but we need to return a
// Vec<u32> from org.gnome.Shell, so newtyping would require an extra allocation or a transmute.
pub type ActionId = u32;

const INVALID_ACTION: ActionId = u32::MAX;

#[derive(Clone)]
pub struct GlobalShortcutsPortalBinds {
    next_action: ActionId,
    binds: Vec<Bind>,
}

impl GlobalShortcutsPortalBinds {
    /// Creates a new instance of these shortcuts.
    pub fn new() -> Self {
        Self {
            next_action: 0,
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
    pub fn ungrab_accelerators(&mut self, actions: &[ActionId]) -> bool {
        let mut unbound = false;
        for action in actions {
            unbound |= self.unbind_action(*action);
        }
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
            allow_inhibiting: false,
            hotkey_overlay_title: None,
        });
        action
    }

    /// Removes all bindings associated with `action`.
    ///
    /// Returns `true` if any key was bound to `action`, false otherwise.
    fn unbind_action(&mut self, action: ActionId) -> bool {
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

            if bound_action == action {
                unbound = true;
                false
            } else {
                true
            }
        });
        unbound
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
        self.next_action += 1;
        next
    }
}

#[derive(Clone)]
pub struct Shell {
    to_niri: calloop::channel::Sender<ShellToNiri>,
    iface: Arc<RwLock<Option<InterfaceRef<Shell>>>>,
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
        &mut self,
        accelerators: Vec<AcceleratorGrab>,
    ) -> fdo::Result<Vec<ActionId>> {
        let (tx, rx) = async_channel::bounded(1);

        if let Err(err) = self.to_niri.send(ShellToNiri::GrabAccelerators {
            accelerators,
            results: tx,
        }) {
            warn!("error sending grab accelerators message to niri: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }
        rx.recv().await.map_err(|err| {
            warn!("error receiving message from niri: {err:?}");
            fdo::Error::Failed("internal error".to_owned())
        })
    }

    async fn ungrab_accelerators(&self, actions: Vec<ActionId>) -> fdo::Result<bool> {
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
            iface: Arc::new(RwLock::new(None)),
        }
    }

    /// Activates the given `action`, with the given `time` being the time the action was pressed.
    pub fn activate_action(&self, action: ActionId, activation_state: &mut XdgActivationState) {
        let iface = self.iface.read().unwrap();
        // Unwrap is safe since we drop the shell before activating any actions if the iface fails
        // to populate.
        let iface = iface.as_ref().unwrap();

        let (activation_token, _) = activation_state.create_external_token(None);

        async_io::block_on(async {
            if let Err(err) = Self::accelerator_activated(
                iface.signal_emitter(),
                action,
                ActivationParameters {
                    // FIXME: This timestamp doesn't **seem** to do anything, but it's probably not
                    // great that we're giving it a dummy value. However, getting the input time
                    // from anything that can trigger a bind is non-trivial (e.g., key-repeat
                    // timers). Ideally we'd get the InputTime for whatever event caused us to
                    // activate this action.
                    timestamp: 0,
                    action_mode: 1,
                    activation_token: activation_token.to_string(),
                },
            )
            .await
            {
                warn!("error emitting global shortcut: {err:?}");
            }
        });
    }

    /// Activates the given `action`, with the given `time` being the time the action was pressed.
    pub fn deactivate_action(&self, action: ActionId, activation_state: &mut XdgActivationState) {
        let iface = self.iface.read().unwrap();
        // Unwrap is safe since we drop the shell before activating any actions if the iface fails
        // to populate.
        let iface = iface.as_ref().unwrap();

        let (activation_token, _) = activation_state.create_external_token(None);

        async_io::block_on(async {
            if let Err(err) = Self::accelerator_deactivated(
                iface.signal_emitter(),
                action,
                ActivationParameters {
                    // FIXME: This timestamp doesn't **seem** to do anything, but it's probably not
                    // great that we're giving it a dummy value. However, getting the input time
                    // from anything that can trigger a bind is non-trivial (e.g., key-repeat
                    // timers). Ideally we'd get the InputTime for whatever event caused us to
                    // deactivate this action.
                    timestamp: 0,
                    action_mode: 1,
                    activation_token: activation_token.to_string(),
                },
            )
            .await
            {
                warn!("error emitting global shortcut: {err:?}");
            }
        });
    }
}

impl Start for Shell {
    fn start(self, monitor: bool) -> anyhow::Result<zbus::blocking::Connection> {
        let iface_arc = self.iface.clone();

        let conn = zbus::blocking::Connection::session()?;

        conn.object_server().at("/org/gnome/Shell", self)?;
        request_name(&conn, "org.gnome.Shell", monitor)?;

        // Cache the iface so we can use it later when emitting signals.
        let iface = match conn.object_server().interface("/org/gnome/Shell") {
            Ok(iface) => iface,
            Err(err) => return Err(err.into()),
        };
        *iface_arc.write().unwrap_or_else(PoisonError::into_inner) = Some(iface);

        Ok(conn)
    }
}

// Gnome portal converts modifiers into this format and back again afterwards. Their settings
// provider seems to use a different format than everything else and their portal expects that.
const GNOME_PORTAL_KEY_MAP: &[(&str, &str)] = &[
    ("<control>", "Ctrl"),
    ("<shift>", "Shift"),
    ("<alt>", "Alt"),
    ("<super>", "Super"),
];

/// Returns either the string with any Gnome modifiers replaced with their "canonical" form, or on
/// failure, the modifier that was unable to be converted.
fn convert_gnome_modifiers(input: &str) -> Result<Cow<'_, str>, &str> {
    let mut parts: Vec<&str> = Vec::new();
    let mut remaining = input;
    while let Some(start) = remaining.find('<') {
        let Some(end) = remaining[start..].find('>') else {
            break;
        };
        let token = &remaining[start..=start + end];
        let found_value = 'found: {
            for (key, value) in GNOME_PORTAL_KEY_MAP {
                if token.eq_ignore_ascii_case(*key) {
                    break 'found value;
                }
            }
            return Err(token);
        };
        parts.push(found_value);
        remaining = &remaining[(start + end + 1)..];
    }

    if parts.is_empty() {
        return Ok(Cow::Borrowed(remaining));
    }

    if !remaining.is_empty() {
        parts.push(remaining);
    }

    Ok(parts.join("+").into())
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
