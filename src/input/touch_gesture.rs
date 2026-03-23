//! Touchscreen multi-finger gesture handling.
//!
//! Processes 3+ finger touch gestures for workspace switching, view scrolling,
//! and overview toggling. Gesture recognition, sensitivity, finger count, and
//! natural scroll are all configurable per-gesture.

use std::cmp::min;
use std::time::Duration;

use smithay::backend::input::{Event as _, TouchEvent};
use smithay::input::touch::{
    DownEvent, GrabStartData as TouchGrabStartData, MotionEvent as TouchMotionEvent, UpEvent,
};
use smithay::utils::SERIAL_COUNTER;

use super::backend_ext::{NiriInputBackend as InputBackend, NiriInputDevice as _};
use super::move_grab::MoveGrab;
use super::touch_overview_grab::TouchOverviewGrab;
use super::{modifiers_from_state, PointerOrTouchStartData};
use crate::niri::{PointerVisibility, State};

impl State {
    pub(super) fn on_touch_down<I: InputBackend>(&mut self, evt: I::TouchDownEvent) {
        let Some(handle) = self.niri.seat.get_touch() else {
            return;
        };
        let Some(pos) = self.compute_touch_location(&evt) else {
            return;
        };
        let slot = evt.slot();

        // Track touch point for multi-finger gesture detection.
        let was_single = self.niri.touch_gesture_points.len() == 1;
        self.niri.touch_gesture_points.insert(Some(slot), pos);

        // When second finger arrives, start cumulative tracking for gesture recognition.
        // Actual gestures (workspace/view/scroll) require 3+ fingers and are processed
        // in on_touch_motion once the third finger arrives and moves.
        if was_single && self.niri.touch_gesture_points.len() == 2 {
            self.niri.touch_gesture_cumulative = Some((0., 0.));
        }

        // Check if we're tracking a multi-finger gesture (2+ fingers).
        // If so, we should not forward events to clients.
        let tracking_gesture = self.niri.touch_gesture_points.len() > 2;

        let serial = SERIAL_COUNTER.next_serial();

        let under = self.niri.contents_under(pos);

        let mod_key = self.backend.mod_key(&self.niri.config.borrow());

        if self.niri.screenshot_ui.is_open() {
            if let Some(output) = under.output.clone() {
                let geom = self.niri.global_space.output_geometry(&output).unwrap();
                let mut point = (pos - geom.loc.to_f64())
                    .to_physical(output.current_scale().fractional_scale())
                    .to_i32_round();

                let size = output.current_mode().unwrap().size;
                let transform = output.current_transform();
                let size = transform.transform_size(size);
                point.x = min(size.w - 1, point.x);
                point.y = min(size.h - 1, point.y);

                if self
                    .niri
                    .screenshot_ui
                    .pointer_down(output, point, Some(slot))
                {
                    self.niri.queue_redraw_all();
                }
            }
        } else if let Some(mru_output) = self.niri.window_mru_ui.output() {
            if let Some((output, pos_within_output)) = self.niri.output_under(pos) {
                if mru_output == output {
                    let id = self.niri.window_mru_ui.pointer_motion(pos_within_output);
                    if id.is_some() {
                        self.confirm_mru();
                    } else {
                        self.niri.cancel_mru();
                    }
                } else {
                    self.niri.cancel_mru();
                }
            }
        } else if !handle.is_grabbed() {
            let mods = self.niri.seat.get_keyboard().unwrap().modifier_state();
            let mods = modifiers_from_state(mods);
            let mod_down = mods.contains(mod_key.to_modifiers());

            if self.niri.layout.is_overview_open()
                && !mod_down
                && under.layer.is_none()
                && under.output.is_some()
            {
                let (output, pos_within_output) = self.niri.output_under(pos).unwrap();
                let output = output.clone();

                let mut matched_narrow = true;
                let mut ws = self.niri.workspace_under(false, pos);
                if ws.is_none() {
                    matched_narrow = false;
                    ws = self.niri.workspace_under(true, pos);
                }
                let ws_id = ws.map(|(_, ws)| ws.id());

                let mapped = self.niri.window_under(pos);
                let window = mapped.map(|mapped| mapped.window.clone());

                let start_data = TouchGrabStartData {
                    focus: None,
                    slot,
                    location: pos,
                };
                let start_timestamp = Duration::from_micros(evt.time());
                let grab = TouchOverviewGrab::new(
                    start_data,
                    start_timestamp,
                    output,
                    pos_within_output,
                    ws_id,
                    matched_narrow,
                    window,
                );
                handle.set_grab(self, grab, serial);
            } else if let Some((window, _)) = under.window {
                self.niri.layout.activate_window(&window);

                // Check if we need to start a touch move grab.
                if mod_down {
                    let start_data = TouchGrabStartData {
                        focus: None,
                        slot,
                        location: pos,
                    };
                    let start_data = PointerOrTouchStartData::Touch(start_data);
                    if let Some(grab) = MoveGrab::new(self, start_data, window.clone(), true, None)
                    {
                        handle.set_grab(self, grab, serial);
                    }
                }

                // FIXME: granular.
                self.niri.queue_redraw_all();
            } else if let Some(output) = under.output {
                self.niri.layout.focus_output(&output);

                // FIXME: granular.
                self.niri.queue_redraw_all();
            }
            self.niri.focus_layer_surface_if_on_demand(under.layer);
        };

        // Only forward to client if not tracking a multi-finger gesture.
        if !tracking_gesture {
            handle.down(
                self,
                under.surface,
                &DownEvent {
                    slot,
                    location: pos,
                    serial,
                    time: evt.time_msec(),
                },
            );
        }

        // We're using touch, hide the pointer.
        self.niri.pointer_visibility = PointerVisibility::Disabled;
    }

