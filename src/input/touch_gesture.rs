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
//! Gesture types:
//!   - Multi-finger (3+): any action via touch-binds (swipe, pinch)
//!   - Edge swipe (1+): touch starts in screen edge zone
//!
//! Actions are mapped via `binds {}` in the KDL config.
//! The compositor infers whether an action is continuous (drives an
//! animation that tracks the finger) or discrete (fires once).
//!
//! IPC gesture events:
//!   Tagged binds (`tag="name"`) emit GestureBegin/Progress/End events
//!   on the IPC event stream, allowing external tools to observe or
//!   drive custom animations. The `noop` action consumes a gesture
//!   for IPC without triggering any compositor action.
//!
//! Note on Mod+touch: On touchscreens, touch serves double duty as
//! both click and gesture input. Mod+touch triggers window move/resize
//! grabs (hardcoded), so Mod+Touch* gesture binds can conflict with
//! Mod+click behavior when fingers land before the gesture threshold.

use std::cmp::min;
use std::time::{Duration, Instant};

use smithay::backend::input::{Event as _, TouchEvent};
use smithay::input::touch::{
    DownEvent, GrabStartData as TouchGrabStartData, MotionEvent as TouchMotionEvent, UpEvent,
};
use smithay::utils::SERIAL_COUNTER;

use super::backend_ext::NiriInputBackend as InputBackend;
use super::move_grab::MoveGrab;
use super::touch_overview_grab::TouchOverviewGrab;
use super::{find_configured_bind, modifiers_from_state, PointerOrTouchStartData};
use niri_config::binds::{
    PinchDirection, RotateDirection, SwipeDirection, Trigger, MAX_FINGERS, MIN_FINGERS,
};
use niri_config::input::{EdgeZone, ScreenEdge};
use niri_config::touch_binds::{
    continuous_gesture_kind, ContinuousGestureKind, TouchGestureType,
};
use niri_config::Action;
use niri_ipc::GestureDelta;

use crate::layout::LayoutElement;
use crate::niri::{ActiveTouchBind, PointerVisibility, State, TapCandidate, TouchEdgeSwipeState};
use crate::utils::with_toplevel_role;

/// Default sensitivity for touchscreen gestures (both edge and multi-finger).
/// Lower than touchpad (1.0) because touchscreen deltas are in screen pixels.
const TOUCH_DEFAULT_SENSITIVITY: f64 = 0.4;

