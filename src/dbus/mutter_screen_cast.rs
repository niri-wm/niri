use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use futures_util::lock::Mutex;
use serde::Deserialize;
use zbus::fdo::RequestNameFlags;
use zbus::object_server::{InterfaceRef, SignalEmitter};
use zbus::zvariant::{DeserializeDict, OwnedObjectPath, SerializeDict, Type, Value};
use zbus::{fdo, interface, ObjectServer};

use super::Start;
use crate::backend::IpcOutputMap;
#[cfg(feature = "xdp-gnome-remote-desktop")]
use crate::dbus::mutter_remote_desktop::shared::RemoteDesktopShared;
use crate::utils::{CastSessionId, CastStreamId};

pub enum ScreenCastToNiri {
    /// Starts a stream associated with a screencast session.
    StartStream {
        session_id: CastSessionId,
        stream_id: CastStreamId,
        target: StreamTargetId,
        cursor_mode: CursorMode,
        signal_ctx: SignalEmitter<'static>,
    },
    /// Stops all streams associated with the specified screencast session.
    StopCast {
        session_id: CastSessionId,
        /// The reason for stopping the screencast, mainly for debugging.
        reason: StopCastReason,
    },
}

#[derive(Debug)]
pub enum StopCastReason {
    RemoteDesktopStopped,
    FromNiriStopCast,
    DbusStop,
    SessionDropped,
}

// == ROOT INTERFACE ==

#[derive(Clone)]
pub struct ScreenCast {
    ipc_outputs: Arc<StdMutex<IpcOutputMap>>,
    to_niri: calloop::channel::Sender<ScreenCastToNiri>,
    #[cfg(feature = "xdp-gnome-remote-desktop")]
    remote_desktop_shared: Arc<Mutex<RemoteDesktopShared>>,
    #[cfg(feature = "xdp-gnome-remote-desktop")]
    remote_desktop_object_server: Option<ObjectServer>,
}

#[derive(Debug, DeserializeDict, Type)]
#[zvariant(signature = "dict")]
struct CreateSessionProperties {
    #[zvariant(rename = "remote-desktop-session-id")]
    remote_desktop_session_id: Option<String>,
}

#[interface(name = "org.gnome.Mutter.ScreenCast")]
impl ScreenCast {
    async fn create_session(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        properties: CreateSessionProperties,
    ) -> fdo::Result<OwnedObjectPath> {
        #[cfg(not(feature = "xdp-gnome-remote-desktop"))]
        if properties.remote_desktop_session_id.is_some() {
            return Err(fdo::Error::Failed(
                "Remote desktop support has been disabled at compile time in Niri".to_owned(),
            ));
        }

        #[cfg(feature = "xdp-gnome-remote-desktop")]
        let rd_session_iface = if let Some(rd_session_id) = properties.remote_desktop_session_id {
            use crate::utils::RemoteDesktopSessionId;

            debug!(rd_session_id, "ScreenCast.CreateSession");
            let rd_session_id: u64 = rd_session_id.parse().map_err(|err| {
                fdo::Error::Failed(format!("Invalid remote desktop session ID: {err}"))
            })?;
            let rd_session_id = RemoteDesktopSessionId::from(rd_session_id);

            let shared = self.remote_desktop_shared.lock().await;

            Some(
                shared
                    .sessions
                    .get(&rd_session_id)
                    .ok_or(fdo::Error::Failed(
                        "No matching remote desktop session".to_owned(),
                    ))?
                    .clone(),
            )
        } else {
            None
        };

        #[cfg(feature = "xdp-gnome-remote-desktop")]
        let rd_session_state = if let Some(iface) = &rd_session_iface {
            let state = iface.get_mut().await;

            if state.active {
                return Err(fdo::Error::Failed(
                    "The remote desktop session has already started".to_owned(),
                ));
            }

            if state.screen_cast_session.is_some() {
                return Err(fdo::Error::Failed(
                    "The remote desktop session already has an associated screencast session"
                        .to_owned(),
                ));
            }

            Some(state)
        } else {
            None
        };

        let session_id = CastSessionId::next();
        let path = format!("/org/gnome/Mutter/ScreenCast/Session/u{}", session_id.get());
        let path = OwnedObjectPath::try_from(path).unwrap();

        let session = Session {
            id: session_id,
            ipc_outputs: self.ipc_outputs.clone(),
            streams: Arc::new(Mutex::new(vec![])),
            to_niri: self.to_niri.clone(),
            stopped: Arc::new(AtomicBool::new(false)),
            sent_stop_cast: Arc::new(AtomicBool::new(false)),
            #[cfg(feature = "xdp-gnome-remote-desktop")]
            rd_session: rd_session_iface.as_ref().and_then(|iface| {
                Some((
                    iface.clone(),
                    self.remote_desktop_object_server.as_ref()?.clone(),
                ))
            }),
        };

        match server.at(&path, session).await {
            Ok(true) => {
                let iface = server.interface(&path).await.unwrap();

                #[cfg(feature = "xdp-gnome-remote-desktop")]
                if let Some(mut state) = rd_session_state {
                    state.screen_cast_session = Some((iface.clone(), server.clone()));
                }
            }
            Ok(false) => return Err(fdo::Error::Failed("session path already exists".to_owned())),
            Err(err) => {
                return Err(fdo::Error::Failed(format!(
                    "error creating session object: {err:?}"
                )))
            }
        }

        Ok(path)
    }