    pub(super) fn on_touch_up<I: InputBackend>(&mut self, evt: I::TouchUpEvent) {
        let Some(handle) = self.niri.seat.get_touch() else {
            return;
        };
        let slot = evt.slot();

        // Check if we're tracking a multi-finger gesture before removing this touch point.
        let tracking_gesture = self.niri.touch_gesture_points.len() > 2;

        // Remove touch point from gesture tracking.
        self.niri.touch_gesture_points.remove(&Some(slot));

        // End gesture when fewer than 2 fingers remain.
        if self.niri.touch_gesture_points.len() < 2 {
            self.niri.touch_gesture_cumulative = None;

            // End any ongoing gesture animations.
            if let Some(output) = self.niri.layout.workspace_switch_gesture_end(Some(true)) {
                self.niri.queue_redraw(&output);
            }
            if let Some(output) = self.niri.layout.view_offset_gesture_end(Some(true)) {
                self.niri.queue_redraw(&output);
            }
            self.niri.layout.overview_gesture_end();
        }

        if let Some(capture) = self.niri.screenshot_ui.pointer_up(Some(slot)) {
            if capture {
                self.confirm_screenshot(true);
            } else {
                self.niri.queue_redraw_all();
            }
        }

        // Only forward to client if not tracking a multi-finger gesture.
        if !tracking_gesture {
            let serial = SERIAL_COUNTER.next_serial();
            handle.up(
                self,
                &UpEvent {
                    slot,
                    serial,
                    time: evt.time_msec(),
                },
            )
        }
    }

