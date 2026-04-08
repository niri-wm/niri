use std::time::Duration;

use smithay::backend::input::ButtonState;
use smithay::desktop::Window;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, CursorIcon, CursorImageStatus, GestureHoldBeginEvent,
    GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
    GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
    GrabStartData as PointerGrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
    RelativeMotionEvent,
};
use smithay::input::touch::{
    self, GrabStartData as TouchGrabStartData, TouchGrab, TouchInnerHandle,
};
use smithay::input::SeatHandler;
use smithay::output::Output;
use smithay::utils::{IsAlive, Logical, Point, Serial, SERIAL_COUNTER};

use crate::input::PointerOrTouchStartData;
use crate::niri::State;
use crate::utils::get_monotonic_time;

pub struct MoveGrab {
    start_data: PointerOrTouchStartData<State>,
    start_output: Output,
    start_pos_within_output: Point<f64, Logical>,
    last_location: Point<f64, Logical>,
    window: Window,
    gesture: GestureState,
    enable_view_offset: bool,
    move_icon: CursorIcon,

    // Accumulated and applied in frame().
    new_location: Point<f64, Logical>,
    event_timestamp: Option<Duration>,
    relative_delta: Option<Point<f64, Logical>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GestureState {
    Recognizing,
    Move,
    ViewOffset,
}

impl MoveGrab {
    pub fn new(
        state: &mut State,
        start_data: PointerOrTouchStartData<State>,
        window: Window,
        enable_view_offset: bool,
        move_icon: Option<CursorIcon>,
    ) -> Option<Self> {
        let location = start_data.location();
        let (output, pos_within_output) = state.niri.output_under(location)?;

        Some(Self {
            last_location: location,
            start_data,
            start_output: output.clone(),
            start_pos_within_output: pos_within_output,
            window,
            gesture: GestureState::Recognizing,
            enable_view_offset,
            // Moving windows by their titlebars uses the default cursor by default.
            move_icon: move_icon.unwrap_or(CursorIcon::Default),
            new_location: location,
            event_timestamp: None,
            relative_delta: None,
        })
    }

    pub fn is_move(&self) -> bool {
        self.gesture == GestureState::Move
    }

    pub fn view_offset_output(&self) -> Option<&Output> {
        (self.gesture == GestureState::ViewOffset).then_some(&self.start_output)
    }

    fn on_ungrab(&mut self, data: &mut State) {
        let layout = &mut data.niri.layout;
        match self.gesture {
            GestureState::Recognizing => {
                // Activate the window on release. This is most prominent in the overview where
                // windows are not activated on click. In the overview, we also try to do a nice
                // synchronized workspace animation.
                if layout.is_overview_open() {
                    let res = layout.workspaces().find_map(|(mon, ws_idx, ws)| {
                        ws.windows()
                            .any(|w| w.window == self.window)
                            .then(|| (mon.map(|mon| mon.output().clone()), ws_idx))
                    });
                    if let Some((Some(output), ws_idx)) = res {
                        layout.focus_output(&output);
                        layout.toggle_overview_to_workspace(ws_idx);
                    }
                }

                layout.activate_window(&self.window);
            }
            GestureState::Move => layout.interactive_move_end(&self.window),
            GestureState::ViewOffset => {
                layout.view_offset_gesture_end(Some(false));
            }
        }

        if self.start_data.is_pointer() {
            data.niri
                .cursor_manager
                .set_cursor_image(CursorImageStatus::default_named());
        }

        // FIXME: only redraw the window output.
        data.niri.queue_redraw_all();
    }

    fn begin_move(&mut self, data: &mut State) -> bool {
        if !data.niri.layout.interactive_move_begin(
            self.window.clone(),
            &self.start_output,
            self.start_pos_within_output,
        ) {
            // Can no longer start the move.
            return false;
        }

        self.gesture = GestureState::Move;

        if self.start_data.is_pointer() {
            data.niri
                .cursor_manager
                .set_cursor_image(CursorImageStatus::Named(self.move_icon));
        }

        true
    }

    fn begin_view_offset(&mut self, data: &mut State) -> bool {
        let layout = &mut data.niri.layout;
        let Some(ws_idx) = layout.workspaces().find_map(|(mon, ws_idx, ws)| {
            let ws_idx = ws
                .windows()
                .any(|w| w.window == self.window)
                .then_some(ws_idx)?;
            let output = mon?.output();

            // If the window moved to a different output, don't start the gesture.
            if *output != self.start_output {
                return None;
            }

            Some(ws_idx)
        }) else {
            // Can no longer start the gesture.
            return false;
        };

        layout.view_offset_gesture_begin(&self.start_output, Some(ws_idx), false);

        self.gesture = GestureState::ViewOffset;

        if self.start_data.is_pointer() {
            data.niri
                .cursor_manager
                .set_cursor_image(CursorImageStatus::Named(CursorIcon::AllScroll));
        }

        true
    }

