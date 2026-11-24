use futures_util::{FutureExt, StreamExt};
use zbus::fdo;
use zbus::names::InterfaceName;

pub enum Login1ToNiri {
    LidClosedChanged(bool),
    PrepareForSleep(bool),
    SleepInhibitor(Option<std::os::fd::OwnedFd>),
}

async fn take_sleep_inhibitor(proxy: &zbus::Proxy<'_>) -> Option<std::os::fd::OwnedFd> {
    let call = proxy
        .call(
            "Inhibit",
            &("sleep", "niri", "Waiting for GPU access", "delay"),
        )
        .fuse();
    let timeout = FutureExt::fuse(async_io::Timer::after(std::time::Duration::from_secs(1)));
    futures_util::pin_mut!(call, timeout);

    let fd: zbus::zvariant::OwnedFd = match futures_util::select! {
        result = call => Some(result),
        _ = timeout => None,
    } {
        Some(Ok(fd)) => fd,
        None => {
            warn!("timed out acquiring sleep inhibitor");
            return None;
        }
        Some(Err(err)) => {
            warn!("failed to acquire sleep inhibitor: {err:?}");
            return None;
        }
    };
    let fd = fd.into();
    if let Err(err) = smithay::reexports::rustix::io::fcntl_setfd(
        &fd,
        smithay::reexports::rustix::io::FdFlags::CLOEXEC,
    ) {
        warn!("failed to set CLOEXEC on sleep inhibitor: {err:?}");
        return None;
    }
    Some(fd)
}

pub fn start(
    to_niri: calloop::channel::Sender<Login1ToNiri>,
) -> anyhow::Result<zbus::blocking::Connection> {
    let conn = zbus::blocking::connection::Builder::system()?
        .method_timeout(std::time::Duration::from_secs(5))
        .build()?;

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

        let mut inhibitor = take_sleep_inhibitor(&proxy).boxed().fuse();
        loop {
            futures_util::select_biased! {
                fd = inhibitor => {
                    if to_niri_sleep.send(Login1ToNiri::SleepInhibitor(fd)).is_err() {
                        return;
                    }
                }
                message = signals.next().fuse() => {
                    let Some(message) = message else { return; };
                    let start: bool = match message.body().deserialize() {
                        Ok(x) => x,
                        Err(err) => {
                            warn!("error parsing PrepareForSleep: {err:?}");
                            continue;
                        }
                    };
                    if to_niri_sleep.send(Login1ToNiri::PrepareForSleep(start)).is_err() {
                        return;
                    }
                    if !start {
                        inhibitor = take_sleep_inhibitor(&proxy).boxed().fuse();
                    }
                }
            }
        }
    };

    conn.inner()
        .executor()
        .spawn(sleep_future, "monitor login1 sleep")
        .detach();

    Ok(conn)
}
