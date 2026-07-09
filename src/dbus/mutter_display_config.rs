use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use smithay::utils::Size;
use zbus::fdo::RequestNameFlags;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{self, OwnedValue, Type};
use zbus::{fdo, interface};

use super::Start;
use crate::backend::IpcOutputMap;
use crate::utils::is_laptop_panel;
use crate::utils::scale::supported_scales;

pub struct DisplayConfig {
    to_niri: calloop::channel::Sender<HashMap<String, Option<niri_config::Output>>>,
    ipc_outputs: Arc<Mutex<IpcOutputMap>>,
}

#[derive(Serialize, Type)]
pub struct Monitor {
    names: (String, String, String, String),
    modes: Vec<Mode>,
    properties: HashMap<String, OwnedValue>,
}

#[derive(Serialize, Type)]
pub struct Mode {
    id: String,
    width: i32,
    height: i32,
    refresh_rate: f64,
    preferred_scale: f64,
    supported_scales: Vec<f64>,
    properties: HashMap<String, OwnedValue>,
}

#[derive(Serialize, Type)]
pub struct LogicalMonitor {
    x: i32,
    y: i32,
    scale: f64,
    transform: u32,
    is_primary: bool,
    monitors: Vec<(String, String, String, String)>,
    properties: HashMap<String, OwnedValue>,
}

// ApplyMonitorsConfig
#[derive(Deserialize, Type)]
pub struct LogicalMonitorConfiguration {
    x: i32,
    y: i32,
    scale: f64,
    transform: u32,
    _is_primary: bool,
    monitors: Vec<(String, String, HashMap<String, OwnedValue>)>,
}

#[interface(name = "org.gnome.Mutter.DisplayConfig")]
impl DisplayConfig {
    async fn get_current_state(
        &self,
    ) -> fdo::Result<(
        u32,
        Vec<Monitor>,
        Vec<LogicalMonitor>,
        HashMap<String, OwnedValue>,
    )> {
        // Construct the DBus response.
        let mut monitors = Vec::new();
        let mut logical_monitors = Vec::new();

        for output in self.ipc_outputs.lock().unwrap().values() {
            // Loosely matches the check in Mutter.
            let c = &output.name;
            let is_laptop_panel = is_laptop_panel(c);
            let display_name = make_display_name(output, is_laptop_panel);

            let mut properties = HashMap::new();
            properties.insert(
                String::from("display-name"),
                OwnedValue::from(zvariant::Str::from(display_name)),
            );
            properties.insert(
                String::from("is-builtin"),
                OwnedValue::from(is_laptop_panel),
            );

            let mut modes: Vec<Mode> = output
                .modes
                .iter()
                .map(|m| {
                    let niri_ipc::Mode {
                        width,
                        height,
                        refresh_rate,
                        is_preferred,
                    } = *m;
                    let width = i32::from(width);
                    let height = i32::from(height);
                    let refresh_rate = refresh_rate as f64 / 1000.;

                    Mode {
                        id: format!("{width}x{height}@{refresh_rate:.3}"),
                        width,
                        height,
                        refresh_rate,
                        preferred_scale: 1.,
                        supported_scales: supported_scales(Size::from((width, height))).collect(),
                        properties: HashMap::from([(
                            String::from("is-preferred"),
                            OwnedValue::from(is_preferred),
                        )]),
                    }
                })
                .collect();
            if let Some(mode) = output.current_mode {
                modes[mode]
                    .properties
                    .insert(String::from("is-current"), OwnedValue::from(true));
            }

            let connector = c.clone();
            let model = output.model.clone();
            let make = output.make.clone();

            // Serial is used for session restore, so fall back to the connector name if it's
            // not available.
            let serial = output.serial.as_ref().unwrap_or(&connector).clone();

            let names = (connector, make, model, serial);

            if let Some(logical) = output.logical.as_ref() {
                let transform = match logical.transform {
                    niri_ipc::Transform::Normal => 0,
                    niri_ipc::Transform::_90 => 1,
                    niri_ipc::Transform::_180 => 2,
                    niri_ipc::Transform::_270 => 3,
                    niri_ipc::Transform::Flipped => 4,
                    niri_ipc::Transform::Flipped90 => 5,
                    niri_ipc::Transform::Flipped180 => 6,
                    niri_ipc::Transform::Flipped270 => 7,
                };

                logical_monitors.push(LogicalMonitor {
                    x: logical.x,
                    y: logical.y,
                    scale: logical.scale,
                    transform,
                    is_primary: false,
                    monitors: vec![names.clone()],
                    properties: HashMap::new(),
                });
            }

            monitors.push(Monitor {
                names,
                modes,
                properties,
            });
        }

        // Sort by connector.
        monitors.sort_unstable_by(|a, b| a.names.0.cmp(&b.names.0));
        logical_monitors.sort_unstable_by(|a, b| a.monitors[0].0.cmp(&b.monitors[0].0));

        let properties = HashMap::from([(String::from("layout-mode"), OwnedValue::from(1u32))]);
        Ok((0, monitors, logical_monitors, properties))
    }

