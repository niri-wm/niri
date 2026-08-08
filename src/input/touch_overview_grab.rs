use std::time::Duration;

use smithay::desktop::Window;
use smithay::input::pointer::{CursorIcon, CursorImageStatus};
use smithay::input::tablet::tool::{TabletToolGrab, TabletToolInnerHandle};
use smithay::input::tablet::TabletSeatHandler;
use smithay::input::touch::{
    DownEvent, GrabStartData as TouchGrabStartData, MotionEvent, OrientationEvent, ShapeEvent,
    TouchGrab, TouchInnerHandle, UpEvent,
};
use smithay::input::{tablet, SeatHandler};
use smithay::output::Output;
use smithay::utils::{IsAlive, Logical, Point, SERIAL_COUNTER};

use crate::input::AnyStartData;
use crate::layout::workspace::{Workspace, WorkspaceId};
use crate::niri::State;
use crate::utils::get_monotonic_time;
use crate::window::Mapped;

// When the touch is stationary for this much time, it becomes an interactive move.
const INTERACTIVE_MOVE_THRESHOLD: Duration = Duration::from_millis(500);

pub struct TouchOverviewGrab {
    start_data: AnyStartData<State>,
    start_timestamp: Duration,
    last_location: Point<f64, Logical>,
    output: Output,
    start_pos_within_output: Point<f64, Logical>,
    workspace_id: Option<WorkspaceId>,
    workspace_matched_narrow: bool,
    window: Option<Window>,
    gesture: GestureState,

    // Accumulated and applied in frame().
    new_location: Point<f64, Logical>,
    event_timestamp: Option<Duration>,
}

#[derive(Debug, Clone, Copy)]
enum GestureState {
    Recognizing,
    ViewOffset,
    WorkspaceSwitch,
    InteractiveMove,
}

impl TouchOverviewGrab {
    pub fn new(
        start_data: AnyStartData<State>,
        start_timestamp: Duration,
        output: Output,
        start_pos_within_output: Point<f64, Logical>,
        workspace_id: Option<WorkspaceId>,
        workspace_matched_narrow: bool,
        window: Option<Window>,
    ) -> Self {
        let location = start_data.location();

        Self {
            last_location: location,
            start_timestamp,
            start_data,
            output,
            start_pos_within_output,
            workspace_id,
            workspace_matched_narrow,
            window,
            gesture: GestureState::Recognizing,
            new_location: location,
            event_timestamp: None,
        }
    }

    fn on_frame(&mut self, data: &mut State) -> bool {
        let Some(timestamp) = self.event_timestamp.take() else {
            return true;
        };

        let layout = &mut data.niri.layout;

        // Check if we should become interactive move.
        if matches!(self.gesture, GestureState::Recognizing) {
            if let Some(window) = self.window.as_ref().filter(|win| win.alive()) {
                let passed = timestamp.saturating_sub(self.start_timestamp);
                if INTERACTIVE_MOVE_THRESHOLD <= passed
                    && layout.interactive_move_begin(
                        window.clone(),
                        &self.output,
                        self.start_pos_within_output,
                    )
                {
                    self.gesture = GestureState::InteractiveMove;

                    if !self.start_data.is_touch() {
                        data.niri
                            .cursor_manager
                            .set_cursor_image(CursorImageStatus::Named(CursorIcon::Grabbing));
                    }
                }
            }
        }

        // Check if we should become a spatial scroll.
        if matches!(self.gesture, GestureState::Recognizing) {
            let c = self.new_location - self.start_data.location();

            // Check if the gesture moved far enough to decide. Threshold copied from libadwaita.
            if c.x * c.x + c.y * c.y >= 16. * 16. {
                if let Some(ws_id) = self.workspace_id.filter(|_| c.x.abs() > c.y.abs()) {
                    if let Some((ws_idx, ws)) = layout.find_workspace_by_id(ws_id) {
                        if ws.current_output() == Some(&self.output) {
                            layout.view_offset_gesture_begin(&self.output, Some(ws_idx), false);
                            self.gesture = GestureState::ViewOffset;

                            if !self.start_data.is_touch() {
                                data.niri.cursor_manager.set_cursor_image(
                                    CursorImageStatus::Named(CursorIcon::AllScroll),
                                );
                            }
                        }
                    }
                }

                if matches!(self.gesture, GestureState::Recognizing) {
                    layout.workspace_switch_gesture_begin(&self.output, false);
                    self.gesture = GestureState::WorkspaceSwitch;

                    if !self.start_data.is_touch() {
                        data.niri
                            .cursor_manager
                            .set_cursor_image(CursorImageStatus::Named(CursorIcon::AllScroll));
                    }
                }
            }
        }

        // Do nothing if still recognizing.
        if matches!(self.gesture, GestureState::Recognizing) {
            return true;
        }

        let delta = self.new_location - self.last_location;
        self.last_location = self.new_location;

        let ongoing = match self.gesture {
            GestureState::Recognizing => unreachable!(),
            GestureState::ViewOffset => layout
                .view_offset_gesture_update(-delta.x, timestamp, false)
                .is_some(),
            GestureState::WorkspaceSwitch => layout
                .workspace_switch_gesture_update(-delta.y, timestamp, false)
                .is_some(),
            GestureState::InteractiveMove => {
                let window = self.window.as_ref().unwrap();
                if let Some((output, pos_within_output)) = data.niri.output_under(self.new_location)
                {
                    let output = output.clone();
                    data.niri.layout.interactive_move_update(
                        window,
                        delta,
                        output,
                        pos_within_output,
                    )
                } else {
                    false
                }
            }
        };

        if ongoing {
            data.niri.queue_redraw_all();
        }

        ongoing
    }

