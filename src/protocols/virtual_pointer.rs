use std::collections::HashSet;
use std::sync::Mutex;

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisRelativeDirection, AxisSource, ButtonState, Device,
    DeviceCapability, Event, InputBackend, PointerAxisEvent, PointerButtonEvent,
    PointerMotionAbsoluteEvent, PointerMotionEvent, UnusedEvent,
};
use smithay::input::pointer::AxisFrame;
use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr;
use smithay::reexports::wayland_server::protocol::wl_pointer;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use wayland_backend::protocol::WEnum;
use wayland_protocols_wlr::virtual_pointer::v1::server::{
    zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
};
use zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

const VERSION: u32 = 2;

pub struct VirtualPointerManagerState {
    virtual_pointers: HashSet<ZwlrVirtualPointerV1>,
}

pub struct VirtualPointerManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub struct VirtualPointerInputBackend;

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct VirtualPointer {
    pointer: ZwlrVirtualPointerV1,
}

#[derive(Debug)]
pub struct VirtualPointerUserData {
    seat: Option<WlSeat>,
    output: Option<Output>,

    axis_frame: Mutex<Option<AxisFrame>>,
}

impl VirtualPointer {
    fn data(&self) -> &VirtualPointerUserData {
        self.pointer.data().unwrap()
    }

    pub fn seat(&self) -> Option<&WlSeat> {
        self.data().seat.as_ref()
    }

    pub fn output(&self) -> Option<&Output> {
        self.data().output.as_ref()
    }

    fn finish_axis_frame(&self) -> Option<AxisFrame> {
        self.data().axis_frame.lock().unwrap().take()
    }

    fn mutate_axis_frame(&self, time: Option<u32>, f: impl FnOnce(AxisFrame) -> AxisFrame) {
        let mut frame = self.data().axis_frame.lock().unwrap();

        *frame = frame.or(time.map(AxisFrame::new)).map(f);
    }
}

impl Device for VirtualPointer {
    fn id(&self) -> String {
        format!("wlr virtual pointer {}", self.pointer.id())
    }

    fn name(&self) -> String {
        String::from("virtual pointer")
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        matches!(capability, DeviceCapability::Pointer)
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<std::path::PathBuf> {
        None
    }
}

pub struct VirtualPointerMotionEvent {
    pointer: VirtualPointer,
    time: u32,
    dx: f64,
    dy: f64,
}

impl Event<VirtualPointerInputBackend> for VirtualPointerMotionEvent {
    fn time(&self) -> u64 {
        self.time as u64 * 1000 // millis to micros
    }

    fn device(&self) -> VirtualPointer {
        self.pointer.clone()
    }
}

impl PointerMotionEvent<VirtualPointerInputBackend> for VirtualPointerMotionEvent {
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

pub struct VirtualPointerMotionAbsoluteEvent {
    pointer: VirtualPointer,
    time: u32,
    x: u32,
    y: u32,
    x_extent: u32,
    y_extent: u32,
}

impl Event<VirtualPointerInputBackend> for VirtualPointerMotionAbsoluteEvent {
    fn time(&self) -> u64 {
        self.time as u64 * 1000 // millis to micros
    }

    fn device(&self) -> VirtualPointer {
        self.pointer.clone()
    }
}

impl AbsolutePositionEvent<VirtualPointerInputBackend> for VirtualPointerMotionAbsoluteEvent {
    fn x(&self) -> f64 {
        self.x as f64 / self.x_extent as f64
    }

    fn y(&self) -> f64 {
        self.y as f64 / self.y_extent as f64
    }

    fn x_transformed(&self, width: i32) -> f64 {
        (self.x as i64 * width as i64) as f64 / self.x_extent as f64
    }

    fn y_transformed(&self, height: i32) -> f64 {
        (self.y as i64 * height as i64) as f64 / self.y_extent as f64
    }
}

pub struct VirtualPointerButtonEvent {
    pointer: VirtualPointer,
    time: u32,
    button: u32,
    state: ButtonState,
}

impl Event<VirtualPointerInputBackend> for VirtualPointerButtonEvent {
    fn time(&self) -> u64 {
        self.time as u64 * 1000 // millis to micros
    }

