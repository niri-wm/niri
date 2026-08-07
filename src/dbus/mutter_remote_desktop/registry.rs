use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use zbus::fdo;

use super::clipboard::{RemoteDesktopClipboardOwner, RemoteDesktopClipboardSession};
use crate::utils::{CastSessionId, CastStreamId};

#[derive(Clone, Debug)]
pub struct RemoteDesktopSessionRegistry {
    pub(super) sessions: Arc<Mutex<HashMap<String, RemoteDesktopSession>>>,
    pub(super) clipboard_owner: Arc<Mutex<RemoteDesktopClipboardOwner>>,
    pub(super) next_clipboard_serial: Arc<AtomicU32>,
}

#[derive(Clone, Debug)]
pub struct RemoteDesktopStream {
    pub stream_id: CastStreamId,
    target: RemoteDesktopStreamTarget,
}

#[derive(Clone, Debug)]
pub struct RemoteDesktopOutputStream {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug)]
pub enum RemoteDesktopStreamTarget {
    Output {
        name: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    Window,
}

#[derive(Clone, Debug)]
pub struct RemoteDesktopStreamPosition {
    pub output_name: String,
    pub x: f64,
    pub y: f64,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteDesktopOutputRegion {
    pub mapping_id: u64,
    pub output_name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Default)]
pub(super) struct RemoteDesktopSession {
    pub(super) started: bool,
    pub(super) screen_cast_session: Option<CastSessionId>,
    pub(super) streams: HashMap<String, RemoteDesktopStream>,
    pub(super) clipboard: RemoteDesktopClipboardSession,
}

impl Default for RemoteDesktopSessionRegistry {
    fn default() -> Self {
        Self {
            sessions: Arc::default(),
            clipboard_owner: Arc::default(),
            next_clipboard_serial: Arc::new(AtomicU32::new(1)),
        }
    }
}

impl RemoteDesktopSessionRegistry {
    pub fn insert_session(&self, session_id: String) -> fdo::Result<()> {
        self.with_sessions(|sessions| {
            if sessions.contains_key(&session_id) {
                return Err(fdo::Error::Failed(format!(
                    "remote desktop session already exists: {session_id}"
                )));
            }

            sessions.insert(session_id, RemoteDesktopSession::default());
            Ok(())
        })
    }

    pub fn remove_session(&self, session_id: &str) -> fdo::Result<()> {
        self.with_sessions(|sessions| {
            sessions.remove(session_id);
            Ok(())
        })
    }

    pub fn mark_started(&self, session_id: &str) -> fdo::Result<()> {
        self.with_session(session_id, |session| {
            session.started = true;
            Ok(())
        })
    }

    pub fn associate_screen_cast(
        &self,
        session_id: &str,
        screen_cast_session: CastSessionId,
    ) -> fdo::Result<()> {
        self.with_session(session_id, |session| {
            if session.started {
                return Err(fdo::Error::Failed(format!(
                    "remote desktop session is already started: {session_id}"
                )));
            }

            if session.screen_cast_session.is_some() {
                return Err(fdo::Error::Failed(format!(
                    "remote desktop session already has a screen cast: {session_id}"
                )));
            }

            session.screen_cast_session = Some(screen_cast_session);
            Ok(())
        })
    }

    pub fn clear_screen_cast(&self, screen_cast_session: CastSessionId) -> fdo::Result<()> {
        self.with_sessions(|sessions| {
            for session in sessions.values_mut() {
                if session.screen_cast_session == Some(screen_cast_session) {
                    session.screen_cast_session = None;
                    session.streams.clear();
                }
            }

            Ok(())
        })
    }

    pub fn register_stream(
        &self,
        screen_cast_session: CastSessionId,
        path: String,
        stream: RemoteDesktopStream,
    ) -> fdo::Result<()> {
        self.with_sessions(|sessions| {
            let Some(session) = sessions
                .values_mut()
                .find(|session| session.screen_cast_session == Some(screen_cast_session))
            else {
                return Err(fdo::Error::Failed(
                    "screen cast session is not attached to a remote desktop session".to_owned(),
                ));
            };

            session.streams.insert(path, stream);
            Ok(())
        })
    }

    pub fn stream_position(
        &self,
        stream_path: &str,
        x: f64,
        y: f64,
    ) -> fdo::Result<RemoteDesktopStreamPosition> {
        if !x.is_finite() || !y.is_finite() {
            return Err(fdo::Error::InvalidArgs(
                "remote desktop coordinates must be finite".to_owned(),
            ));
        }

        self.with_sessions(|sessions| {
            let Some(stream) = sessions
                .values()
                .find_map(|session| session.streams.get(stream_path))
            else {
                return Err(fdo::Error::Failed(format!(
                    "remote desktop stream not found: {stream_path}"
                )));
            };

            stream.position(x, y)
        })
    }

    pub fn output_regions(&self, session_id: &str) -> fdo::Result<Vec<RemoteDesktopOutputRegion>> {
        self.with_session(session_id, |session| {
            let mut regions = session
                .streams
                .values()
                .filter_map(RemoteDesktopStream::output_region)
                .collect::<Vec<_>>();
            regions.sort_by_key(|region| region.mapping_id);
            Ok(regions)
        })
    }

