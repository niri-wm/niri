use std::collections::HashMap;
use std::os::fd::IntoRawFd;
use std::os::unix::io::FromRawFd;
use std::sync::mpsc;

use zbus::fdo::RequestNameFlags;
use zbus::zvariant::{OwnedObjectPath, Value};
use zbus::{fdo, interface};

use super::mutter_screen_cast::{CursorMode, ScreenCastToNiri, StreamTargetId};
use super::Start;
use crate::backend::IpcOutputMap;
use crate::utils::{CastSessionId, CastStreamId};

#[derive(Clone)]
pub struct PortalScreenCast {
    ipc_outputs: std::sync::Arc<std::sync::Mutex<IpcOutputMap>>,
    to_niri: calloop::channel::Sender<ScreenCastToNiri>,
}

#[interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl PortalScreenCast {
    async fn create_session(
        &self,
        _handle: OwnedObjectPath,
        _session_handle: OwnedObjectPath,
        _app_id: &str,
        _options: HashMap<&str, Value<'_>>,
    ) -> fdo::Result<(u32, HashMap<String, Value<'static>>)> {
        tracing::info!("portal: CreateSession");
        Ok((0, HashMap::new()))
    }

    async fn select_sources(
        &self,
        _request_handle: OwnedObjectPath,
        _session_handle: OwnedObjectPath,
        _app_id: &str,
        _options: HashMap<&str, Value<'_>>,
    ) -> fdo::Result<(u32, HashMap<String, Value<'static>>)> {
        let mut r: HashMap<String, Value<'static>> = HashMap::new();
        r.insert("available_source_types".into(), Value::U32(1));
        r.insert("available_cursor_modes".into(), Value::U32(7));
        Ok((0, r))
    }

    async fn start(
        &self,
        _request_handle: OwnedObjectPath,
        _session_handle: OwnedObjectPath,
        _app_id: &str,
        _parent_window: &str,
        _options: HashMap<&str, Value<'_>>,
    ) -> fdo::Result<(u32, HashMap<String, Value<'static>>)> {
        let output = {
            let outputs = self.ipc_outputs.lock().unwrap();
            outputs.values().find(|o| o.logical.is_some()).cloned()
        };
        let Some(output) = output else {
            return Err(fdo::Error::Failed("no output available".into()));
        };
        let logical = output.logical.as_ref().unwrap();

        let stream_id = CastStreamId::next();
        let (tx, rx) = mpsc::channel();

        self.to_niri.send(ScreenCastToNiri::StartCast {
            session_id: CastSessionId::next(),
            stream_id,
            target: StreamTargetId::Output {
                name: output.name.clone(),
            },
            cursor_mode: CursorMode::Embedded,
            signal_ctx: None,
            node_tx: Some(tx),
        })
        .map_err(|e| fdo::Error::Failed(format!("start cast: {e}")))?;

        let node_id = rx.recv().map_err(|_| fdo::Error::Failed("no pipewire node".into()))?;

        let mut stream_props: HashMap<String, Value<'static>> = HashMap::new();
        stream_props.insert("source_type".into(), Value::U32(1));
        stream_props.insert(
            "size".into(),
            Value::from((logical.width as i32, logical.height as i32)),
        );

        let mut results: HashMap<String, Value<'static>> = HashMap::new();
        results.insert("streams".into(), Value::from(vec![(node_id, stream_props)]));
        Ok((0, results))
    }

    async fn open_pipewire_remote(
        &self,
        _session_handle: OwnedObjectPath,
        _options: HashMap<&str, Value<'_>>,
    ) -> fdo::Result<zbus::zvariant::Fd<'static>> {
        let path = std::env::var("PIPEWIRE_REMOTE").unwrap_or_else(|_| {
            let dir = std::env::var("XDG_RUNTIME_DIR")
                .unwrap_or_else(|_| "/run/user/1000".into());
            format!("{dir}/pipewire-0")
        });
        let socket = std::fs::File::open(&path)
            .map_err(|e| fdo::Error::Failed(format!("open {path}: {e}")))?;
        let fd = socket.into_raw_fd();
        let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
        Ok(zbus::zvariant::Fd::Owned(owned))
    }

    async fn close(
        &self,
        _session_handle: OwnedObjectPath,
        _app_id: &str,
        _options: HashMap<&str, Value<'_>>,
    ) -> fdo::Result<()> {
        tracing::info!("portal: Close");
        Ok(())
    }

    #[zbus(property)]
    fn version(&self) -> u32 { 5 }
    #[zbus(property)]
    fn available_source_types(&self) -> u32 { 1 }
    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 { 7 }
}

impl Start for PortalScreenCast {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::Connection::session()?;
        conn.object_server()
            .at("/org/freedesktop/portal/desktop", self)?;
        conn.request_name_with_flags(
            "org.freedesktop.impl.portal.desktop.niri",
            RequestNameFlags::AllowReplacement
                | RequestNameFlags::ReplaceExisting
                | RequestNameFlags::DoNotQueue,
        )?;
        Ok(conn)
    }
}

impl PortalScreenCast {
    pub fn new(
        ipc_outputs: std::sync::Arc<std::sync::Mutex<IpcOutputMap>>,
        to_niri: calloop::channel::Sender<ScreenCastToNiri>,
    ) -> Self {
        Self { ipc_outputs, to_niri }
    }
}