    fn device(&self) -> VirtualPointer {
        self.pointer.clone()
    }
}

impl PointerButtonEvent<VirtualPointerInputBackend> for VirtualPointerButtonEvent {
    fn button_code(&self) -> u32 {
        self.button
    }

    fn state(&self) -> ButtonState {
        self.state
    }
}

pub struct VirtualPointerAxisEvent {
    pointer: VirtualPointer,
    frame: AxisFrame,
}

impl Event<VirtualPointerInputBackend> for VirtualPointerAxisEvent {
    fn time(&self) -> u64 {
        self.frame.time as u64 * 1000 // millis to micros
    }

    fn device(&self) -> VirtualPointer {
        self.pointer.clone()
    }
}

fn tuple_axis<T>(tuple: (T, T), axis: Axis) -> T {
    match axis {
        Axis::Horizontal => tuple.0,
        Axis::Vertical => tuple.1,
    }
}

impl PointerAxisEvent<VirtualPointerInputBackend> for VirtualPointerAxisEvent {
    fn amount(&self, axis: Axis) -> Option<f64> {
        Some(tuple_axis(self.frame.axis, axis))
    }

    fn amount_v120(&self, axis: Axis) -> Option<f64> {
        self.frame.v120.map(|v120| tuple_axis(v120, axis) as f64)
    }

    fn source(&self) -> AxisSource {
        self.frame.source.unwrap_or_else(|| {
            warn!("AxisSource: no source set, giving bogus value");
            AxisSource::Continuous
        })
    }

    fn relative_direction(&self, axis: Axis) -> AxisRelativeDirection {
        tuple_axis(self.frame.relative_direction, axis)
    }
}

impl PointerMotionAbsoluteEvent<VirtualPointerInputBackend> for VirtualPointerMotionAbsoluteEvent {}

impl InputBackend for VirtualPointerInputBackend {
    type Device = VirtualPointer;

    type KeyboardKeyEvent = UnusedEvent;
    type PointerAxisEvent = VirtualPointerAxisEvent;
    type PointerButtonEvent = VirtualPointerButtonEvent;
    type PointerMotionEvent = VirtualPointerMotionEvent;
    type PointerMotionAbsoluteEvent = VirtualPointerMotionAbsoluteEvent;

    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;

    type TouchDownEvent = UnusedEvent;
    type TouchUpEvent = UnusedEvent;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;

    type SwitchToggleEvent = UnusedEvent;

    type SpecialEvent = UnusedEvent;
}

pub trait VirtualPointerHandler {
    fn virtual_pointer_manager_state(&mut self) -> &mut VirtualPointerManagerState;

    fn create_virtual_pointer(&mut self, pointer: VirtualPointer) {
        let _ = pointer;
    }
    fn destroy_virtual_pointer(&mut self, pointer: VirtualPointer) {
        let _ = pointer;
    }

    fn on_virtual_pointer_motion(&mut self, event: VirtualPointerMotionEvent);
    fn on_virtual_pointer_motion_absolute(&mut self, event: VirtualPointerMotionAbsoluteEvent);
    fn on_virtual_pointer_button(&mut self, event: VirtualPointerButtonEvent);
    fn on_virtual_pointer_axis(&mut self, event: VirtualPointerAxisEvent);
}

impl VirtualPointerManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrVirtualPointerManagerV1, VirtualPointerManagerGlobalData>,
        D: Dispatch<ZwlrVirtualPointerManagerV1, ()>,
        D: Dispatch<ZwlrVirtualPointerV1, VirtualPointerUserData>,
        D: VirtualPointerHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = VirtualPointerManagerGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, ZwlrVirtualPointerManagerV1, _>(VERSION, global_data);

        Self {
            virtual_pointers: HashSet::new(),
        }
    }
}