    fn on_ungrab(&mut self, state: &mut State) {
        let layout = &mut state.niri.layout;
        match self.gesture {
            GestureState::Recognizing => {
                // Tap to activate.
                layout.focus_output(&self.output);

                // Activate the workspace if necessary.
                if self.window.is_some() || self.workspace_matched_narrow {
                    // When activating a window, we want to activate the window's current
                    // workspace. Otherwise, find the workspace that we tapped on.
                    let ws_matches = |ws: &Workspace<Mapped>| {
                        if let Some(window) = &self.window {
                            ws.has_window(window)
                        } else if let Some(ws_id) = self.workspace_id {
                            ws.id() == ws_id
                        } else {
                            false
                        }
                    };

                    let ws_idx = if let Some((Some(mon), ws_idx, _)) =
                        layout.workspaces().find(|(_, _, ws)| ws_matches(ws))
                    {
                        // The workspace could've moved to a different output in the meantime.
                        (*mon.output() == self.output).then_some(ws_idx)
                    } else {
                        None
                    };

                    if let Some(ws_idx) = ws_idx {
                        layout.toggle_overview_to_workspace(ws_idx);
                    }
                }

                if let Some(window) = self.window.as_ref() {
                    layout.activate_window(window);
                }
            }
            GestureState::ViewOffset => {
                layout.view_offset_gesture_end(Some(false));
            }
            GestureState::WorkspaceSwitch => {
                layout.workspace_switch_gesture_end(Some(false));
            }
            GestureState::InteractiveMove => {
                layout.interactive_move_end(self.window.as_ref().unwrap());
            }
        };

        if !self.start_data.is_touch() {
            state
                .niri
                .cursor_manager
                .set_cursor_image(CursorImageStatus::default_named());
        }

        state.niri.queue_redraw_all();
    }
}

impl TouchGrab<State> for TouchOverviewGrab {
    fn down(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::TouchFocus, Point<f64, Logical>)>,
        event: &DownEvent,
    ) {
        handle.down(data, None, event);

        if event.slot == self.start_data.unwrap_touch().slot {
            return;
        }

        if matches!(self.gesture, GestureState::InteractiveMove) {
            if let Some(window) = &self.window.as_ref() {
                data.niri.layout.toggle_window_floating(Some(window));
                data.niri.queue_redraw_all();
            }
        }
    }

    fn up(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>, event: &UpEvent) {
        handle.up(data, event);

        if event.slot != self.start_data.unwrap_touch().slot {
            return;
        }

        handle.unset_grab(self, data);
    }

    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::TouchFocus, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);

        if event.slot != self.start_data.unwrap_touch().slot {
            return;
        }

        self.new_location = event.location;
        self.event_timestamp = Some(Duration::from_millis(u64::from(event.time)));
    }

    fn frame(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>) {
        handle.frame(data);

        if !self.on_frame(data) {
            // The gesture is no longer ongoing.
            handle.unset_grab(self, data);
        }
    }

    fn cancel(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>) {
        handle.cancel(data);
        handle.unset_grab(self, data);
    }

    fn shape(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        event: &ShapeEvent,
    ) {
        handle.shape(data, event);
    }

    fn orientation(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        event: &OrientationEvent,
    ) {
        handle.orientation(data, event);
    }

    fn start_data(&self) -> &TouchGrabStartData<State> {
        self.start_data.unwrap_touch()
    }

    fn unset(&mut self, data: &mut State) {
        self.on_ungrab(data);
    }
}

impl TabletToolGrab<State> for TouchOverviewGrab {
    fn start_data(&self) -> &tablet::tool::GrabStartData<State> {
        self.start_data.unwrap_tablet_tool()
    }

    fn proximity_out(
        &mut self,
        data: &mut State,
        handle: &mut TabletToolInnerHandle<'_, State>,
        event: &tablet::tool::ProximityOutEvent,
    ) {
        handle.proximity_out(data, event);
        handle.unset_grab(self, data, event.serial, event.time, false);
    }

    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut TabletToolInnerHandle<'_, State>,
        _focus: Option<(<State as TabletSeatHandler>::ToolFocus, Point<f64, Logical>)>,
        event: &tablet::tool::MotionEvent,
    ) {
        handle.motion(data, None, event);

        self.new_location = event.location;
        self.event_timestamp = Some(Duration::from_millis(u64::from(event.time)));
    }

    fn down(
        &mut self,
        data: &mut State,
        handle: &mut TabletToolInnerHandle<'_, State>,
        event: &tablet::tool::DownEvent,
    ) {
        handle.down(data, event);
    }

    fn up(
        &mut self,
        data: &mut State,
        handle: &mut TabletToolInnerHandle<'_, State>,
        event: &tablet::tool::UpEvent,
    ) {
        handle.up(data, event);
        handle.unset_grab(self, data, event.serial, event.time, true);
    }

    fn button(
        &mut self,
        data: &mut State,
        handle: &mut TabletToolInnerHandle<'_, State>,
        event: &tablet::tool::ButtonEvent,
    ) {
        handle.button(data, event);
    }

    fn axis(
        &mut self,
        data: &mut State,
        handle: &mut TabletToolInnerHandle<'_, State>,
        frame: tablet::tool::AxisFrame,
    ) {
        handle.axis(data, frame);
    }

    fn frame(
        &mut self,
        data: &mut State,
        handle: &mut TabletToolInnerHandle<'_, State>,
        time: u32,
    ) {
        handle.frame(data, time);

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

    fn unset(&mut self, data: &mut State) {
        self.on_ungrab(data);
    }
}
