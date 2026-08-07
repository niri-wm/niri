use std::collections::HashMap;
use std::os::fd::OwnedFd as StdOwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use smithay::wayland::selection::data_device::{
    clear_data_device_selection, current_data_device_selection_userdata,
    request_data_device_client_selection, set_data_device_selection, SelectionRequestError,
};
use zbus::fdo;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::Value;

use super::registry::RemoteDesktopSessionRegistry;
use crate::handlers::{write_selection_bytes, SelectionData};
use crate::niri::State;

pub type RemoteDesktopClipboardReply = async_channel::Sender<Result<(), String>>;
pub type RemoteDesktopClipboardReadReply = async_channel::Sender<Result<StdOwnedFd, String>>;

#[derive(Debug, Default)]
pub(super) struct RemoteDesktopClipboardSession {
    pub(super) enabled: bool,
    pub(super) signal_emitter: Option<SignalEmitter<'static>>,
    pub(super) pending_writes: HashMap<u32, StdOwnedFd>,
}

#[derive(Debug, Default)]
pub(super) struct RemoteDesktopClipboardOwner {
    pub(super) session_id: Option<String>,
    pub(super) mime_types: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct RemoteDesktopClipboardSelection {
    session_id: String,
    registry: RemoteDesktopSessionRegistry,
    mime_types: Arc<[String]>,
}

impl RemoteDesktopClipboardSelection {
    pub fn new(
        session_id: String,
        registry: RemoteDesktopSessionRegistry,
        mime_types: Vec<String>,
    ) -> Self {
        Self {
            session_id,
            registry,
            mime_types: mime_types.into(),
        }
    }

