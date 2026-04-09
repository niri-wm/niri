use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use niri_config::{Config, OutputName};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::utils::Size;

use super::{IpcOutputMap, OutputId, VirtualOutputMarker};
use crate::frame_clock::FrameClock;
use crate::niri::Niri;
use crate::utils::logical_output;

pub(super) struct BuiltVirtualOutput {
    pub name: String,
    pub output: Output,
    pub output_id: OutputId,
    pub refresh_interval: Duration,
    pub ipc_output: niri_ipc::Output,
}

pub(super) fn refresh_interval_from_millihz(refresh_mhz: u64) -> Duration {
    let refresh_mhz = refresh_mhz.max(1);
    let interval_nanos = 1_000_000_000_000 / refresh_mhz;

    // Clamp extremely low refresh rates to a reasonable pacing interval.
    // This matches existing virtual-output code paths (off/on + mode changes).
    if interval_nanos >= 1_000_000_000 {
        Duration::from_micros(16_667)
    } else {
        Duration::from_nanos(interval_nanos)
    }
}

pub(super) fn build_headless_virtual_output(
    counter: &mut u32,
    width: u16,
    height: u16,
    refresh_rate: u32,
) -> BuiltVirtualOutput {
    let refresh_rate = if refresh_rate < 2 { 60 } else { refresh_rate };

    *counter += 1;
    let n = *counter;

    let connector = format!("HEADLESS-{n}");
    let make = "niri".to_string();
    let model = "virtual".to_string();
    let serial = n.to_string();

    let refresh_mhz = i32::try_from(refresh_rate.saturating_mul(1000)).unwrap_or(60_000);

    let output = Output::new(
        connector.clone(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: make.clone(),
            model: model.clone(),
            serial_number: serial.clone(),
        },
    );

    let mode = Mode {
        size: Size::from((i32::from(width), i32::from(height))),
        refresh: refresh_mhz.max(1),
    };
    output.change_current_state(Some(mode), None, None, None);
    output.set_preferred(mode);

    output.user_data().insert_if_missing(|| OutputName {
        connector: connector.clone(),
        make: Some(make),
        model: Some(model),
        serial: Some(serial),
    });

    output
        .user_data()
        .insert_if_missing(VirtualOutputMarker::default);

    let output_id = OutputId::next();

    let physical_properties = output.physical_properties();
    let ipc_output = niri_ipc::Output {
        name: output.name(),
        make: physical_properties.make,
        model: physical_properties.model,
        serial: None,
        physical_size: None,
        modes: vec![niri_ipc::Mode {
            width,
            height,
            refresh_rate: refresh_rate * 1000,
            is_preferred: true,
        }],
        current_mode: Some(0),
        is_custom_mode: true,
        vrr_supported: false,
        vrr_enabled: false,
        logical: Some(logical_output(&output)),
    };

    let refresh_interval = refresh_interval_from_millihz(u64::from(refresh_rate) * 1000);

    BuiltVirtualOutput {
        name: connector,
        output,
        output_id,
        refresh_interval,
        ipc_output,
    }
}

pub(super) fn remove_virtual_output_from_map(
    niri: &mut Niri,
    ipc_outputs: &Arc<Mutex<IpcOutputMap>>,
    outputs: &mut HashMap<String, (Output, OutputId)>,
    name: &str,
    kind: &str,
) -> Result<(), String> {
    let (output, output_id) = outputs
        .remove(name)
        .ok_or_else(|| format!("{kind} '{name}' not found"))?;

    ipc_outputs.lock().unwrap().remove(&output_id);
    niri.remove_output(&output);

    Ok(())
}

pub(super) fn apply_config_to_managed_virtual_outputs(
    niri: &mut Niri,
    outputs: &mut HashMap<String, (Output, OutputId)>,
    ipc_outputs: Option<&Arc<Mutex<IpcOutputMap>>>,
    config: &Rc<RefCell<Config>>,
) {
    let config = config.borrow();
    let mut resized_outputs = Vec::new();

    // Apply config to all managed virtual outputs, even if currently disconnected (off).
    // This allows `off`/`on` to work without losing internal state or IPC entries.
    for (output_name, (output, output_id)) in outputs.iter_mut() {
        let name = output.user_data().get::<OutputName>().unwrap();
        let output_config = config.outputs.find(name);

        let is_off = output_config.is_some_and(|c| c.off);
        let new_mode = output_config.and_then(|config| {
            config.mode.as_ref().map(|mode_config| {
                let refresh_hz = mode_config.mode.refresh.unwrap_or(60.0);
                let refresh_mhz = (refresh_hz * 1000.0).round().clamp(1.0, i32::MAX as f64) as i32;
                Mode {
                    size: Size::from((
                        i32::from(mode_config.mode.width),
                        i32::from(mode_config.mode.height),
                    )),
                    refresh: refresh_mhz,
                }
            })
        });

        // Apply mode changes even when off so the mode is ready when re-enabled.
        let mut mode_changed = false;
        if let Some(new_mode) = new_mode {
            if output.current_mode() != Some(new_mode) {
                output.change_current_state(Some(new_mode), None, None, None);
                output.set_preferred(new_mode);
                mode_changed = true;

                if let Some(ipc_outputs) = ipc_outputs {
                    if let Some(ipc_output) = ipc_outputs.lock().unwrap().get_mut(output_id) {
                        ipc_output.modes = vec![niri_ipc::Mode {
                            width: new_mode.size.w as u16,
                            height: new_mode.size.h as u16,
                            refresh_rate: new_mode.refresh as u32,
                            is_preferred: true,
                        }];
                        ipc_output.current_mode = Some(0);
                        ipc_output.is_custom_mode = true;
                    }
                }
            }
        }

        let was_connected = niri
            .global_space
            .outputs()
            .any(|o| o.name() == *output_name);

        // Handle off/on by removing/adding the output from/to the space.
        match (is_off, was_connected) {
            (true, true) => {
                niri.remove_output(output);
                niri.ipc_outputs_changed = true;
            }
            (false, false) => {
                let refresh_mhz = output
                    .current_mode()
                    .map(|m| m.refresh.max(1) as u64)
                    .unwrap_or(60_000);
                let refresh_interval = refresh_interval_from_millihz(refresh_mhz);
                niri.add_output(output.clone(), Some(refresh_interval), false);
                niri.ipc_outputs_changed = true;
                resized_outputs.push(output.clone());
            }
            _ => {}
        }

        let is_connected = niri
            .global_space
            .outputs()
            .any(|o| o.name() == *output_name);

        if mode_changed && !is_off && is_connected {
            if let Some(new_mode) = output.current_mode() {
                // Keep refresh pacing in sync with the new mode when connected.
                if let Some(output_state) = niri.output_state.get_mut(output) {
                    let refresh_mhz = new_mode.refresh.max(1) as u64;
                    let refresh_interval = refresh_interval_from_millihz(refresh_mhz);
                    output_state.frame_clock = FrameClock::new(Some(refresh_interval), false);
                }
            }

            resized_outputs.push(output.clone());
        }

        if let Some(ipc_outputs) = ipc_outputs {
            if let Some(ipc_output) = ipc_outputs.lock().unwrap().get_mut(output_id) {
                ipc_output.logical = is_connected.then(|| logical_output(output));
            }
        }
    }

    if !resized_outputs.is_empty() {
        niri.ipc_outputs_changed = true;
        for output in resized_outputs {
            if niri.output_state.contains_key(&output) {
                niri.output_resized(&output);
                niri.queue_redraw(&output);
            }
        }
    }
}
