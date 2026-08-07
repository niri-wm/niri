use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisRelativeDirection, AxisSource, ButtonState, Event,
    InputBackend, KeyState, KeyboardKeyEvent, Keycode, PointerAxisEvent, PointerButtonEvent,
    PointerMotionAbsoluteEvent, PointerMotionEvent, TouchDownEvent, TouchEvent, TouchFrameEvent,
    TouchMotionEvent, TouchSlot, TouchUpEvent, UnusedEvent,
};

use super::device::{RemoteDesktopDevice, RemoteDesktopEventBase, RemoteDesktopInputBackend};
use super::{RemoteDesktopAxisFrame, RemoteDesktopPosition};

#[derive(Debug, Clone)]
pub(super) struct RemoteDesktopKeyboardEvent {
    pub(super) base: RemoteDesktopEventBase,
    pub(super) keycode: Keycode,
    pub(super) state: KeyState,
}

#[derive(Debug, Clone)]
pub(super) struct RemoteDesktopPointerMotionEvent {
    pub(super) base: RemoteDesktopEventBase,
    pub(super) dx: f64,
    pub(super) dy: f64,
}

#[derive(Debug, Clone)]
pub(super) struct RemoteDesktopPointerMotionAbsoluteEvent {
    pub(super) base: RemoteDesktopEventBase,
    pub(super) position: RemoteDesktopPosition,
}

#[derive(Debug, Clone)]
pub(super) struct RemoteDesktopPointerButtonEvent {
    pub(super) base: RemoteDesktopEventBase,
    pub(super) button: u32,
    pub(super) state: ButtonState,
}

#[derive(Debug, Clone)]
pub(super) struct RemoteDesktopPointerAxisEvent {
    pub(super) base: RemoteDesktopEventBase,
    pub(super) frame: RemoteDesktopAxisFrame,
}

#[derive(Debug, Clone)]
pub(super) struct RemoteDesktopTouchPositionEvent {
    pub(super) base: RemoteDesktopEventBase,
    pub(super) slot: u32,
    pub(super) position: RemoteDesktopPosition,
}

#[derive(Debug, Clone)]
pub(super) struct RemoteDesktopTouchSlotEvent {
    pub(super) base: RemoteDesktopEventBase,
    pub(super) slot: u32,
}

#[derive(Debug, Clone)]
pub(super) struct RemoteDesktopTouchFrameEvent {
    pub(super) base: RemoteDesktopEventBase,
}

fn tuple_axis<T: Copy>(tuple: (T, T), axis: Axis) -> T {
    match axis {
        Axis::Horizontal => tuple.0,
        Axis::Vertical => tuple.1,
    }
}

macro_rules! impl_event {
    ($ty:ty) => {
        impl Event<RemoteDesktopInputBackend> for $ty {
            fn time_msec(&self) -> u32 {
                self.base.time_msec
            }

            fn time(&self) -> u64 {
                u64::from(self.base.time_msec).saturating_mul(1000)
            }

            fn device(&self) -> RemoteDesktopDevice {
                self.base.device.clone()
            }
        }
    };
}

impl_event!(RemoteDesktopKeyboardEvent);
impl_event!(RemoteDesktopPointerMotionEvent);
impl_event!(RemoteDesktopPointerMotionAbsoluteEvent);
impl_event!(RemoteDesktopPointerButtonEvent);
impl_event!(RemoteDesktopPointerAxisEvent);
impl_event!(RemoteDesktopTouchPositionEvent);
impl_event!(RemoteDesktopTouchSlotEvent);
impl_event!(RemoteDesktopTouchFrameEvent);

impl KeyboardKeyEvent<RemoteDesktopInputBackend> for RemoteDesktopKeyboardEvent {
    fn key_code(&self) -> Keycode {
        self.keycode
    }

    fn state(&self) -> KeyState {
        self.state
    }

    fn count(&self) -> u32 {
        1
    }
}

impl PointerMotionEvent<RemoteDesktopInputBackend> for RemoteDesktopPointerMotionEvent {
    fn delta_x(&self) -> f64 {
        self.dx
    }

    fn delta_y(&self) -> f64 {
        self.dy
    }

    fn delta_x_unaccel(&self) -> f64 {
        self.dx
    }

