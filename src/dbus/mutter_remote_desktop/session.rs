use std::collections::HashMap;
use std::os::fd::OwnedFd as StdOwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use calloop::channel::Sender;
use smithay::backend::input::Keycode;
use zbus::message::Header;
use zbus::names::OwnedUniqueName;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedFd, Value};
use zbus::{fdo, interface, ObjectServer};

use super::clipboard::parse_clipboard_mime_types;
use super::device_types::RemoteDesktopDeviceTypes;
use super::eis::RemoteDesktopEisConnection;
use super::input::{RemoteDesktopPosition, RemoteDesktopToNiri};
use super::keymap::{
    current_keymap, keymap_capabilities, parse_set_keymap_options, RemoteDesktopKeymapChange,
};
use super::legacy_input::{
    button_state, discrete_axis_frame, key_state, smooth_axis_frame, XKB_KEYCODE_OFFSET,
};
use super::registry::RemoteDesktopSessionRegistry;

#[derive(Clone)]
pub struct Session {
    id: u64,
    session_id: String,
    peer_name: OwnedUniqueName,
    to_niri: Sender<RemoteDesktopToNiri>,
    registry: RemoteDesktopSessionRegistry,
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    session_keymap: Arc<AtomicBool>,
}

#[interface(name = "org.gnome.Mutter.RemoteDesktop.Session")]
impl Session {
    async fn start(&self, #[zbus(header)] hdr: Header<'_>) -> fdo::Result<()> {
        self.ensure_open_for_peer(&hdr)?;
        self.registry.mark_started(&self.session_id)?;
        self.started.store(true, Ordering::SeqCst);
        debug!(id = self.id, "RemoteDesktop session started");
        Ok(())
    }

    async fn stop(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(signal_context)] ctxt: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        self.ensure_open_for_peer(&hdr)?;
        debug!(id = self.id, "RemoteDesktop session stopped");

        if self.stopped.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        if let Err(err) = self.registry.remove_session(&self.session_id) {
            warn!("error removing RemoteDesktop session registry entry: {err:?}");
        }

        if let Err(err) = Session::closed(&ctxt).await {
            warn!("error emitting RemoteDesktop Closed signal: {err:?}");
        }

        server
            .remove::<Session, _>(ctxt.path())
            .await
            .map_err(|err| fdo::Error::Failed(format!("error removing session object: {err:?}")))?;

