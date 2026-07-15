use futures_util::StreamExt;
use zbus::fdo;
use zbus::names::InterfaceName;

pub enum Login1ToNiri {
    LidClosedChanged(bool),
    PrepareForSleep(bool),
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    #[zbus(signal)]
    fn prepare_for_sleep(start: bool) -> zbus::Result<()>;
}

pub fn start(
    to_niri: calloop::channel::Sender<Login1ToNiri>,
) -> anyhow::Result<zbus::blocking::Connection> {
    let conn = zbus::blocking::Connection::system()?;

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

        let login1 = Login1ManagerProxy::new(&async_conn).await;
        let login1 = match login1 {
            Ok(x) => x,
            Err(err) => {
                warn!("error creating login1 manager proxy: {err:?}");
                return;
            }
        };

        let prepare_for_sleep = login1.receive_prepare_for_sleep().await;
        let mut prepare_for_sleep = match prepare_for_sleep {
            Ok(x) => x,
            Err(err) => {
                warn!("error subscribing to PrepareForSleep: {err:?}");
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

        loop {
            futures_util::select! {
                signal = props_changed.next() => {
                    let Some(signal) = signal else {
                        return;
                    };

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

                    if !changed || new_lid_closed == lid_closed {
                        continue;
                    }

                    lid_closed = new_lid_closed;
                    if let Err(err) = to_niri.send(Login1ToNiri::LidClosedChanged(lid_closed)) {
                        warn!("error sending message to niri: {err:?}");
                        return;
                    };
                }
                signal = prepare_for_sleep.next() => {
                    let Some(signal) = signal else {
                        return;
                    };

                    let args = match signal.args() {
                        Ok(args) => args,
                        Err(err) => {
                            warn!("error parsing PrepareForSleep args: {err:?}");
                            return;
                        }
                    };

                    if let Err(err) = to_niri.send(Login1ToNiri::PrepareForSleep(args.start)) {
                        warn!("error sending message to niri: {err:?}");
                        return;
                    };
                }
            }
        }
    };

    let task = conn
        .inner()
        .executor()
        .spawn(future, "monitor login1 property changes");
    task.detach();

    Ok(conn)
}
