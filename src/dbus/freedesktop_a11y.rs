// References:
// - https://invent.kde.org/plasma/kwin/-/blob/397fbbe52a8f2d855ad0c9817b51a9bdf06a68e2/src/a11ykeyboardmonitor.cpp#L41
// - https://gitlab.gnome.org/GNOME/mutter/-/blob/cbb7295ac1f93a2dfd55a7c0544688e7e5c4d2e2/src/backends/meta-a11y-manager.c

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::Context;
use futures_util::StreamExt;
use smithay::backend::input::{KeyState, Keycode};
use smithay::input::keyboard::{xkb, Keysym};
use zbus::blocking::object_server::InterfaceRef;
use zbus::fdo::{self, RequestNameFlags};
use zbus::interface;
use zbus::message::Header;
use zbus::names::{BusName, OwnedUniqueName, UniqueName};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::NoneValue;

use super::Start;
use crate::niri::State;

#[derive(Debug, Default)]
struct Data {
    clients: HashMap<OwnedUniqueName, Client>,

    grabbed_mods: HashSet<Keysym>,
    grabbed_mod_last_press_time: HashMap<Keysym, Duration>,
    suppressed_keys: HashSet<Keysym>,
}

#[derive(Debug, Default)]
struct Client {
    watched: bool,
    grabbed: bool,
    modifiers: HashSet<Keysym>,
    keystrokes: Vec<(Keysym, u32)>,
}

#[derive(Clone)]
pub struct KeyboardMonitor {
    data: Arc<Mutex<Data>>,
    iface: Arc<OnceLock<InterfaceRef<Self>>>,
}

/// Keyboard monitor key block reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KbMonBlock {
    /// Not blocked.
    Pass,
    /// Blocked, and this is the first press/release of the a11y modifier.
    ModifierFirstPress,
    /// Blocked, and this is not the a11y modifier.
    Block,
}

/// Interface for monitoring of keyboard input by assistive technologies.
///
/// This interface is used by assistive technologies to monitor keyboard input of the compositor.
/// The compositor is expected to listen on the well-known bus name "org.freedesktop.a11y.Manager"
/// at the object path "/org/freedesktop/a11y/Manager".
#[interface(name = "org.freedesktop.a11y.KeyboardMonitor")]
impl KeyboardMonitor {
    // Starts grabbing all key events. The client receives the events through the KeyEvent signal,
    // and in addition, the events aren't handled normally by the compositor. This includes changes
    // to the state of toggles like Caps Lock, Num Lock, and Scroll Lock.
    //
    // This behavior stays in effect until the same client calls UngrabKeyboard or closes its D-Bus
    // connection.
    async fn grab_keyboard(&self, #[zbus(header)] hdr: Header<'_>) -> fdo::Result<()> {
        let Some(sender) = hdr.sender() else {
            return Err(fdo::Error::Failed("no sender".to_owned()));
        };
        let sender = OwnedUniqueName::from(sender.to_owned());
        trace!("enabling keyboard grab for {sender}");

        let mut data = self.data.lock().unwrap();
        let client = data.clients.entry(sender).or_default();
        client.grabbed = true;

        Ok(())
    }

    // Reverses the effect of calling GrabKeyboard. If GrabKeyboard wasn't previously called, this
    // method does nothing.
    //
    // After calling this method, the key grabs specified in the last call to SetKeyGrabs, if any,
    // are still in effect. Also, the client will still receive key events through the KeyEvent
    // signal, if it has called WatchKeyboard.
    async fn ungrab_keyboard(&self, #[zbus(header)] hdr: Header<'_>) -> fdo::Result<()> {
        let Some(sender) = hdr.sender() else {
            return Err(fdo::Error::Failed("no sender".to_owned()));
        };
        let sender = OwnedUniqueName::from(sender.to_owned());

        let mut data = self.data.lock().unwrap();
        if let Some(client) = data.clients.get_mut(&sender) {
            trace!("disabling keyboard grab for {sender}");
            client.grabbed = false;
        }

        Ok(())
    }

    // Starts watching all key events. The client receives the events through the KeyEvent signal,
    // but the events are still handled normally by the compositor. This includes changes to the
    // state of toggles like Caps Lock, Num Lock, and Scroll Lock.
    //
    // This behavior stays in effect until the same client calls UnwatchKeyboard or closes its D-Bus
    // connection.
    async fn watch_keyboard(&self, #[zbus(header)] hdr: Header<'_>) -> fdo::Result<()> {
        let Some(sender) = hdr.sender() else {
            return Err(fdo::Error::Failed("no sender".to_owned()));
        };
        let sender = OwnedUniqueName::from(sender.to_owned());
        trace!("enabling keyboard watch for {sender}");

        let mut data = self.data.lock().unwrap();
        let client = data.clients.entry(sender).or_default();
        client.watched = true;

        Ok(())
    }