/// Extract gesture info from a matched bind: continuous kind, sensitivity,
/// natural scroll, tag, and action.
fn extract_bind_info(
    bind: niri_config::Bind,
) -> (Option<ContinuousGestureKind>, f64, bool, Option<String>, Action) {
    let kind = continuous_gesture_kind(&bind.action);
    let sensitivity = bind.sensitivity.unwrap_or(TOUCH_DEFAULT_SENSITIVITY);
    (kind, sensitivity, bind.natural_scroll, bind.tag, bind.action)
}

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

        // When ANY new finger arrives, reset cumulative and spread so
        // detection is based on movement with the current finger count.
        // If the gesture was already locked (direction decided with fewer
        // fingers), unlock and re-evaluate — this allows 5-finger gestures
        // to override a 3-finger decision when more fingers land.
        if !was_empty {
            if self.niri.touch_gesture_locked {
                tracing::debug!(
                    target: "niri::input::touch_gesture",
                    "TOUCH-DBG UNLOCK reason=new-finger was_locked=true now={}",
                    self.niri.touch_gesture_points.len(),
                );
                // Unlock: end current gesture animations, restart recognition.
                // If the gesture being interrupted was tagged, emit GestureEnd
                // with completed=false — a consumer that received GestureBegin
                // is contractually owed a matching GestureEnd even when the
                // gesture is cancelled by a new finger landing.
                self.niri.touch_gesture_locked = false;
                let cancelled_tag = self
                    .niri
                    .touch_active_bind
                    .take()
                    .and_then(ActiveTouchBind::into_tag);
                self.niri.layout.workspace_switch_gesture_end(Some(false));
                self.niri.layout.view_offset_gesture_end(Some(false));
                self.niri.layout.overview_gesture_end();
                if let Some(tag) = cancelled_tag {
                    self.ipc_gesture_end(tag, false);
                }
            }
            self.niri.touch_gesture_cumulative = Some((0., 0.));
            if self.niri.touch_gesture_points.len() >= 3 {
                self.niri.touch_gesture_initial_spread =
                    Some(calculate_spread(&self.niri.touch_gesture_points));
                // Initialize rotation tracking basis: record the current
                // per-slot angles so the next motion frame can compute a
                // finite delta, and zero the cumulative so recognition
                // decisions see a fresh gesture.
                self.niri.touch_gesture_cumulative_rotation = 0.0;
                self.niri.touch_gesture_previous_angles =
                    calculate_per_slot_angles(&self.niri.touch_gesture_points);
            }
            tracing::debug!(
                target: "niri::input::touch_gesture",
                "TOUCH-DBG FINGER-LAND fingers={} reset=recognition",
                self.niri.touch_gesture_points.len(),
            );

            // Tap candidate tracking: initialize when finger count reaches 3,
            // update peak_fingers when more fingers land. Runs in parallel
            // with swipe/pinch/rotate recognition. Passthrough windows skip
            // tap detection (same as swipe).
            let finger_count = self.niri.touch_gesture_points.len();
            if finger_count >= 3 && !self.niri.touchscreen_gesture_passthrough {
                if let Some(ref mut tap) = self.niri.touch_tap_candidate {
                    // More fingers landed — update peak and record new position.
                    if tap.alive {
                        tap.peak_fingers = tap.peak_fingers.max(finger_count as u8);
                        tap.initial_positions.insert(Some(slot), pos);
                    }
                } else {
                    // First time reaching 3+ fingers — start tap candidate.
                    self.niri.touch_tap_candidate = Some(TapCandidate {
                        start_time: Instant::now(),
                        peak_fingers: finger_count as u8,
                        initial_positions: self.niri.touch_gesture_points.clone(),
                        alive: true,
                    });
                    tracing::debug!(
                        target: "niri::input::touch_gesture",
                        "TOUCH-DBG TAP started fingers={}",
                        finger_count,
                    );
                }
            }
        }

        // First finger: check if it's in a screen edge zone for edge swipe detection.
        if was_empty && self.niri.touch_edge_swipe.is_none() {
            if let Some((output, pos_within_output)) = self.niri.output_under(pos) {
                let size = self.niri.global_space.output_geometry(output).unwrap().size;
                let config = self.niri.config.borrow();
                let threshold = config.input.touchscreen.edge_start_distance();
                if let Some((edge, zone)) = detect_edge(pos_within_output, size, threshold) {
                    // Lookup order: zoned trigger first, then unzoned parent
                    // as fallback. `zoned` records which one hit so that all
                    // downstream lookups and the IPC name emitted on
                    // gesture-begin stay consistent with the bind that fired.
                    let mod_key = self.backend.mod_key(&config);
                    let mods = self.niri.seat.get_keyboard().unwrap()
                        .modifier_state();
                    let zoned_trigger = Trigger::TouchEdge {
                        edge,
                        zone: Some(zone),
                    };
                    let parent_trigger = Trigger::TouchEdge { edge, zone: None };
                    let zoned_hit = find_configured_bind(
                        config.binds.0.iter(),
                        mod_key,
                        zoned_trigger,
                        mods,
                    ).is_some();
                    let parent_hit = !zoned_hit
                        && find_configured_bind(
                            config.binds.0.iter(),
                            mod_key,
                            parent_trigger,
                            mods,
                        ).is_some();
                    if zoned_hit || parent_hit {
                        self.niri.touch_edge_swipe = Some(TouchEdgeSwipeState::Pending {
                            edge,
                            zone,
                            zoned: zoned_hit,
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
        // Passthrough mode overrides — when set, the whole gesture forwards
        // raw to the client regardless of finger count.
        let tracking_gesture = (self.niri.touch_gesture_points.len() > 2
            || self.niri.touch_gesture_locked)
            && !self.niri.touchscreen_gesture_passthrough;
        let in_edge_zone = self.niri.touch_edge_swipe.is_some();

        let serial = SERIAL_COUNTER.next_serial();

        let under = self.niri.contents_under(pos);

        let mod_key = self.backend.mod_key(&self.niri.config.borrow());

        // Touchscreen gesture passthrough: if this is the first finger and it
        // landed on a window whose rule opts into passthrough, flip the state
        // flag so the recognizer stays out of the way for the whole gesture.
        // Mod+ always bypasses (escape hatch — user explicitly asked for a
        // compositor action). Edge zones also take priority and are handled
        // above, so a passthrough window in a screen-edge zone still yields
        // the edge swipe to niri.
        if was_empty && !in_edge_zone && !self.niri.touchscreen_gesture_passthrough {
            let mods = self.niri.seat.get_keyboard().unwrap().modifier_state();
            let mods = modifiers_from_state(mods);
            let mod_down = mods.contains(mod_key.to_modifiers());
            if !mod_down {
                if let Some(mapped) = self.niri.window_under(pos) {
                    if mapped.rules().touchscreen_gesture_passthrough == Some(true) {
                        self.niri.touchscreen_gesture_passthrough = true;
                    }
                }
            }
        }

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
                TouchEdgeSwipeState::Active { kind, tag, .. } => {
                    let kind = *kind;
                    let tag = tag.clone();
                    self.niri.touch_edge_swipe = None;
                    // End the gesture animation.
                    end_continuous_gesture(self, kind);
                    // Emit IPC GestureEnd for tagged edge swipe.
                    if let Some(tag) = tag {
                        self.ipc_gesture_end(tag, true);
                    }
                    self.niri.touch_gesture_points.remove(&Some(slot));
                    return;
                }
            }
        }

        // Check if we're tracking a multi-finger gesture before removing this touch point.
        // Passthrough gestures forward all up events to the client regardless of finger count.
        let tracking_gesture = (self.niri.touch_gesture_points.len() > 2
            || self.niri.touch_gesture_locked)
            && !self.niri.touchscreen_gesture_passthrough;

        // Remove touch point from gesture tracking.
        self.niri.touch_gesture_points.remove(&Some(slot));

        // Pinch basis rebase on finger-lift.
        //
        // `calculate_spread()` is a purely geometric quantity of the point
        // set (average distance from centroid). When a finger lifts, the
        // set changes and the spread can jump by hundreds of pixels in a
        // single event — not because fingers moved, but because the
        // geometry did. Feeding that spurious delta into the SwipeTracker
        // contaminates both `pos()` and `velocity()` and causes
        // `projected_end_pos` to overshoot the commit threshold, which is
        // why pinch gestures were always snapping to overview-open
        // regardless of direction.
        //
        // Fix: rebase `last_spread` to the post-removal spread so the next
        // motion event computes `incremental ≈ 0` across the
        // discontinuity. Shift `start_spread` by the same delta so the IPC
        // absolute offset `(current - start)` stays continuous for
        // tagged consumers.
        if self.niri.touch_gesture_locked {
            if let Some(ActiveTouchBind::Pinch {
                start_spread,
                last_spread,
                ..
            }) = self.niri.touch_active_bind.as_mut()
            {
                let new_spread = calculate_spread(&self.niri.touch_gesture_points);
                let shift = new_spread - *last_spread;
                *last_spread = new_spread;
                *start_spread += shift;
            }
        }

        // Rotation basis rebase on finger-lift.
        //
        // Same hazard as the pinch rebase above, different metric. When a
        // finger lifts, the cluster centroid shifts and the per-slot angles
        // computed relative to the new centroid can differ from the old
        // ones by tens of degrees — not because fingers rotated, but
        // because the reference point moved. The next motion frame would
        // compute a spurious rotation delta from that discontinuity and
        // feed it into the animation.
        //
        // Fix: overwrite `previous_angles` with fresh angles taken against
        // the post-removal centroid. No delta is accumulated for this step;
        // the next real motion event starts fresh. Because `ipc_progress`
        // for rotation is computed as
        // `(cumulative_rotation - start_rotation) / progress_distance`,
        // leaving both values untouched keeps the IPC progress continuous
        // across the discontinuity with no need for a compensating shift.
        //
        // This rebase applies whether the active bind is Rotate (mid-gesture
        // finger lift of an active rotation) OR another variant (unlocked
        // recognition phase with 3+ fingers still down, where rotation may
        // still become the chosen classification on the next frame).
        if !self.niri.touch_gesture_points.is_empty() {
            self.niri.touch_gesture_previous_angles =
                calculate_per_slot_angles(&self.niri.touch_gesture_points);
        }

        // Spread basis rebase on finger-lift (recognition phase only).
        //
        // `spread_change = (current_spread - initial_spread).abs()` is the
        // signal pinch recognition latches on. When a finger lifts during
        // recognition, `current_spread` jumps because the geometry changed,
        // not because fingers moved — and the jump typically exceeds
        // `pinch_trigger_distance` immediately, causing a spurious
        // PinchIn/PinchOut lock on the very next frame. This was visible in debug logs as
        // users trying to retry a 5-finger rotation by lifting one finger
        // and ending up with an unwanted PinchIn at fingers=4.
        //
        // Fix: during unlocked recognition, rebase `initial_spread` to the
        // post-removal geometry so `spread_change` resets to zero across
        // the discontinuity. Only applies while unlocked — once a pinch
        // is already active the rebase above (at the `ActiveTouchBind::Pinch`
        // branch) handles the locked case with continuous IPC progress.
        if !self.niri.touch_gesture_points.is_empty()
            && !self.niri.touch_gesture_locked
            && self.niri.touch_gesture_points.len() >= 3
        {
            self.niri.touch_gesture_initial_spread =
                Some(calculate_spread(&self.niri.touch_gesture_points));
        }

        // End gesture when all fingers are lifted.
        if self.niri.touch_gesture_points.is_empty() {
            // Tap detection: if the candidate is still alive and within
            // the timeout, fire the TouchTap trigger.
            if let Some(tap) = self.niri.touch_tap_candidate.take() {
                if tap.alive && !self.niri.touch_gesture_locked {
                    let elapsed_ms = tap.start_time.elapsed().as_millis() as f64;
                    let timeout = {
                        let config = self.niri.config.borrow();
                        config.input.touchscreen.tap_timeout_ms()
                    };
                    if elapsed_ms <= timeout {
                        let trigger = Trigger::TouchTap {
                            fingers: tap.peak_fingers,
                        };
                        let bind_info = {
                            let config = self.niri.config.borrow();
                            let mod_key = self.backend.mod_key(&config);
                            let mods = self.niri.seat.get_keyboard().unwrap()
                                .modifier_state();
                            find_configured_bind(
                                config.binds.0.iter(),
                                mod_key,
                                trigger,
                                mods,
                            )
                        };
                        let bind_matched = bind_info.is_some();
                        tracing::debug!(
                            target: "niri::input::touch_gesture",
                            "TOUCH-DBG TAP fired fingers={} bind={} elapsed={:.0}ms",
                            tap.peak_fingers,
                            if bind_matched { "yes" } else { "no" },
                            elapsed_ms,
                        );
                        if let Some(bind) = bind_info {
                            let tag = bind.tag.clone();
                            let trigger_name = format!(
                                "TouchTap fingers={}", tap.peak_fingers,
                            );
                            // Emit GestureBegin + immediate GestureEnd for IPC.
                            self.ipc_gesture_begin(
                                tag.clone().unwrap_or_default(),
                                trigger_name,
                                tap.peak_fingers,
                                false,
                            );
                            if !matches!(bind.action, Action::Noop) {
                                self.do_action(bind.action, false);
                            }
                            self.ipc_gesture_end(
                                tag.unwrap_or_default(),
                                true,
                            );
                        }
                    } else {
                        tracing::debug!(
                            target: "niri::input::touch_gesture",
                            "TOUCH-DBG TAP killed reason=timeout elapsed={:.0}ms",
                            elapsed_ms,
                        );
                    }
                }
            }

            self.niri.touch_gesture_cumulative = None;
            self.niri.touch_gesture_locked = false;
            self.niri.touchscreen_gesture_passthrough = false;
            self.niri.touch_frame_dirty = false;
            self.niri.touch_frame_delta = (0., 0.);
            self.niri.touch_frame_edge_delta = (0., 0.);
            // Take the active bind to get the tag before clearing.
            // We track `had_active` separately so we can emit GestureEnd
            // even for untagged binds (debug tools rely on it).
            let active_bind = self.niri.touch_active_bind.take();
            let had_active = active_bind.is_some();
            let active_tag = active_bind.and_then(ActiveTouchBind::into_tag);
            self.niri.touch_gesture_initial_spread = None;
            self.niri.touch_gesture_cumulative_rotation = 0.0;
            self.niri.touch_gesture_previous_angles.clear();

            // End any ongoing gesture animations.
            if let Some(output) = self.niri.layout.workspace_switch_gesture_end(Some(true)) {
                self.niri.queue_redraw(&output);
            }
            if let Some(output) = self.niri.layout.view_offset_gesture_end(Some(true)) {
                self.niri.queue_redraw(&output);
            }
            self.niri.layout.overview_gesture_end();

            // Emit IPC GestureEnd for every committed multi-finger
            // gesture, tagged or not — empty tag for untagged binds.
            if had_active {
                self.ipc_gesture_end(active_tag.unwrap_or_default(), true);
            }
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

        // Track touch gesture with 2+ fingers. Skipped entirely under
        // touchscreen gesture passthrough so the whole motion stream forwards
        // raw to the client. `touch_gesture_points` is left untouched — slot
        // cleanup in on_touch_up will still clear it.
        let mut gesture_handled = false;
        let tracked_slot = if self.niri.touchscreen_gesture_passthrough {
            None
        } else {
            self.niri.touch_gesture_points.get(&Some(slot)).copied()
        };
        if let Some(old_pos) = tracked_slot {
            // Calculate delta from this finger's movement.
            let delta_x = pos.x - old_pos.x;
            let delta_y = pos.y - old_pos.y;

            // Update stored position.
            self.niri.touch_gesture_points.insert(Some(slot), pos);

            // Handle edge swipe gesture: accumulate deltas per-slot,
            // defer threshold check and feed to on_touch_frame.
            if let Some(ref mut state) = self.niri.touch_edge_swipe {
                match state {
                    TouchEdgeSwipeState::Pending {
                        cumulative,
                        slot: edge_slot,
                        ..
                    } if Some(slot) == *edge_slot => {
                        cumulative.0 += delta_x;
                        cumulative.1 += delta_y;
                    }
                    TouchEdgeSwipeState::Active { slot: edge_slot, .. }
                        if Some(slot) == *edge_slot =>
                    {
                        // Track edge slot's delta separately so the feed
                        // doesn't include other fingers' motion.
                        self.niri.touch_frame_edge_delta.0 += delta_x;
                        self.niri.touch_frame_edge_delta.1 += delta_y;
                        gesture_handled = true;
                    }
                    TouchEdgeSwipeState::Active { .. } => {
                        gesture_handled = true;
                    }
                    _ => {}
                }
            }

            // Accumulate per-frame deltas for batched processing in
            // on_touch_frame. Position update already happened above.
            self.niri.touch_frame_delta.0 += delta_x;
            self.niri.touch_frame_delta.1 += delta_y;
            self.niri.touch_frame_dirty = true;
            self.niri.touch_frame_timestamp = Duration::from_micros(evt.time());

            // Tap wobble check runs per-motion (reads positions, cheap,
            // and needs to kill the candidate as soon as any finger drifts).
            if let Some(ref mut tap) = self.niri.touch_tap_candidate {
                if tap.alive {
                    let wobble_threshold = {
                        let config = self.niri.config.borrow();
                        config.input.touchscreen.tap_wobble_threshold()
                    };
                    let wobble_sq = wobble_threshold * wobble_threshold;
                    let mut wobble_killed = false;
                    for (s, current_pos) in &self.niri.touch_gesture_points {
                        if let Some(initial) = tap.initial_positions.get(s) {
                            let dx = current_pos.x - initial.x;
                            let dy = current_pos.y - initial.y;
                            if dx * dx + dy * dy > wobble_sq {
                                wobble_killed = true;
                                break;
                            }
                        }
                    }

                    if wobble_killed {
                        let peak_fingers = tap.peak_fingers;
                        tap.alive = false;

                        // Check minimum hold duration before allowing
                        // tap-hold-drag. If fingers moved too quickly,
                        // skip hold-drag and let normal swipe recognition
                        // handle it.
                        let hold_delay_ms = {
                            let config = self.niri.config.borrow();
                            config.input.touchscreen.tap_hold_trigger_delay_ms()
                        };
                        let elapsed_ms = tap.start_time.elapsed().as_millis() as f64;
                        let hold_long_enough = elapsed_ms >= hold_delay_ms;

                        // Compute centroid delta from initial positions for
                        // direction detection.
                        let (mut cx, mut cy) = (0.0, 0.0);
                        let mut count = 0usize;
                        for (s, current_pos) in &self.niri.touch_gesture_points {
                            if let Some(initial) = tap.initial_positions.get(s) {
                                cx += current_pos.x - initial.x;
                                cy += current_pos.y - initial.y;
                                count += 1;
                            }
                        }
                        if count > 0 {
                            cx /= count as f64;
                            cy /= count as f64;
                        }

                        // Check for TouchTapHoldDrag bind — only if held
                        // long enough to distinguish from a fast swipe.
                        let bind_info = if hold_long_enough {
                            let is_horizontal = cx.abs() > cy.abs();
                            let direction = match (is_horizontal, cx, cy) {
                                (true, cx, _) if cx > 0.0 => SwipeDirection::Right,
                                (true, _, _) => SwipeDirection::Left,
                                (false, _, cy) if cy > 0.0 => SwipeDirection::Down,
                                _ => SwipeDirection::Up,
                            };

                            let config = self.niri.config.borrow();
                            let mod_key = self.backend.mod_key(&config);
                            let mods = self.niri.seat.get_keyboard().unwrap()
                                .modifier_state();
                            // Try directional first.
                            let directional = Trigger::TouchTapHoldDrag {
                                fingers: peak_fingers,
                                direction: Some(direction),
                            };
                            let bind = find_configured_bind(
                                config.binds.0.iter(),
                                mod_key,
                                directional,
                                mods,
                            );
                            if bind.is_some() {
                                bind.map(|b| (extract_bind_info(b), directional))
                            } else {
                                // Fall back to omnidirectional.
                                let omni = Trigger::TouchTapHoldDrag {
                                    fingers: peak_fingers,
                                    direction: None,
                                };
                                find_configured_bind(
                                    config.binds.0.iter(),
                                    mod_key,
                                    omni,
                                    mods,
                                ).map(|b| (extract_bind_info(b), omni))
                            }
                        } else {
                            None
                        };

                        if let Some(((kind, sensitivity, natural_scroll, tag, action), trigger)) =
                            bind_info
                        {
                            tracing::debug!(
                                target: "niri::input::touch_gesture",
                                "TOUCH-DBG TAP killed reason=wobble → \
                                 TouchTapHoldDrag fingers={} trigger={:?} \
                                 hold={:.0}ms",
                                peak_fingers, trigger, elapsed_ms,
                            );

                            // Lock the gesture so normal swipe/pinch/rotate
                            // recognition doesn't also fire.
                            self.niri.touch_gesture_locked = true;
                            self.niri.touch_gesture_cumulative = None;
                            let handle = self.niri.seat.get_touch().unwrap();
                            handle.cancel(self);

                            let trigger_name = trigger_to_ipc_name(trigger);
                            self.ipc_gesture_begin(
                                tag.clone().unwrap_or_default(),
                                trigger_name,
                                peak_fingers,
                                kind.is_some(),
                            );

                            if let Some(kind) = kind {
                                begin_continuous_gesture(self, kind, pos);
                                let active = ActiveTouchBind::Swipe {
                                    kind,
                                    sensitivity,
                                    natural_scroll,
                                    tag,
                                    ipc_progress: 0.0,
                                };
                                self.niri.touch_active_bind = Some(active);
                            } else {
                                if !matches!(action, Action::Noop) {
                                    self.do_action(action, false);
                                }
                                self.ipc_gesture_end(
                                    tag.clone().unwrap_or_default(),
                                    true,
                                );
                            }
                        } else {
                            tracing::debug!(
                                target: "niri::input::touch_gesture",
                                "TOUCH-DBG TAP killed reason=wobble \
                                 hold={:.0}ms (need {:.0}ms for hold-drag)",
                                elapsed_ms, hold_delay_ms,
                            );
                        }
                    }
                }
            }

            // Suppress client forwarding if we're in a multi-finger gesture
            // or an active edge swipe. Processing happens in on_touch_frame.
            let finger_count = self.niri.touch_gesture_points.len();
            let gesture_active = finger_count >= 3 || self.niri.touch_gesture_locked;
            if gesture_active && self.niri.touch_edge_swipe.is_none() {
                gesture_handled = true;
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

        // Process batched touch motion events. All per-slot position updates
        // and delta accumulation happened in on_touch_motion; here we run
        // the expensive processing once per hardware scan frame instead of
        // once per finger.
        if self.niri.touch_frame_dirty {
            self.niri.touch_frame_dirty = false;
            let delta_x = self.niri.touch_frame_delta.0;
            let delta_y = self.niri.touch_frame_delta.1;
            self.niri.touch_frame_delta = (0., 0.);
            let timestamp = self.niri.touch_frame_timestamp;

            // Compute centroid for use in lock transition (window_under,
            // begin_continuous_gesture). More correct than the last-moved
            // finger's position for multi-finger gestures.
            let pos = {
                let points = &self.niri.touch_gesture_points;
                if points.is_empty() {
                    smithay::utils::Point::from((0., 0.))
                } else {
                    let n = points.len() as f64;
                    let (sx, sy) = points.values().fold((0., 0.), |(ax, ay), p| {
                        (ax + p.x, ay + p.y)
                    });
                    smithay::utils::Point::from((sx / n, sy / n))
                }
            };

            // Edge swipe: threshold check (Pending → Active) and active feed.
            // These were deferred from on_touch_motion to avoid per-slot feeds.
            if let Some(ref state) = self.niri.touch_edge_swipe {
                match state {
                    TouchEdgeSwipeState::Pending {
                        edge, zone, zoned, cumulative, slot: edge_slot, ..
                    } => {
                        let edge = *edge;
                        let zone = *zone;
                        let zoned = *zoned;
                        let (cx, cy) = *cumulative;
                        let edge_slot = *edge_slot;
                        let threshold = {
                            let config = self.niri.config.borrow();
                            config.input.touchscreen.swipe_trigger_distance()
                        };

                        if cx * cx + cy * cy >= threshold * threshold {
                            let trigger = Trigger::TouchEdge {
                                edge,
                                zone: if zoned { Some(zone) } else { None },
                            };
                            let bind_info = {
                                let config = self.niri.config.borrow();
                                let mod_key = self.backend.mod_key(&config);
                                let mods = self.niri.seat.get_keyboard().unwrap()
                                    .modifier_state();
                                find_configured_bind(
                                    config.binds.0.iter(),
                                    mod_key,
                                    trigger,
                                    mods,
                                )
                            };
                            let bind_info = bind_info.map(extract_bind_info);

                            if let Some((kind, sensitivity, natural_scroll, tag, action)) = bind_info {
                                if let Some(ref tag) = tag {
                                    let trigger_name = trigger_to_ipc_name(trigger);
                                    self.ipc_gesture_begin(
                                        tag.clone(),
                                        trigger_name,
                                        1,
                                        kind.is_some(),
                                    );
                                }

                                if let Some(kind) = kind {
                                    self.niri.touch_edge_swipe =
                                        Some(TouchEdgeSwipeState::Active {
                                            edge,
                                            zone,
                                            zoned,
                                            kind,
                                            sensitivity,
                                            natural_scroll,
                                            slot: edge_slot,
                                            tag,
                                            ipc_progress: 0.0,
                                        });
                                    handle.cancel(self);
                                    begin_continuous_gesture(self, kind, pos);
                                    self.niri.queue_redraw_all();
                                } else {
                                    handle.cancel(self);
                                    if !matches!(action, Action::Noop) {
                                        self.do_action(action, false);
                                    }
                                    if let Some(ref tag) = tag {
                                        self.ipc_gesture_end(tag.clone(), true);
                                    }
                                    self.niri.touch_edge_swipe = None;
                                }
                            } else {
                                self.niri.touch_edge_swipe = None;
                            }
                        }
                    }
                    TouchEdgeSwipeState::Active {
                        kind, sensitivity, natural_scroll, tag, ..
                    } => {
                        let kind = *kind;
                        let sensitivity = *sensitivity;
                        let natural = *natural_scroll;
                        let tag = tag.clone();
                        // Use edge-slot-only delta, not the combined
                        // multi-finger delta.
                        let (edge_dx, edge_dy) = self.niri.touch_frame_edge_delta;
                        self.niri.touch_frame_edge_delta = (0., 0.);
                        feed_continuous_gesture(
                            self, kind, edge_dx, edge_dy, sensitivity, natural, timestamp,
                            tag.as_deref(),
                        );
                    }
                }
            }

            // Process multi-finger gesture (3+ fingers or locked), no edge swipe.
            let finger_count = self.niri.touch_gesture_points.len();
            let gesture_active = finger_count >= 3 || self.niri.touch_gesture_locked;
            if gesture_active && self.niri.touch_edge_swipe.is_none() {
                // Feed ongoing continuous gesture if one is active.
                if let Some(ref active) = self.niri.touch_active_bind {
                    match active {
                        ActiveTouchBind::Swipe {
                            kind,
                            sensitivity,
                            natural_scroll,
                            tag,
                            ..
                        } => {
                            let kind = *kind;
                            let sensitivity = *sensitivity;
                            let natural = *natural_scroll;
                            let tag = tag.clone();
                            feed_continuous_gesture(
                                self, kind, delta_x, delta_y, sensitivity, natural, timestamp,
                                tag.as_deref(),
                            );
                        }
                        ActiveTouchBind::Pinch { kind, tag, .. } => {
                            let kind = *kind;
                            let tag = tag.clone();
                            feed_continuous_pinch(self, kind, timestamp, tag.as_deref());
                        }
                        ActiveTouchBind::Rotate { kind, tag, .. } => {
                            let kind = *kind;
                            let tag = tag.clone();
                            feed_continuous_rotation(self, kind, timestamp, tag.as_deref());
                        }
                    }
                } else if let Some((cx, cy)) = &mut self.niri.touch_gesture_cumulative {
                    // Recognition phase: accumulate batched deltas.
                    *cx += delta_x;
                    *cy += delta_y;

                    let finger_count_f = finger_count.max(1) as f64;
                    let (cx, cy) = (*cx / finger_count_f, *cy / finger_count_f);
                    let swipe_distance = (cx * cx + cy * cy).sqrt();

                    let (frame_rotation, new_angles) = calculate_rotation_delta(
                        &self.niri.touch_gesture_points,
                        &self.niri.touch_gesture_previous_angles,
                    );
                    self.niri.touch_gesture_previous_angles = new_angles;
                    self.niri.touch_gesture_cumulative_rotation += frame_rotation;

                    let (
                        swipe_trigger,
                        pinch_trigger,
                        pinch_dom,
                        rotation_trigger,
                        rotation_dom,
                    ) = {
                        let config = self.niri.config.borrow();
                        (
                            config
                                .input
                                .touchscreen
                                .scaled_swipe_trigger_distance(finger_count),
                            config.input.touchscreen.pinch_trigger_distance(),
                            config.input.touchscreen.pinch_dominance_ratio(),
                            config.input.touchscreen.rotation_trigger_angle(),
                            config.input.touchscreen.rotation_dominance_ratio(),
                        )
                    };

                    let current_spread = calculate_spread(&self.niri.touch_gesture_points);
                    let initial_spread =
                        self.niri.touch_gesture_initial_spread.unwrap_or(current_spread);
                    let spread_change = (current_spread - initial_spread).abs();

                    let cumulative_rotation =
                        self.niri.touch_gesture_cumulative_rotation;
                    let rotation_arc = cumulative_rotation.abs() * current_spread;
                    let rotation_arc_trigger_distance =
                        rotation_trigger * current_spread;

                    let is_rotate = finger_count >= 3
                        && rotation_arc >= rotation_arc_trigger_distance
                        && rotation_arc >= swipe_distance * rotation_dom
                        && rotation_arc >= spread_change * rotation_dom;

                    let is_pinch = spread_change > pinch_trigger
                        && spread_change > swipe_distance * pinch_dom
                        && !is_rotate;

                    let closest = {
                        let swipe_frac = swipe_distance / swipe_trigger.max(1e-9);
                        let pinch_frac = spread_change / pinch_trigger.max(1e-9);
                        let rotate_frac =
                            cumulative_rotation.abs() / rotation_trigger.max(1e-9);
                        if rotate_frac >= swipe_frac && rotate_frac >= pinch_frac {
                            "rotate"
                        } else if pinch_frac >= swipe_frac {
                            "pinch"
                        } else {
                            "swipe"
                        }
                    };
                    tracing::debug!(
                        target: "niri::input::touch_gesture",
                        "TOUCH-DBG FRAME fingers={} \
                         swipe={:.1}/{:.1} \
                         spread={:.1}/{:.1} \
                         rot={:.3}/{:.3}rad ({:.1}°) \
                         arc={:.1} \
                         is_rotate={} is_pinch={} closest={}",
                        finger_count,
                        swipe_distance, swipe_trigger,
                        spread_change, pinch_trigger,
                        cumulative_rotation.abs(), rotation_trigger,
                        cumulative_rotation.to_degrees(),
                        rotation_arc,
                        is_rotate, is_pinch, closest,
                    );

                    #[cfg(debug_assertions)]
                    self.ipc_recognition_frame(
                        finger_count as u8,
                        swipe_distance,
                        swipe_trigger,
                        current_spread - initial_spread,
                        pinch_trigger,
                        cumulative_rotation,
                        rotation_trigger,
                        rotation_arc,
                        rotation_arc_trigger_distance,
                        is_rotate,
                        is_pinch,
                        closest.to_string(),
                        timestamp.as_millis() as u32,
                    );

                    let rotation_candidate = finger_count >= 3
                        && rotation_arc >= rotation_arc_trigger_distance;

                    if is_rotate
                        || (swipe_distance >= swipe_trigger && !rotation_candidate)
                        || is_pinch
                    {
                        self.niri.touch_gesture_cumulative = None;

                        if let Some(ref mut tap) = self.niri.touch_tap_candidate {
                            if tap.alive {
                                tap.alive = false;
                                tracing::debug!(
                                    target: "niri::input::touch_gesture",
                                    "TOUCH-DBG TAP killed reason=lock",
                                );
                            }
                        }

                        if let Some(mapped) = self.niri.window_under(pos) {
                            let app_id = with_toplevel_role(mapped.toplevel(), |role| {
                                role.app_id.clone()
                            });
                            tracing::debug!(
                                "touch: captured {}-finger gesture over app-id={:?}",
                                finger_count,
                                app_id.unwrap_or_default(),
                            );
                        }

                        self.niri.touch_gesture_locked = true;
                        let handle = self.niri.seat.get_touch().unwrap();
                        handle.cancel(self);

                        let gesture_type = if is_rotate {
                            if cumulative_rotation > 0.0 {
                                TouchGestureType::RotateCcw
                            } else {
                                TouchGestureType::RotateCw
                            }
                        } else if is_pinch {
                            if current_spread < initial_spread {
                                TouchGestureType::PinchIn
                            } else {
                                TouchGestureType::PinchOut
                            }
                        } else if cx.abs() > cy.abs() {
                            if cx > 0.0 {
                                TouchGestureType::SwipeRight
                            } else {
                                TouchGestureType::SwipeLeft
                            }
                        } else {
                            if cy > 0.0 {
                                TouchGestureType::SwipeDown
                            } else {
                                TouchGestureType::SwipeUp
                            }
                        };

                        let bind_info = {
                            let config = self.niri.config.borrow();
                            let trigger = touch_gesture_to_trigger(
                                gesture_type,
                                finger_count as u8,
                            );
                            let mod_key = self.backend.mod_key(&config);
                            let mods = self.niri.seat.get_keyboard().unwrap()
                                .modifier_state();
                            trigger.and_then(|t| {
                                find_configured_bind(
                                    config.binds.0.iter(),
                                    mod_key,
                                    t,
                                    mods,
                                )
                            })
                        };
                        let bind_info = bind_info.map(extract_bind_info);

                        {
                            let trigger_name = touch_gesture_to_trigger(
                                gesture_type,
                                finger_count as u8,
                            )
                            .map(trigger_to_ipc_name)
                            .unwrap_or_else(|| "Unknown".to_string());
                            let (bind_matched, kind_str, tag_str) =
                                match bind_info.as_ref() {
                                    Some((kind, _, _, tag, _)) => (
                                        "yes",
                                        kind.map(|k| format!("{:?}", k))
                                            .unwrap_or_else(|| "discrete".to_string()),
                                        tag.clone().unwrap_or_else(|| "-".to_string()),
                                    ),
                                    None => (
                                        "no",
                                        "-".to_string(),
                                        "-".to_string(),
                                    ),
                                };
                            tracing::debug!(
                                target: "niri::input::touch_gesture",
                                "TOUCH-DBG LOCK fingers={} type={:?} \
                                 trigger={} bind={} kind={} tag={}",
                                finger_count,
                                gesture_type,
                                trigger_name,
                                bind_matched,
                                kind_str,
                                tag_str,
                            );
                        }

                        if let Some((kind, sensitivity, natural_scroll, tag, action)) = bind_info {
                            {
                                let trigger_name =
                                    touch_gesture_to_trigger(gesture_type, finger_count as u8)
                                        .map(trigger_to_ipc_name)
                                        .unwrap_or_else(|| "Unknown".to_string());
                                self.ipc_gesture_begin(
                                    tag.clone().unwrap_or_default(),
                                    trigger_name,
                                    finger_count as u8,
                                    kind.is_some(),
                                );
                            }

                            if let Some(kind) = kind {
                                begin_continuous_gesture(self, kind, pos);
                                let active = if is_rotate {
                                    ActiveTouchBind::Rotate {
                                        kind,
                                        tag,
                                        ipc_progress: 0.0,
                                        start_rotation: cumulative_rotation,
                                    }
                                } else if is_pinch {
                                    ActiveTouchBind::Pinch {
                                        kind,
                                        tag,
                                        ipc_progress: 0.0,
                                        start_spread: current_spread,
                                        last_spread: current_spread,
                                    }
                                } else {
                                    ActiveTouchBind::Swipe {
                                        kind,
                                        sensitivity,
                                        natural_scroll,
                                        tag,
                                        ipc_progress: 0.0,
                                    }
                                };
                                self.niri.touch_active_bind = Some(active);
                            } else {
                                if !matches!(action, Action::Noop) {
                                    self.do_action(action, false);
                                }
                                self.ipc_gesture_end(
                                    tag.clone().unwrap_or_default(),
                                    true,
                                );
                            }
                        }
                    }
                }
            }
        }

        handle.frame(self);
    }

    pub(super) fn on_touch_cancel<I: InputBackend>(&mut self, _evt: I::TouchCancelEvent) {
        let Some(handle) = self.niri.seat.get_touch() else {
            return;
        };

        // Collect tags for IPC GestureEnd before clearing state.
        // Track `had_active` separately so we can emit a cancelled
        // GestureEnd for untagged multi-finger binds too.
        let active_bind = self.niri.touch_active_bind.take();
        let had_active = active_bind.is_some();
        let active_tag = active_bind.and_then(ActiveTouchBind::into_tag);
        let edge_tag = match &self.niri.touch_edge_swipe {
            Some(TouchEdgeSwipeState::Active { tag, .. }) => tag.clone(),
            _ => None,
        };

        // Clear all touch gesture state.
        self.niri.touch_gesture_points.clear();
        self.niri.touch_gesture_cumulative = None;
        self.niri.touch_edge_swipe = None;
        self.niri.touch_gesture_locked = false;
        self.niri.touch_gesture_initial_spread = None;
        self.niri.touch_gesture_cumulative_rotation = 0.0;
        self.niri.touch_gesture_previous_angles.clear();
        self.niri.touch_tap_candidate = None;
        self.niri.touchscreen_gesture_passthrough = false;
        self.niri.touch_frame_dirty = false;
        self.niri.touch_frame_delta = (0., 0.);
        self.niri.touch_frame_edge_delta = (0., 0.);

        // Cancel any ongoing gesture animations.
        self.niri.layout.workspace_switch_gesture_end(Some(false));
        self.niri.layout.view_offset_gesture_end(Some(false));
        self.niri.layout.overview_gesture_end();

        // Emit IPC GestureEnd (cancelled) for any committed multi-finger
        // bind (tagged or untagged), and tagged edge swipes.
        if had_active {
            self.ipc_gesture_end(active_tag.unwrap_or_default(), false);
        }
        if let Some(tag) = edge_tag {
            self.ipc_gesture_end(tag, false);
        }

        handle.cancel(self);
    }
}

/// Convert a TouchGestureType + finger count to a Trigger for bind lookup.
fn touch_gesture_to_trigger(gesture: TouchGestureType, finger_count: u8) -> Option<Trigger> {
    use TouchGestureType::*;
    // Reject finger counts outside the supported range. Edge swipes are
    // always single-finger so they're allowed through regardless.
    if !(MIN_FINGERS..=MAX_FINGERS).contains(&finger_count)
        && !matches!(
            gesture,
            EdgeSwipeLeft | EdgeSwipeRight | EdgeSwipeTop | EdgeSwipeBottom
        )
    {
        return None;
    }
    let fingers = finger_count;
    match gesture {
        SwipeUp => Some(Trigger::TouchSwipe {
            fingers,
            direction: SwipeDirection::Up,
        }),
        SwipeDown => Some(Trigger::TouchSwipe {
            fingers,
            direction: SwipeDirection::Down,
        }),
        SwipeLeft => Some(Trigger::TouchSwipe {
            fingers,
            direction: SwipeDirection::Left,
        }),
        SwipeRight => Some(Trigger::TouchSwipe {
            fingers,
            direction: SwipeDirection::Right,
        }),
        PinchIn => Some(Trigger::TouchPinch {
            fingers,
            direction: PinchDirection::In,
        }),
        PinchOut => Some(Trigger::TouchPinch {
            fingers,
            direction: PinchDirection::Out,
        }),
        RotateCw => Some(Trigger::TouchRotate {
            fingers,
            direction: RotateDirection::Cw,
        }),
        RotateCcw => Some(Trigger::TouchRotate {
            fingers,
            direction: RotateDirection::Ccw,
        }),
        Tap => Some(Trigger::TouchTap { fingers }),
        EdgeSwipeLeft => Some(Trigger::TouchEdge {
            edge: ScreenEdge::Left,
            zone: None,
        }),
        EdgeSwipeRight => Some(Trigger::TouchEdge {
            edge: ScreenEdge::Right,
            zone: None,
        }),
        EdgeSwipeTop => Some(Trigger::TouchEdge {
            edge: ScreenEdge::Top,
            zone: None,
        }),
        EdgeSwipeBottom => Some(Trigger::TouchEdge {
            edge: ScreenEdge::Bottom,
            zone: None,
        }),
    }
}

/// Detect which screen edge a touch position is near, if any, and which
/// third of that edge it lies in.
///
/// The edge is the one closest to the touch point within `threshold`. The
/// zone splits the perpendicular axis into equal thirds: for Top/Bottom the
/// split is across x (Start = leftmost third, End = rightmost third); for
/// Left/Right it is across y (Start = topmost third, End = bottommost third).
fn detect_edge(
    pos: smithay::utils::Point<f64, smithay::utils::Logical>,
    size: smithay::utils::Size<i32, smithay::utils::Logical>,
    threshold: f64,
) -> Option<(ScreenEdge, EdgeZone)> {
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

    let (edge, _) = closest?;

    // Classify the perpendicular-axis position into thirds.
    let (pos_along, extent) = match edge {
        ScreenEdge::Top | ScreenEdge::Bottom => (x, w),
        ScreenEdge::Left | ScreenEdge::Right => (y, h),
    };
    let third = extent / 3.0;
    let zone = if pos_along < third {
        EdgeZone::Start
    } else if pos_along < third * 2.0 {
        EdgeZone::Center
    } else {
        EdgeZone::End
    };

    Some((edge, zone))
}

/// Begin a continuous gesture animation.
fn begin_continuous_gesture(
    state: &mut State,
    kind: ContinuousGestureKind,
    pos: smithay::utils::Point<f64, smithay::utils::Logical>,
) {
    match kind {
        ContinuousGestureKind::OverviewToggle => {
            state.niri.layout.overview_gesture_begin();
        }
        ContinuousGestureKind::WorkspaceSwitch => {
            if let Some((output, _)) = state.niri.output_under(pos) {
                let output = output.clone();
                state
                    .niri
                    .layout
                    .workspace_switch_gesture_begin(&output, true);
            }
        }
        ContinuousGestureKind::ViewScroll => {
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
        ContinuousGestureKind::Noop => {
            // No compositor animation — IPC events are emitted by the caller.
        }
    }
}

/// Feed delta to an active continuous gesture.
fn feed_continuous_gesture(
    state: &mut State,
    kind: ContinuousGestureKind,
    delta_x: f64,
    delta_y: f64,
    sensitivity: f64,
    natural: bool,
    timestamp: Duration,
    tag: Option<&str>,
) {
    // Compute progress: accumulate the adjusted (post-sensitivity, post-natural)
    // primary axis delta. gesture-pixel-distance px ≈ 1 unit.
    let progress_unit = {
        let config = state.niri.config.borrow();
        config.input.touchscreen.swipe_progress_distance()
    };

    match kind {
        ContinuousGestureKind::WorkspaceSwitch => {
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
        ContinuousGestureKind::ViewScroll => {
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
        ContinuousGestureKind::OverviewToggle => {
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
        ContinuousGestureKind::Noop => {
            // No compositor animation — IPC progress is emitted below.
        }
    }

    // Emit IPC GestureProgress if this bind has a tag.
    if let Some(tag) = tag {
        // Compute adjusted delta for progress accumulation.
        let adjusted_delta = match kind {
            ContinuousGestureKind::WorkspaceSwitch | ContinuousGestureKind::OverviewToggle => {
                let dy = if natural { -delta_y } else { delta_y };
                dy * sensitivity
            }
            ContinuousGestureKind::ViewScroll => {
                let dx = if natural { -delta_x } else { delta_x };
                dx * sensitivity
            }
            ContinuousGestureKind::Noop => {
                // Use the dominant axis
                let dy = if natural { -delta_y } else { delta_y };
                let dx = if natural { -delta_x } else { delta_x };
                if dy.abs() > dx.abs() { dy * sensitivity } else { dx * sensitivity }
            }
        };

        // Update accumulated progress on the active Swipe bind or edge swipe.
        // Pinches take the `feed_continuous_pinch` path and never reach here.
        let progress = if let Some(ActiveTouchBind::Swipe { ipc_progress, .. }) =
            state.niri.touch_active_bind.as_mut()
        {
            *ipc_progress += adjusted_delta / progress_unit;
            *ipc_progress
        } else if let Some(TouchEdgeSwipeState::Active {
            ref mut ipc_progress, ..
        }) = state.niri.touch_edge_swipe
        {
            *ipc_progress += adjusted_delta / progress_unit;
            *ipc_progress
        } else {
            // Fallback: no accumulator reachable (shouldn't happen on the
            // hot path — the caller populates one of the two state slots
            // before calling here).
            adjusted_delta / progress_unit
        };

        let ts_ms = timestamp.as_millis() as u32;
        state.ipc_gesture_progress(
            tag.to_string(),
            progress,
            GestureDelta::Swipe {
                dx: delta_x,
                dy: delta_y,
            },
            ts_ms,
        );
    }
}

/// Feed spread delta to an active continuous pinch gesture.
///
/// Mirrors `feed_continuous_gesture` but drives the animation from change in
/// finger spread instead of linear dx/dy. Works for any finger count ≥ 3
/// (3-finger, 4-finger, 5-finger pinches all ride this path).
///
/// Sign convention: positive incremental spread = pinch-out (fingers spreading),
/// negative = pinch-in. For OverviewToggle we negate so pinch-in opens, matching
/// the legacy hardcoded behavior.
///
/// Uses `pinch_sensitivity` from the touchscreen gestures config for the
/// animation drive — not the bind's `sensitivity` property. Pinch has its
/// own tuning knob because raw spread-delta pixels need very different
/// scaling from linear swipe distances. At the default `1.0`, one pixel of
/// spread change contributes one pixel to the underlying gesture
/// accumulator, matching the scale swipes use.
fn feed_continuous_pinch(
    state: &mut State,
    kind: ContinuousGestureKind,
    timestamp: Duration,
    tag: Option<&str>,
) {
    // Batch the two config reads so we only borrow RefCell once per call.
    let (pinch_sensitivity, progress_unit) = {
        let config = state.niri.config.borrow();
        (
            config.input.touchscreen.pinch_sensitivity(),
            config.input.touchscreen.pinch_progress_distance(),
        )
    };

    let current_spread = calculate_spread(&state.niri.touch_gesture_points);

    // Destructure the active Pinch variant directly. If the active bind is
    // anything else (or None), something is badly wrong with the dispatch in
    // on_touch_motion — bail out cleanly rather than panic.
    let Some(ActiveTouchBind::Pinch {
        start_spread,
        last_spread,
        ..
    }) = state.niri.touch_active_bind.as_mut()
    else {
        return;
    };
    let incremental = current_spread - *last_spread;
    *last_spread = current_spread;
    let start_spread = *start_spread;

    match kind {
        ContinuousGestureKind::OverviewToggle => {
            // Pinch-in (negative incremental) → positive anim delta → overview opens.
            let delta = -incremental * pinch_sensitivity;
            if let Some(redraw) = state
                .niri
                .layout
                .overview_gesture_update(delta, timestamp)
            {
                if redraw {
                    state.niri.queue_redraw_all();
                }
            }
        }
        ContinuousGestureKind::WorkspaceSwitch => {
            // Semantically odd but not broken: pinch-out scrolls workspaces down.
            let delta = incremental * pinch_sensitivity;
            if state
                .niri
                .layout
                .workspace_switch_gesture_update(delta, timestamp, true)
                .is_some()
            {
                state.niri.queue_redraw_all();
            }
        }
        ContinuousGestureKind::ViewScroll => {
            let delta = incremental * pinch_sensitivity;
            if state
                .niri
                .layout
                .view_offset_gesture_update(delta, timestamp, true)
                .is_some()
            {
                state.niri.queue_redraw_all();
            }
        }
        ContinuousGestureKind::Noop => {
            // No compositor animation — IPC progress is emitted below.
        }
    }

    // Emit IPC GestureProgress for tagged pinch binds.
    if let Some(tag) = tag {
        // Signed, unbounded: positive = pinch-out, negative = pinch-in.
        // Unlike swipes, pinch progress is absolute (computed from start_spread
        // each frame) rather than accumulated — reversing the pinch gives a
        // direct inverse, with no drift from accumulated float error.
        let progress = (current_spread - start_spread) / progress_unit;
        if let Some(ActiveTouchBind::Pinch { ipc_progress, .. }) =
            state.niri.touch_active_bind.as_mut()
        {
            *ipc_progress = progress;
        }
        let ts_ms = timestamp.as_millis() as u32;
        state.ipc_gesture_progress(
            tag.to_string(),
            progress,
            GestureDelta::Pinch {
                d_spread: incremental,
            },
            ts_ms,
        );
    }
}

/// Feed the per-frame rotation delta to an active continuous rotation gesture.
///
/// Mirrors `feed_continuous_pinch`, but the scalar driving the animation is a
/// signed angular delta (radians, CCW positive) rather than a spread delta.
/// Unlike pinch, rotation must accumulate frame-to-frame because `atan2` wraps
/// at ±π and because fingers lifting shift the centroid; see
/// `calculate_rotation_delta` for the math and `rebase_rotation_basis` for
/// the finger-lift handling.
///
/// The rotation is converted to a linear animation delta by multiplying by
/// `pinch_sensitivity` (same knob as pinch — rotation shares the "radial
/// gesture" category). For OverviewToggle, CCW opens the overview to mirror
/// the pinch-in → open convention (both are "gather inward" motions).
fn feed_continuous_rotation(
    state: &mut State,
    kind: ContinuousGestureKind,
    timestamp: Duration,
    tag: Option<&str>,
) {
    // Batch config reads to hold the RefCell once per call.
    let (pinch_sensitivity, rotation_progress_angle) = {
        let config = state.niri.config.borrow();
        (
            config.input.touchscreen.pinch_sensitivity(),
            config.input.touchscreen.rotation_progress_angle(),
        )
    };

    // Compute this frame's angular delta and update the previous-angle basis.
    let (frame_rotation, new_angles) = calculate_rotation_delta(
        &state.niri.touch_gesture_points,
        &state.niri.touch_gesture_previous_angles,
    );
    state.niri.touch_gesture_previous_angles = new_angles;
    state.niri.touch_gesture_cumulative_rotation += frame_rotation;
    let cumulative_rotation = state.niri.touch_gesture_cumulative_rotation;

    // Destructure the active Rotate variant to read its start_rotation;
    // bail if misdispatched.
    let Some(ActiveTouchBind::Rotate { start_rotation, .. }) =
        state.niri.touch_active_bind.as_ref()
    else {
        return;
    };
    let start_rotation = *start_rotation;

    // Convert angular motion to an animation-accumulator scalar. Arc length
    // at a unit radius is the angular delta itself; scale by pinch_sensitivity
    // so users with pinch tuned to their taste get rotation that feels the
    // same. Multiply by a radius of 100 px to get units comparable to swipe
    // pixel deltas (π/2 rad ≈ 157 px of "motion").
    const ROTATION_PIXEL_RADIUS: f64 = 100.0;
    let anim_delta = frame_rotation * ROTATION_PIXEL_RADIUS * pinch_sensitivity;

    match kind {
        ContinuousGestureKind::OverviewToggle => {
            // CCW (positive frame_rotation) → positive anim delta → overview
            // opens. Matches the pinch-in "gather inward" convention.
            if let Some(redraw) = state
                .niri
                .layout
                .overview_gesture_update(anim_delta, timestamp)
            {
                if redraw {
                    state.niri.queue_redraw_all();
                }
            }
        }
        ContinuousGestureKind::WorkspaceSwitch => {
            if state
                .niri
                .layout
                .workspace_switch_gesture_update(anim_delta, timestamp, true)
                .is_some()
            {
                state.niri.queue_redraw_all();
            }
        }
        ContinuousGestureKind::ViewScroll => {
            if state
                .niri
                .layout
                .view_offset_gesture_update(anim_delta, timestamp, true)
                .is_some()
            {
                state.niri.queue_redraw_all();
            }
        }
        ContinuousGestureKind::Noop => {
            // No compositor animation — IPC progress is emitted below.
        }
    }

    // Emit IPC GestureProgress for tagged rotation binds.
    if let Some(tag) = tag {
        // Signed, unbounded: positive = CCW, negative = CW. Progress is the
        // rotation since recognition, normalized by the progress distance.
        // `cumulative_rotation - start_rotation` keeps the running metric
        // out of the progress math so the recognition-phase rotation isn't
        // included in the animation drive.
        let progress = (cumulative_rotation - start_rotation) / rotation_progress_angle;
        if let Some(ActiveTouchBind::Rotate { ipc_progress, .. }) =
            state.niri.touch_active_bind.as_mut()
        {
            *ipc_progress = progress;
        }
        let ts_ms = timestamp.as_millis() as u32;
        state.ipc_gesture_progress(
            tag.to_string(),
            progress,
            GestureDelta::Rotate {
                d_radians: frame_rotation,
            },
            ts_ms,
        );
    }
}

/// End a continuous gesture animation.
fn end_continuous_gesture(state: &mut State, kind: ContinuousGestureKind) {
    match kind {
        ContinuousGestureKind::WorkspaceSwitch => {
            if let Some(output) = state.niri.layout.workspace_switch_gesture_end(Some(true)) {
                state.niri.queue_redraw(&output);
            }
        }
        ContinuousGestureKind::ViewScroll => {
            if let Some(output) = state.niri.layout.view_offset_gesture_end(Some(true)) {
                state.niri.queue_redraw(&output);
            }
        }
        ContinuousGestureKind::OverviewToggle => {
            state.niri.layout.overview_gesture_end();
        }
        ContinuousGestureKind::Noop => {
            // No compositor animation to end.
        }
    }
}

/// Calculate the average spread of touch points (average distance from centroid).
fn calculate_spread(
    points: &std::collections::HashMap<
        Option<smithay::backend::input::TouchSlot>,
        smithay::utils::Point<f64, smithay::utils::Logical>,
    >,
) -> f64 {
    if points.len() < 2 {
        return 0.0;
    }

    let n = points.len() as f64;
    let (sum_x, sum_y) = points.values().fold((0.0, 0.0), |(sx, sy), p| {
        (sx + p.x, sy + p.y)
    });
    let centroid_x = sum_x / n;
    let centroid_y = sum_y / n;

    let total_dist: f64 = points.values().map(|p| {
        let dx = p.x - centroid_x;
        let dy = p.y - centroid_y;
        (dx * dx + dy * dy).sqrt()
    }).sum();

    total_dist / n
}

/// Compute per-slot angles (in radians) from the cluster centroid.
///
/// Only slots that have an actual `TouchSlot` identifier (not `None`) are
/// returned — angles have to be tracked across frames by slot, and `None`
/// slots can't be followed. Returns an empty map if fewer than 2 real slots
/// are present.
fn calculate_per_slot_angles(
    points: &std::collections::HashMap<
        Option<smithay::backend::input::TouchSlot>,
        smithay::utils::Point<f64, smithay::utils::Logical>,
    >,
) -> std::collections::HashMap<smithay::backend::input::TouchSlot, f64> {
    let mut out = std::collections::HashMap::new();
    let slotted: Vec<_> = points
        .iter()
        .filter_map(|(slot, pt)| slot.map(|s| (s, pt)))
        .collect();
    if slotted.len() < 2 {
        return out;
    }
    let n = slotted.len() as f64;
    let (sx, sy) = slotted.iter().fold((0.0, 0.0), |(ax, ay), (_, p)| {
        (ax + p.x, ay + p.y)
    });
    let cx = sx / n;
    let cy = sy / n;
    for (slot, pt) in slotted {
        // atan2(-dy, dx): screen y grows downward, so we flip the y axis to
        // get the mathematical convention where positive angles are
        // counter-clockwise *as the user sees them on the screen*. Without
        // the flip, a CCW rotation on the glass would produce a negative
        // angle delta in screen space, which is confusing for users.
        out.insert(slot, (-(pt.y - cy)).atan2(pt.x - cx));
    }
    out
}

/// Compute the averaged frame-to-frame rotation delta (in radians) across all
/// fingers present in both frames.
///
/// Returns `(frame_delta, new_angles)`:
/// - `frame_delta` is the signed average angular delta across fingers
///   present in both frames, with ±π unwrap applied. Positive = CCW.
///   A noise floor of 0.001 rad is applied: smaller values clamp to 0 to
///   prevent sub-threshold drift from accumulating into a false rotation on
///   held-still fingers.
/// - `new_angles` is the fresh per-slot angle map to store for the next
///   frame's comparison.
///
/// Returns `(0.0, new_angles)` with no accumulated delta when fewer than 2
/// fingers overlap between frames — the caller should still overwrite its
/// stored map so the next frame has a basis.
fn calculate_rotation_delta(
    current_points: &std::collections::HashMap<
        Option<smithay::backend::input::TouchSlot>,
        smithay::utils::Point<f64, smithay::utils::Logical>,
    >,
    previous_angles: &std::collections::HashMap<smithay::backend::input::TouchSlot, f64>,
) -> (
    f64,
    std::collections::HashMap<smithay::backend::input::TouchSlot, f64>,
) {
    use std::f64::consts::{PI, TAU};
    const NOISE_FLOOR: f64 = 0.001;

    let new_angles = calculate_per_slot_angles(current_points);
    if new_angles.is_empty() || previous_angles.is_empty() {
        return (0.0, new_angles);
    }

    let mut sum = 0.0;
    let mut count = 0usize;
    for (slot, &curr) in &new_angles {
        let Some(&prev) = previous_angles.get(slot) else {
            continue;
        };
        let raw = curr - prev;
        // Unwrap across the ±π boundary: any delta with |Δ| > π is on the
        // wrong side of the wrap; shift by 2π to get the short-way delta.
        let unwrapped = if raw > PI {
            raw - TAU
        } else if raw < -PI {
            raw + TAU
        } else {
            raw
        };
        sum += unwrapped;
        count += 1;
    }

    if count == 0 {
        return (0.0, new_angles);
    }

    let avg = sum / count as f64;
    let filtered = if avg.abs() < NOISE_FLOOR { 0.0 } else { avg };
    (filtered, new_angles)
}

/// Convert a gesture Trigger to its KDL config name for IPC events. The
/// emitted string echoes the same property form users write in `binds {}`
/// (e.g. `TouchSwipe fingers=3 direction="up"`) so IPC consumers can
/// string-match against their own config 1:1. Non-gesture variants fall
/// through to `"Unknown"` — this function is only meant for gesture
/// triggers.
pub(crate) fn trigger_to_ipc_name(trigger: Trigger) -> String {
    match trigger {
        Trigger::TouchSwipe { fingers, direction } => {
            format!(
                "TouchSwipe fingers={fingers} direction=\"{}\"",
                swipe_dir_name(direction)
            )
        }
        Trigger::TouchpadSwipe { fingers, direction } => {
            format!(
                "TouchpadSwipe fingers={fingers} direction=\"{}\"",
                swipe_dir_name(direction)
            )
        }
        Trigger::TouchPinch { fingers, direction } => {
            format!(
                "TouchPinch fingers={fingers} direction=\"{}\"",
                pinch_dir_name(direction)
            )
        }
        Trigger::TouchRotate { fingers, direction } => {
            format!(
                "TouchRotate fingers={fingers} direction=\"{}\"",
                rotate_dir_name(direction)
            )
        }
        Trigger::TouchTap { fingers } => {
            format!("TouchTap fingers={fingers}")
        }
        Trigger::TouchpadTapHold { fingers } => {
            format!("TouchpadTapHold fingers={fingers}")
        }
        Trigger::TouchpadTapHoldDrag { fingers } => {
            format!("TouchpadTapHoldDrag fingers={fingers}")
        }
        Trigger::TouchTapHoldDrag { fingers, direction } => match direction {
            Some(d) => format!(
                "TouchTapHoldDrag fingers={fingers} direction=\"{}\"",
                swipe_dir_name(d)
            ),
            None => format!("TouchTapHoldDrag fingers={fingers}"),
        },
        Trigger::TouchEdge { edge, zone } => {
            let edge_str = edge.as_kdl_name();
            match zone {
                None => format!("TouchEdge edge=\"{edge_str}\""),
                Some(z) => format!(
                    "TouchEdge edge=\"{edge_str}\" zone=\"{}\"",
                    niri_config::input::zone_kdl_name(edge, z)
                ),
            }
        }
        // Every current caller only passes gesture triggers. If that
        // invariant ever breaks we want to hear about it loudly in dev
        // rather than silently emitting "Unknown" into the IPC stream.
        other => {
            debug_assert!(
                false,
                "trigger_to_ipc_name called with non-gesture trigger: {other:?}"
            );
            "Unknown".to_string()
        }
    }
}

fn swipe_dir_name(d: SwipeDirection) -> &'static str {
    match d {
        SwipeDirection::Up => "up",
        SwipeDirection::Down => "down",
        SwipeDirection::Left => "left",
        SwipeDirection::Right => "right",
    }
}

fn pinch_dir_name(d: PinchDirection) -> &'static str {
    match d {
        PinchDirection::In => "in",
        PinchDirection::Out => "out",
    }
}

fn rotate_dir_name(d: RotateDirection) -> &'static str {
    match d {
        RotateDirection::Cw => "cw",
        RotateDirection::Ccw => "ccw",
    }
}


#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::f64::consts::{FRAC_PI_2, PI};

    use smithay::backend::input::TouchSlot;
    use smithay::utils::Point;

    use super::{calculate_per_slot_angles, calculate_rotation_delta};

    fn slot(n: u32) -> TouchSlot {
        // TouchSlot is From<Option<u32>>.
        TouchSlot::from(Some(n))
    }

    fn point(x: f64, y: f64) -> Point<f64, smithay::utils::Logical> {
        Point::from((x, y))
    }

    fn points_from(
        items: &[(u32, f64, f64)],
    ) -> HashMap<Option<TouchSlot>, Point<f64, smithay::utils::Logical>> {
        items
            .iter()
            .map(|(n, x, y)| (Some(slot(*n)), point(*x, *y)))
            .collect()
    }

    #[test]
    fn angles_empty_for_single_finger() {
        let pts = points_from(&[(0, 5.0, 5.0)]);
        assert!(calculate_per_slot_angles(&pts).is_empty());
    }

    #[test]
    fn angles_three_fingers_around_origin() {
        // Three fingers spaced 120° apart around the origin, so the
        // centroid is exactly (0, 0) and each finger lands on a known
        // angle in the screen-flipped math convention.
        //   0°: (10, 0) screen
        //   +120°: screen (10·cos 120°, -10·sin 120°) = (-5, -8.660)
        //   -120°: screen (10·cos -120°, -10·sin -120°) = (-5, +8.660)
        let r: f64 = 10.0;
        let pts = points_from(&[
            (0, r, 0.0),
            (1, r * 120.0_f64.to_radians().cos(), -r * 120.0_f64.to_radians().sin()),
            (2, r * (-120.0_f64).to_radians().cos(), -r * (-120.0_f64).to_radians().sin()),
        ]);
        let angles = calculate_per_slot_angles(&pts);
        let tolerance = 1e-9;
        assert!((angles[&slot(0)] - 0.0).abs() < tolerance, "slot 0 = {}", angles[&slot(0)]);
        assert!(
            (angles[&slot(1)] - 120.0_f64.to_radians()).abs() < tolerance,
            "slot 1 = {}",
            angles[&slot(1)]
        );
        assert!(
            (angles[&slot(2)] - (-120.0_f64).to_radians()).abs() < tolerance,
            "slot 2 = {}",
            angles[&slot(2)]
        );
    }

    /// Build a point set with N fingers arranged around the origin at the
    /// given angles (screen-flipped math convention: +x right, +y up on
    /// screen). Each finger is placed at radius 10.
    fn ring_points(
        angles: &[(u32, f64)],
    ) -> HashMap<Option<TouchSlot>, Point<f64, smithay::utils::Logical>> {
        let r = 10.0_f64;
        let items: Vec<(u32, f64, f64)> = angles
            .iter()
            .map(|(n, a)| (*n, r * a.cos(), -r * a.sin()))
            .collect();
        points_from(&items)
    }

    #[test]
    fn rotation_static_frames_is_zero() {
        let pts = ring_points(&[(0, 0.0), (1, 120.0_f64.to_radians()), (2, -120.0_f64.to_radians())]);
        let prev = calculate_per_slot_angles(&pts);
        let (delta, _) = calculate_rotation_delta(&pts, &prev);
        assert_eq!(delta, 0.0);
    }

    #[test]
    fn rotation_quarter_turn_ccw() {
        // Three fingers equally spaced 120° apart. Rotate the entire cluster
        // +90° (CCW as seen on screen) around the origin.
        let initial = ring_points(&[
            (0, 0.0),
            (1, 120.0_f64.to_radians()),
            (2, -120.0_f64.to_radians()),
        ]);
        let rotated = ring_points(&[
            (0, 90.0_f64.to_radians()),
            (1, 210.0_f64.to_radians()),
            (2, -30.0_f64.to_radians()),
        ]);
        let prev = calculate_per_slot_angles(&initial);
        let (delta, _) = calculate_rotation_delta(&rotated, &prev);
        // +90° CCW = +π/2.
        let tolerance = 1e-9;
        assert!((delta - FRAC_PI_2).abs() < tolerance, "delta = {delta}");
    }

    #[test]
    fn rotation_wrap_across_positive_pi() {
        // Two fingers 180° apart, prev at +170° and -10°. Both rotate +20° CCW:
        //   slot 0: +170° → +190° ≡ -170°  (wrap across +π)
        //   slot 1: -10°  → +10°            (normal)
        // Raw subtraction for slot 0 is (-170 - 170) = -340°, unwrap → +20°.
        // Average across fingers = +20° = +0.349 rad.
        let prev_points = ring_points(&[
            (0, 170.0_f64.to_radians()),
            (1, -10.0_f64.to_radians()),
        ]);
        let prev = calculate_per_slot_angles(&prev_points);
        let curr = ring_points(&[
            (0, -170.0_f64.to_radians()),
            (1, 10.0_f64.to_radians()),
        ]);
        let (delta, _) = calculate_rotation_delta(&curr, &prev);
        let expected = 20.0_f64.to_radians();
        assert!(
            (delta - expected).abs() < 1e-9,
            "delta = {delta}, expected ~{expected}"
        );
    }

    #[test]
    fn rotation_noise_floor_zeroes_tiny_delta() {
        // Two fingers nudged by < 0.001 rad each: averaged delta is
        // below the noise floor and should clamp to exactly 0.0.
        let prev_points = ring_points(&[(0, 0.0), (1, PI)]);
        let prev = calculate_per_slot_angles(&prev_points);
        let eps = 0.0005_f64;
        let curr = ring_points(&[(0, eps), (1, PI + eps)]);
        let (delta, _) = calculate_rotation_delta(&curr, &prev);
        assert_eq!(delta, 0.0);
    }
}
