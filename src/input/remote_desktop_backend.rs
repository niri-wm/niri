//! Input backend for emulated input events received directly via `org.gnome.Mutter.RemoteDesktop`
//! without EI.

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisRelativeDirection, AxisSource, ButtonState, Device,
    DeviceCapability, Event, InputBackend, KeyState, KeyboardKeyEvent, Keycode, PointerAxisEvent,
    PointerButtonEvent, PointerMotionAbsoluteEvent, PointerMotionEvent, TouchCancelEvent,
    TouchDownEvent, TouchEvent, TouchFrameEvent, TouchMotionEvent, TouchUpEvent, UnusedEvent,
};
use smithay::output::Output;
use smithay::utils::Point;

use crate::input::backend_ext::NiriInputDevice;
use crate::niri::State;
use crate::utils::RemoteDesktopSessionId;

pub struct RdInputBackend;

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct RdVirtualDevice {
    /// Remote desktop session ID
    pub session_id: RemoteDesktopSessionId,
}

impl InputBackend for RdInputBackend {
    type Device = RdVirtualDevice;

    type KeyboardKeyEvent = RdEventAdapter<RdKeyboardKeyEvent>;

    type PointerAxisEvent = RdEventAdapter<RdPointerAxisEvent>;
    type PointerButtonEvent = RdEventAdapter<RdPointerButtonEvent>;
    type PointerMotionEvent = RdEventAdapter<RdPointerMotionEvent>;
    type PointerMotionAbsoluteEvent = RdEventAdapter<RdPointerMotionAbsoluteEvent>;

    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;

    type TouchDownEvent = RdEventAdapter<RdTouchEvent<RdAbsolutePosition>>;
    type TouchMotionEvent = RdEventAdapter<RdTouchEvent<RdAbsolutePosition>>;
    type TouchUpEvent = RdEventAdapter<RdTouchEvent<()>>;
    type TouchCancelEvent = RdEventAdapter<RdTouchEvent<()>>;
    type TouchFrameEvent = RdEventAdapter<RdTouchEvent<()>>;

    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;

    type SwitchToggleEvent = UnusedEvent;

    type SpecialEvent = UnusedEvent;
}

impl Device for RdVirtualDevice {
    fn id(&self) -> String {
        format!("Remote desktop virtual device {}", self.session_id)
    }

    fn name(&self) -> String {
        String::from("Remote desktop virtual device")
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        // TODO: only actual selected capabilities?
        matches!(
            capability,
            DeviceCapability::Keyboard | DeviceCapability::Pointer | DeviceCapability::Touch
        )
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<std::path::PathBuf> {
        None
    }
}

impl NiriInputDevice for RdVirtualDevice {
    fn output(&self, _state: &State) -> Option<Output> {
        // This would map local output coordinates to global output
        // coordinates if devices were per-output.
        None
    }
}

pub struct RdEventAdapter<Ev> {
    /// Remote desktop session ID
    pub session_id: RemoteDesktopSessionId,
    /// Timestamp in microseconds
    pub time: u64,
    pub inner: Ev,
}

impl<Ev> Event<RdInputBackend> for RdEventAdapter<Ev> {
    fn time(&self) -> u64 {
        self.time
    }

    fn device(&self) -> <RdInputBackend as InputBackend>::Device {
        RdVirtualDevice {
            session_id: self.session_id,
        }
    }
}

pub struct RdKeyboardKeyEvent {
    /// X11 keycode
    pub keycode: Keycode,
    pub state: KeyState,
}

impl KeyboardKeyEvent<RdInputBackend> for RdEventAdapter<RdKeyboardKeyEvent> {
    fn key_code(&self) -> Keycode {
        self.inner.keycode
    }

    fn state(&self) -> KeyState {
        self.inner.state
    }

    fn count(&self) -> u32 {
        // No idea
        1
    }
}

pub struct RdPointerAxisEvent {
    /// Note: set to AxisSource::Wheel for discrete axis events
    pub source: AxisSource,
    /// Continuous scrolling, like on a touchpad.
    pub delta: Option<(f64, f64)>,
    /// 120-notch scrolling, like on a traditional mouse wheel.
    pub discrete: Option<(i32, i32)>,
}

fn tuple_axis<T>(tuple: (T, T), axis: Axis) -> T {
    match axis {
        Axis::Horizontal => tuple.0,
        Axis::Vertical => tuple.1,
    }
}

impl PointerAxisEvent<RdInputBackend> for RdEventAdapter<RdPointerAxisEvent> {
    // TODO: test on both continuous and wheel
    fn amount(&self, axis: Axis) -> Option<f64> {
        Some(tuple_axis(self.inner.delta?, axis))
    }

