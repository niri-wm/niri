use std::time::Duration;

use smithay::input::pointer::{
    AxisFrame, ButtonEvent, CursorImageStatus, GestureHoldBeginEvent, GestureHoldEndEvent,
    GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
    GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
    MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
};
use smithay::input::SeatHandler;
use smithay::output::Output;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use crate::layout::workspace::WorkspaceId;
use crate::niri::State;
use crate::utils::get_monotonic_time;

pub struct SpatialMovementGrab {
    start_data: PointerGrabStartData<State>,
    last_location: Point<f64, Logical>,
    output: Output,
    workspace_id: WorkspaceId,
    gesture: GestureState,

    // Accumulated and applied in frame().
    new_location: Point<f64, Logical>,
    event_timestamp: Option<Duration>,
    relative_delta: Option<Point<f64, Logical>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GestureState {
    Recognizing,
    ViewOffset,
    WorkspaceSwitch,
}

impl SpatialMovementGrab {
    pub fn new(
        start_data: PointerGrabStartData<State>,
        output: Output,
        workspace_id: WorkspaceId,
        is_view_offset: bool,
    ) -> Self {
        let location = start_data.location;
        let gesture = if is_view_offset {
            GestureState::ViewOffset
        } else {
            GestureState::Recognizing
        };

        Self {
            last_location: location,
            start_data,
            output,
            workspace_id,
            gesture,
            new_location: location,
            event_timestamp: None,
            relative_delta: None,
        }
    }

    pub fn view_offset_output(&self) -> Option<&Output> {
        (self.gesture == GestureState::ViewOffset).then_some(&self.output)
    }

    pub fn workspace_switch_output(&self) -> Option<&Output> {
        (self.gesture == GestureState::WorkspaceSwitch).then_some(&self.output)
    }

    fn on_frame(&mut self, data: &mut State) -> bool {
        let Some(timestamp) = self.event_timestamp.take() else {
            return true;
        };

        let delta = self
            .relative_delta
            .take()
            .unwrap_or(self.new_location - self.last_location);
        self.last_location = self.new_location;

        let layout = &mut data.niri.layout;
        let res = match self.gesture {
            GestureState::Recognizing => {
                let c = self.new_location - self.start_data.location;

                // Check if the gesture moved far enough to decide. Threshold copied from GTK 4.
                if c.x * c.x + c.y * c.y >= 8. * 8. {
                    if c.x.abs() > c.y.abs() {
                        self.gesture = GestureState::ViewOffset;
                        if let Some((ws_idx, ws)) = layout.find_workspace_by_id(self.workspace_id) {
                            if ws.current_output() == Some(&self.output) {
                                layout.view_offset_gesture_begin(&self.output, Some(ws_idx), false);
                                layout.view_offset_gesture_update(-c.x, timestamp, false)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        self.gesture = GestureState::WorkspaceSwitch;
                        layout.workspace_switch_gesture_begin(&self.output, false);
                        layout.workspace_switch_gesture_update(-c.y, timestamp, false)
                    }
                } else {
                    Some(None)
                }
            }
            GestureState::ViewOffset => {
                layout.view_offset_gesture_update(-delta.x, timestamp, false)
            }
            GestureState::WorkspaceSwitch => {
                layout.workspace_switch_gesture_update(-delta.y, timestamp, false)
            }
        };

        if let Some(output) = res {
            if let Some(output) = output {
                data.niri.queue_redraw(&output);
            }
            true
        } else {
            false
        }
    }

    fn on_ungrab(&mut self, state: &mut State) {
        let layout = &mut state.niri.layout;
        let res = match self.gesture {
            GestureState::Recognizing => None,
            GestureState::ViewOffset => layout.view_offset_gesture_end(Some(false)),
            GestureState::WorkspaceSwitch => layout.workspace_switch_gesture_end(Some(false)),
        };

        if let Some(output) = res {
            state.niri.queue_redraw(&output);
        }

        state
            .niri
            .cursor_manager
            .set_cursor_image(CursorImageStatus::default_named());
    }
}

impl PointerGrab<State> for SpatialMovementGrab {
    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus.
        handle.motion(data, None, event);

        self.new_location = event.location;

        // Relative motion takes precedence over normal motion.
        if self.relative_delta.is_none() {
            self.event_timestamp = Some(Duration::from_millis(u64::from(event.time)));
        }
    }

    fn relative_motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        // While the grab is active, no client has pointer focus.
        handle.relative_motion(data, None, event);

        *self.relative_delta.get_or_insert_default() += event.delta;
        self.event_timestamp = Some(Duration::from_micros(event.utime));
    }

    fn button(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        if handle.current_pressed().is_empty() {
            // No more buttons are pressed, release the grab.
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        handle.frame(data);

        if !self.on_frame(data) {
            // The gesture is no longer ongoing.
            handle.unset_grab(
                self,
                data,
                SERIAL_COUNTER.next_serial(),
                get_monotonic_time().as_millis() as u32,
                true,
            );
        }
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &PointerGrabStartData<State> {
        &self.start_data
    }

    fn unset(&mut self, data: &mut State) {
        self.on_ungrab(data);
    }
}