    fn delta_y_unaccel(&self) -> f64 {
        self.dy
    }
}

impl AbsolutePositionEvent<RemoteDesktopInputBackend> for RemoteDesktopPointerMotionAbsoluteEvent {
    fn x(&self) -> f64 {
        self.position.x / f64::from(self.position.width)
    }

    fn y(&self) -> f64 {
        self.position.y / f64::from(self.position.height)
    }

    fn x_transformed(&self, width: i32) -> f64 {
        self.position.x * f64::from(width) / f64::from(self.position.width)
    }

    fn y_transformed(&self, height: i32) -> f64 {
        self.position.y * f64::from(height) / f64::from(self.position.height)
    }
}

impl PointerMotionAbsoluteEvent<RemoteDesktopInputBackend>
    for RemoteDesktopPointerMotionAbsoluteEvent
{
}

impl PointerButtonEvent<RemoteDesktopInputBackend> for RemoteDesktopPointerButtonEvent {
    fn button_code(&self) -> u32 {
        self.button
    }

    fn state(&self) -> ButtonState {
        self.state
    }
}

impl PointerAxisEvent<RemoteDesktopInputBackend> for RemoteDesktopPointerAxisEvent {
    fn amount(&self, axis: Axis) -> Option<f64> {
        if tuple_axis(self.frame.stop, axis) {
            return Some(0.0);
        }

        self.frame.smooth.map(|smooth| tuple_axis(smooth, axis))
    }

    fn amount_v120(&self, axis: Axis) -> Option<f64> {
        if tuple_axis(self.frame.stop, axis) {
            return None;
        }

        self.frame.v120.map(|v120| tuple_axis(v120, axis))
    }

    fn source(&self) -> AxisSource {
        self.frame.source
    }

    fn relative_direction(&self, _axis: Axis) -> AxisRelativeDirection {
        AxisRelativeDirection::Identical
    }
}

impl TouchEvent<RemoteDesktopInputBackend> for RemoteDesktopTouchPositionEvent {
    fn slot(&self) -> TouchSlot {
        TouchSlot::from(Some(self.slot))
    }
}

impl AbsolutePositionEvent<RemoteDesktopInputBackend> for RemoteDesktopTouchPositionEvent {
    fn x(&self) -> f64 {
        self.position.x / f64::from(self.position.width)
    }

    fn y(&self) -> f64 {
        self.position.y / f64::from(self.position.height)
    }

    fn x_transformed(&self, width: i32) -> f64 {
        self.position.x * f64::from(width) / f64::from(self.position.width)
    }

    fn y_transformed(&self, height: i32) -> f64 {
        self.position.y * f64::from(height) / f64::from(self.position.height)
    }
}

impl TouchDownEvent<RemoteDesktopInputBackend> for RemoteDesktopTouchPositionEvent {}
impl TouchMotionEvent<RemoteDesktopInputBackend> for RemoteDesktopTouchPositionEvent {}

impl TouchEvent<RemoteDesktopInputBackend> for RemoteDesktopTouchSlotEvent {
    fn slot(&self) -> TouchSlot {
        TouchSlot::from(Some(self.slot))
    }
}

impl TouchUpEvent<RemoteDesktopInputBackend> for RemoteDesktopTouchSlotEvent {}
impl TouchFrameEvent<RemoteDesktopInputBackend> for RemoteDesktopTouchFrameEvent {}

impl InputBackend for RemoteDesktopInputBackend {
    type Device = RemoteDesktopDevice;
    type KeyboardKeyEvent = RemoteDesktopKeyboardEvent;
    type PointerAxisEvent = RemoteDesktopPointerAxisEvent;
    type PointerButtonEvent = RemoteDesktopPointerButtonEvent;
    type PointerMotionEvent = RemoteDesktopPointerMotionEvent;
    type PointerMotionAbsoluteEvent = RemoteDesktopPointerMotionAbsoluteEvent;
    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;
    type TouchDownEvent = RemoteDesktopTouchPositionEvent;
    type TouchUpEvent = RemoteDesktopTouchSlotEvent;
    type TouchMotionEvent = RemoteDesktopTouchPositionEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = RemoteDesktopTouchFrameEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;
    type SwitchToggleEvent = UnusedEvent;
    type SpecialEvent = UnusedEvent;
}