    #[zbus(property)]
    async fn version(&self) -> i32 {
        4
    }
}

impl ScreenCast {
    pub fn new(
        ipc_outputs: Arc<StdMutex<IpcOutputMap>>,
        to_niri: calloop::channel::Sender<ScreenCastToNiri>,
        #[cfg(feature = "xdp-gnome-remote-desktop")] remote_desktop_shared: Arc<
            Mutex<RemoteDesktopShared>,
        >,
        #[cfg(feature = "xdp-gnome-remote-desktop")] remote_desktop_object_server: Option<
            ObjectServer,
        >,
    ) -> Self {
        Self {
            ipc_outputs,
            to_niri,
            #[cfg(feature = "xdp-gnome-remote-desktop")]
            remote_desktop_shared,
            #[cfg(feature = "xdp-gnome-remote-desktop")]
            remote_desktop_object_server,
        }
    }
}

impl Start for ScreenCast {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::Connection::session()?;
        let flags = RequestNameFlags::AllowReplacement
            | RequestNameFlags::ReplaceExisting
            | RequestNameFlags::DoNotQueue;

        conn.object_server()
            .at("/org/gnome/Mutter/ScreenCast", self)?;
        conn.request_name_with_flags("org.gnome.Mutter.ScreenCast", flags)?;

        Ok(conn)
    }
}

// == SESSION ==

pub struct Session {
    id: CastSessionId,
    pub ipc_outputs: Arc<StdMutex<IpcOutputMap>>,
    to_niri: calloop::channel::Sender<ScreenCastToNiri>,
    #[allow(clippy::type_complexity)]
    streams: Arc<Mutex<Vec<InterfaceRef<Stream>>>>,
    stopped: Arc<AtomicBool>,
    sent_stop_cast: Arc<AtomicBool>,
    #[cfg(feature = "xdp-gnome-remote-desktop")]
    rd_session: Option<(
        InterfaceRef<super::mutter_remote_desktop::Session>,
        ObjectServer,
    )>,
}

#[derive(Debug, Default, Deserialize, Type, Clone, Copy, PartialEq, Eq)]
pub enum CursorMode {
    #[default]
    Hidden = 0,
    Embedded = 1,
    Metadata = 2,
}

#[derive(Debug, DeserializeDict, Type)]
#[zvariant(signature = "dict")]
struct RecordMonitorProperties {
    #[zvariant(rename = "cursor-mode")]
    cursor_mode: Option<CursorMode>,
    #[zvariant(rename = "is-recording")]
    _is_recording: Option<bool>,
}

#[derive(Debug, DeserializeDict, Type)]
#[zvariant(signature = "dict")]
struct RecordWindowProperties {
    #[zvariant(rename = "window-id")]
    window_id: u64,
    #[zvariant(rename = "cursor-mode")]
    cursor_mode: Option<CursorMode>,
    #[zvariant(rename = "is-recording")]
    _is_recording: Option<bool>,
}

#[interface(name = "org.gnome.Mutter.ScreenCast.Session")]
impl Session {
    /// Starts the streams of this screencast session.
    #[zbus(name = "Start")]
    async fn start_dbus(&self) -> fdo::Result<()> {
        debug!("start");

        #[cfg(feature = "xdp-gnome-remote-desktop")]
        if self.rd_session.is_some() {
            return Err(fdo::Error::Failed(
                "This session must be started from the linked remote desktop session".to_owned(),
            ));
        }

        self.start().await;
        Ok(())
    }

    #[zbus(name = "Stop")]
    async fn stop_dbus(
        &mut self,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(signal_context)] ctxt: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        debug!("stop");

        #[cfg(feature = "xdp-gnome-remote-desktop")]
        if self.rd_session.is_some() {
            return Err(fdo::Error::Failed(
                "This session must be stopped from the linked remote desktop session".to_owned(),
            ));
        }