impl<D> GlobalDispatch<ZwlrVirtualPointerManagerV1, VirtualPointerManagerGlobalData, D>
    for VirtualPointerManagerState
where
    D: GlobalDispatch<ZwlrVirtualPointerManagerV1, VirtualPointerManagerGlobalData>,
    D: Dispatch<ZwlrVirtualPointerManagerV1, ()>,
    D: Dispatch<ZwlrVirtualPointerV1, VirtualPointerUserData>,
    D: VirtualPointerHandler,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<ZwlrVirtualPointerManagerV1>,
        _manager_state: &VirtualPointerManagerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ());
    }

    fn can_view(client: Client, global_data: &VirtualPointerManagerGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ZwlrVirtualPointerManagerV1, (), D> for VirtualPointerManagerState
where
    D: Dispatch<ZwlrVirtualPointerManagerV1, ()>,
    D: Dispatch<ZwlrVirtualPointerV1, VirtualPointerUserData>,
    D: VirtualPointerHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &ZwlrVirtualPointerManagerV1,
        request: <ZwlrVirtualPointerManagerV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let (id, seat, output) = match request {
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointer { seat, id } => {
                (id, seat, None)
            }
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointerWithOutput {
                seat,
                output,
                id,
            } => (id, seat, output.as_ref().and_then(Output::from_resource)),
            zwlr_virtual_pointer_manager_v1::Request::Destroy => return,
            _ => unreachable!(),
        };

        let pointer = data_init.init(
            id,
            VirtualPointerUserData {
                seat,
                output,
                axis_frame: Mutex::new(None),
            },
        );
        state
            .virtual_pointer_manager_state()
            .virtual_pointers
            .insert(pointer.clone());

        state.create_virtual_pointer(VirtualPointer { pointer });
    }
}

