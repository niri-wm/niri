use std::os::fd::OwnedFd as StdOwnedFd;
use std::sync::atomic::Ordering;

use zbus::fdo;
use zbus::object_server::SignalEmitter;

use super::clipboard::{fd_pair, RemoteDesktopClipboardOwner};
use super::clipboard_signals::{copy_fd, emit_owner_changed, emit_selection_transfer};
use super::registry::RemoteDesktopSessionRegistry;

impl RemoteDesktopSessionRegistry {
    pub fn set_clipboard_signal_emitter(
        &self,
        session_id: &str,
        signal_emitter: SignalEmitter<'static>,
    ) -> fdo::Result<()> {
        self.with_session(session_id, |session| {
            session.clipboard.signal_emitter = Some(signal_emitter);
            Ok(())
        })
    }

    pub fn enable_clipboard(&self, session_id: &str) -> Result<(), String> {
        self.with_session(session_id, |session| {
            if session.clipboard.enabled {
                return Err(fdo::Error::Failed(
                    "clipboard is already enabled".to_owned(),
                ));
            }

            session.clipboard.enabled = true;
            Ok(())
        })
        .map_err(|err| format!("{err:?}"))
    }

    pub fn disable_clipboard(&self, session_id: &str) -> Result<bool, String> {
        self.with_session(session_id, |session| {
            session.clipboard.enabled = false;
            session.clipboard.pending_writes.clear();
            Ok(())
        })
        .map_err(|err| format!("{err:?}"))?;

        let mut owner = self
            .clipboard_owner
            .lock()
            .map_err(|_| "remote desktop clipboard owner is poisoned".to_owned())?;
        let was_owner = owner.session_id.as_deref() == Some(session_id);
        if was_owner {
            *owner = RemoteDesktopClipboardOwner::default();
        }

        Ok(was_owner)
    }

    pub fn clear_clipboard_owner_if_session(&self, session_id: &str) -> Result<bool, String> {
        let mut owner = self
            .clipboard_owner
            .lock()
            .map_err(|_| "remote desktop clipboard owner is poisoned".to_owned())?;
        if owner.session_id.as_deref() != Some(session_id) {
            return Ok(false);
        }

        *owner = RemoteDesktopClipboardOwner::default();
        Ok(true)
    }

    pub fn ensure_clipboard_enabled(&self, session_id: &str) -> Result<(), String> {
        self.with_session(session_id, |session| {
            if !session.clipboard.enabled {
                return Err(fdo::Error::Failed("clipboard is not enabled".to_owned()));
            }

            Ok(())
        })
        .map_err(|err| format!("{err:?}"))
    }

    pub fn selection_write(&self, session_id: &str, serial: u32) -> fdo::Result<StdOwnedFd> {
        self.with_session(session_id, |session| {
            if !session.clipboard.enabled {
                return Err(fdo::Error::Failed("clipboard is not enabled".to_owned()));
            }

            let Some(fd) = session.clipboard.pending_writes.remove(&serial) else {
                return Err(fdo::Error::InvalidArgs(format!(
                    "unknown clipboard transfer serial: {serial}"
                )));
            };

            Ok(fd)
        })
    }

    pub fn selection_write_done(&self, session_id: &str, serial: u32) -> fdo::Result<()> {
        self.with_session(session_id, |session| {
            if !session.clipboard.enabled {
                return Err(fdo::Error::Failed("clipboard is not enabled".to_owned()));
            }

            session.clipboard.pending_writes.remove(&serial);
            Ok(())
        })
    }

    pub fn request_clipboard_transfer(
        &self,
        session_id: &str,
        mime_type: String,
        target_fd: StdOwnedFd,
    ) -> fdo::Result<()> {
        let serial = self.next_clipboard_serial.fetch_add(1, Ordering::SeqCst);
        let (read_fd, write_fd) = fd_pair().map_err(fdo::Error::Failed)?;
        let signal_emitter = self.with_session(session_id, |session| {
            if !session.clipboard.enabled {
                return Err(fdo::Error::Failed("clipboard is not enabled".to_owned()));
            }

            let Some(signal_emitter) = session.clipboard.signal_emitter.clone() else {
                return Err(fdo::Error::Failed(
                    "clipboard signal emitter is not registered".to_owned(),
                ));
            };

            session.clipboard.pending_writes.insert(serial, write_fd);
            Ok(signal_emitter)
        })?;

        copy_fd(read_fd, target_fd);
        emit_selection_transfer(signal_emitter, mime_type, serial);
        Ok(())
    }

    pub fn notify_clipboard_owner_changed(
        &self,
        owner_session_id: Option<String>,
        mime_types: Vec<String>,
    ) {
        if let Ok(mut owner) = self.clipboard_owner.lock() {
            owner.session_id = owner_session_id.clone();
            owner.mime_types = mime_types.clone();
        } else {
            warn!("remote desktop clipboard owner is poisoned");
            return;
        }

        let emissions =
            match self.clipboard_owner_emissions(owner_session_id.as_deref(), mime_types) {
                Ok(emissions) => emissions,
                Err(err) => {
                    warn!("error collecting remote desktop clipboard notifications: {err:?}");
                    return;
                }
            };

        for (signal_emitter, session_is_owner, mime_types) in emissions {
            emit_owner_changed(signal_emitter, session_is_owner, mime_types);
        }
    }

    pub fn notify_session_current_clipboard_owner(&self, session_id: &str) -> fdo::Result<()> {
        let owner = self.clipboard_owner.lock().map_err(|_| {
            fdo::Error::Failed("remote desktop clipboard owner is poisoned".to_owned())
        })?;
        let session_is_owner = owner.session_id.as_deref() == Some(session_id);
        let mime_types = owner.mime_types.clone();
        drop(owner);

        let signal_emitter = self.with_session(session_id, |session| {
            if !session.clipboard.enabled {
                return Err(fdo::Error::Failed("clipboard is not enabled".to_owned()));
            }

            session.clipboard.signal_emitter.clone().ok_or_else(|| {
                fdo::Error::Failed("clipboard signal emitter is not registered".to_owned())
            })
        })?;

        emit_owner_changed(signal_emitter, session_is_owner, mime_types);
        Ok(())
    }

    fn clipboard_owner_emissions(
        &self,
        owner_session_id: Option<&str>,
        mime_types: Vec<String>,
    ) -> fdo::Result<Vec<(SignalEmitter<'static>, bool, Vec<String>)>> {
        self.with_sessions(|sessions| {
            Ok(sessions
                .iter()
                .filter_map(|(session_id, session)| {
                    if !session.clipboard.enabled {
                        return None;
                    }
                    let signal_emitter = session.clipboard.signal_emitter.clone()?;
                    let session_is_owner = owner_session_id == Some(session_id.as_str());
                    Some((signal_emitter, session_is_owner, mime_types.clone()))
                })
                .collect())
        })
    }
}
