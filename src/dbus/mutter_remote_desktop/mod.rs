mod clipboard;
mod clipboard_registry;
mod clipboard_signals;
#[cfg(test)]
mod clipboard_tests;
mod device_types;
mod eis;
mod input;
mod keymap;
mod legacy_input;
mod registry;
mod session;

use std::sync::atomic::{AtomicU64, Ordering};

use calloop::channel::Sender;
use zbus::fdo::{self, RequestNameFlags};
use zbus::message::Header;
use zbus::names::OwnedUniqueName;
use zbus::zvariant::OwnedObjectPath;
use zbus::{interface, ObjectServer};

pub use self::clipboard::RemoteDesktopClipboardSelection;
use self::device_types::supported_device_types;
pub use self::input::{dispatch_to_niri, RemoteDesktopToNiri};
pub use self::registry::{
    RemoteDesktopOutputStream, RemoteDesktopSessionRegistry, RemoteDesktopStream,
};
pub use self::session::Session;
use super::Start;

const REMOTE_DESKTOP_VERSION: i32 = 1;
const SESSION_PATH_PREFIX: &str = "/org/gnome/Mutter/RemoteDesktop/Session/u";

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct RemoteDesktop {
    to_niri: Sender<RemoteDesktopToNiri>,
    sessions: RemoteDesktopSessionRegistry,
}

#[interface(name = "org.gnome.Mutter.RemoteDesktop")]
impl RemoteDesktop {
    async fn create_session(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> fdo::Result<OwnedObjectPath> {
        let peer_name = peer_name(&hdr)?;
        let id = NEXT_SESSION_ID.fetch_add(1, Ordering::SeqCst);
        let path = session_path(id)?;
        let session_id = format!("niri-remote-desktop-session-{id}");
        let session = Session::new(
            id,
            session_id.clone(),
            peer_name,
            self.to_niri.clone(),
            self.sessions.clone(),
        );

        self.sessions.insert_session(session_id.clone())?;

        match server.at(&path, session).await {
            Ok(true) => {
                let iface = server.interface::<_, Session>(&path).await.map_err(|err| {
                    fdo::Error::Failed(format!(
                        "error looking up remote desktop session interface: {err:?}"
                    ))
                })?;
                self.sessions
                    .set_clipboard_signal_emitter(&session_id, iface.signal_emitter().to_owned())?;
                debug!(id, "RemoteDesktop session created");
                Ok(path)
            }
            Ok(false) => {
                self.sessions.remove_session(&session_id)?;
                Err(fdo::Error::ObjectPathInUse(format!(
                    "session path already exists: {path}"
                )))
            }
            Err(err) => {
                self.sessions.remove_session(&session_id)?;
                Err(fdo::Error::Failed(format!(
                    "error creating remote desktop session: {err:?}"
                )))
            }
        }
    }

    #[zbus(property)]
    async fn supported_device_types(&self) -> u32 {
        supported_device_types()
    }

    #[zbus(property)]
    async fn version(&self) -> i32 {
        REMOTE_DESKTOP_VERSION
    }
}

impl RemoteDesktop {
    pub fn new(
        to_niri: Sender<RemoteDesktopToNiri>,
        sessions: RemoteDesktopSessionRegistry,
    ) -> Self {
        Self { to_niri, sessions }
    }
}

impl Start for RemoteDesktop {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::Connection::session()?;
        let flags = RequestNameFlags::AllowReplacement
            | RequestNameFlags::ReplaceExisting
            | RequestNameFlags::DoNotQueue;

        conn.object_server()
            .at("/org/gnome/Mutter/RemoteDesktop", self)?;
        conn.request_name_with_flags("org.gnome.Mutter.RemoteDesktop", flags)?;

        Ok(conn)
    }
}

fn session_path(id: u64) -> fdo::Result<OwnedObjectPath> {
    OwnedObjectPath::try_from(format!("{SESSION_PATH_PREFIX}{id}"))
        .map_err(|err| fdo::Error::Failed(format!("invalid session object path: {err}")))
}

fn peer_name(hdr: &Header<'_>) -> fdo::Result<OwnedUniqueName> {
    let Some(sender) = hdr.sender() else {
        return Err(fdo::Error::Failed(
            "remote desktop request has no sender".to_owned(),
        ));
    };

    Ok(OwnedUniqueName::from(sender.to_owned()))
}

#[cfg(test)]
mod tests {
    use calloop::channel;
    use zbus::object_server::Interface;

    use super::*;

    #[test]
    fn advertises_truthful_mutter_contract() {
        assert_eq!(REMOTE_DESKTOP_VERSION, 1);
        assert_eq!(supported_device_types(), 7);
    }

    #[test]
    fn generated_session_paths_are_valid_object_paths() {
        assert_eq!(
            session_path(7).unwrap().as_str(),
            "/org/gnome/Mutter/RemoteDesktop/Session/u7"
        );
    }

    #[test]
    fn introspection_exposes_mutter_entrypoints() {
        let (to_niri, _from_remote_desktop) = channel::channel();
        let remote_desktop =
            RemoteDesktop::new(to_niri.clone(), RemoteDesktopSessionRegistry::default());
        let mut xml = String::new();
        remote_desktop.introspect_to_writer(&mut xml, 0);
        assert!(xml.contains(r#"<method name="CreateSession">"#));
        assert!(xml.contains(r#"name="SupportedDeviceTypes""#));
        assert!(xml.contains(r#"type="u""#));

        let mut xml = String::new();
        Session::new(
            7,
            "niri-remote-desktop-session-7".to_owned(),
            OwnedUniqueName::try_from(":1.7").unwrap(),
            to_niri,
            RemoteDesktopSessionRegistry::default(),
        )
        .introspect_to_writer(&mut xml, 0);
        assert!(xml.contains(r#"name="SessionId""#));
        assert!(xml.contains(r#"type="s""#));
        assert!(xml.contains(r#"<method name="ConnectToEIS">"#));
        assert!(xml.contains(r#"<method name="SelectionWrite">"#));
    }
}
