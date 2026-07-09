use smithay::backend::input::ButtonState;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, CursorImageStatus, GestureHoldBeginEvent, GestureHoldEndEvent,
    GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
    GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
    MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
};
use smithay::input::SeatHandler;
use smithay::utils::{Logical, Point};

use crate::niri::State;
use crate::window::Mapped;

pub struct PickWindowGrab {
    start_data: PointerGrabStartData<State>,
}

impl PickWindowGrab {
    pub fn new(start_data: PointerGrabStartData<State>) -> Self {
        Self { start_data }
    }

    fn on_ungrab(&mut self, state: &mut State) {
        if let Some(tx) = state.niri.pick_window.take() {
            let _ = tx.send_blocking(None);
        }
        state
            .niri
            .cursor_manager
            .set_cursor_image(CursorImageStatus::default_named());
        // Redraw to update the cursor.
        state.niri.queue_redraw_all();
    }
}

impl PointerGrab<State> for PickWindowGrab {
    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);
    }

    fn relative_motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, None, event);
    }

    fn button(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        if event.state != ButtonState::Pressed {
            return;
        }

        // We're handling this press, don't send the release to the window.
        data.niri.suppressed_buttons.insert(event.button);

        if let Some(tx) = data.niri.pick_window.take() {
            let _ = tx.send_blocking(
                data.niri
                    .window_under(handle.current_location())
                    .map(Mapped::id),
            );
        }

        handle.unset_grab(self, data, event.serial, event.time, true);
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
