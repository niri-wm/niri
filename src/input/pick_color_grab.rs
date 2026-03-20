use niri_ipc::PickedColor;
use smithay::backend::allocator::Fourcc;
use smithay::backend::input::ButtonState;
use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
use smithay::backend::renderer::ExportMem as _;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, CursorImageStatus, GestureHoldBeginEvent, GestureHoldEndEvent,
    GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
    GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
    MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
};
use smithay::input::SeatHandler;
use smithay::utils::{Logical, Physical, Point, Scale, Size, Transform};

use crate::niri::State;
use crate::render_helpers::{render_and_download, RenderTarget};

pub struct PickColorGrab {
    start_data: PointerGrabStartData<State>,
}

impl PickColorGrab {
    pub fn new(start_data: PointerGrabStartData<State>) -> Self {
        Self { start_data }
    }

    fn on_ungrab(&mut self, state: &mut State) {
        if let Some(tx) = state.niri.pick_color.take() {
            let _ = tx.send_blocking(None);
        }
        state
            .niri
            .cursor_manager
            .set_cursor_image(CursorImageStatus::default_named());
        state.niri.queue_redraw_all();
    }

    fn pick_color_at_point(location: Point<f64, Logical>, data: &mut State) -> Option<PickedColor> {
        let (output, pos_within_output) = data.niri.output_under(location)?;
        let output = output.clone();

        data.backend
            .with_primary_renderer(|renderer| {
                data.niri.update_render_elements(Some(&output));

                let scale = Scale::from(output.current_scale().fractional_scale());
                // FIXME: perhaps replace floor with round once we figure out the pointer behavior
                // at the bottom/right edges of the monitors.
                let pos = pos_within_output.to_physical_precise_floor(scale);
                let size = Size::<i32, Physical>::from((1, 1));

                let elements = data.niri.render(
                    renderer,
                    &output,
                    false,
                    // This is an interactive operation so we can render without blocking out.
                    RenderTarget::Output,
                );

                let mapping = match render_and_download(
                    renderer,
                    size,
                    scale,
                    Transform::Normal,
                    Fourcc::Abgr8888,
                    elements.iter().rev().map(|elem| {
                        let offset = pos.upscale(-1);
                        RelocateRenderElement::from_element(elem, offset, Relocate::Relative)
                    }),
                ) {
                    Ok(mapping) => mapping,
                    Err(_) => return None,
                };
                let pixels = match renderer.map_texture(&mapping) {
                    Ok(pixels) => pixels,
                    Err(_) => return None,
                };

                if pixels.len() == 4 {
                    let rgb = [
                        f64::from(pixels[0]) / 255.0,
                        f64::from(pixels[1]) / 255.0,
                        f64::from(pixels[2]) / 255.0,
                    ];
                    Some(PickedColor { rgb })
                } else {
                    error!(
                        "unexpected pixel data length: {} (expected 4)",
                        pixels.len()
                    );
                    None
                }
            })
            .flatten()
    }
}

impl PointerGrab<State> for PickColorGrab {
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

        if let Some(tx) = data.niri.pick_color.take() {
            let color = Self::pick_color_at_point(handle.current_location(), data);
            let _ = tx.send_blocking(color);
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