    pub(super) fn on_touch_motion<I: InputBackend>(&mut self, evt: I::TouchMotionEvent) {
        let Some(handle) = self.niri.seat.get_touch() else {
            return;
        };
        let Some(pos) = self.compute_touch_location(&evt) else {
            return;
        };
        let slot = evt.slot();

        // Track touch gesture with 2+ fingers.
        let mut gesture_handled = false;
        if let Some(old_pos) = self.niri.touch_gesture_points.get(&Some(slot)).copied() {
            // Calculate delta from this finger's movement.
            let delta_x = pos.x - old_pos.x;
            let delta_y = pos.y - old_pos.y;

            // Update stored position.
            self.niri.touch_gesture_points.insert(Some(slot), pos);

            // Process gesture if we're tracking (3+ fingers).
            if self.niri.touch_gesture_points.len() >= 3 {
                let timestamp = Duration::from_micros(evt.time());
                gesture_handled = true;

                // Check if we're still in recognition phase.
                if let Some((cx, cy)) = &mut self.niri.touch_gesture_cumulative {
                    *cx += delta_x;
                    *cy += delta_y;

                    // Extract config values upfront to avoid borrow conflicts.
                    let (threshold, ov_enabled, ov_fingers, ws_enabled, ws_fingers,
                         vs_enabled, vs_fingers) = {
                        let config = self.niri.config.borrow();
                        let touch = &config.input.touch;
                        (
                            touch.gesture_threshold(),
                            touch.overview_toggle_enabled(),
                            touch.overview_toggle_fingers(),
                            touch.workspace_switch_enabled(),
                            touch.workspace_switch_fingers(),
                            touch.view_scroll_enabled(),
                            touch.view_scroll_fingers(),
                        )
                    };

                    // Check if gesture moved far enough to decide direction.
                    let (cx, cy) = (*cx, *cy);
                    if cx * cx + cy * cy >= threshold * threshold {
                        self.niri.touch_gesture_cumulative = None;

                        let finger_count = self.niri.touch_gesture_points.len();

                        if ov_enabled && finger_count >= ov_fingers {
                            // Overview toggle gesture.
                            self.niri.layout.overview_gesture_begin();
                        } else if let Some((output, _pos_within_output)) =
                            self.niri.output_under(pos)
                        {
                            let output = output.clone();
                            let is_overview_open = self.niri.layout.is_overview_open();

                            if cx.abs() > cy.abs() {
                                // Horizontal gesture: view offset (scroll within workspace).
                                if vs_enabled && finger_count >= vs_fingers {
                                    let output_ws = if is_overview_open {
                                        self.niri.workspace_under(true, pos)
                                    } else {
                                        self.niri
                                            .layout
                                            .monitor_for_output(&output)
                                            .map(|mon| {
                                                (output.clone(), mon.active_workspace_ref())
                                            })
                                    };

                                    if let Some((output, ws)) = output_ws {
                                        let ws_idx = self
                                            .niri
                                            .layout
                                            .find_workspace_by_id(ws.id())
                                            .unwrap()
                                            .0;
                                        self.niri.layout.view_offset_gesture_begin(
                                            &output,
                                            Some(ws_idx),
                                            true,
                                        );
                                    }
                                }
                            } else {
                                // Vertical gesture: workspace switch.
                                if ws_enabled && finger_count >= ws_fingers {
                                    self.niri
                                        .layout
                                        .workspace_switch_gesture_begin(&output, true);
                                }
                            }
                        }
                    }
                }

                // Read config for per-gesture natural scroll and sensitivity.
                // Extract all values upfront to drop the borrow before mutable calls.
                let (ws_natural, ws_sensitivity, vs_natural, vs_sensitivity,
                     ov_natural, ov_sensitivity) = {
                    let config = self.niri.config.borrow();
                    let touch = &config.input.touch;
                    (
                        touch.workspace_switch_natural_scroll(),
                        touch.workspace_switch_sensitivity(),
                        touch.view_scroll_natural_scroll(),
                        touch.view_scroll_sensitivity(),
                        touch.overview_toggle_natural_scroll(),
                        touch.overview_toggle_sensitivity(),
                    )
                };

                // Continue ongoing gesture animations with per-gesture sensitivity
                // and per-gesture natural scroll.
                let ws_delta_y = if ws_natural { -delta_y } else { delta_y };
                if self
                    .niri
                    .layout
                    .workspace_switch_gesture_update(
                        ws_delta_y * ws_sensitivity,
                        timestamp,
                        true,
                    )
                    .is_some()
                {
                    self.niri.queue_redraw_all();
                }

                let vs_delta_x = if vs_natural { -delta_x } else { delta_x };
                if self
                    .niri
                    .layout
                    .view_offset_gesture_update(
                        vs_delta_x * vs_sensitivity,
                        timestamp,
                        true,
                    )
                    .is_some()
                {
                    self.niri.queue_redraw_all();
                }

                // Overview gesture uses vertical delta like touchpad.
                let ov_delta_y = if ov_natural { delta_y } else { -delta_y };
                if let Some(redraw) = self
                    .niri
                    .layout
                    .overview_gesture_update(ov_delta_y * ov_sensitivity, timestamp)
                {
                    if redraw {
                        self.niri.queue_redraw_all();
                    }
                }
            }
        }

        if let Some(output) = self.niri.screenshot_ui.selection_output().cloned() {
            let geom = self.niri.global_space.output_geometry(&output).unwrap();
            let mut point = (pos - geom.loc.to_f64())
                .to_physical(output.current_scale().fractional_scale())
                .to_i32_round::<i32>();

            let size = output.current_mode().unwrap().size;
            let transform = output.current_transform();
            let size = transform.transform_size(size);
            point.x = point.x.clamp(0, size.w - 1);
            point.y = point.y.clamp(0, size.h - 1);

            self.niri.screenshot_ui.pointer_motion(point, Some(slot));
            self.niri.queue_redraw(&output);
        }

        // Only forward to client if not handling a multi-finger gesture.
        if !gesture_handled {
            let under = self.niri.contents_under(pos);
            handle.motion(
                self,
                under.surface,
                &TouchMotionEvent {
                    slot,
                    location: pos,
                    time: evt.time_msec(),
                },
            );
        }

        // Inform the layout of an ongoing DnD operation.
        let is_dnd_grab = handle
            .with_grab(|_, grab| Self::is_dnd_grab(grab.as_any()))
            .unwrap_or(false);
        if is_dnd_grab {
            if let Some((output, pos_within_output)) = self.niri.output_under(pos) {
                let output = output.clone();
                self.niri.layout.dnd_update(output, pos_within_output);
            }
        }
    }

    pub(super) fn on_touch_frame<I: InputBackend>(&mut self, _evt: I::TouchFrameEvent) {
        let Some(handle) = self.niri.seat.get_touch() else {
            return;
        };
        handle.frame(self);
    }

    pub(super) fn on_touch_cancel<I: InputBackend>(&mut self, _evt: I::TouchCancelEvent) {
        let Some(handle) = self.niri.seat.get_touch() else {
            return;
        };

        // Clear all touch gesture state.
        self.niri.touch_gesture_points.clear();
        self.niri.touch_gesture_cumulative = None;

        // Cancel any ongoing gesture animations.
        self.niri.layout.workspace_switch_gesture_end(Some(false));
        self.niri.layout.view_offset_gesture_end(Some(false));
        self.niri.layout.overview_gesture_end();

        handle.cancel(self);
    }
}
