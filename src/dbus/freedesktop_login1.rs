use futures_util::{select, StreamExt};

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    #[zbus(property)]
    fn lid_closed(&self) -> zbus::Result<bool>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

pub enum Login1ToNiri {
    LidClosedChanged(bool),
    PrepareForSleep(bool),
}

pub fn start(
    to_niri: calloop::channel::Sender<Login1ToNiri>,
) -> anyhow::Result<zbus::blocking::Connection> {
    let conn = zbus::blocking::Connection::system()?;

    let async_conn = conn.inner().clone();
    let future = async move {
        let proxy = match Login1ManagerProxy::new(&async_conn).await {
            Ok(x) => x,
            Err(err) => {
                warn!("error creating login1 ManagerProxy: {err:?}");
                return;
            }
        };

        let mut prepare_for_sleep = match proxy.receive_prepare_for_sleep().await {
            Ok(x) => x,
            Err(err) => {
                warn!("error subscribing to PrepareForSleep: {err:?}");
                return;
            }
        }
        .fuse();
        let mut lid_closed_changed = proxy.receive_lid_closed_changed().await.fuse();

        let mut lid_closed = match proxy.lid_closed().await {
            Ok(x) => x,
            Err(err) => {
                warn!("error receiving initial lid state: {err:?}");
                return;
            }
        };

        if let Err(err) = to_niri.send(Login1ToNiri::LidClosedChanged(lid_closed)) {
            warn!("error sending initial lid state to niri: {err:?}");
            return;
        };

        loop {
            select! {
                changed = lid_closed_changed.next() => {
                    let Some(changed) = changed else {
                        warn!("LidClosed property change stream ended");
                        return;
                    };
                    let new_lid_closed = match changed.get().await {
                        Ok(x) => x,
                        Err(err) => {
                            warn!("error receiving changed lid state: {err:?}");
                            return;
                        }
                    };

                    if new_lid_closed == lid_closed {
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
                        warn!("PrepareForSleep signal stream ended");
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
                    }
                }
            }
        }
    };

    let task = conn
        .inner()
        .executor()
        .spawn(future, "monitor login1 changes");
    task.detach();

    Ok(conn)
}
