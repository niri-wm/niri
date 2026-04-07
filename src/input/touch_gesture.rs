//! Touchscreen gesture handling.
//!
//! This file handles **touchscreen** (finger-on-screen) gestures only.
//! Touchpad/trackpad gestures are handled separately in `input/mod.rs`
//! via `on_gesture_swipe_*` using libinput gesture events.
//!
//! Naming convention (follows upstream niri):
//!   `touch_*` fields on Niri  → touchscreen
//!   `gesture_swipe_*` fields  → touchpad/trackpad
//!
//! Two gesture types:
//!   - Multi-finger (3+): workspace switch, view scroll, overview toggle
//!   - Edge swipe (1+): touch starts in screen edge zone, any direction

use std::cmp::min;
use std::time::Duration;

use smithay::backend::input::{Event as _, TouchEvent};
use smithay::input::touch::{
    DownEvent, GrabStartData as TouchGrabStartData, MotionEvent as TouchMotionEvent, UpEvent,
};
use smithay::utils::SERIAL_COUNTER;

use super::backend_ext::NiriInputBackend as InputBackend;
use super::move_grab::MoveGrab;
use super::touch_overview_grab::TouchOverviewGrab;
use super::{modifiers_from_state, PointerOrTouchStartData};
use niri_config::input::{EdgeSwipeAction, ScreenEdge};

