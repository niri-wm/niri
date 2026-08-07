// allow: SIZE_OK — Mutter ScreenCast's zbus object tree is kept together here; splitting the
// existing interface file would be unrelated churn for the RemoteDesktop linkage patch.
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use zbus::fdo::RequestNameFlags;
use zbus::object_server::{InterfaceRef, SignalEmitter};
use zbus::zvariant::{DeserializeDict, OwnedObjectPath, SerializeDict, Type, Value};
use zbus::{fdo, interface, ObjectServer};

use super::mutter_remote_desktop::{
    RemoteDesktopOutputStream, RemoteDesktopSessionRegistry, RemoteDesktopStream,
};
use super::Start;
use crate::backend::IpcOutputMap;
use crate::utils::{CastSessionId, CastStreamId};

#[derive(Clone)]
pub struct ScreenCast {
    ipc_outputs: Arc<Mutex<IpcOutputMap>>,
    to_niri: calloop::channel::Sender<ScreenCastToNiri>,
    remote_desktop_sessions: RemoteDesktopSessionRegistry,
    #[allow(clippy::type_complexity)]
    sessions: Arc<Mutex<Vec<(Session, InterfaceRef<Session>)>>>,
}

#[derive(Clone)]
pub struct Session {
    id: CastSessionId,
    ipc_outputs: Arc<Mutex<IpcOutputMap>>,
    to_niri: calloop::channel::Sender<ScreenCastToNiri>,
    remote_desktop_session_id: Option<String>,
    remote_desktop_sessions: RemoteDesktopSessionRegistry,
    #[allow(clippy::type_complexity)]
    streams: Arc<Mutex<Vec<(Stream, InterfaceRef<Stream>)>>>,
    stopped: Arc<AtomicBool>,
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

#[derive(Clone)]
pub struct Stream {
    id: CastStreamId,
    session_id: CastSessionId,
    target: StreamTarget,
    cursor_mode: CursorMode,
    was_started: Arc<AtomicBool>,
    to_niri: calloop::channel::Sender<ScreenCastToNiri>,
}

#[derive(Clone)]
enum StreamTarget {
    // FIXME: update on scale changes and whatnot.
    Output(niri_ipc::Output),
    Window { id: u64 },
}

#[derive(Debug, Default, DeserializeDict, Type)]
#[zvariant(signature = "dict")]
struct CreateSessionProperties {
    #[zvariant(rename = "remote-desktop-session-id")]
    remote_desktop_session_id: Option<String>,
    #[zvariant(rename = "disable-animations")]
    _disable_animations: Option<bool>,
}

#[derive(Debug, Clone)]
pub enum StreamTargetId {
    Output { name: String },
    Window { id: u64 },
}

#[derive(Debug, SerializeDict, Type, Value)]
#[zvariant(signature = "dict")]
struct StreamParameters {
    /// Position of the stream in logical coordinates.
    position: (i32, i32),
    /// Size of the stream in logical coordinates.
    size: (i32, i32),
}

pub enum ScreenCastToNiri {
    StartCast {
        session_id: CastSessionId,
        stream_id: CastStreamId,
        target: StreamTargetId,
        cursor_mode: CursorMode,
        signal_ctx: SignalEmitter<'static>,
    },
    StopCast {
        session_id: CastSessionId,
    },
}

#[interface(name = "org.gnome.Mutter.ScreenCast")]
impl ScreenCast {
    async fn create_session(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        properties: CreateSessionProperties,
    ) -> fdo::Result<OwnedObjectPath> {
        let session_id = CastSessionId::next();
        let path = format!("/org/gnome/Mutter/ScreenCast/Session/u{}", session_id.get());
        let path = object_path(path)?;
        let remote_desktop_session_id = properties.remote_desktop_session_id;

        if let Some(remote_desktop_session_id) = &remote_desktop_session_id {
            self.remote_desktop_sessions
                .associate_screen_cast(remote_desktop_session_id, session_id)?;
        }

        let session = Session::new(
            session_id,
            self.ipc_outputs.clone(),
            self.to_niri.clone(),
            remote_desktop_session_id.clone(),
            self.remote_desktop_sessions.clone(),
        );
        match server.at(&path, session.clone()).await {
            Ok(true) => {
                let iface = match server.interface(&path).await {
                    Ok(iface) => iface,
                    Err(err) => {
                        self.remote_desktop_sessions.clear_screen_cast(session_id)?;
                        return Err(fdo::Error::Failed(format!(
                            "error retrieving session interface: {err:?}"
                        )));
                    }
                };
                let mut sessions = match self.sessions.lock() {
                    Ok(sessions) => sessions,
                    Err(_) => {
                        self.remote_desktop_sessions.clear_screen_cast(session_id)?;
                        return Err(fdo::Error::Failed(
                            "screen cast session list is poisoned".to_owned(),
                        ));
                    }
                };
                sessions.push((session, iface));
            }
            Ok(false) => {
                self.remote_desktop_sessions.clear_screen_cast(session_id)?;
                return Err(fdo::Error::Failed("session path already exists".to_owned()));
            }
            Err(err) => {
                self.remote_desktop_sessions.clear_screen_cast(session_id)?;
                return Err(fdo::Error::Failed(format!(
                    "error creating session object: {err:?}"
                )));
            }
        }

        Ok(path)
    }

