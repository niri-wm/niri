use std::collections::HashMap;

use futures_util::StreamExt;
use niri_config::{AppearanceSettings, ColorScheme, Contrast};
use zbus::zvariant;

pub enum PortalSettingsToNiri {
    AppearanceChanged(AppearanceSettings),
}

pub fn start(
    to_niri: calloop::channel::Sender<PortalSettingsToNiri>,
) -> anyhow::Result<zbus::blocking::Connection> {
    let conn = zbus::blocking::Connection::session()?;

    let async_conn = conn.inner().clone();
    let future = async move {
        let proxy = zbus::Proxy::new(
            &async_conn,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .await;
        let proxy = match proxy {
            Ok(x) => x,
            Err(err) => {
                warn!("error creating portal Settings proxy: {err:?}");
                return;
            }
        };

        let mut changed = match proxy.receive_signal("SettingChanged").await {
            Ok(x) => x,
            Err(err) => {
                warn!("error subscribing to portal SettingChanged: {err:?}");
                return;
            }
        };

        let mut appearance = AppearanceSettings::default();

        let read_all = proxy
            .call::<_, _, HashMap<String, HashMap<String, zvariant::OwnedValue>>>(
                "ReadAll",
                &(vec!["org.freedesktop.appearance"],),
            )
            .await;
        match read_all {
            Ok(values) => {
                if let Some(namespace) = values.get("org.freedesktop.appearance") {
                    apply_appearance_updates(&mut appearance, namespace.iter());
                }
            }
            Err(err) => {
                warn!("error reading portal appearance settings: {err:?}");
            }
        }

        if let Err(err) = to_niri.send(PortalSettingsToNiri::AppearanceChanged(appearance)) {
            warn!("error sending portal appearance settings to niri: {err:?}");
            return;
        };

        while let Some(signal) = changed.next().await {
            let args = signal
                .body()
                .deserialize::<(String, String, zvariant::OwnedValue)>();
            let (namespace, key, value) = match args {
                Ok(x) => x,
                Err(err) => {
                    warn!("error parsing portal SettingChanged args: {err:?}");
                    continue;
                }
            };

            if namespace != "org.freedesktop.appearance" {
                continue;
            }

            let mut updated = appearance;
            apply_appearance_updates(&mut updated, std::iter::once((&key, &value)));
            if updated == appearance {
                continue;
            }

            appearance = updated;
            if let Err(err) = to_niri.send(PortalSettingsToNiri::AppearanceChanged(appearance)) {
                warn!("error sending portal appearance settings to niri: {err:?}");
                return;
            };
        }
    };

    let task = conn
        .inner()
        .executor()
        .spawn(future, "monitor portal settings changes");
    task.detach();

    Ok(conn)
}

fn apply_appearance_updates<'a>(
    appearance: &mut AppearanceSettings,
    updates: impl IntoIterator<Item = (&'a String, &'a zvariant::OwnedValue)>,
) {
    for (key, value) in updates {
        let Ok(value) = value.downcast_ref::<u32>() else {
            continue;
        };

        match key.as_str() {
            "color-scheme" => {
                appearance.color_scheme = match value {
                    1 => Some(ColorScheme::Dark),
                    2 => Some(ColorScheme::Light),
                    _ => None,
                };
            }
            "contrast" => {
                appearance.contrast = match value {
                    1 => Some(Contrast::High),
                    _ => None,
                };
            }
            "reduced-motion" => {
                appearance.reduced_motion = value == 1;
            }
            _ => (),
        }
    }
}
