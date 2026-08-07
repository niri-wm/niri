use std::collections::HashMap;
use std::io::Read;
use std::os::fd::OwnedFd as StdOwnedFd;

use anyhow::Context;
use smithay::input::keyboard::{xkb, KeymapFile, Layout};
use zbus::fdo;
use zbus::zvariant::Value;

use crate::niri::State;
use crate::utils::expand_home;

const KEYMAP_TYPE_XKB: u32 = 0;
const KEYMAP_SOURCE_EXTERNAL: u32 = 0;
const KEYMAP_SOURCE_SESSION: u32 = 1;
const XKB_KEYMAP_FORMAT_TEXT_V1: u32 = 1;
const XKB_KEYMAP_FORMAT_TEXT_V2: u32 = 2;

#[derive(Debug)]
pub enum RemoteDesktopKeymapChange {
    Reset,
    Xkb { keymap: String, layout_index: u32 },
}

pub type RemoteDesktopKeymapReply = async_channel::Sender<Result<(), String>>;

pub fn keymap_capabilities() -> HashMap<String, Value<'static>> {
    HashMap::from([
        (
            "supported-keymap-types".to_owned(),
            Value::new(vec![KEYMAP_TYPE_XKB]),
        ),
        (
            "supported-xkb-keymap-formats".to_owned(),
            Value::new(vec![XKB_KEYMAP_FORMAT_TEXT_V1]),
        ),
    ])
}

pub fn current_keymap(session_keymap: bool) -> HashMap<String, Value<'static>> {
    let source = if session_keymap {
        KEYMAP_SOURCE_SESSION
    } else {
        KEYMAP_SOURCE_EXTERNAL
    };

    HashMap::from([
        ("source".to_owned(), Value::new(source)),
        ("name".to_owned(), Value::new("niri")),
    ])
}

pub fn parse_set_keymap_options(
    options: &HashMap<&str, Value<'_>>,
) -> fdo::Result<RemoteDesktopKeymapChange> {
    let Some(keymap_type) = optional_u32(options, "keymap-type")? else {
        return Ok(RemoteDesktopKeymapChange::Reset);
    };

    if keymap_type != KEYMAP_TYPE_XKB {
        return Err(fdo::Error::NotSupported(format!(
            "remote desktop keymap type {keymap_type} is not supported"
        )));
    }

    let format = optional_u32(options, "xkb-keymap-format")?.unwrap_or(XKB_KEYMAP_FORMAT_TEXT_V1);
    match format {
        XKB_KEYMAP_FORMAT_TEXT_V1 => (),
        XKB_KEYMAP_FORMAT_TEXT_V2 => {
            return Err(fdo::Error::NotSupported(
                "XKB keymap text v2 is not supported".to_owned(),
            ));
        }
        _ => {
            return Err(fdo::Error::InvalidArgs(format!(
                "unknown XKB keymap format: {format}"
            )));
        }
    }

    if optional_bool(options, "lock-keymap")?.unwrap_or(false) {
        return Err(fdo::Error::NotSupported(
            "locked remote desktop keymaps are not supported".to_owned(),
        ));
    }

    let keymap = read_keymap(options)?;
    let layout_index = optional_u32(options, "xkb-keymap-layout-index")?.unwrap_or(0);

    Ok(RemoteDesktopKeymapChange::Xkb {
        keymap,
        layout_index,
    })
}

pub fn apply_keymap_change(
    state: &mut State,
    change: RemoteDesktopKeymapChange,
) -> Result<(), String> {
    match change {
        RemoteDesktopKeymapChange::Reset => reset_keymap(state),
        RemoteDesktopKeymapChange::Xkb {
            keymap,
            layout_index,
        } => {
            set_keymap_from_string(state, keymap)?;
            set_layout_index(state, layout_index)
        }
    }
}

pub fn set_layout_index(state: &mut State, index: u32) -> Result<(), String> {
    let keyboard = state.niri.seat.get_keyboard().ok_or_else(|| {
        "remote desktop keymap layout cannot be changed without a keyboard".to_owned()
    })?;
    keyboard.with_xkb_state(state, |mut context| {
        context.set_layout(Layout(index));
    });
    Ok(())
}