impl<D> Dispatch<ZwlrVirtualPointerV1, VirtualPointerUserData, D> for VirtualPointerManagerState
where
    D: Dispatch<ZwlrVirtualPointerV1, VirtualPointerUserData>,
    D: VirtualPointerHandler,
    D: 'static,
{
    fn request(
        handler: &mut D,
        _client: &Client,
        resource: &ZwlrVirtualPointerV1,
        request: <ZwlrVirtualPointerV1 as Resource>::Request,
        _data: &VirtualPointerUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let pointer = VirtualPointer {
            pointer: resource.clone(),
        };
        match request {
            zwlr_virtual_pointer_v1::Request::Motion { time, dx, dy } => {
                let event = VirtualPointerMotionEvent {
                    pointer,
                    time,
                    dx,
                    dy,
                };
                handler.on_virtual_pointer_motion(event);
            }
            zwlr_virtual_pointer_v1::Request::MotionAbsolute {
                time,
                x,
                y,
                x_extent,
                y_extent,
            } => {
                let event = VirtualPointerMotionAbsoluteEvent {
                    pointer,
                    time,
                    x,
                    y,
                    x_extent,
                    y_extent,
                };
                handler.on_virtual_pointer_motion_absolute(event);
            }
            zwlr_virtual_pointer_v1::Request::Button {
                time,
                button,
                state,
            } => {
                // state is an enum but wlroots treats it as a C boolean (zero or nonzero)
                // so we emulate that behaviour too. ButtonState::Pressed and any invalid value
                // counts as pressed.
                // https://gitlab.freedesktop.org/wlroots/wlroots/-/blob/3187479c07c34a4de82c06a316a763a36a0499da/types/wlr_virtual_pointer_v1.c#L74
                let state = match state {
                    WEnum::Value(wl_pointer::ButtonState::Released) => ButtonState::Released,
                    _ => ButtonState::Pressed,
                };
                let event = VirtualPointerButtonEvent {
                    pointer,
                    time,
                    button,
                    state,
                };
                handler.on_virtual_pointer_button(event);
            }
            zwlr_virtual_pointer_v1::Request::Axis { time, axis, value } => {
                let axis = match axis {
                    WEnum::Value(wl_pointer::Axis::VerticalScroll) => Axis::Vertical,
                    WEnum::Value(wl_pointer::Axis::HorizontalScroll) => Axis::Horizontal,
                    _ => {
                        warn!("Axis: invalid axis");
                        resource.post_error(
                            zwlr_virtual_pointer_v1::Error::InvalidAxis,
                            "invalid axis",
                        );
                        return;
                    }
                };

                pointer.mutate_axis_frame(Some(time), |frame| frame.value(axis, value));
            }
            zwlr_virtual_pointer_v1::Request::Frame => {
                if let Some(frame) = pointer.finish_axis_frame() {
                    let event = VirtualPointerAxisEvent { pointer, frame };
                    handler.on_virtual_pointer_axis(event);
                }
            }
            zwlr_virtual_pointer_v1::Request::AxisSource { axis_source } => {
                let axis_source = match axis_source {
                    WEnum::Value(wl_pointer::AxisSource::Wheel) => AxisSource::Wheel,
                    WEnum::Value(wl_pointer::AxisSource::Finger) => AxisSource::Finger,
                    WEnum::Value(wl_pointer::AxisSource::Continuous) => AxisSource::Continuous,
                    WEnum::Value(wl_pointer::AxisSource::WheelTilt) => AxisSource::WheelTilt,

                    _ => {
                        warn!("AxisSource: invalid axis source");
                        resource.post_error(
                            zwlr_virtual_pointer_v1::Error::InvalidAxisSource,
                            "invalid axis source",
                        );
                        return;
                    }
                };

                pointer.mutate_axis_frame(None, |frame| frame.source(axis_source));
            }
            zwlr_virtual_pointer_v1::Request::AxisStop { time, axis } => {
                let axis = match axis {
                    WEnum::Value(wl_pointer::Axis::VerticalScroll) => Axis::Vertical,
                    WEnum::Value(wl_pointer::Axis::HorizontalScroll) => Axis::Horizontal,
                    _ => {
                        warn!("AxisStop: invalid axis");
                        resource.post_error(
                            zwlr_virtual_pointer_v1::Error::InvalidAxis,
                            "invalid axis",
                        );
                        return;
                    }
                };

                pointer.mutate_axis_frame(Some(time), |frame| frame.stop(axis));
            }
            zwlr_virtual_pointer_v1::Request::AxisDiscrete {
                time,
                axis,
                value,
                discrete,
            } => {
                let axis = match axis {
                    WEnum::Value(wl_pointer::Axis::VerticalScroll) => Axis::Vertical,
                    WEnum::Value(wl_pointer::Axis::HorizontalScroll) => Axis::Horizontal,
                    _ => {
                        warn!("AxisDiscrete: invalid axis");
                        resource.post_error(
                            zwlr_virtual_pointer_v1::Error::InvalidAxis,
                            "invalid axis",
                        );
                        return;
                    }
                };
                pointer.mutate_axis_frame(Some(time), |frame| {
                    frame.value(axis, value).v120(axis, discrete * 120)
                });
            }
            zwlr_virtual_pointer_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }

    fn destroyed(
        handler: &mut D,
        _client: wayland_backend::server::ClientId,
        resource: &ZwlrVirtualPointerV1,
        _data: &VirtualPointerUserData,
    ) {
        let pointer = VirtualPointer {
            pointer: resource.clone(),
        };

        handler.destroy_virtual_pointer(pointer);
        handler
            .virtual_pointer_manager_state()
            .virtual_pointers
            .remove(resource);
    }
}

#[macro_export]
macro_rules! delegate_virtual_pointer {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::server::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1: $crate::protocols::virtual_pointer::VirtualPointerManagerGlobalData
            ] => $crate::protocols::virtual_pointer::VirtualPointerManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::server::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1: ()
        ] => $crate::protocols::virtual_pointer::VirtualPointerManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::server::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1:  $crate::protocols::virtual_pointer::VirtualPointerUserData
        ] => $crate::protocols::virtual_pointer::VirtualPointerManagerState);
    };
}