    fn on_frame(&mut self, data: &mut State) -> bool {
        let Some(timestamp) = self.event_timestamp.take() else {
            return true;
        };

        let mut delta = self.new_location - self.last_location;
        let mut relative_delta = self.relative_delta.take().unwrap_or(delta);
        self.last_location = self.new_location;

        // Try to recognize the gesture.
        if self.gesture == GestureState::Recognizing {
            // Check if the window has closed.
            if !self.window.alive() {
                return false;
            }

            // Check if the gesture moved far enough to decide.
            let c = self.new_location - self.start_data.location();
            if c.x * c.x + c.y * c.y >= 8. * 8. {
                let is_floating = data
                    .niri
                    .layout
                    .workspaces()
                    .find_map(|(_, _, ws)| {
                        ws.windows()
                            .any(|w| w.window == self.window)
                            .then(|| ws.is_floating(&self.window))
                    })
                    .unwrap_or(false);

                let is_view_offset =
                    self.enable_view_offset && !is_floating && c.x.abs() > c.y.abs();

                let started = if is_view_offset {
                    self.begin_view_offset(data)
                } else {
                    self.begin_move(data)
                };
                if !started {
                    return false;
                }

                // Apply the whole delta that accumulated during recognizing.
                delta = c;
                relative_delta = c;
            }
        }

        match self.gesture {
            GestureState::Recognizing => return true,
            GestureState::Move => {
                let Some((output, pos_within_output)) = data.niri.output_under(self.last_location)
                else {
                    return true;
                };
                let output = output.clone();

                // Interactive move always uses absolute delta since the window must remain pinned
                // to the cursor even when it's clamped to monitor bounds.
                let ongoing = data.niri.layout.interactive_move_update(
                    &self.window,
                    delta,
                    output,
                    pos_within_output,
                );
                if ongoing {
                    // FIXME: only redraw the previous and the new output.
                    data.niri.queue_redraw_all();
                    return true;
                }
            }
            GestureState::ViewOffset => {
                let res = data.niri.layout.view_offset_gesture_update(
                    -relative_delta.x,
                    timestamp,
                    false,
                );
                if let Some(output) = res {
                    if let Some(output) = output {
                        data.niri.queue_redraw(&output);
                    }
                    return true;
                }
            }
        }

        false
    }

    fn on_toggle_floating(&mut self, data: &mut State) -> bool {
        if self.gesture == GestureState::ViewOffset {
            return true;
        }

        // Start move if still recognizing.
        if self.gesture == GestureState::Recognizing {
            let Some((output, pos_within_output)) = data.niri.output_under(self.last_location)
            else {
                return false;
            };
            let output = output.clone();

            if !self.begin_move(data) {
                return false;
            }

            // Apply the delta accumulated during recognizing.
            let ongoing = data.niri.layout.interactive_move_update(
                &self.window,
                self.last_location - self.start_data.location(),
                output,
                pos_within_output,
            );
            if !ongoing {
                return false;
            }
        }

        data.niri.layout.toggle_window_floating(Some(&self.window));
        data.niri.queue_redraw_all();

        true
    }
}

impl PointerGrab<State> for MoveGrab {
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

        let start_data = self.start_data.unwrap_pointer();

        if !handle.current_pressed().contains(&start_data.button) {
            // The button that initiated the grab was released.
            handle.unset_grab(self, data, event.serial, event.time, true);
            return;
        }

        // When moving with the left button, right toggles floating, and vice versa.
        let toggle_floating_button = if start_data.button == 0x110 {
            0x111
        } else {
            0x110
        };
        if event.state != ButtonState::Pressed || event.button != toggle_floating_button {
            return;
        }

        if !self.on_toggle_floating(data) {
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
        self.start_data.unwrap_pointer()
    }

    fn unset(&mut self, data: &mut State) {
        self.on_ungrab(data);
    }
}

impl TouchGrab<State> for MoveGrab {
    fn down(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::TouchFocus, Point<f64, Logical>)>,
        event: &touch::DownEvent,
        seq: Serial,
    ) {
        handle.down(data, None, event, seq);

        if event.slot == self.start_data.unwrap_touch().slot {
            return;
        }

        if !self.on_toggle_floating(data) {
            handle.unset_grab(self, data);
        }
    }

    fn up(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        event: &touch::UpEvent,
        seq: Serial,
    ) {
        handle.up(data, event, seq);

        if event.slot == self.start_data.unwrap_touch().slot {
            handle.unset_grab(self, data);
        }
    }

    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::TouchFocus, Point<f64, Logical>)>,
        event: &touch::MotionEvent,
        seq: Serial,
    ) {
        handle.motion(data, None, event, seq);

        if event.slot != self.start_data.unwrap_touch().slot {
            return;
        }

        self.new_location = event.location;
        self.event_timestamp = Some(Duration::from_millis(u64::from(event.time)));
    }

    fn frame(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>, seq: Serial) {
        handle.frame(data, seq);

        if !self.on_frame(data) {
            // The gesture is no longer ongoing.
            handle.unset_grab(self, data);
        }
    }

    fn cancel(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>, seq: Serial) {
        handle.cancel(data, seq);
        handle.unset_grab(self, data);
    }

    fn shape(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        event: &touch::ShapeEvent,
        seq: Serial,
    ) {
        handle.shape(data, event, seq);
    }

    fn orientation(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        event: &touch::OrientationEvent,
        seq: Serial,
    ) {
        handle.orientation(data, event, seq);
    }

    fn start_data(&self) -> &TouchGrabStartData<State> {
        self.start_data.unwrap_touch()
    }

    fn unset(&mut self, data: &mut State) {
        self.on_ungrab(data);
    }
}