    pub(super) fn with_sessions<T>(
        &self,
        f: impl FnOnce(&mut HashMap<String, RemoteDesktopSession>) -> fdo::Result<T>,
    ) -> fdo::Result<T> {
        let mut sessions = self.sessions.lock().map_err(|_| {
            fdo::Error::Failed("remote desktop session registry is poisoned".to_owned())
        })?;
        f(&mut sessions)
    }

    pub(super) fn with_session<T>(
        &self,
        session_id: &str,
        f: impl FnOnce(&mut RemoteDesktopSession) -> fdo::Result<T>,
    ) -> fdo::Result<T> {
        self.with_sessions(|sessions| {
            let Some(session) = sessions.get_mut(session_id) else {
                return Err(fdo::Error::Failed(format!(
                    "remote desktop session not found: {session_id}"
                )));
            };

            f(session)
        })
    }
}

impl RemoteDesktopStream {
    pub fn output(stream_id: CastStreamId, output: RemoteDesktopOutputStream) -> Self {
        Self {
            stream_id,
            target: RemoteDesktopStreamTarget::Output {
                name: output.name,
                x: output.x,
                y: output.y,
                width: output.width,
                height: output.height,
            },
        }
    }

    pub fn window(stream_id: CastStreamId) -> Self {
        Self {
            stream_id,
            target: RemoteDesktopStreamTarget::Window,
        }
    }

    fn position(&self, x: f64, y: f64) -> fdo::Result<RemoteDesktopStreamPosition> {
        match &self.target {
            RemoteDesktopStreamTarget::Output {
                name,
                x: _,
                y: _,
                width,
                height,
            } => Ok(RemoteDesktopStreamPosition {
                output_name: name.clone(),
                x,
                y,
                width: *width,
                height: *height,
            }),
            RemoteDesktopStreamTarget::Window => Err(fdo::Error::NotSupported(
                "remote desktop absolute input for window streams is not implemented".to_owned(),
            )),
        }
    }

    fn output_region(&self) -> Option<RemoteDesktopOutputRegion> {
        match &self.target {
            RemoteDesktopStreamTarget::Output {
                name,
                x,
                y,
                width,
                height,
            } => Some(RemoteDesktopOutputRegion {
                mapping_id: self.stream_id.get(),
                output_name: name.clone(),
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            }),
            RemoteDesktopStreamTarget::Window => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_rejects_screen_cast_association_after_session_start() {
        let registry = RemoteDesktopSessionRegistry::default();
        let session_id = "started-session".to_owned();

        registry.insert_session(session_id.clone()).unwrap();
        registry.mark_started(&session_id).unwrap();
        let err = registry
            .associate_screen_cast(&session_id, CastSessionId::next())
            .unwrap_err();

        assert!(format!("{err:?}").contains("already started"));
    }

    #[test]
    fn registry_resolves_output_stream_positions() {
        let registry = RemoteDesktopSessionRegistry::default();
        let session_id = "stream-session".to_owned();
        let screen_cast_session = CastSessionId::next();
        let stream_id = CastStreamId::next();

        registry.insert_session(session_id.clone()).unwrap();
        registry
            .associate_screen_cast(&session_id, screen_cast_session)
            .unwrap();
        registry
            .register_stream(
                screen_cast_session,
                "/org/gnome/Mutter/ScreenCast/Stream/u999".to_owned(),
                RemoteDesktopStream::output(
                    stream_id,
                    RemoteDesktopOutputStream {
                        name: "HDMI-A-1".to_owned(),
                        x: 0,
                        y: 0,
                        width: 1920,
                        height: 1080,
                    },
                ),
            )
            .unwrap();

        let position = registry
            .stream_position("/org/gnome/Mutter/ScreenCast/Stream/u999", 320.0, 240.0)
            .unwrap();

        assert_eq!(position.output_name, "HDMI-A-1");
        assert_eq!(position.x, 320.0);
        assert_eq!(position.y, 240.0);
        assert_eq!(position.width, 1920);
        assert_eq!(position.height, 1080);
    }

    #[test]
    fn registry_lists_remote_desktop_output_regions() {
        let registry = RemoteDesktopSessionRegistry::default();
        let session_id = "stream-session".to_owned();
        let screen_cast_session = CastSessionId::next();
        let stream_id = CastStreamId::next();

        registry.insert_session(session_id.clone()).unwrap();
        registry
            .associate_screen_cast(&session_id, screen_cast_session)
            .unwrap();
        registry
            .register_stream(
                screen_cast_session,
                "/org/gnome/Mutter/ScreenCast/Stream/u999".to_owned(),
                RemoteDesktopStream::output(
                    stream_id,
                    RemoteDesktopOutputStream {
                        name: "HDMI-A-1".to_owned(),
                        x: -1920,
                        y: 120,
                        width: 1920,
                        height: 1080,
                    },
                ),
            )
            .unwrap();

        let regions = registry.output_regions(&session_id).unwrap();

        assert_eq!(
            regions,
            vec![RemoteDesktopOutputRegion {
                mapping_id: stream_id.get(),
                output_name: "HDMI-A-1".to_owned(),
                x: -1920,
                y: 120,
                width: 1920,
                height: 1080,
            }]
        );
    }
}
