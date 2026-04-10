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
use std::time::Duration;

use smithay::backend::input::{Event as _, TouchEvent};
use smithay::input::touch::{
    DownEvent, GrabStartData as TouchGrabStartData, MotionEvent as TouchMotionEvent, UpEvent,
};
use smithay::utils::SERIAL_COUNTER;

use super::backend_ext::NiriInputBackend as InputBackend;
use super::move_grab::MoveGrab;
use super::touch_overview_grab::TouchOverviewGrab;
use super::{find_configured_bind, modifiers_from_state, PointerOrTouchStartData};
use niri_config::binds::Trigger;
use niri_config::input::{EdgeZone, ScreenEdge};
use niri_config::touch_binds::{
    continuous_gesture_kind, ContinuousGestureKind, TouchGestureType,
};
use niri_config::Action;

use crate::layout::LayoutElement;
use crate::niri::{ActiveTouchBind, PointerVisibility, State, TouchEdgeSwipeState};
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
            }
        }

        // First finger: check if it's in a screen edge zone for edge swipe detection.
        if was_empty && self.niri.touch_edge_swipe.is_none() {
            if let Some((output, pos_within_output)) = self.niri.output_under(pos) {
                let size = self.niri.global_space.output_geometry(output).unwrap().size;
                let config = self.niri.config.borrow();
                let threshold = config.input.touchscreen.edge_threshold();
                if let Some((edge, zone)) = detect_edge(pos_within_output, size, threshold) {
                    // Lookup order: zoned trigger first, then unzoned parent
                    // as fallback. `zoned` records which one hit so that all
                    // downstream lookups and the IPC name emitted on
                    // gesture-begin stay consistent with the bind that fired.
                    let mod_key = self.backend.mod_key(&config);
                    let mods = self.niri.seat.get_keyboard().unwrap()
                        .modifier_state();
                    let zoned_trigger = edge_zone_to_trigger(edge, zone);
                    let parent_trigger = edge_to_trigger(edge);
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

        // End gesture when all fingers are lifted.
        if self.niri.touch_gesture_points.is_empty() {
            self.niri.touch_gesture_cumulative = None;
            self.niri.touch_gesture_locked = false;
            self.niri.touchscreen_gesture_passthrough = false;
            // Take the active bind to get the tag before clearing.
            let active_tag = self
                .niri
                .touch_active_bind
                .take()
                .and_then(ActiveTouchBind::into_tag);
            self.niri.touch_gesture_initial_spread = None;

            // End any ongoing gesture animations.
            if let Some(output) = self.niri.layout.workspace_switch_gesture_end(Some(true)) {
                self.niri.queue_redraw(&output);
            }
            if let Some(output) = self.niri.layout.view_offset_gesture_end(Some(true)) {
                self.niri.queue_redraw(&output);
            }
            self.niri.layout.overview_gesture_end();

            // Emit IPC GestureEnd for tagged multi-finger gestures.
            if let Some(tag) = active_tag {
                self.ipc_gesture_end(tag, true);
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

            // Handle edge swipe gesture (takes priority over multi-finger gestures).
            // Extract state to avoid borrow conflicts with self.
            enum EdgeAction {
                None,
                PendingAccumulate {
                    edge: ScreenEdge,
                    zone: EdgeZone,
                    zoned: bool,
                    cx: f64,
                    cy: f64,
                    edge_slot: Option<smithay::backend::input::TouchSlot>,
                },
                ActiveFeed {
                    kind: ContinuousGestureKind,
                    sensitivity: f64,
                    natural: bool,
                    tag: Option<String>,
                },
            }

            let edge_action = match &mut self.niri.touch_edge_swipe {
                Some(TouchEdgeSwipeState::Pending {
                    edge,
                    zone,
                    zoned,
                    cumulative,
                    slot: edge_slot,
                }) if Some(slot) == *edge_slot => {
                    cumulative.0 += delta_x;
                    cumulative.1 += delta_y;
                    EdgeAction::PendingAccumulate {
                        edge: *edge,
                        zone: *zone,
                        zoned: *zoned,
                        cx: cumulative.0,
                        cy: cumulative.1,
                        edge_slot: *edge_slot,
                    }
                }
                Some(TouchEdgeSwipeState::Active {
                    kind, sensitivity, natural_scroll, tag, ..
                }) => EdgeAction::ActiveFeed {
                    kind: *kind,
                    sensitivity: *sensitivity,
                    natural: *natural_scroll,
                    tag: tag.clone(),
                },
                _ => EdgeAction::None,
            };

            match edge_action {
                EdgeAction::PendingAccumulate {
                    edge, zone, zoned, cx, cy, edge_slot,
                } => {
                    let threshold = {
                        let config = self.niri.config.borrow();
                        config.input.touchscreen.gesture_threshold()
                    };

                    if cx * cx + cy * cy >= threshold * threshold {
                        // Re-look-up the bind, preferring the zoned trigger
                        // if that's the one that matched at touch-down. The
                        // `zoned` flag was decided in `on_touch_down` so the
                        // same bind fires here regardless of whether a zoned
                        // or parent bind is in the config.
                        let trigger = if zoned {
                            edge_zone_to_trigger(edge, zone)
                        } else {
                            edge_to_trigger(edge)
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
                            // Emit IPC GestureBegin if this bind has a tag.
                            if let Some(ref tag) = tag {
                                let trigger_name = trigger_to_ipc_name(Some(trigger));
                                self.ipc_gesture_begin(
                                    tag.clone(),
                                    trigger_name,
                                    1, // edge swipes are single-finger
                                    kind.is_some(),
                                );
                            }

                            if let Some(kind) = kind {
                                // Continuous edge swipe gesture.
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
                                // Discrete edge swipe action — fire once and clear.
                                handle.cancel(self);
                                if !matches!(action, Action::Noop) {
                                    self.do_action(action, false);
                                }
                                // Emit immediate GestureEnd for discrete gestures.
                                if let Some(ref tag) = tag {
                                    self.ipc_gesture_end(tag.clone(), true);
                                }
                                self.niri.touch_edge_swipe = None;
                            }
                        } else {
                            self.niri.touch_edge_swipe = None;
                        }
                    }
                    // During Pending, don't suppress client motion events.
                }
                EdgeAction::ActiveFeed {
                    kind, sensitivity, natural, tag,
                } => {
                    let timestamp = Duration::from_micros(evt.time());
                    feed_continuous_gesture(
                        self, kind, delta_x, delta_y, sensitivity, natural, timestamp,
                        tag.as_deref(),
                    );
                    gesture_handled = true;
                }
                EdgeAction::None => {}
            }

            // Process gesture if tracking (3+ fingers or locked) and no edge swipe active.
            let finger_count = self.niri.touch_gesture_points.len();
            let gesture_active = finger_count >= 3 || self.niri.touch_gesture_locked;
            if gesture_active && self.niri.touch_edge_swipe.is_none() {
                let timestamp = Duration::from_micros(evt.time());
                gesture_handled = true;

                // Feed ongoing continuous gesture if one is active. Swipe and
                // pinch ride the same `touch_active_bind` slot but take
                // different feed paths — swipes are driven by linear dx/dy,
                // pinches by finger spread delta.
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
                    }
                } else if let Some((cx, cy)) = &mut self.niri.touch_gesture_cumulative {
                    // Recognition phase: accumulate raw deltas.
                    *cx += delta_x;
                    *cy += delta_y;

                    // Normalize by finger count at read time — 5 fingers each
                    // moving 5px shouldn't count as 25px of movement.
                    let finger_count_f = finger_count.max(1) as f64;
                    let (cx, cy) = (*cx / finger_count_f, *cy / finger_count_f);
                    let swipe_distance = (cx * cx + cy * cy).sqrt();

                    // Scale threshold by finger count — more fingers need more
                    // deliberate movement. This works because unlock-on-new-finger
                    // resets cumulative on EVERY new finger landing, so the user
                    // starts fresh with the correct finger count each time.
                    let (threshold, pinch_threshold, pinch_ratio) = {
                        let config = self.niri.config.borrow();
                        (
                            config.input.touchscreen.scaled_threshold(finger_count),
                            config.input.touchscreen.pinch_threshold(),
                            config.input.touchscreen.pinch_ratio(),
                        )
                    };

                    // Check if we've moved far enough for either swipe or pinch.
                    let current_spread = calculate_spread(&self.niri.touch_gesture_points);
                    let initial_spread =
                        self.niri.touch_gesture_initial_spread.unwrap_or(current_spread);
                    let spread_change = (current_spread - initial_spread).abs();

                    // Pinch detection: spread change must exceed both the
                    // pinch_threshold AND the swipe distance * pinch_ratio.
                    // This ensures pinch only fires when spread movement
                    // dominates over linear swipe movement.
                    let is_pinch = spread_change > pinch_threshold
                        && spread_change > swipe_distance * pinch_ratio;

                    // Also detect pinch when spread change is large enough on
                    // its own, even if swipe distance is also high. This handles
                    // the case where a pinch gesture also has a linear component
                    // (fingers moving inward AND slightly down).
                    let is_pinch = is_pinch
                        || (spread_change > pinch_threshold
                            && spread_change > swipe_distance);

                    // Entry: swipe crossed threshold, or pinch conditions met
                    // with spread_change also exceeding threshold (prevents wobble).
                    if swipe_distance >= threshold
                        || (is_pinch && spread_change >= threshold)
                    {
                        // Gesture recognized — clear cumulative.
                        self.niri.touch_gesture_cumulative = None;

                        // Discoverability log: surface the app-id of whatever
                        // window was under the touch at lock time, so users
                        // debugging "why isn't my app getting gestures" can
                        // see which app-id to add `touchscreen-gesture-passthrough`
                        // for in their window rules.
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

                        // Lock the gesture.
                        self.niri.touch_gesture_locked = true;
                        let handle = self.niri.seat.get_touch().unwrap();
                        handle.cancel(self);

                        // Determine gesture type.
                        let gesture_type = if is_pinch {
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

                        // Look up matching bind in the main binds {} block.
                        let bind_info = {
                            let config = self.niri.config.borrow();
                            let trigger = touch_gesture_to_trigger(
                                gesture_type,
                                finger_count as u8,
                            );
                            let mod_key = self.backend.mod_key(&config);
                            // Check current keyboard modifiers for Mod+Touch combos.
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

                        if let Some((kind, sensitivity, natural_scroll, tag, action)) = bind_info {
                            // Emit IPC GestureBegin if this bind has a tag.
                            if let Some(ref tag) = tag {
                                let trigger_name = trigger_to_ipc_name(
                                    touch_gesture_to_trigger(gesture_type, finger_count as u8),
                                );
                                self.ipc_gesture_begin(
                                    tag.clone(),
                                    trigger_name,
                                    finger_count as u8,
                                    kind.is_some(),
                                );
                            }

                            if let Some(kind) = kind {
                                // Continuous gesture — begin animation and store active bind.
                                begin_continuous_gesture(self, kind, pos);
                                let active = if is_pinch {
                                    ActiveTouchBind::Pinch {
                                        kind,
                                        tag,
                                        ipc_progress: 0.0,
                                        start_spread: current_spread,
                                        // Initialize last_spread = start_spread so the
                                        // first feed frame computes incremental ≈ 0,
                                        // avoiding a spurious jump on the recognition frame.
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
                                // Discrete action — fire once.
                                if !matches!(action, Action::Noop) {
                                    self.do_action(action, false);
                                }
                                // Emit immediate GestureEnd for discrete gestures.
                                if let Some(ref tag) = tag {
                                    self.ipc_gesture_end(tag.clone(), true);
                                }
                            }
                        }
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

        // Collect tags for IPC GestureEnd before clearing state.
        let active_tag = self
            .niri
            .touch_active_bind
            .take()
            .and_then(ActiveTouchBind::into_tag);
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

        // Cancel any ongoing gesture animations.
        self.niri.layout.workspace_switch_gesture_end(Some(false));
        self.niri.layout.view_offset_gesture_end(Some(false));
        self.niri.layout.overview_gesture_end();

        // Emit IPC GestureEnd (cancelled) for any tagged gestures.
        if let Some(tag) = active_tag {
            self.ipc_gesture_end(tag, false);
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
    match (gesture, finger_count) {
        (SwipeUp, 3) => Some(Trigger::TouchSwipe3Up),
        (SwipeDown, 3) => Some(Trigger::TouchSwipe3Down),
        (SwipeLeft, 3) => Some(Trigger::TouchSwipe3Left),
        (SwipeRight, 3) => Some(Trigger::TouchSwipe3Right),
        (SwipeUp, 4) => Some(Trigger::TouchSwipe4Up),
        (SwipeDown, 4) => Some(Trigger::TouchSwipe4Down),
        (SwipeLeft, 4) => Some(Trigger::TouchSwipe4Left),
        (SwipeRight, 4) => Some(Trigger::TouchSwipe4Right),
        (SwipeUp, 5) => Some(Trigger::TouchSwipe5Up),
        (SwipeDown, 5) => Some(Trigger::TouchSwipe5Down),
        (SwipeLeft, 5) => Some(Trigger::TouchSwipe5Left),
        (SwipeRight, 5) => Some(Trigger::TouchSwipe5Right),
        (PinchIn, 3) => Some(Trigger::TouchPinch3In),
        (PinchOut, 3) => Some(Trigger::TouchPinch3Out),
        (PinchIn, 4) => Some(Trigger::TouchPinch4In),
        (PinchOut, 4) => Some(Trigger::TouchPinch4Out),
        (PinchIn, 5) => Some(Trigger::TouchPinch5In),
        (PinchOut, 5) => Some(Trigger::TouchPinch5Out),
        (EdgeSwipeLeft, _) => Some(Trigger::TouchEdgeLeft),
        (EdgeSwipeRight, _) => Some(Trigger::TouchEdgeRight),
        (EdgeSwipeTop, _) => Some(Trigger::TouchEdgeTop),
        (EdgeSwipeBottom, _) => Some(Trigger::TouchEdgeBottom),
        _ => None,
    }
}

/// Convert a screen edge to its unzoned (parent) Trigger for bind lookup.
///
/// Used both as the "match any zone" fallback when no zoned bind exists, and
/// as the IPC name when an unzoned bind is the one that fired.
fn edge_to_trigger(edge: ScreenEdge) -> Trigger {
    match edge {
        ScreenEdge::Left => Trigger::TouchEdgeLeft,
        ScreenEdge::Right => Trigger::TouchEdgeRight,
        ScreenEdge::Top => Trigger::TouchEdgeTop,
        ScreenEdge::Bottom => Trigger::TouchEdgeBottom,
    }
}

/// Convert an (edge, zone) pair to its zoned Trigger for bind lookup.
///
/// The zone suffix uses directional words rotated per edge axis: Top/Bottom
/// edges split along x (Left/Center/Right), Left/Right edges split along y
/// (Top/Center/Bottom). `EdgeZone::{Start, Center, End}` is the abstract
/// per-axis ordering; this function maps it to the concrete variant.
fn edge_zone_to_trigger(edge: ScreenEdge, zone: EdgeZone) -> Trigger {
    match (edge, zone) {
        (ScreenEdge::Top, EdgeZone::Start) => Trigger::TouchEdgeTopLeft,
        (ScreenEdge::Top, EdgeZone::Center) => Trigger::TouchEdgeTopCenter,
        (ScreenEdge::Top, EdgeZone::End) => Trigger::TouchEdgeTopRight,
        (ScreenEdge::Bottom, EdgeZone::Start) => Trigger::TouchEdgeBottomLeft,
        (ScreenEdge::Bottom, EdgeZone::Center) => Trigger::TouchEdgeBottomCenter,
        (ScreenEdge::Bottom, EdgeZone::End) => Trigger::TouchEdgeBottomRight,
        (ScreenEdge::Left, EdgeZone::Start) => Trigger::TouchEdgeLeftTop,
        (ScreenEdge::Left, EdgeZone::Center) => Trigger::TouchEdgeLeftCenter,
        (ScreenEdge::Left, EdgeZone::End) => Trigger::TouchEdgeLeftBottom,
        (ScreenEdge::Right, EdgeZone::Start) => Trigger::TouchEdgeRightTop,
        (ScreenEdge::Right, EdgeZone::Center) => Trigger::TouchEdgeRightCenter,
        (ScreenEdge::Right, EdgeZone::End) => Trigger::TouchEdgeRightBottom,
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
        config.input.touchscreen.gesture_progress_distance()
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
        state.ipc_gesture_progress(tag.to_string(), progress, delta_x, delta_y, ts_ms);
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
        // Pinch has no natural 2D delta: the gesture is radial, not linear.
        // We report (delta_x=0, delta_y=incremental_spread) by convention:
        //   - Consumers that only read `progress` get the right answer.
        //   - Consumers that want per-frame motion magnitude can read delta_y.
        //   - Consumers that happen to read delta_x for a pinch get a stable 0
        //     instead of garbage, so a reversed-sign bug is impossible.
        // This convention is documented on the GestureProgress struct in
        // niri-ipc/src/lib.rs.
        state.ipc_gesture_progress(tag.to_string(), progress, 0.0, incremental, ts_ms);
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

/// Convert a Trigger to its KDL config name for IPC events.
fn trigger_to_ipc_name(trigger: Option<Trigger>) -> String {
    let Some(trigger) = trigger else {
        return "Unknown".to_string();
    };
    match trigger {
        Trigger::TouchSwipe3Up => "TouchSwipe3Up",
        Trigger::TouchSwipe3Down => "TouchSwipe3Down",
        Trigger::TouchSwipe3Left => "TouchSwipe3Left",
        Trigger::TouchSwipe3Right => "TouchSwipe3Right",
        Trigger::TouchSwipe4Up => "TouchSwipe4Up",
        Trigger::TouchSwipe4Down => "TouchSwipe4Down",
        Trigger::TouchSwipe4Left => "TouchSwipe4Left",
        Trigger::TouchSwipe4Right => "TouchSwipe4Right",
        Trigger::TouchSwipe5Up => "TouchSwipe5Up",
        Trigger::TouchSwipe5Down => "TouchSwipe5Down",
        Trigger::TouchSwipe5Left => "TouchSwipe5Left",
        Trigger::TouchSwipe5Right => "TouchSwipe5Right",
        Trigger::TouchPinch3In => "TouchPinch3In",
        Trigger::TouchPinch3Out => "TouchPinch3Out",
        Trigger::TouchPinch4In => "TouchPinch4In",
        Trigger::TouchPinch4Out => "TouchPinch4Out",
        Trigger::TouchPinch5In => "TouchPinch5In",
        Trigger::TouchPinch5Out => "TouchPinch5Out",
        Trigger::TouchEdgeLeft => "TouchEdgeLeft",
        Trigger::TouchEdgeRight => "TouchEdgeRight",
        Trigger::TouchEdgeTop => "TouchEdgeTop",
        Trigger::TouchEdgeBottom => "TouchEdgeBottom",
        Trigger::TouchEdgeTopLeft => "TouchEdgeTop:Left",
        Trigger::TouchEdgeTopCenter => "TouchEdgeTop:Center",
        Trigger::TouchEdgeTopRight => "TouchEdgeTop:Right",
        Trigger::TouchEdgeBottomLeft => "TouchEdgeBottom:Left",
        Trigger::TouchEdgeBottomCenter => "TouchEdgeBottom:Center",
        Trigger::TouchEdgeBottomRight => "TouchEdgeBottom:Right",
        Trigger::TouchEdgeLeftTop => "TouchEdgeLeft:Top",
        Trigger::TouchEdgeLeftCenter => "TouchEdgeLeft:Center",
        Trigger::TouchEdgeLeftBottom => "TouchEdgeLeft:Bottom",
        Trigger::TouchEdgeRightTop => "TouchEdgeRight:Top",
        Trigger::TouchEdgeRightCenter => "TouchEdgeRight:Center",
        Trigger::TouchEdgeRightBottom => "TouchEdgeRight:Bottom",
        _ => "Unknown",
    }
    .to_string()
}