        self.stop(server, &ctxt, StopCastReason::DbusStop).await;
        Ok(())
    }

    /// Creates a [`Stream`] that records a monitor.
    ///
    /// Mutter replaces `connector=""` with the primary monitor and gnome-remote-desktop
    /// utilizes it.
    async fn record_monitor(
        &mut self,
        #[zbus(object_server)] server: &ObjectServer,
        connector: &str,
        properties: RecordMonitorProperties,
    ) -> fdo::Result<OwnedObjectPath> {
        debug!(connector, ?properties, "record_monitor");

        let connector = if connector.is_empty() {
            let new_connector = "DP-2";
            warn!("Replacing connector={connector:?} with connector={new_connector:?} for gnome-remote-desktop");
            new_connector
            // FIXME: HACK: remove this!!
        } else {
            connector
        };

        let output = {
            let ipc_outputs = self.ipc_outputs.lock().unwrap();
            ipc_outputs.values().find(|o| o.name == connector).cloned()
        };
        let Some(output) = output else {
            return Err(fdo::Error::Failed(format!(
                "No such monitor with connector/name: {connector}"
            )));
        };

        if output.logical.is_none() {
            return Err(fdo::Error::Failed("Monitor is disabled".to_owned()));
        }

        let target = StreamTarget::Output(output);
        let cursor_mode = properties.cursor_mode.unwrap_or_default();

        self.record_shared(target, cursor_mode, server).await
    }

    /// Creates a [`Stream`] that records a window.
    async fn record_window(
        &mut self,
        #[zbus(object_server)] server: &ObjectServer,
        properties: RecordWindowProperties,
    ) -> fdo::Result<OwnedObjectPath> {
        debug!(?properties, "record_window");

        let target = StreamTarget::Window {
            id: properties.window_id,
        };
        let cursor_mode = properties.cursor_mode.unwrap_or_default();

        self.record_shared(target, cursor_mode, server).await
    }

    /// Event that the session has closed.
    #[zbus(signal)]
    async fn closed(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;
}

impl Session {
    pub async fn start(&self) {
        for iface in &*self.streams.lock().await {
            iface.get().await.start(iface.signal_emitter().clone());
        }
    }

    /// Stops the session.
    async fn stop(
        &mut self,
        server: &ObjectServer,
        ctxt: &SignalEmitter<'_>,
        reason: StopCastReason,
    ) {
        if self.stop_inner(server, ctxt, reason).await {
            #[cfg(feature = "xdp-gnome-remote-desktop")]
            if let Some((iface, server)) = &self.rd_session {
                iface
                    .get_mut()
                    .await
                    .stop(server, iface.signal_emitter())
                    .await;
            }
        }
    }
    pub async fn stop_from_stopcast(&mut self, server: &ObjectServer, ctxt: &SignalEmitter<'_>) {
        self.stop(server, ctxt, StopCastReason::FromNiriStopCast)
            .await
    }

    /// Stops the session without trying to stop any associated remote desktop session.
    #[cfg(feature = "xdp-gnome-remote-desktop")]
    pub(super) async fn stop_no_remote_desktop(
        &mut self,
        server: &ObjectServer,
        ctxt: &SignalEmitter<'_>,
    ) {
        self.stop_inner(server, ctxt, StopCastReason::RemoteDesktopStopped)
            .await;
    }

    async fn stop_inner(
        &mut self,
        server: &ObjectServer,
        ctxt: &SignalEmitter<'_>,
        reason: StopCastReason,
    ) -> bool {
        if self.stopped.swap(true, Ordering::SeqCst) {
            // Already stopped.
            return false;
        }

        // Remove reference to the remote desktop interface so it can be dropped
        self.rd_session = None;

        Session::closed(ctxt).await.unwrap();

        if !self.sent_stop_cast.swap(true, Ordering::SeqCst) {
            if let Err(err) = self.to_niri.send(ScreenCastToNiri::StopCast {
                session_id: self.id,
                reason,
            }) {
                warn!("error sending StopCast to niri: {err:?}");
            }
        }

        let streams = mem::take(&mut *self.streams.lock().await);
        for iface in streams.iter() {
            server
                .remove::<Stream, _>(iface.signal_emitter().path())
                .await
                .unwrap();
        }

        server.remove::<Session, _>(ctxt.path()).await.unwrap();

        true
    }