use crate::niri::{PointerVisibility, State, TouchEdgeSwipeState};

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
        let was_empty = self.niri.touch_gesture_points.is_empty();
        let was_single = self.niri.touch_gesture_points.len() == 1;
        self.niri.touch_gesture_points.insert(Some(slot), pos);

        // First finger: check if it's in a screen edge zone for edge swipe detection.
        if was_empty && self.niri.touch_edge_swipe.is_none() {
            if let Some((output, pos_within_output)) = self.niri.output_under(pos) {
                let size = self.niri.global_space.output_geometry(output).unwrap().size;
                let config = self.niri.config.borrow();
                let threshold = config.input.touchscreen.edge_threshold();
                if let Some(edge) = detect_edge(pos_within_output, size, threshold) {
                    if let Some(action) = config.input.touchscreen.edge_swipe_action(edge) {
                        self.niri.touch_edge_swipe = Some(TouchEdgeSwipeState::Pending {
                            edge,
                            action,
                            cumulative: (0., 0.),
                            slot: Some(slot),
                        });
                    }
                }
            }
        }

        // When second finger arrives, start cumulative tracking for gesture recognition
        // (unless an edge swipe is pending/active — edge swipe takes priority).
        if was_single
            && self.niri.touch_gesture_points.len() == 2
            && self.niri.touch_edge_swipe.is_none()
        {
            self.niri.touch_gesture_cumulative = Some((0., 0.));
        }

        // Check if we're tracking a multi-finger gesture (2+ fingers),
        // a locked gesture (direction decided), or an edge swipe.
        // If so, don't forward events to clients.
        let tracking_gesture = self.niri.touch_gesture_points.len() > 2
            || self.niri.touch_gesture_locked;
        let in_edge_zone = self.niri.touch_edge_swipe.is_some();

        let serial = SERIAL_COUNTER.next_serial();

        let under = self.niri.contents_under(pos);

        let mod_key = self.backend.mod_key(&self.niri.config.borrow());

        if in_edge_zone {
            // Edge zone touch — skip window activation and client forwarding.
            // The gesture will either activate (swipe) or cancel (lift = no-op).
        } else if self.niri.screenshot_ui.is_open() {
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

        // Only forward to client if not tracking a multi-finger gesture or edge swipe.
        if !tracking_gesture && !in_edge_zone {
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

        // Handle edge swipe state on finger lift.
        if let Some(ref state) = self.niri.touch_edge_swipe {
            match state {
                TouchEdgeSwipeState::Pending { slot: edge_slot, .. } => {
                    if Some(slot) == *edge_slot {
                        // Finger lifted before threshold — normal tap, clear state.
                        self.niri.touch_edge_swipe = None;
                    }
                }
                TouchEdgeSwipeState::Active { action, .. } => {
                    let action = *action;
                    self.niri.touch_edge_swipe = None;
                    // End the gesture animation.
                    end_edge_swipe_gesture(self, action);
                    self.niri.touch_gesture_points.remove(&Some(slot));
                    return;
                }
            }
        }

        // Check if we're tracking a multi-finger gesture before removing this touch point.
        let tracking_gesture = self.niri.touch_gesture_points.len() > 2
            || self.niri.touch_gesture_locked;

        // Remove touch point from gesture tracking.
        self.niri.touch_gesture_points.remove(&Some(slot));

        // End gesture when all fingers are lifted.
        if self.niri.touch_gesture_points.is_empty() {
            self.niri.touch_gesture_cumulative = None;
            self.niri.touch_gesture_locked = false;

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

            // Handle edge swipe gesture (takes priority over multi-finger gestures).
            // Extract state to avoid borrow conflicts with self.
            enum EdgeAction {
                None,
                PendingAccumulate {
                    edge: ScreenEdge,
                    action: EdgeSwipeAction,
                    cx: f64,
                    cy: f64,
                    edge_slot: Option<smithay::backend::input::TouchSlot>,
                },
                ActiveFeed {
                    action: EdgeSwipeAction,
                    sensitivity: f64,
                    natural: bool,
                },
            }

            let edge_action = match &mut self.niri.touch_edge_swipe {
                Some(TouchEdgeSwipeState::Pending {
                    edge,
                    action,
                    cumulative,
                    slot: edge_slot,
                }) if Some(slot) == *edge_slot => {
                    cumulative.0 += delta_x;
                    cumulative.1 += delta_y;
                    EdgeAction::PendingAccumulate {
                        edge: *edge,
                        action: *action,
                        cx: cumulative.0,
                        cy: cumulative.1,
                        edge_slot: *edge_slot,
                    }
                }
                Some(TouchEdgeSwipeState::Active { action, edge, .. }) => {
                    let (sensitivity, natural) = {
                        let config = self.niri.config.borrow();
                        let touch = &config.input.touchscreen;
                        (
                            touch.edge_swipe_sensitivity(*edge),
                            match action {
                                EdgeSwipeAction::WorkspaceSwitch => {
                                    touch.workspace_switch_natural_scroll()
                                }
                                EdgeSwipeAction::ViewScroll => {
                                    touch.view_scroll_natural_scroll()
                                }
                                EdgeSwipeAction::OverviewToggle => {
                                    touch.overview_toggle_natural_scroll()
                                }
                            },
                        )
                    };
                    EdgeAction::ActiveFeed {
                        action: *action,
                        sensitivity,
                        natural,
                    }
                }
                _ => EdgeAction::None,
            };

            match edge_action {
                EdgeAction::PendingAccumulate {
                    edge, action, cx, cy, edge_slot,
                } => {
                    let threshold = {
                        let config = self.niri.config.borrow();
                        config.input.touchscreen.gesture_threshold()
                    };

                    if cx * cx + cy * cy >= threshold * threshold {
                        // Edge zone is the activation area — any swipe direction
                        // triggers the gesture. The action determines which axis
                        // matters (overview needs vertical, view-scroll needs
                        // horizontal, etc.).
                        self.niri.touch_edge_swipe =
                            Some(TouchEdgeSwipeState::Active {
                                edge,
                                action,
                                slot: edge_slot,
                            });
                        handle.cancel(self);
                        begin_edge_swipe_gesture(self, pos, edge, action);
                        self.niri.queue_redraw_all();
                    }
                    // During Pending, don't suppress client motion events.
                }
                EdgeAction::ActiveFeed {
                    action, sensitivity, natural, ..
                } => {
                    let timestamp = Duration::from_micros(evt.time());
                    feed_edge_swipe_gesture(
                        self, action, delta_x, delta_y, sensitivity, natural, timestamp,
                    );
                    gesture_handled = true;
                }
                EdgeAction::None => {}
            }

            // Process gesture if tracking (3+ fingers or locked) and no edge swipe active.
            let gesture_active = self.niri.touch_gesture_points.len() >= 3
                || self.niri.touch_gesture_locked;
            if gesture_active && self.niri.touch_edge_swipe.is_none() {
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
                        let touch = &config.input.touchscreen;
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

                        // Lock the gesture — suppress client events until all
                        // fingers are lifted, even if count drops below 3.
                        // Cancel the client's touch sequence since we already
                        // forwarded touch-down for fingers 1 and 2.
                        self.niri.touch_gesture_locked = true;
                        let handle = self.niri.seat.get_touch().unwrap();
                        handle.cancel(self);

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
                let (ws_natural, ws_sensitivity, vs_natural, vs_sensitivity,
                     ov_natural, ov_sensitivity) = {
                    let config = self.niri.config.borrow();
                    let touch = &config.input.touchscreen;
                    (
                        touch.workspace_switch_natural_scroll(),
                        touch.workspace_switch_sensitivity(),
                        touch.view_scroll_natural_scroll(),
                        touch.view_scroll_sensitivity(),
                        touch.overview_toggle_natural_scroll(),
                        touch.overview_toggle_sensitivity(),
                    )
                };

                // Apply per-gesture natural scroll inversion.
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
        self.niri.touch_edge_swipe = None;
        self.niri.touch_gesture_locked = false;

        // Cancel any ongoing gesture animations.
        self.niri.layout.workspace_switch_gesture_end(Some(false));
        self.niri.layout.view_offset_gesture_end(Some(false));
        self.niri.layout.overview_gesture_end();

        handle.cancel(self);
    }
}

/// Detect which screen edge a touch position is near, if any.
fn detect_edge(
    pos: smithay::utils::Point<f64, smithay::utils::Logical>,
    size: smithay::utils::Size<i32, smithay::utils::Logical>,
    threshold: f64,
) -> Option<ScreenEdge> {
    let x = pos.x;
    let y = pos.y;
    let w = size.w as f64;
    let h = size.h as f64;

    let left = x;
    let right = w - x;
    let top = y;
    let bottom = h - y;

    // Find the closest edge within threshold.
    let mut closest: Option<(ScreenEdge, f64)> = None;
    for (edge, dist) in [
        (ScreenEdge::Left, left),
        (ScreenEdge::Right, right),
        (ScreenEdge::Top, top),
        (ScreenEdge::Bottom, bottom),
    ] {
        if dist < threshold {
            if closest.map_or(true, |(_, d)| dist < d) {
                closest = Some((edge, dist));
            }
        }
    }

    closest.map(|(edge, _)| edge)
}

/// Start the appropriate gesture animation for an edge swipe.
fn begin_edge_swipe_gesture(
    state: &mut State,
    pos: smithay::utils::Point<f64, smithay::utils::Logical>,
    _edge: ScreenEdge,
    action: niri_config::input::EdgeSwipeAction,
) {
    use niri_config::input::EdgeSwipeAction;

    match action {
        EdgeSwipeAction::OverviewToggle => {
            state.niri.layout.overview_gesture_begin();
        }
        EdgeSwipeAction::WorkspaceSwitch => {
            if let Some((output, _)) = state.niri.output_under(pos) {
                let output = output.clone();
                state
                    .niri
                    .layout
                    .workspace_switch_gesture_begin(&output, true);
            }
        }
        EdgeSwipeAction::ViewScroll => {
            if let Some((output, _)) = state.niri.output_under(pos) {
                let output = output.clone();
                let is_overview_open = state.niri.layout.is_overview_open();

                let output_ws = if is_overview_open {
                    state.niri.workspace_under(true, pos)
                } else {
                    state
                        .niri
                        .layout
                        .monitor_for_output(&output)
                        .map(|mon| (output.clone(), mon.active_workspace_ref()))
                };

                if let Some((output, ws)) = output_ws {
                    let ws_idx = state
                        .niri
                        .layout
                        .find_workspace_by_id(ws.id())
                        .unwrap()
                        .0;
                    state
                        .niri
                        .layout
                        .view_offset_gesture_begin(&output, Some(ws_idx), true);
                }
            }
        }
    }
}

/// Feed delta to the active edge swipe gesture.
fn feed_edge_swipe_gesture(
    state: &mut State,
    action: niri_config::input::EdgeSwipeAction,
    delta_x: f64,
    delta_y: f64,
    sensitivity: f64,
    natural: bool,
    timestamp: Duration,
) {
    use niri_config::input::EdgeSwipeAction;

    match action {
        EdgeSwipeAction::WorkspaceSwitch => {
            let dy = if natural { -delta_y } else { delta_y };
            if state
                .niri
                .layout
                .workspace_switch_gesture_update(dy * sensitivity, timestamp, true)
                .is_some()
            {
                state.niri.queue_redraw_all();
            }
        }
        EdgeSwipeAction::ViewScroll => {
            let dx = if natural { -delta_x } else { delta_x };
            if state
                .niri
                .layout
                .view_offset_gesture_update(dx * sensitivity, timestamp, true)
                .is_some()
            {
                state.niri.queue_redraw_all();
            }
        }
        EdgeSwipeAction::OverviewToggle => {
            let dy = if natural { delta_y } else { -delta_y };
            if let Some(redraw) = state
                .niri
                .layout
                .overview_gesture_update(dy * sensitivity, timestamp)
            {
                if redraw {
                    state.niri.queue_redraw_all();
                }
            }
        }
    }
}

/// End the edge swipe gesture animation.
fn end_edge_swipe_gesture(state: &mut State, action: niri_config::input::EdgeSwipeAction) {
    use niri_config::input::EdgeSwipeAction;

    match action {
        EdgeSwipeAction::WorkspaceSwitch => {
            if let Some(output) = state.niri.layout.workspace_switch_gesture_end(Some(true)) {
                state.niri.queue_redraw(&output);
            }
        }
        EdgeSwipeAction::ViewScroll => {
            if let Some(output) = state.niri.layout.view_offset_gesture_end(Some(true)) {
                state.niri.queue_redraw(&output);
            }
        }
        EdgeSwipeAction::OverviewToggle => {
            state.niri.layout.overview_gesture_end();
        }
    }
}