        Ok(())
    }

    #[zbus(signal)]
    async fn closed(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    async fn session_id(&self) -> String {
        self.session_id.clone()
    }

    async fn notify_keyboard_keycode(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        keycode: u32,
        state: bool,
    ) -> fdo::Result<()> {
        self.ensure_started_for_peer(&hdr)?;
        let keycode = keycode.checked_add(XKB_KEYCODE_OFFSET).ok_or_else(|| {
            fdo::Error::InvalidArgs("remote desktop keycode is too large".to_owned())
        })?;
        self.send(RemoteDesktopToNiri::KeyboardKeycode {
            keycode: Keycode::from(keycode),
            state: key_state(state),
        })
    }

    async fn notify_keyboard_keysym(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        _keysym: u32,
        _state: bool,
    ) -> fdo::Result<()> {
        self.ensure_started_for_peer(&hdr)?;
        self.unsupported("remote desktop keysym input")
    }

    async fn notify_pointer_button(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        button: i32,
        state: bool,
    ) -> fdo::Result<()> {
        self.ensure_started_for_peer(&hdr)?;
        let button = u32::try_from(button).map_err(|_| {
            fdo::Error::InvalidArgs("remote desktop button must be non-negative".to_owned())
        })?;
        self.send(RemoteDesktopToNiri::PointerButton {
            button,
            state: button_state(state),
        })
    }

    async fn notify_pointer_axis(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        dx: f64,
        dy: f64,
        flags: u32,
    ) -> fdo::Result<()> {
        self.ensure_started_for_peer(&hdr)?;
        self.send(RemoteDesktopToNiri::PointerAxis {
            frame: smooth_axis_frame(dx, dy, flags)?,
        })
    }

    async fn notify_pointer_axis_discrete(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        axis: u32,
        steps: i32,
    ) -> fdo::Result<()> {
        self.ensure_started_for_peer(&hdr)?;
        if steps == 0 {
            return Err(fdo::Error::InvalidArgs(
                "remote desktop discrete axis steps must not be zero".to_owned(),
            ));
        }

        self.send(RemoteDesktopToNiri::PointerAxis {
            frame: discrete_axis_frame(axis, steps)?,
        })
    }

    async fn notify_pointer_motion_relative(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        dx: f64,
        dy: f64,
    ) -> fdo::Result<()> {
        self.ensure_started_for_peer(&hdr)?;
        self.send(RemoteDesktopToNiri::PointerMotionRelative { dx, dy })
    }

    async fn notify_pointer_motion_absolute(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        stream: &str,
        x: f64,
        y: f64,
    ) -> fdo::Result<()> {
        self.ensure_started_for_peer(&hdr)?;
        self.send(RemoteDesktopToNiri::PointerMotionAbsolute {
            position: self.stream_position(stream, x, y)?,
        })
    }

    async fn notify_touch_down(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        stream: &str,
        slot: u32,
        x: f64,
        y: f64,
    ) -> fdo::Result<()> {
        self.ensure_started_for_peer(&hdr)?;
        self.send(RemoteDesktopToNiri::TouchDown {
            slot,
            position: self.stream_position(stream, x, y)?,
        })
    }

    async fn notify_touch_motion(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        stream: &str,
        slot: u32,
        x: f64,
        y: f64,
    ) -> fdo::Result<()> {
        self.ensure_started_for_peer(&hdr)?;
        self.send(RemoteDesktopToNiri::TouchMotion {
            slot,
            position: self.stream_position(stream, x, y)?,
        })
    }

    async fn notify_touch_up(&self, #[zbus(header)] hdr: Header<'_>, slot: u32) -> fdo::Result<()> {
        self.ensure_started_for_peer(&hdr)?;
        self.send(RemoteDesktopToNiri::TouchUp { slot })
    }

    async fn enable_clipboard(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> fdo::Result<()> {
        self.ensure_open_for_peer(&hdr)?;
        let mime_types = parse_clipboard_mime_types(&options)?;
        self.send_clipboard_enable(mime_types).await
    }

    async fn disable_clipboard(&self, #[zbus(header)] hdr: Header<'_>) -> fdo::Result<()> {
        self.ensure_open_for_peer(&hdr)?;
        self.send_clipboard_disable().await
    }

    async fn set_selection(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> fdo::Result<()> {
        self.ensure_open_for_peer(&hdr)?;
        let mime_types = parse_clipboard_mime_types(&options)?;
        self.send_clipboard_selection(mime_types).await
    }

    async fn selection_write(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        serial: u32,
    ) -> fdo::Result<OwnedFd> {
        self.ensure_open_for_peer(&hdr)?;
        let fd = self.registry.selection_write(&self.session_id, serial)?;
        Ok(OwnedFd::from(fd))
    }

    async fn selection_write_done(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        serial: u32,
        _success: bool,
    ) -> fdo::Result<()> {
        self.ensure_open_for_peer(&hdr)?;
        self.registry.selection_write_done(&self.session_id, serial)
    }

    async fn selection_read(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        mime_type: &str,
    ) -> fdo::Result<OwnedFd> {
        self.ensure_open_for_peer(&hdr)?;
        self.send_clipboard_read(mime_type.to_owned()).await
    }

    #[zbus(signal)]
    pub async fn selection_owner_changed(
        ctxt: &SignalEmitter<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn selection_transfer(
        ctxt: &SignalEmitter<'_>,
        mime_type: &str,
        serial: u32,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    async fn caps_lock_state(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn num_lock_state(&self) -> bool {
        false
    }

    #[zbus(name = "ConnectToEIS")]
    async fn connect_to_eis(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> fdo::Result<OwnedFd> {
        self.ensure_open_for_peer(&hdr)?;
        let device_types = RemoteDesktopDeviceTypes::from_options(&options)?;
        let (server_socket, client_socket) = UnixStream::pair()
            .map_err(|err| fdo::Error::Failed(format!("failed to create EIS socket: {err:?}")))?;
        let connection = RemoteDesktopEisConnection::new(
            server_socket,
            self.session_id.clone(),
            self.registry.clone(),
            device_types,
        );
        self.send(RemoteDesktopToNiri::ConnectEis(connection))?;

        let client_fd: StdOwnedFd = client_socket.into();
        Ok(OwnedFd::from(client_fd))
    }

    #[zbus(property)]
    async fn keymap_capabilities(&self) -> HashMap<String, Value<'static>> {
        keymap_capabilities()
    }

    async fn set_keymap(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> fdo::Result<()> {
        self.ensure_open_for_peer(&hdr)?;
        let change = parse_set_keymap_options(&options)?;
        let is_session_keymap = matches!(change, RemoteDesktopKeymapChange::Xkb { .. });
        self.send_keymap_change(change).await?;
        self.session_keymap
            .store(is_session_keymap, Ordering::SeqCst);
        Ok(())
    }

    async fn set_keymap_layout_index(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        index: u32,
    ) -> fdo::Result<()> {
        self.ensure_open_for_peer(&hdr)?;
        if !self.session_keymap.load(Ordering::SeqCst) {
            return Err(fdo::Error::Failed(
                "remote desktop session keymap is not current".to_owned(),
            ));
        }

        let (reply, rx) = async_channel::bounded(1);
        self.send(RemoteDesktopToNiri::SetKeymapLayoutIndex { index, reply })?;
        rx.recv()
            .await
            .map_err(|_| fdo::Error::Failed("niri event loop is gone".to_owned()))?
            .map_err(fdo::Error::Failed)
    }

    #[zbus(property)]
    async fn current_keymap(&self) -> HashMap<String, Value<'static>> {
        current_keymap(self.session_keymap.load(Ordering::SeqCst))
    }
}

impl Session {
    pub fn new(
        id: u64,
        session_id: String,
        peer_name: OwnedUniqueName,
        to_niri: Sender<RemoteDesktopToNiri>,
        registry: RemoteDesktopSessionRegistry,
    ) -> Self {
        Self {
            id,
            session_id,
            peer_name,
            to_niri,
            registry,
            started: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicBool::new(false)),
            session_keymap: Arc::new(AtomicBool::new(false)),
        }
    }

    fn stream_position(&self, stream: &str, x: f64, y: f64) -> fdo::Result<RemoteDesktopPosition> {
        let position = self.registry.stream_position(stream, x, y)?;
        Ok(RemoteDesktopPosition {
            output_name: position.output_name,
            x: position.x,
            y: position.y,
            width: position.width,
            height: position.height,
        })
    }

    fn send(&self, msg: RemoteDesktopToNiri) -> fdo::Result<()> {
        self.to_niri
            .send(msg)
            .map_err(|_| fdo::Error::Failed("niri event loop is gone".to_owned()))
    }

    async fn send_keymap_change(&self, change: RemoteDesktopKeymapChange) -> fdo::Result<()> {
        let (reply, rx) = async_channel::bounded(1);
        self.send(RemoteDesktopToNiri::SetKeymap { change, reply })?;
        rx.recv()
            .await
            .map_err(|_| fdo::Error::Failed("niri event loop is gone".to_owned()))?
            .map_err(fdo::Error::Failed)
    }

    async fn send_clipboard_enable(&self, mime_types: Option<Vec<String>>) -> fdo::Result<()> {
        let (reply, rx) = async_channel::bounded(1);
        self.send(RemoteDesktopToNiri::EnableClipboard {
            session_id: self.session_id.clone(),
            mime_types,
            reply,
        })?;
        recv_clipboard_reply(rx).await
    }

    async fn send_clipboard_disable(&self) -> fdo::Result<()> {
        let (reply, rx) = async_channel::bounded(1);
        self.send(RemoteDesktopToNiri::DisableClipboard {
            session_id: self.session_id.clone(),
            reply,
        })?;
        recv_clipboard_reply(rx).await
    }

    async fn send_clipboard_selection(&self, mime_types: Option<Vec<String>>) -> fdo::Result<()> {
        let (reply, rx) = async_channel::bounded(1);
        self.send(RemoteDesktopToNiri::SetSelection {
            session_id: self.session_id.clone(),
            mime_types,
            reply,
        })?;
        recv_clipboard_reply(rx).await
    }

    async fn send_clipboard_read(&self, mime_type: String) -> fdo::Result<OwnedFd> {
        let (reply, rx) = async_channel::bounded(1);
        self.send(RemoteDesktopToNiri::ReadClipboard {
            session_id: self.session_id.clone(),
            mime_type,
            reply,
        })?;
        let fd = rx
            .recv()
            .await
            .map_err(|_| fdo::Error::Failed("niri event loop is gone".to_owned()))?
            .map_err(fdo::Error::Failed)?;
        Ok(OwnedFd::from(fd))
    }

    fn unsupported<T>(&self, feature: &str) -> fdo::Result<T> {
        self.ensure_open()?;
        Err(fdo::Error::NotSupported(format!(
            "{feature} is not implemented"
        )))
    }

    fn ensure_started(&self) -> fdo::Result<()> {
        self.ensure_open()?;
        if !self.started.load(Ordering::SeqCst) {
            return Err(fdo::Error::Failed("session not started".to_owned()));
        }
        Ok(())
    }

    fn ensure_open(&self) -> fdo::Result<()> {
        if self.stopped.load(Ordering::SeqCst) {
            return Err(fdo::Error::Failed("session is stopped".to_owned()));
        }
        Ok(())
    }

    fn ensure_started_for_peer(&self, hdr: &Header<'_>) -> fdo::Result<()> {
        self.check_permission(hdr)?;
        self.ensure_started()
    }

    fn ensure_open_for_peer(&self, hdr: &Header<'_>) -> fdo::Result<()> {
        self.check_permission(hdr)?;
        self.ensure_open()
    }

    fn check_permission(&self, hdr: &Header<'_>) -> fdo::Result<()> {
        let Some(sender) = hdr.sender() else {
            return Err(fdo::Error::Failed(
                "remote desktop request has no sender".to_owned(),
            ));
        };

        if sender.as_str() != self.peer_name.as_str() {
            return Err(fdo::Error::AccessDenied(
                "remote desktop session belongs to another peer".to_owned(),
            ));
        }

        Ok(())
    }
}

async fn recv_clipboard_reply(rx: async_channel::Receiver<Result<(), String>>) -> fdo::Result<()> {
    rx.recv()
        .await
        .map_err(|_| fdo::Error::Failed("niri event loop is gone".to_owned()))?
        .map_err(fdo::Error::Failed)
}