    #[zbus(property)]
    async fn version(&self) -> i32 {
        4
    }
}

#[interface(name = "org.gnome.Mutter.ScreenCast.Session")]
impl Session {
    async fn start(&self) {
        debug!("start");

        let streams = match self.streams.lock() {
            Ok(streams) => streams,
            Err(_) => {
                warn!("screen cast stream list is poisoned");
                return;
            }
        };

        for (stream, iface) in &*streams {
            stream.start(iface.signal_emitter().clone());
        }
    }

    pub async fn stop(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(signal_context)] ctxt: SignalEmitter<'_>,
    ) {
        debug!("stop");

        if self.stopped.swap(true, Ordering::SeqCst) {
            // Already stopped.
            return;
        }

        if let Err(err) = self.remote_desktop_sessions.clear_screen_cast(self.id) {
            warn!("error clearing remote desktop screen cast association: {err:?}");
        }

        if let Err(err) = Session::closed(&ctxt).await {
            warn!("error emitting ScreenCast Closed signal: {err:?}");
        }

        if let Err(err) = self.to_niri.send(ScreenCastToNiri::StopCast {
            session_id: self.id,
        }) {
            warn!("error sending StopCast to niri: {err:?}");
        }

        let streams = match self.streams.lock() {
            Ok(mut streams) => mem::take(&mut *streams),
            Err(_) => {
                warn!("screen cast stream list is poisoned");
                Vec::new()
            }
        };

        for (_, iface) in streams.iter() {
            if let Err(err) = server
                .remove::<Stream, _>(iface.signal_emitter().path())
                .await
            {
                warn!("error removing stream object: {err:?}");
            }
        }

        if let Err(err) = server.remove::<Session, _>(ctxt.path()).await {
            warn!("error removing screen cast session object: {err:?}");
        }
    }