    pub fn contains_mime_type(&self, mime_type: &str) -> bool {
        self.mime_types
            .iter()
            .any(|candidate| candidate == mime_type)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn request_transfer(&self, mime_type: String, fd: StdOwnedFd) -> Result<(), String> {
        if !self.contains_mime_type(&mime_type) {
            return Err(format!(
                "remote clipboard selection does not offer {mime_type}"
            ));
        }

        self.registry
            .request_clipboard_transfer(&self.session_id, mime_type, fd)
            .map_err(|err| format!("{err:?}"))
    }
}

pub fn parse_clipboard_mime_types(
    options: &HashMap<&str, Value<'_>>,
) -> fdo::Result<Option<Vec<String>>> {
    let Some(value) = options.get("mime-types") else {
        return Ok(None);
    };

    let value = value
        .try_clone()
        .map_err(|err| fdo::Error::Failed(format!("failed to clone mime-types: {err:?}")))?;
    let mime_types = Vec::<String>::try_from(
        value
            .try_clone()
            .map_err(|err| fdo::Error::Failed(format!("failed to clone mime-types: {err:?}")))?,
    )
    .or_else(|_| <(Vec<String>,)>::try_from(value).map(|tuple| tuple.0))
    .map_err(|_| fdo::Error::InvalidArgs("mime-types must be a string array".to_owned()))?;
    validate_mime_types(mime_types).map(Some)
}

pub fn enable_clipboard(
    state: &mut State,
    session_id: String,
    mime_types: Option<Vec<String>>,
) -> Result<(), String> {
    let registry = remote_desktop_registry(state)?;
    registry.enable_clipboard(&session_id)?;

    if let Some(mime_types) = mime_types {
        set_remote_clipboard_selection(state, registry, session_id, mime_types)
    } else {
        registry
            .notify_session_current_clipboard_owner(&session_id)
            .map_err(|err| format!("{err:?}"))
    }
}

pub fn disable_clipboard(state: &mut State, session_id: String) -> Result<(), String> {
    let registry = remote_desktop_registry(state)?;
    let was_owner = registry.disable_clipboard(&session_id)?;

    if was_owner {
        clear_data_device_selection(&state.niri.display_handle, &state.niri.seat);
        registry.notify_clipboard_owner_changed(None, Vec::new());
    }

    Ok(())
}

pub fn set_selection(
    state: &mut State,
    session_id: String,
    mime_types: Option<Vec<String>>,
) -> Result<(), String> {
    let registry = remote_desktop_registry(state)?;
    registry.ensure_clipboard_enabled(&session_id)?;

    if let Some(mime_types) = mime_types {
        set_remote_clipboard_selection(state, registry, session_id, mime_types)
    } else if registry.clear_clipboard_owner_if_session(&session_id)? {
        clear_data_device_selection(&state.niri.display_handle, &state.niri.seat);
        registry.notify_clipboard_owner_changed(None, Vec::new());
        Ok(())
    } else {
        Ok(())
    }
}

pub fn read_clipboard(
    state: &mut State,
    session_id: String,
    mime_type: String,
) -> Result<StdOwnedFd, String> {
    let registry = remote_desktop_registry(state)?;
    registry.ensure_clipboard_enabled(&session_id)?;

    let (read_fd, write_fd) = fd_pair()?;

    if let Some(selection) = current_data_device_selection_userdata(&state.niri.seat) {
        send_compositor_selection(&session_id, &mime_type, write_fd, &selection)?;
        return Ok(read_fd);
    }

    request_data_device_client_selection(&state.niri.seat, mime_type, write_fd)
        .map_err(selection_request_error)?;
    Ok(read_fd)
}

pub(super) fn fd_pair() -> Result<(StdOwnedFd, StdOwnedFd), String> {
    let (reader, writer) =
        UnixStream::pair().map_err(|err| format!("failed to create clipboard pipe: {err:?}"))?;
    Ok((reader.into(), writer.into()))
}

fn set_remote_clipboard_selection(
    state: &mut State,
    registry: RemoteDesktopSessionRegistry,
    session_id: String,
    mime_types: Vec<String>,
) -> Result<(), String> {
    let mime_types = validate_mime_types(mime_types).map_err(|err| format!("{err:?}"))?;
    let selection = RemoteDesktopClipboardSelection::new(
        session_id.clone(),
        registry.clone(),
        mime_types.clone(),
    );
    set_data_device_selection(
        &state.niri.display_handle,
        &state.niri.seat,
        mime_types.clone(),
        SelectionData::RemoteDesktop(selection),
    );
    registry.notify_clipboard_owner_changed(Some(session_id), mime_types);
    Ok(())
}

fn send_compositor_selection(
    requesting_session_id: &str,
    mime_type: &str,
    fd: StdOwnedFd,
    selection: &SelectionData,
) -> Result<(), String> {
    if !selection.contains_mime_type(mime_type) {
        return Err(format!("clipboard selection does not offer {mime_type}"));
    }

    match selection {
        SelectionData::Bytes { data, .. } => write_selection_bytes(fd, data.clone()),
        SelectionData::RemoteDesktop(remote_selection) => {
            if remote_selection.session_id() == requesting_session_id {
                return Err("remote desktop session cannot read its own clipboard".to_owned());
            }

            remote_selection.request_transfer(mime_type.to_owned(), fd)
        }
    }
}

fn remote_desktop_registry(state: &State) -> Result<RemoteDesktopSessionRegistry, String> {
    state
        .niri
        .dbus
        .as_ref()
        .and_then(|dbus| dbus.remote_desktop_sessions.clone())
        .ok_or_else(|| "remote desktop session registry is not available".to_owned())
}

fn validate_mime_types(mime_types: Vec<String>) -> fdo::Result<Vec<String>> {
    if mime_types.is_empty() {
        return Err(fdo::Error::InvalidArgs(
            "mime-types must not be empty".to_owned(),
        ));
    }

    if mime_types.iter().any(|mime_type| mime_type.is_empty()) {
        return Err(fdo::Error::InvalidArgs(
            "mime-types must not contain empty values".to_owned(),
        ));
    }

    Ok(mime_types)
}

fn selection_request_error(err: SelectionRequestError) -> String {
    match err {
        SelectionRequestError::InvalidMimetype => "requested clipboard MIME type is not available",
        SelectionRequestError::ServerSideSelection => "current clipboard selection is server-side",
        SelectionRequestError::NoSelection => "no clipboard selection is active",
    }
    .to_owned()
}
