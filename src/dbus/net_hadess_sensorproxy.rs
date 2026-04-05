use futures_util::StreamExt;
use zbus::fdo;

pub enum SensorProxyToNiri {
    SensorProxyChanged(String),
}

pub fn start(
    to_niri: calloop::channel::Sender<SensorProxyToNiri>,
) -> anyhow::Result<zbus::blocking::Connection> {
    let conn = zbus::blocking::Connection::system()?;

    let async_conn = conn.inner().clone();
    let future = async move {
        let proxy = fdo::PropertiesProxy::new(
            &async_conn,
            "net.hadess.SensorProxy",
            "/net/hadess/SensorProxy",
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

        let mut accel_ori = String::from("normal");

        // do we need send initial transfor property to niri or can we just rely
        // on default behaviour which is Normal transform
        // // Send the initial properties.
        // if let Err(err) = to_niri.send(Locale1ToNiri::XkbChanged(xkb.clone())) {
        //     warn!("error sending message to niri: {err:?}");
        //     return;
        // };

        while let Some(changed) = props_changed.next().await {
            let args = match changed.args() {
                Ok(args) => args,
                Err(err) => {
                    warn!("error parsing sensorproxy PropertiesChanged args: {err:?}");
                    return;
                }
            };

            let mut changed = false;
            for (name, value) in args.changed_properties() {
                let value = value.to_string();

                match *name {
                    "AccelerometerOrientation" => {
                        if accel_ori != value {
                            accel_ori = String::from(value);
                            changed = true;
                        }
                    }
                    _ => (),
                }
            }

            if !changed {
                continue;
            }

            if let Err(err) = to_niri.send(SensorProxyToNiri::SensorProxyChanged(accel_ori.clone())) {
                warn!("error sending message to niri: {err:?}");
                return;
            };
        }
    };

    let task = conn
        .inner()
        .executor()
        .spawn(future, "monitor SensorProxy property changes");
    task.detach();

    Ok(conn)
}
