use futures_util::StreamExt;
use zbus::fdo;
use zbus::names::InterfaceName;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

pub enum Login1ToNiri {
    LidClosedChanged(bool),
    PrepareForSleep(bool),
}

pub fn take_sleep_inhibitor(conn: &zbus::blocking::Connection) -> Option<std::os::fd::OwnedFd> {
    let proxy = zbus::blocking::Proxy::new(
        conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .ok()?;
    let fd: zbus::zvariant::OwnedFd = proxy
        .call(
            "Inhibit",
            &("sleep", "niri", "Preparing GPU for sleep", "delay"),
        )
        .ok()?;
    Some(fd.into())
}

pub fn take_sleep_inhibitor_system() -> Option<std::os::fd::OwnedFd> {
    let conn = zbus::blocking::Connection::system().ok()?;
    take_sleep_inhibitor(&conn)
}

pub fn start(
    to_niri: calloop::channel::Sender<Login1ToNiri>,
) -> anyhow::Result<zbus::blocking::Connection> {
    let conn = zbus::blocking::Connection::system()?;

    let async_conn = conn.inner().clone();

    // Listen for system PrepareForSleep signals.
    let manager_proxy = Login1ManagerProxy::builder(&async_conn).build();
    let to_niri_sleep = to_niri.clone();
    async_conn
        .executor()
        .spawn(
            async move {
                let Ok(proxy) = manager_proxy.await else {
                    return;
                };
                let Ok(mut stream) = proxy.receive_prepare_for_sleep().await else {
                    return;
                };
                while let Some(signal) = stream.next().await {
                    if let Ok(args) = signal.args() {
                        debug!("login1 PrepareForSleep signal: start={}", args.start);
                        let _ = to_niri_sleep.send(Login1ToNiri::PrepareForSleep(args.start));
                    }
                }
            },
            "monitor login1 PrepareForSleep",
        )
        .detach();

    let future = async move {
        let proxy = fdo::PropertiesProxy::new(
            &async_conn,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
        )
        .await;
        let proxy = match proxy {
            Ok(x) => x,
            Err(err) => {
                warn!("error creating PropertiesProxy: {err:?}");
                return;
            }
        };

        let mut props_changed = match proxy.receive_properties_changed().await {
            Ok(x) => x,
            Err(err) => {
                warn!("error subscribing to PropertiesChanged: {err:?}");
                return;
            }
        };

        let props = proxy
            .get_all(InterfaceName::try_from("org.freedesktop.login1.Manager").unwrap())
            .await;
        let mut props = match props {
            Ok(x) => x,
            Err(err) => {
                warn!("error receiving initial properties: {err:?}");
                return;
            }
        };

        trace!("initial properties: {props:?}");

        let mut lid_closed = props
            .remove("LidClosed")
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or_default();

        if let Err(err) = to_niri.send(Login1ToNiri::LidClosedChanged(lid_closed)) {
            warn!("error sending initial lid state to niri: {err:?}");
            return;
        };

        while let Some(signal) = props_changed.next().await {
            let args = match signal.args() {
                Ok(args) => args,
                Err(err) => {
                    warn!("error parsing PropertiesChanged args: {err:?}");
                    return;
                }
            };

            let mut new_lid_closed = lid_closed;
            let mut changed = false;
            for (name, value) in args.changed_properties() {
                trace!("changed property: {name} => {value:?}");
                if *name != "LidClosed" {
                    continue;
                }

                new_lid_closed = bool::try_from(value).unwrap_or(new_lid_closed);
                changed = true;
            }

            if !changed {
                continue;
            }

            if new_lid_closed == lid_closed {
                continue;
            }

            lid_closed = new_lid_closed;
            if let Err(err) = to_niri.send(Login1ToNiri::LidClosedChanged(lid_closed)) {
                warn!("error sending message to niri: {err:?}");
                return;
            };
        }
    };

    let task = conn
        .inner()
        .executor()
        .spawn(future, "monitor login1 property changes");
    task.detach();

    Ok(conn)
}