pub fn current_keymap_file(state: &State) -> Result<KeymapFile, String> {
    let configured = state.niri.config.borrow().input.keyboard.xkb.clone();
    let xkb_config = if configured == niri_config::Xkb::default() {
        state.niri.xkb_from_locale1.clone().unwrap_or(configured)
    } else {
        configured
    };

    let keymap = if let Some(path) = xkb_config.file.clone() {
        let path = std::path::PathBuf::from(path);
        let path = expand_home(&path)
            .map_err(|err| format!("failed to expand keymap path: {err:?}"))?
            .unwrap_or(path);
        let keymap = std::fs::read_to_string(path)
            .map_err(|err| format!("failed to read configured keymap file: {err:?}"))?;
        xkb::Keymap::new_from_string(
            &xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            keymap,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or_else(|| "failed to compile configured keymap file".to_owned())?
    } else {
        let config = xkb_config.to_xkb_config();
        xkb::Keymap::new_from_names(
            &xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            config.rules,
            config.model,
            config.layout,
            config.variant,
            config.options,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or_else(|| "failed to compile configured keymap".to_owned())?
    };

    Ok(KeymapFile::new(&keymap))
}

fn reset_keymap(state: &mut State) -> Result<(), String> {
    let xkb_config = state.niri.config.borrow().input.keyboard.xkb.clone();
    if let Some(path) = xkb_config.file {
        let path = std::path::PathBuf::from(path);
        let path = expand_home(&path)
            .map_err(|err| format!("failed to expand keymap path: {err:?}"))?
            .unwrap_or(path);
        let keymap = std::fs::read_to_string(path)
            .map_err(|err| format!("failed to read configured keymap file: {err:?}"))?;
        set_keymap_from_string(state, keymap)
    } else {
        let xkb_config = if xkb_config == niri_config::Xkb::default() {
            state.niri.xkb_from_locale1.clone().unwrap_or(xkb_config)
        } else {
            xkb_config
        };
        state.set_xkb_config(xkb_config.to_xkb_config());
        Ok(())
    }
}

fn set_keymap_from_string(state: &mut State, keymap: String) -> Result<(), String> {
    let keyboard =
        state.niri.seat.get_keyboard().ok_or_else(|| {
            "remote desktop keymap cannot be changed without a keyboard".to_owned()
        })?;
    let num_lock = keyboard.modifier_state().num_lock;

    keyboard
        .set_keymap_from_string(state, keymap)
        .map_err(|err| format!("failed to set remote desktop keymap: {err:?}"))?;

    let mut mods_state = keyboard.modifier_state();
    if mods_state.num_lock != num_lock {
        mods_state.num_lock = num_lock;
        keyboard.set_modifier_state(mods_state);
    }

    Ok(())
}

fn read_keymap(options: &HashMap<&str, Value<'_>>) -> fdo::Result<String> {
    let value = options.get("xkb-keymap").ok_or_else(|| {
        fdo::Error::InvalidArgs("xkb-keymap fd is required for XKB keymaps".to_owned())
    })?;
    let fd = match value
        .try_clone()
        .map_err(|err| fdo::Error::Failed(format!("failed to clone keymap fd: {err:?}")))?
    {
        Value::Fd(fd) => StdOwnedFd::try_from(fd)
            .map_err(|err| fdo::Error::Failed(format!("failed to own keymap fd: {err:?}")))?,
        _ => {
            return Err(fdo::Error::InvalidArgs(
                "xkb-keymap must be a file descriptor".to_owned(),
            ));
        }
    };
    let mut file = std::fs::File::from(fd);
    let mut keymap = String::new();
    file.read_to_string(&mut keymap)
        .context("failed to read XKB keymap fd")
        .map_err(|err| fdo::Error::Failed(format!("{err:?}")))?;

    Ok(keymap)
}

fn optional_u32(options: &HashMap<&str, Value<'_>>, name: &str) -> fdo::Result<Option<u32>> {
    options
        .get(name)
        .map(|value| {
            value
                .downcast_ref::<u32>()
                .map_err(|_| fdo::Error::InvalidArgs(format!("{name} must be a uint32")))
        })
        .transpose()
}

fn optional_bool(options: &HashMap<&str, Value<'_>>, name: &str) -> fdo::Result<Option<bool>> {
    options
        .get(name)
        .map(|value| {
            value
                .downcast_ref::<bool>()
                .map_err(|_| fdo::Error::InvalidArgs(format!("{name} must be a boolean")))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_keymap_without_keymap_type_resets_to_external_keymap() {
        let change = parse_set_keymap_options(&HashMap::new()).unwrap();

        assert!(matches!(change, RemoteDesktopKeymapChange::Reset));
    }

    #[test]
    fn set_keymap_rejects_unsupported_xkb_v2_format() {
        let options = HashMap::from([
            ("keymap-type", Value::new(KEYMAP_TYPE_XKB)),
            ("xkb-keymap-format", Value::new(XKB_KEYMAP_FORMAT_TEXT_V2)),
        ]);
        let err = parse_set_keymap_options(&options).unwrap_err();

        assert!(format!("{err:?}").contains("text v2"));
    }
}