    async fn apply_monitors_config(
        &self,
        _serial: u32,
        method: u32,
        logical_monitor_configs: Vec<LogicalMonitorConfiguration>,
        _properties: HashMap<String, OwnedValue>,
    ) -> fdo::Result<()> {
        let current_conf = self.ipc_outputs.lock().unwrap();
        let mut new_conf = HashMap::new();
        for requested_config in logical_monitor_configs {
            if requested_config.monitors.len() > 1 {
                return Err(zbus::fdo::Error::Failed(
                    "Mirroring is not yet supported".to_owned(),
                ));
            }
            for (connector, mode, _props) in requested_config.monitors {
                if !current_conf.values().any(|o| o.name == connector) {
                    return Err(zbus::fdo::Error::Failed(format!(
                        "Connector '{connector}' not found",
                    )));
                }
                new_conf.insert(
                    connector.clone(),
                    Some(niri_config::Output {
                        off: false,
                        name: connector,
                        scale: Some(niri_config::FloatOrInt(requested_config.scale)),
                        transform: match requested_config.transform {
                            0 => niri_ipc::Transform::Normal,
                            1 => niri_ipc::Transform::_90,
                            2 => niri_ipc::Transform::_180,
                            3 => niri_ipc::Transform::_270,
                            4 => niri_ipc::Transform::Flipped,
                            5 => niri_ipc::Transform::Flipped90,
                            6 => niri_ipc::Transform::Flipped180,
                            7 => niri_ipc::Transform::Flipped270,
                            x => {
                                return Err(zbus::fdo::Error::Failed(format!(
                                    "Unknown transform {x}",
                                )))
                            }
                        },
                        position: Some(niri_config::Position {
                            x: requested_config.x,
                            y: requested_config.y,
                        }),
                        mode: Some(niri_config::output::Mode {
                            custom: false,
                            mode: niri_ipc::ConfiguredMode::from_str(&mode).map_err(|e| {
                                zbus::fdo::Error::Failed(format!(
                                    "Could not parse mode '{mode}': {e}"
                                ))
                            })?,
                        }),
                        // FIXME: VRR
                        ..Default::default()
                    }),
                );
            }
        }
        if new_conf.is_empty() {
            return Err(zbus::fdo::Error::Failed(
                "At least one output must be enabled".to_owned(),
            ));
        }
        for output in current_conf.values() {
            if !new_conf.contains_key(&output.name) {
                new_conf.insert(output.name.clone(), None);
            }
        }
        if method == 0 {
            // 0 means "verify", so don't actually apply here
            return Ok(());
        }
        if let Err(err) = self.to_niri.send(new_conf) {
            warn!("error sending message to niri: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }
        Ok(())
    }

    #[zbus(signal)]
    pub async fn monitors_changed(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn power_save_mode(&self) -> i32 {
        -1
    }

    #[zbus(property)]
    fn set_power_save_mode(&self, _mode: i32) -> zbus::Result<()> {
        Err(zbus::Error::Unsupported)
    }

    #[zbus(property)]
    fn panel_orientation_managed(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn apply_monitors_config_allowed(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn night_light_supported(&self) -> bool {
        false
    }
}

impl DisplayConfig {
    pub fn new(
        to_niri: calloop::channel::Sender<HashMap<String, Option<niri_config::Output>>>,
        ipc_outputs: Arc<Mutex<IpcOutputMap>>,
    ) -> Self {
        Self {
            to_niri,
            ipc_outputs,
        }
    }
}

impl Start for DisplayConfig {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::Connection::session()?;
        let flags = RequestNameFlags::AllowReplacement
            | RequestNameFlags::ReplaceExisting
            | RequestNameFlags::DoNotQueue;

        conn.object_server()
            .at("/org/gnome/Mutter/DisplayConfig", self)?;
        conn.request_name_with_flags("org.gnome.Mutter.DisplayConfig", flags)?;

        Ok(conn)
    }
}

// Adapted from Mutter.
fn make_display_name(output: &niri_ipc::Output, is_laptop_panel: bool) -> String {
    if is_laptop_panel {
        return String::from("Built-in display");
    }

    let make = &output.make;
    let model = &output.model;
    if let Some(diagonal) = output.physical_size.map(|(width_mm, height_mm)| {
        let diagonal = f64::hypot(f64::from(width_mm), f64::from(height_mm)) / 25.4;
        format_diagonal(diagonal)
    }) {
        format!("{make} {diagonal}")
    } else if model != "Unknown" {
        format!("{make} {model}")
    } else {
        make.clone()
    }
}

fn format_diagonal(diagonal_inches: f64) -> String {
    let known = [12.1, 13.3, 15.6];
    if let Some(d) = known.iter().find(|d| (*d - diagonal_inches).abs() < 0.1) {
        format!("{d:.1}″")
    } else {
        format!("{}″", diagonal_inches.round() as u32)
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::*;

    #[test]
    fn test_format_diagonal() {
        assert_snapshot!(format_diagonal(12.11), @"12.1″");
        assert_snapshot!(format_diagonal(13.28), @"13.3″");
        assert_snapshot!(format_diagonal(15.6), @"15.6″");
        assert_snapshot!(format_diagonal(23.2), @"23″");
        assert_snapshot!(format_diagonal(24.8), @"25″");
    }
}