    async fn record_shared(
        &mut self,
        target: StreamTarget,
        cursor_mode: CursorMode,
        server: &ObjectServer,
    ) -> fdo::Result<OwnedObjectPath> {
        let stream_id = CastStreamId::next();
        let path = format!("/org/gnome/Mutter/ScreenCast/Stream/u{}", stream_id.get());
        let path = OwnedObjectPath::try_from(path).unwrap();

        let stream = Stream {
            id: stream_id,
            session_id: self.id,
            target,
            cursor_mode,
            was_started: Arc::new(AtomicBool::new(false)),
            to_niri: self.to_niri.clone(),
            #[cfg(feature = "xdp-gnome-remote-desktop")]
            has_remote_desktop_session: self.rd_session.is_some(),
        };

        match server.at(&path, stream).await {
            Ok(true) => {
                let iface = server.interface(&path).await.unwrap();
                self.streams.lock().await.push(iface);
            }
            Ok(false) => return Err(fdo::Error::Failed("stream path already exists".to_owned())),
            Err(err) => {
                return Err(fdo::Error::Failed(format!(
                    "error creating stream object: {err:?}"
                )))
            }
        }

        Ok(path)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.sent_stop_cast.swap(true, Ordering::SeqCst) {
            let _ = self.to_niri.send(ScreenCastToNiri::StopCast {
                session_id: self.id,
                reason: StopCastReason::SessionDropped,
            });
        }
    }
}

// == STREAM ==

pub struct Stream {
    id: CastStreamId,
    session_id: CastSessionId,
    target: StreamTarget,
    cursor_mode: CursorMode,
    was_started: Arc<AtomicBool>,
    to_niri: calloop::channel::Sender<ScreenCastToNiri>,
    #[cfg(feature = "xdp-gnome-remote-desktop")]
    has_remote_desktop_session: bool,
}

#[derive(Clone)]
enum StreamTarget {
    // FIXME: update on scale changes and whatnot.
    Output(niri_ipc::Output),
    Window { id: u64 },
}

impl StreamTarget {
    fn make_id(&self) -> StreamTargetId {
        match self {
            StreamTarget::Output(output) => StreamTargetId::Output {
                name: output.name.clone(),
            },
            StreamTarget::Window { id } => StreamTargetId::Window { id: *id },
        }
    }
}

#[derive(Debug, Clone)]
pub enum StreamTargetId {
    Output { name: String },
    Window { id: u64 },
}

#[derive(Debug, SerializeDict, Type, Value)]
#[zvariant(signature = "dict")]
pub struct StreamParameters {
    /// Position of the stream in logical coordinates.
    pub position: (i32, i32),
    /// Size of the stream in logical coordinates.
    pub size: (i32, i32),
    /// Unique identifier used to map the stream to a corresponding region on an EI
    /// absolute device (remote desktop).
    ///
    /// Currently output names (like `eDP-1`) are used.
    #[zvariant(rename = "mapping-id")]
    pub mapping_id: Option<String>,
}

#[interface(name = "org.gnome.Mutter.ScreenCast.Stream")]
impl Stream {
    #[zbus(signal)]
    pub async fn pipe_wire_stream_added(ctxt: &SignalEmitter<'_>, node_id: u32)
        -> zbus::Result<()>;

    #[zbus(property)]
    pub(crate) fn parameters(&self) -> StreamParameters {
        match &self.target {
            StreamTarget::Output(output) => {
                let logical = output.logical.as_ref().unwrap();
                StreamParameters {
                    position: (logical.x, logical.y),
                    size: (logical.width as i32, logical.height as i32),
                    #[cfg(feature = "xdp-gnome-remote-desktop")]
                    mapping_id: self.has_remote_desktop_session.then(|| output.name.clone()),
                    #[cfg(not(feature = "xdp-gnome-remote-desktop"))]
                    mapping_id: None,
                }
            }
            StreamTarget::Window { .. } => {
                // Does any consumer need this?
                StreamParameters {
                    position: (0, 0),
                    size: (1, 1),
                    // TODO: does anyone need to remote-desktop to
                    // a specific window?
                    mapping_id: None,
                }
            }
        }
    }

    /// Starts this stream of an already started session.
    #[zbus(name = "Start")]
    async fn start_dbus(&self, #[zbus(signal_context)] ctxt: SignalEmitter<'_>) -> fdo::Result<()> {
        debug!("start");

        self.start(ctxt.to_owned());
        Ok(())
    }
}

impl Stream {
    /// Starts this stream.
    fn start(&self, ctxt: SignalEmitter<'static>) {
        if self.was_started.load(Ordering::SeqCst) {
            return;
        }
        self.was_started.store(true, Ordering::SeqCst);

        let msg = ScreenCastToNiri::StartStream {
            session_id: self.session_id,
            stream_id: self.id,
            target: self.target.make_id(),
            cursor_mode: self.cursor_mode,
            signal_ctx: ctxt,
        };

        if let Err(err) = self.to_niri.send(msg) {
            warn!("error sending StartStream to niri: {err:?}");
        }
    }
}
