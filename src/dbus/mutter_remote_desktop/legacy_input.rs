use smithay::backend::input::{AxisSource, ButtonState, KeyState};
use zbus::fdo;

use super::input::RemoteDesktopAxisFrame;

pub const XKB_KEYCODE_OFFSET: u32 = 8;

const AXIS_FLAG_FINISH: u32 = 1 << 0;
const AXIS_FLAG_SOURCE_WHEEL: u32 = 1 << 1;
const AXIS_FLAG_SOURCE_FINGER: u32 = 1 << 2;
const AXIS_FLAG_SOURCE_CONTINUOUS: u32 = 1 << 3;
const SOURCE_FLAGS: u32 =
    AXIS_FLAG_SOURCE_WHEEL | AXIS_FLAG_SOURCE_FINGER | AXIS_FLAG_SOURCE_CONTINUOUS;

pub fn key_state(state: bool) -> KeyState {
    if state {
        KeyState::Pressed
    } else {
        KeyState::Released
    }
}

pub fn button_state(state: bool) -> ButtonState {
    if state {
        ButtonState::Pressed
    } else {
        ButtonState::Released
    }
}

pub fn smooth_axis_frame(dx: f64, dy: f64, flags: u32) -> fdo::Result<RemoteDesktopAxisFrame> {
    let source = axis_source(flags)?;
    let is_finished = flags & AXIS_FLAG_FINISH != 0;
    let amount = if is_finished { (0.0, 0.0) } else { (dx, dy) };

    Ok(RemoteDesktopAxisFrame {
        smooth: Some(amount),
        v120: None,
        source,
        stop: (is_finished, is_finished),
    })
}

pub fn discrete_axis_frame(axis: u32, steps: i32) -> fdo::Result<RemoteDesktopAxisFrame> {
    let value = f64::from(steps) * 120.0;
    let v120 = match axis {
        0 => (0.0, value),
        1 => (value, 0.0),
        _ => {
            return Err(fdo::Error::InvalidArgs(
                "remote desktop discrete axis must be 0 or 1".to_owned(),
            ));
        }
    };

    Ok(RemoteDesktopAxisFrame {
        smooth: None,
        v120: Some(v120),
        source: AxisSource::Wheel,
        stop: (false, false),
    })
}

fn axis_source(flags: u32) -> fdo::Result<AxisSource> {
    match flags & SOURCE_FLAGS {
        0 | AXIS_FLAG_SOURCE_FINGER => Ok(AxisSource::Finger),
        AXIS_FLAG_SOURCE_WHEEL => Ok(AxisSource::Wheel),
        AXIS_FLAG_SOURCE_CONTINUOUS => Ok(AxisSource::Continuous),
        _ => Err(fdo::Error::InvalidArgs(
            "remote desktop axis flags contain multiple sources".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_axis_frame_defaults_to_finger_when_no_source_flag_is_set() {
        let frame = smooth_axis_frame(2.0, -3.0, 0).unwrap();

        assert_eq!(frame.source, AxisSource::Finger);
        assert_eq!(frame.smooth, Some((2.0, -3.0)));
        assert_eq!(frame.stop, (false, false));
    }

    #[test]
    fn smooth_axis_frame_converts_finish_flag_to_zero_stop_frame() {
        let frame = smooth_axis_frame(2.0, -3.0, AXIS_FLAG_FINISH).unwrap();

        assert_eq!(frame.source, AxisSource::Finger);
        assert_eq!(frame.smooth, Some((0.0, 0.0)));
        assert_eq!(frame.stop, (true, true));
    }

    #[test]
    fn smooth_axis_frame_rejects_multiple_source_flags() {
        let err = smooth_axis_frame(
            0.0,
            0.0,
            AXIS_FLAG_SOURCE_WHEEL | AXIS_FLAG_SOURCE_CONTINUOUS,
        )
        .unwrap_err();

        assert!(format!("{err:?}").contains("multiple sources"));
    }

    #[test]
    fn discrete_axis_frame_converts_steps_to_v120_wheel_scroll() {
        let vertical = discrete_axis_frame(0, -2).unwrap();
        let horizontal = discrete_axis_frame(1, 3).unwrap();

        assert_eq!(vertical.source, AxisSource::Wheel);
        assert_eq!(vertical.v120, Some((0.0, -240.0)));
        assert_eq!(horizontal.v120, Some((360.0, 0.0)));
    }
}