    // Reverses the effect of calling WatchKeyboard. If WatchKeyboard wasn't previously called, this
    // method does nothing.
    //
    // After calling this method, the key grabs specified in the last call to SetKeyGrabs, if any,
    // are still in effect, but other key events are no longer reported to this client.
    async fn unwatch_keyboard(&self, #[zbus(header)] hdr: Header<'_>) -> fdo::Result<()> {
        let Some(sender) = hdr.sender() else {
            return Err(fdo::Error::Failed("no sender".to_owned()));
        };
        let sender = OwnedUniqueName::from(sender.to_owned());

        let mut data = self.data.lock().unwrap();
        if let Some(client) = data.clients.get_mut(&sender) {
            trace!("disabling keyboard watch for {sender}");
            client.watched = false;
        }

        Ok(())
    }

    // Sets the current key grabs for the calling client, overriding any previous call to this
    // method. For grabbed key events, the KeyEvent signal is emitted, and normal key event handling
    // is suppressed, including state changes for toggles like Caps Lock and Num Lock.
    //
    // The grabs set by this method stay in effect until the same client calls this method again, or
    // until that client closes its D-Bus connection.
    //
    // Each item in `modifiers` is an XKB keysym. All keys in this list will be grabbed, and keys
    // pressed while any of these keys are down will also be grabbed.
    //
    // Each item in `keystrokes` is a struct with the following fields:
    //
    // - the XKB keysym of the non-modifier key
    // - the XKB modifier mask of the modifiers, if any, for this keystroke
    //
    // If any of the keys in `modifiers` is pressed alone, the compositor is required to ignore the
    // key press and release event if a second key press of the same modifier is not received within
    // a reasonable time frame, for example, the key repeat delay. If such event is received, this
    // second event is processed normally.
    async fn set_key_grabs(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        modifiers: Vec<u32>,
        keystrokes: Vec<(u32, u32)>,
    ) -> fdo::Result<()> {
        let Some(sender) = hdr.sender() else {
            return Err(fdo::Error::Failed("no sender".to_owned()));
        };
        let sender = OwnedUniqueName::from(sender.to_owned());
        trace!("updating key grabs for {sender}");

        let mut data = self.data.lock().unwrap();
        let client = data.clients.entry(sender).or_default();
        client.modifiers = HashSet::from_iter(modifiers.into_iter().map(Keysym::new));
        client.keystrokes =
            Vec::from_iter(keystrokes.into_iter().map(|(k, v)| (Keysym::new(k), v)));

        data.rebuild_grabbed_mods();

        Ok(())
    }

    // The compositor emits this signal for each key press or release.
    //
    // - `released`: whether this is a key-up event
    // - `state`: XKB modifier mask for currently pressed modifiers
    // - `keysym`: XKB keysym for this key
    // - `unichar`: Unicode character for this key, or 0 if none
    // - `keycode`: hardware-dependent keycode for this key
    #[zbus(signal)]
    pub async fn key_event(
        ctxt: &SignalEmitter<'_>,
        released: bool,
        state: u32,
        keysym: u32,
        unichar: u32,
        keycode: u16,
    ) -> zbus::Result<()>;
}

impl KeyboardMonitor {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(Data::default())),
            iface: Arc::new(OnceLock::new()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process_key(
        &self,
        repeat_delay: Duration,
        time: Duration,
        keycode: Keycode,
        released: bool,
        mods: u32,
        keysym: Keysym,
        unichar: u32,
    ) -> KbMonBlock {
        let _span = tracy_client::span!("KeyboardMonitor::process_key");

        let mut ctxt = self.iface.get().unwrap().signal_emitter().clone();

        let mut data = self.data.lock().unwrap();

        // Emit key events as necessary.
        for (name, client) in &data.clients {
            if client.should_watch_keypress(&data.suppressed_keys, mods, keysym) {
                let _span = tracy_client::span!("emitting key event");

                // Emit to that client only.
                ctxt = ctxt.set_destination(BusName::Unique(name.as_ref()));
                let ctxt = &ctxt;
                async_io::block_on(async move {
                    if let Err(err) = KeyboardMonitor::key_event(
                        ctxt,
                        released,
                        mods,
                        keysym.raw(),
                        unichar,
                        keycode.raw() as u16,
                    )
                    .await
                    {
                        warn!("error emitting key_event: {err:?}");
                    }
                });
            }
        }

        // Check for double-pressed grabbed modifier that should not be captured.
        if data.grabbed_mods.contains(&keysym) {
            if released {
                // If missing from suppressed keys, then this is a release corresponding to the
                // second press that got handled normally.
                if !data.suppressed_keys.contains(&keysym) {
                    trace!("handling release for second press of grabbed modifier: {keysym:?}");
                    return KbMonBlock::Pass;
                }
            } else {
                let last_press_entry = data
                    .grabbed_mod_last_press_time
                    .entry(keysym)
                    .or_insert(Duration::ZERO);
                let last_press = *last_press_entry;
                *last_press_entry = time;

                // Modifier pressed twice; handle it as normal.
                if time <= last_press.saturating_add(repeat_delay) {
                    trace!("handling second press of grabbed modifier: {keysym:?}");
                    return KbMonBlock::Pass;
                }
            }
        }

        let mut block = false;

        if released {
            // This is a release for a key that was grabbed.
            if data.suppressed_keys.remove(&keysym) {
                trace!("blocking release for previously suppressed key: {keysym:?}");
                block = true;
            }
        } else if data.suppressed_keys.contains(&keysym) {
            // Second press for an already-pressed key, e.g. from two keyboards.
            trace!("blocking press for already-pressed key: {keysym:?}");
            block = true;
        } else {
            // Check if it's grabbed by any client.
            if data
                .clients
                .values()
                .any(|client| client.should_grab_keypress(&data.suppressed_keys, mods, keysym))
            {
                trace!("blocking press for grabbed key: {keysym:?}");
                data.suppressed_keys.insert(keysym);
                block = true;
            }
        }

        if !block {
            KbMonBlock::Pass
        } else if data.grabbed_mods.contains(&keysym) {
            KbMonBlock::ModifierFirstPress
        } else {
            KbMonBlock::Block
        }
    }
}