    fn amount_v120(&self, axis: Axis) -> Option<f64> {
        // TODO: divide by 120?
        Some(tuple_axis(self.inner.discrete?, axis) as f64)
    }

    fn source(&self) -> AxisSource {
        self.inner.source
    }

    fn relative_direction(&self, _axis: Axis) -> AxisRelativeDirection {
        AxisRelativeDirection::Identical
    }
}

pub struct RdPointerButtonEvent {
    /// Evdev button code
    pub button: i32,
    /// True = pressed, false = released
    pub state: bool,
}

impl PointerButtonEvent<RdInputBackend> for RdEventAdapter<RdPointerButtonEvent> {
    fn button_code(&self) -> u32 {
        self.inner.button as u32
    }

    fn state(&self) -> ButtonState {
        match self.inner.state {
            false => ButtonState::Released,
            true => ButtonState::Pressed,
        }
    }
}

pub struct RdPointerMotionEvent {
    pub dx: f64,
    pub dy: f64,
}

impl PointerMotionEvent<RdInputBackend> for RdEventAdapter<RdPointerMotionEvent> {
    fn delta_x(&self) -> f64 {
        self.inner.dx
    }

    fn delta_y(&self) -> f64 {
        self.inner.dy
    }

    // Virtual pointer impl does this
    fn delta_x_unaccel(&self) -> f64 {
        self.inner.dx
    }

    fn delta_y_unaccel(&self) -> f64 {
        self.inner.dy
    }
}

pub struct RdPointerMotionAbsoluteEvent(pub RdAbsolutePosition);

impl AbsolutePositionEvent<RdInputBackend> for RdEventAdapter<RdPointerMotionAbsoluteEvent> {
    // Basically unused
    fn x(&self) -> f64 {
        self.inner.0.pos.x
    }

    fn y(&self) -> f64 {
        self.inner.0.pos.y
    }

    // Position in global bounding rectangle
    fn x_transformed(&self, width: i32) -> f64 {
        self.inner.0.pos.x * width as f64
    }

    fn y_transformed(&self, height: i32) -> f64 {
        self.inner.0.pos.y * height as f64
    }
}

impl PointerMotionAbsoluteEvent<RdInputBackend> for RdEventAdapter<RdPointerMotionAbsoluteEvent> {}

pub struct RdTouchEvent<Extra> {
    pub slot: u32,
    pub extra: Extra,
}

/// [`Point`] kind for points represented by numbers between 0 and 1.
pub struct UnitIntervalPointKind;

pub struct RdAbsolutePosition {
    /// Absolute position in the global bounding rectangle.
    pub pos: Point<f64, UnitIntervalPointKind>,
}

impl<Extra> TouchEvent<RdInputBackend> for RdEventAdapter<RdTouchEvent<Extra>> {
    fn slot(&self) -> smithay::backend::input::TouchSlot {
        Some(self.inner.slot).into()
    }
}

impl AbsolutePositionEvent<RdInputBackend> for RdEventAdapter<RdTouchEvent<RdAbsolutePosition>> {
    // Basically unused
    fn x(&self) -> f64 {
        self.inner.extra.pos.x
    }

    fn y(&self) -> f64 {
        self.inner.extra.pos.y
    }

    // Position in global bounding rectangle
    fn x_transformed(&self, width: i32) -> f64 {
        self.inner.extra.pos.x * width as f64
    }

    fn y_transformed(&self, height: i32) -> f64 {
        self.inner.extra.pos.y * height as f64
    }
}

impl TouchDownEvent<RdInputBackend> for RdEventAdapter<RdTouchEvent<RdAbsolutePosition>> {}
impl TouchMotionEvent<RdInputBackend> for RdEventAdapter<RdTouchEvent<RdAbsolutePosition>> {}
impl TouchUpEvent<RdInputBackend> for RdEventAdapter<RdTouchEvent<()>> {}
impl TouchCancelEvent<RdInputBackend> for RdEventAdapter<RdTouchEvent<()>> {}
impl TouchFrameEvent<RdInputBackend> for RdEventAdapter<RdTouchEvent<()>> {}
