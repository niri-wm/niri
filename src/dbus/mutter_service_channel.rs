use std::os::unix::net::UnixStream;

use zbus::{fdo, interface, zvariant};

use super::Start;
use crate::niri::NewClient;

pub struct ServiceChannel {
    to_niri: calloop::channel::Sender<NewClient>,
}

#[interface(name = "org.gnome.Mutter.ServiceChannel")]
impl ServiceChannel {
    async fn open_wayland_service_connection(
        &mut self,
        service_client_type: u32,
    ) -> fdo::Result<zvariant::OwnedFd> {
        if service_client_type != 1 {
            return Err(fdo::Error::InvalidArgs(
                "Invalid service client type".to_owned(),
            ));
        }

        let (sock1, sock2) = UnixStream::pair().unwrap();
        let client = NewClient {
            client: sock2,
            restricted: false,
            // FIXME: maybe you can get the PID from D-Bus somehow?
            credentials_unknown: true,
        };
        if let Err(err) = self.to_niri.send(client) {
            warn!("error sending message to niri: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        Ok(zvariant::OwnedFd::from(std::os::fd::OwnedFd::from(sock1)))
    }
}

impl ServiceChannel {
    pub fn new(to_niri: calloop::channel::Sender<NewClient>) -> Self {
        Self { to_niri }
    }
}

impl Start for ServiceChannel {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::connection::Builder::session()?
            .name("org.gnome.Mutter.ServiceChannel")?
            .serve_at("/org/gnome/Mutter/ServiceChannel", self)?
            .build()?;
        Ok(conn)
    }
}