impl Data {
    fn rebuild_grabbed_mods(&mut self) {
        self.grabbed_mods.clear();
        for client in self.clients.values() {
            self.grabbed_mods.extend(&client.modifiers);
        }
    }
}

impl Client {
    fn should_grab_keypress(
        &self,
        suppressed_keys: &HashSet<Keysym>,
        mods: u32,
        keysym: Keysym,
    ) -> bool {
        // Grabbing all keys.
        if self.grabbed {
            return true;
        }

        for modifier in &self.modifiers {
            // This is a grabbed modifier, or a grabbed modifier is currently down.
            if *modifier == keysym || suppressed_keys.contains(modifier) {
                return true;
            }
        }

        for (grabbed_keysym, grabbed_mods) in &self.keystrokes {
            // This is a grabbed keystroke.
            if *grabbed_keysym == keysym && *grabbed_mods == mods {
                return true;
            }
        }

        false
    }

    fn should_watch_keypress(
        &self,
        suppressed_keys: &HashSet<Keysym>,
        mods: u32,
        keysym: Keysym,
    ) -> bool {
        if self.watched {
            return true;
        }

        self.should_grab_keypress(suppressed_keys, mods, keysym)
    }
}

async fn monitor_disappeared_clients(
    conn: &zbus::Connection,
    data: Arc<Mutex<Data>>,
) -> anyhow::Result<()> {
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

        if args.new_owner().is_none() {
            trace!("keyboard monitor client disconnected: {name}");

            let name = OwnedUniqueName::from(name.to_owned());
            let mut data = data.lock().unwrap();
            data.clients.remove(&name);
            data.rebuild_grabbed_mods();
        } else {
            error!("non-null new_owner should've been filtered out");
        }
    }

    Ok(())
}

impl Start for KeyboardMonitor {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let data = self.data.clone();

        let conn = zbus::blocking::Connection::session()?;
        let flags = RequestNameFlags::AllowReplacement
            | RequestNameFlags::ReplaceExisting
            | RequestNameFlags::DoNotQueue;

        conn.object_server()
            .at("/org/freedesktop/a11y/Manager", self.clone())?;
        conn.request_name_with_flags("org.freedesktop.a11y.Manager", flags)?;

        let iface = conn
            .object_server()
            .interface("/org/freedesktop/a11y/Manager")?;
        let _ = self.iface.set(iface);

        let async_conn = conn.inner().clone();
        let future = async move {
            if let Err(err) = monitor_disappeared_clients(&async_conn, data.clone()).await {
                warn!("error monitoring keyboard monitor clients: {err:?}");

                // Since the monitor is now broken, prevent any further communication.
                if let Err(err) = async_conn.close().await {
                    warn!("error closing connection: {err:?}");
                }

                let mut data = data.lock().unwrap();
                data.clients.clear();
                data.rebuild_grabbed_mods();
            }
        };
        let task = conn
            .inner()
            .executor()
            .spawn(future, "monitor disappearing keyboard clients");
        task.detach();

        Ok(conn)
    }
}

impl State {
    pub fn a11y_process_key(
        &mut self,
        time: Duration,
        keycode: Keycode,
        state: KeyState,
    ) -> KbMonBlock {
        if self.niri.a11y_keyboard_monitor.is_none() {
            return KbMonBlock::Pass;
        }

        let keyboard = self.niri.seat.get_keyboard().unwrap();

        let (mods, keysym, unichar) = keyboard.with_xkb_state(self, |context| {
            let xkb = context.xkb().lock().unwrap();
            // SAFETY: we're not changing the ref count.
            let state = unsafe { xkb.state() };

            let keysym = state.key_get_one_sym(keycode);
            let mods = state.serialize_mods(xkb::STATE_MODS_EFFECTIVE);
            let unichar = state.key_get_utf32(keycode);

            (mods, keysym, unichar)
        });

        let config = self.niri.config.borrow();
        let repeat_delay = Duration::from_millis(u64::from(config.input.keyboard.repeat_delay));
        let released = state == KeyState::Released;

        let Some(monitor) = &self.niri.a11y_keyboard_monitor else {
            return KbMonBlock::Pass;
        };
        monitor.process_key(repeat_delay, time, keycode, released, mods, keysym, unichar)
    }
}
