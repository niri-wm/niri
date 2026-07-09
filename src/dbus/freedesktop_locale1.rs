use futures_util::StreamExt;
use niri_config::Xkb;
use zbus::names::InterfaceName;
use zbus::{fdo, zvariant};

pub enum Locale1ToNiri {
    XkbChanged(Xkb),
}

pub fn start(
    to_niri: calloop::channel::Sender<Locale1ToNiri>,
) -> anyhow::Result<zbus::blocking::Connection> {
    let conn = zbus::blocking::Connection::system()?;

    let async_conn = conn.inner().clone();
    let future = async move {
        let proxy = fdo::PropertiesProxy::new(
            &async_conn,
            "org.freedesktop.locale1",
            "/org/freedesktop/locale1",
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
            .get_all(InterfaceName::try_from("org.freedesktop.locale1").unwrap())
            .await;
        let mut props = match props {
            Ok(x) => x,
            Err(err) => {
                warn!("error receiving initial properties: {err:?}");
                return;
            }
        };

        trace!("initial properties: {props:?}");

        let mut get = |name| {
            props
                .remove(name)
                .and_then(|x| String::try_from(x).ok())
                .unwrap_or_default()
        };

        let mut xkb = Xkb {
            rules: String::new(),
            model: get("X11Model"),
            layout: get("X11Layout"),
            variant: get("X11Variant"),
            options: match get("X11Options") {
                x if x.is_empty() => None,
                x => Some(x),
            },
            file: None,
        };

        // Send the initial properties.
        if let Err(err) = to_niri.send(Locale1ToNiri::XkbChanged(xkb.clone())) {
            warn!("error sending message to niri: {err:?}");
            return;
        };

        while let Some(changed) = props_changed.next().await {
            let args = match changed.args() {
                Ok(args) => args,
                Err(err) => {
                    warn!("error parsing locale1 PropertiesChanged args: {err:?}");
                    return;
                }
            };

            let mut changed = false;
            for (name, value) in args.changed_properties() {
                trace!("changed property: {name} => {value:?}");

                let value = zvariant::Str::try_from(value).unwrap_or_default();
                let value = value.as_str();

                match *name {
                    "X11Model" => {
                        if xkb.model != value {
                            xkb.model = String::from(value);
                            changed = true;
                        }
                    }
                    "X11Layout" => {
                        if xkb.layout != value {
                            xkb.layout = String::from(value);
                            changed = true;
                        }
                    }
                    "X11Variant" => {
                        if xkb.variant != value {
                            xkb.variant = String::from(value);
                            changed = true;
                        }
                    }
                    "X11Options" => {
                        let value = match value {
                            "" => None,
                            x => Some(x),
                        };
                        if xkb.options.as_deref() != value {
                            xkb.options = value.map(String::from);
                            changed = true;
                        }
                    }
                    _ => (),
                }
            }

            if !changed {
                continue;
            }

            if let Err(err) = to_niri.send(Locale1ToNiri::XkbChanged(xkb.clone())) {
                warn!("error sending message to niri: {err:?}");
                return;
            };
        }
    };

    let task = conn
        .inner()
        .executor()
        .spawn(future, "monitor locale1 property changes");
    task.detach();

    Ok(conn)
}
