use futures_util::StreamExt;
use zbus::fdo;
use zbus::names::InterfaceName;

pub enum Login1ToNiri {
    LidClosedChanged(bool),
    PrepareForSleep(bool),
}

pub fn start(
    to_niri: calloop::channel::Sender<Login1ToNiri>,
) -> anyhow::Result<zbus::blocking::Connection> {
    let conn = zbus::blocking::Connection::system()?;

    let to_niri_sleep = to_niri.clone();

    let async_conn = conn.inner().clone();
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

    let async_conn = conn.inner().clone();
    let sleep_future = async move {
        let proxy = zbus::Proxy::new(
            &async_conn,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .await;
        let proxy = match proxy {
            Ok(x) => x,
            Err(err) => {
                warn!("error creating login1 Manager proxy: {err:?}");
                return;
            }
        };

        let mut signals = match proxy.receive_signal("PrepareForSleep").await {
            Ok(x) => x,
            Err(err) => {
                warn!("error subscribing to PrepareForSleep: {err:?}");
                return;
            }
        };

        while let Some(message) = signals.next().await {
            let start: bool = match message.body().deserialize() {
                Ok(x) => x,
                Err(err) => {
                    warn!("error parsing PrepareForSleep: {err:?}");
                    continue;
                }
            };

            if let Err(err) = to_niri_sleep.send(Login1ToNiri::PrepareForSleep(start)) {
                warn!("error sending message to niri: {err:?}");
                return;
            }
        }
    };

    conn.inner()
        .executor()
        .spawn(sleep_future, "monitor login1 sleep")
        .detach();

    Ok(conn)
}
