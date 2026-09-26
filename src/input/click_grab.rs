use smithay::input::pointer::{
    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
    RelativeMotionEvent,
};
use smithay::input::SeatHandler;
use smithay::utils::{Logical, Point};

// ClickGrab from Smithay, but without the grab location updates.
//
// When clicking on a partially off-screen window, such that it will scroll into the view, updating
// the focus location will cause a move event, essentially a short DnD, which might result in some
// unintended action.
//
// Not updating in this case avoids any unintentional DnD. It does cause offset cursor movement if
// you hold and keep moving, but that can't cause mistakes as easily, so it's the lesser evil.
pub struct ClickGrab<D: SeatHandler> {
    start_data: GrabStartData<D>,
    focus: Option<(D::PointerFocus, Point<f64, Logical>)>,
}

impl<D: SeatHandler> ClickGrab<D> {
    pub fn new(start_data: GrabStartData<D>) -> Self {
        Self {
            focus: start_data.focus.clone(),
            start_data,
        }
    }
}

impl<D: SeatHandler + 'static> PointerGrab<D> for ClickGrab<D> {
    fn motion(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        _focus: Option<(<D as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, self.focus.clone(), event);
    }

    fn relative_motion(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        focus: Option<(<D as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            // no more buttons are pressed, release the grab
            handle.unset_grab(self, data, event.serial, event.time, false);
        }
    }

    fn axis(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>, details: AxisFrame) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &GrabStartData<D> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut D) {}
}
