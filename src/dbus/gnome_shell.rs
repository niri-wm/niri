use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use niri_config::{GlobalShortcuts, Key, ModKey, Modifiers};
use serde::{Deserialize, Serialize};
use smithay::input::keyboard::{Keysym, ModifiersState};
use zbus::blocking::object_server::InterfaceRef;
use zbus::fdo::{self, RequestNameFlags};
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{SerializeDict, Type, Value};

use super::Start;

type Action = u32;

#[derive(Clone)]
pub struct Shell {
    to_niri: calloop::channel::Sender<ShellToNiri>,
    data: Arc<Mutex<Data>>,
}

#[derive(Debug, Clone)]
struct Data {
    action: std::ops::Range<Action>,
    bound_keys: HashMap<niri_config::Key, HashSet<Action>>,
}

pub enum ShellToNiri {
    GrabAccelerators {
        accelerators: Vec<AcceleratorGrab>,
        results: async_channel::Sender<Vec<Action>>,
    },
    UngrabAccelerators {
        actions: Vec<Action>,
        result: async_channel::Sender<bool>,
    },
}

#[derive(Debug, Clone, Deserialize, Type, Default)]
pub struct AcceleratorGrab {
    pub accelerator: String,

    // GNOME parameters, unused by us
    // Shell.ActionMode
    _mode_flags: u32,
    // Meta.KeyBindingFlags
    _grab_flags: u32,
}

#[derive(Debug, Clone, SerializeDict, Type, Value)]
#[zvariant(signature = "dict")]
pub struct ActivationParameters {
    // GNOME portal/shell use u32 -- despite GlobalShortcuts interface using `t` (u64)
    timestamp: u32,
    // GNOME shell uses this to signal state and block shortcuts in some states
    // GNOME portal seems to not care what this is set to
    // see Shell.ActionMode for the relevant enum
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
    ) -> fdo::Result<Vec<u32>> {
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

    async fn ungrab_accelerators(&self, actions: Vec<u32>) -> fdo::Result<bool> {
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
    pub async fn accelerator_activated(
        ctxt: &SignalEmitter<'_>,
        action: u32,
        parameters: ActivationParameters,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn accelerator_deactivated(
        ctxt: &SignalEmitter<'_>,
        action: u32,
        parameters: ActivationParameters,
    ) -> zbus::Result<()>;
}

impl Shell {
    pub fn new(to_niri: calloop::channel::Sender<ShellToNiri>) -> Self {
        Self {
            to_niri,
            data: Arc::new(Mutex::new(Data::default())),
        }
    }

    /// Handles global shortcut (de)activations
    ///
    /// returns true if any action was triggered
    #[allow(clippy::too_many_arguments)]
    pub fn process_key(
        &self,
        keysym: Keysym,
        mods: ModifiersState,
        mod_key: ModKey,
        pressed: bool,
        time: u32,
        iface: InterfaceRef<Shell>,
        shortcuts: &GlobalShortcuts,
    ) -> bool {
        let key = niri_config::Key {
            trigger: niri_config::Trigger::Keysym(keysym),
            modifiers: mod_state_to_modifier(&mods, mod_key),
        };

        let data = self.data.lock().unwrap();
        let Some(actions) = data.bound_keys.get(&key) else {
            return false;
        };

        let ctxt = iface.signal_emitter().clone();
        for action in actions {
            let ctxt = &ctxt;
            async_io::block_on(async move {
                // FIXME: implementing activation_token would allow programs to notify on
                // shortcut activations
                let parameters = ActivationParameters {
                    timestamp: time,
                    action_mode: 1,
                    activation_token: String::default(),
                };

                let result = if pressed {
                    Self::accelerator_activated(ctxt, *action, parameters.clone()).await
                } else {
                    Self::accelerator_deactivated(ctxt, *action, parameters.clone()).await
                };

                if let Err(err) = result {
                    warn!("error emitting global shortcut: {err:?}");
                }
            })
        }

        // Conditionally intercept keypresses based on config
        shortcuts
            .0
            .iter()
            .find(|shortcut| shortcut.trigger == key)
            .map(|shortcut| shortcut.intercept)
            .unwrap_or(false)
    }

    /// Adds a key to global shortcut tracking
    ///
    /// Returns the `Action` id generated for the passed shortcut, returns `None` if grabbing
    /// failed.
    pub fn grab_key(&mut self, key: Key) -> Option<Action> {
        let mut data = self.data.lock().unwrap();
        data.action.next().inspect(|action| {
            data.bound_keys.entry(key).or_default().insert(*action);
        })
    }

    /// Removes an `Action` from global shortcut tracking.
    ///
    /// Returns `true` if `Action` had previously been grabbed for any keys.
    pub fn ungrab_action(&mut self, action: Action) -> bool {
        let mut data = self.data.lock().unwrap();

        let mut found = false;
        let mut drained = Vec::new();
        data.bound_keys.iter_mut().for_each(|(k, v)| {
            if v.contains(&action) {
                v.remove(&action);
                found = true;
            }
            if v.is_empty() {
                drained.push(*k);
            }
        });

        for k in drained {
            data.bound_keys.remove(&k);
        }

        found
    }
}

impl Start for Shell {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::Connection::session()?;
        let flags = RequestNameFlags::AllowReplacement
            | RequestNameFlags::ReplaceExisting
            | RequestNameFlags::DoNotQueue;

        conn.object_server().at("/org/gnome/Shell", self)?;
        conn.request_name_with_flags("org.gnome.Shell", flags)?;

        Ok(conn)
    }
}

impl Default for Data {
    fn default() -> Self {
        Self {
            action: (0..Action::MAX),
            bound_keys: Default::default(),
        }
    }
}

fn mod_state_to_modifier(mods: &ModifiersState, mod_key: ModKey) -> Modifiers {
    let mut out = Modifiers::empty();

    let mapping = [
        (mods.ctrl, ModKey::Ctrl, Modifiers::CTRL),
        (mods.shift, ModKey::Shift, Modifiers::SHIFT),
        (mods.alt, ModKey::Alt, Modifiers::ALT),
        (mods.logo, ModKey::Super, Modifiers::SUPER),
        (
            mods.iso_level3_shift,
            ModKey::IsoLevel3Shift,
            Modifiers::ISO_LEVEL3_SHIFT,
        ),
        (
            mods.iso_level5_shift,
            ModKey::IsoLevel5Shift,
            Modifiers::ISO_LEVEL5_SHIFT,
        ),
    ];

    for (is_pressed, mod_key_pred, modifier) in mapping {
        if is_pressed {
            // `mod_key` could shadow any one modifier
            if mod_key == mod_key_pred {
                out |= Modifiers::COMPOSITOR;
            } else {
                out |= modifier;
            }
        }
    }

    out
}
