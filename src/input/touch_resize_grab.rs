use smithay::desktop::Window;
use smithay::input::touch::{
    DownEvent, GrabStartData as TouchGrabStartData, MotionEvent, OrientationEvent, ShapeEvent,
    TouchGrab, TouchInnerHandle, UpEvent,
};
use smithay::input::SeatHandler;
use smithay::utils::{IsAlive, Logical, Point};

use crate::niri::State;

pub struct TouchResizeGrab {
    start_data: TouchGrabStartData<State>,
    window: Window,
}

impl TouchResizeGrab {
    pub fn new(start_data: TouchGrabStartData<State>, window: Window) -> Self {
        Self { start_data, window }
    }

    fn on_ungrab(&mut self, state: &mut State) {
        state.niri.layout.interactive_resize_end(&self.window);
    }
}

impl TouchGrab<State> for TouchResizeGrab {
    fn down(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::TouchFocus, Point<f64, Logical>)>,
        event: &DownEvent,
    ) {
        handle.down(data, None, event);
    }

    fn up(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>, event: &UpEvent) {
        handle.up(data, event);

        if event.slot != self.start_data.slot {
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

        if event.slot != self.start_data.slot {
            return;
        }

        if self.window.alive() {
            let delta = event.location - self.start_data.location;
            let ongoing = data
                .niri
                .layout
                .interactive_resize_update(&self.window, delta);
            if ongoing {
                return;
            }
        }

        // The resize is no longer ongoing.
        handle.unset_grab(self, data);
    }

    fn frame(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>) {
        handle.frame(data);
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
        &self.start_data
    }

    fn unset(&mut self, data: &mut State) {
        self.on_ungrab(data);
    }
}