    async fn record_monitor(
        &mut self,
        #[zbus(object_server)] server: &ObjectServer,
        connector: &str,
        properties: RecordMonitorProperties,
    ) -> fdo::Result<OwnedObjectPath> {
        debug!(connector, ?properties, "record_monitor");

        let output = {
            let ipc_outputs = self
                .ipc_outputs
                .lock()
                .map_err(|_| fdo::Error::Failed("screen cast output map is poisoned".to_owned()))?;
            ipc_outputs.values().find(|o| o.name == connector).cloned()
        };
        let Some(output) = output else {
            return Err(fdo::Error::Failed("no such monitor".to_owned()));
        };

        let stream_id = CastStreamId::next();
        let remote_stream = remote_desktop_output_stream(stream_id, &output)?;
        let path = format!("/org/gnome/Mutter/ScreenCast/Stream/u{}", stream_id.get());
        let path = object_path(path)?;

        let cursor_mode = properties.cursor_mode.unwrap_or_default();

        let target = StreamTarget::Output(output);
        let stream = Stream::new(
            stream_id,
            self.id,
            target,
            cursor_mode,
            self.to_niri.clone(),
        );
        match server.at(&path, stream.clone()).await {
            Ok(true) => {
                let iface = server.interface(&path).await.map_err(|err| {
                    fdo::Error::Failed(format!("error retrieving stream interface: {err:?}"))
                })?;
                self.register_remote_desktop_stream(&path, remote_stream)?;
                self.streams
                    .lock()
                    .map_err(|_| {
                        fdo::Error::Failed("screen cast stream list is poisoned".to_owned())
                    })?
                    .push((stream, iface));
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

    async fn record_window(
        &mut self,
        #[zbus(object_server)] server: &ObjectServer,
        properties: RecordWindowProperties,
    ) -> fdo::Result<OwnedObjectPath> {
        debug!(?properties, "record_window");

        let stream_id = CastStreamId::next();
        let path = format!("/org/gnome/Mutter/ScreenCast/Stream/u{}", stream_id.get());
        let path = object_path(path)?;

        let cursor_mode = properties.cursor_mode.unwrap_or_default();

        let target = StreamTarget::Window {
            id: properties.window_id,
        };
        let stream = Stream::new(
            stream_id,
            self.id,
            target,
            cursor_mode,
            self.to_niri.clone(),
        );
        match server.at(&path, stream.clone()).await {
            Ok(true) => {
                let iface = server.interface(&path).await.map_err(|err| {
                    fdo::Error::Failed(format!("error retrieving stream interface: {err:?}"))
                })?;
                self.register_remote_desktop_stream(&path, RemoteDesktopStream::window(stream_id))?;
                self.streams
                    .lock()
                    .map_err(|_| {
                        fdo::Error::Failed("screen cast stream list is poisoned".to_owned())
                    })?
                    .push((stream, iface));
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

    #[zbus(signal)]
    async fn closed(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[interface(name = "org.gnome.Mutter.ScreenCast.Stream")]
impl Stream {
    #[zbus(signal)]
    pub async fn pipe_wire_stream_added(ctxt: &SignalEmitter<'_>, node_id: u32)
        -> zbus::Result<()>;

    #[zbus(property)]
    async fn parameters(&self) -> fdo::Result<StreamParameters> {
        match &self.target {
            StreamTarget::Output(output) => output_stream_parameters(output),
            StreamTarget::Window { .. } => Ok(
                // Does any consumer need this?
                StreamParameters {
                    position: (0, 0),
                    size: (1, 1),
                },
            ),
        }
    }
}

impl ScreenCast {
    pub fn new(
        ipc_outputs: Arc<Mutex<IpcOutputMap>>,
        to_niri: calloop::channel::Sender<ScreenCastToNiri>,
        remote_desktop_sessions: RemoteDesktopSessionRegistry,
    ) -> Self {
        Self {
            ipc_outputs,
            to_niri,
            remote_desktop_sessions,
            sessions: Arc::new(Mutex::new(vec![])),
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

impl Session {
    pub fn new(
        id: CastSessionId,
        ipc_outputs: Arc<Mutex<IpcOutputMap>>,
        to_niri: calloop::channel::Sender<ScreenCastToNiri>,
        remote_desktop_session_id: Option<String>,
        remote_desktop_sessions: RemoteDesktopSessionRegistry,
    ) -> Self {
        Self {
            id,
            ipc_outputs,
            remote_desktop_session_id,
            remote_desktop_sessions,
            streams: Arc::new(Mutex::new(vec![])),
            to_niri,
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    fn register_remote_desktop_stream(
        &self,
        path: &OwnedObjectPath,
        stream: RemoteDesktopStream,
    ) -> fdo::Result<()> {
        if self.remote_desktop_session_id.is_none() {
            return Ok(());
        }

        self.remote_desktop_sessions
            .register_stream(self.id, path.as_str().to_owned(), stream)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.remote_desktop_sessions.clear_screen_cast(self.id);
        let _ = self.to_niri.send(ScreenCastToNiri::StopCast {
            session_id: self.id,
        });
    }
}

impl Stream {
    fn new(
        id: CastStreamId,
        session_id: CastSessionId,
        target: StreamTarget,
        cursor_mode: CursorMode,
        to_niri: calloop::channel::Sender<ScreenCastToNiri>,
    ) -> Self {
        Self {
            id,
            session_id,
            target,
            cursor_mode,
            was_started: Arc::new(AtomicBool::new(false)),
            to_niri,
        }
    }

    fn start(&self, ctxt: SignalEmitter<'static>) {
        if self.was_started.load(Ordering::SeqCst) {
            return;
        }

        let msg = ScreenCastToNiri::StartCast {
            session_id: self.session_id,
            stream_id: self.id,
            target: self.target.make_id(),
            cursor_mode: self.cursor_mode,
            signal_ctx: ctxt,
        };

        if let Err(err) = self.to_niri.send(msg) {
            warn!("error sending StartCast to niri: {err:?}");
        }
    }
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

fn object_path(path: String) -> fdo::Result<OwnedObjectPath> {
    OwnedObjectPath::try_from(path)
        .map_err(|err| fdo::Error::Failed(format!("invalid object path: {err}")))
}

fn remote_desktop_output_stream(
    stream_id: CastStreamId,
    output: &niri_ipc::Output,
) -> fdo::Result<RemoteDesktopStream> {
    let logical = output
        .logical
        .as_ref()
        .ok_or_else(|| fdo::Error::Failed("monitor is disabled".to_owned()))?;
    let (width, height) = logical_size(logical)?;
    Ok(RemoteDesktopStream::output(
        stream_id,
        RemoteDesktopOutputStream {
            name: output.name.clone(),
            x: logical.x,
            y: logical.y,
            width,
            height,
        },
    ))
}

fn output_stream_parameters(output: &niri_ipc::Output) -> fdo::Result<StreamParameters> {
    let logical = output
        .logical
        .as_ref()
        .ok_or_else(|| fdo::Error::Failed("monitor is disabled".to_owned()))?;
    Ok(StreamParameters {
        position: (logical.x, logical.y),
        size: logical_size(logical)?,
    })
}

fn logical_size(logical: &niri_ipc::LogicalOutput) -> fdo::Result<(i32, i32)> {
    if logical.width == 0 || logical.height == 0 {
        return Err(fdo::Error::Failed(
            "logical output size must be non-zero".to_owned(),
        ));
    }

    let width = i32::try_from(logical.width)
        .map_err(|_| fdo::Error::Failed("logical output width is too large".to_owned()))?;
    let height = i32::try_from(logical.height)
        .map_err(|_| fdo::Error::Failed("logical output height is too large".to_owned()))?;
    Ok((width, height))
}
