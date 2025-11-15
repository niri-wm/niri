// Inspiration: https://invent.kde.org/plasma/kwin/-/blob/96bc84d33d2a5913bfec17b96686b5f4bb4e41c8/src/plugins/shakecursor/shakedetector.cpp
//
// This module implements cursor scaling based on shake detection, inspired by KDE's shake cursor
// plugin. The algorithm detects when a user is "shaking" their mouse (moving it rapidly back and
// forth) to temporarily enlarge the cursor, making it easier to locate on screen - similar to
// macOS's cursor shake-to-find feature.
//
// ## How Shake Detection Works
//
// The algorithm is elegant in its simplicity: it compares the actual distance traveled by the
// cursor against the straight-line distance (diagonal of the bounding box) of the movement.
//
// Key insight: When you shake your mouse back and forth, you travel a long distance while staying
// in a relatively small area. This creates a high ratio of path-length to bounding-box-diagonal.
//
// Example:
// - Straight movement: distance ≈ diagonal, ratio ≈ 1.0 (no shake)
// - Shake movement: distance >> diagonal, ratio > 2.0 (shake detected!)
//
// The algorithm maintains a history of cursor positions within a time window (400ms by default).
// It optimizes this history by only adding new points when the direction changes - movements in
// the same direction simply update the last point. This reduces noise and makes the calculation
// more efficient while preserving the essence of the movement pattern.
//
// When the shake factor (distance/diagonal) exceeds the sensitivity threshold (2.0) and the
// movement covers sufficient area (100px diagonal), the cursor smoothly animates to a larger size.
// After a period of inactivity, it smoothly returns to normal size.

use std::time::Instant;

use niri_config::ShakeConfig;
use smithay::utils::{Logical, Point};

use crate::animation::{Animation, Clock, Curve};
use crate::cursor::CursorManager;

/// Tolerance in pixels for direction change detection.
/// Movements smaller than this are considered direction-neutral.
const TOLERANCE: f64 = 1.0;

#[derive(Debug, Clone)]
pub struct CursorScaleParams {
    off: bool,
    max_mult: f64,
    inactivity_timeout_ms: u64,
    expand_duration_ms: u64,
    decay_duration_ms: u64,
    shake_interval_ms: u64,
    shake_sensitivity: f64,
    min_diagonal: f64,
}

impl From<ShakeConfig> for CursorScaleParams {
    fn from(config: ShakeConfig) -> Self {
        Self {
            off: config.off,
            max_mult: config.max_multiplier,
            inactivity_timeout_ms: config.inactivity_timeout_ms,
            expand_duration_ms: config.expand_duration_ms,
            decay_duration_ms: config.decay_duration_ms,
            shake_interval_ms: config.shake_interval_ms,
            shake_sensitivity: config.sensitivity,
            min_diagonal: config.min_diagonal,
        }
    }
}

#[derive(Debug, Clone)]
struct HistoryItem {
    position: Point<f64, Logical>,
    timestamp: Instant,
}

/// Tracks cursor movement and detects shake gestures to scale the cursor.
#[derive(Debug)]
pub struct CursorScaleTracker {
    history: Vec<HistoryItem>,
    // global position history
    global_history: Vec<HistoryItem>,
    last_motion_instant: Option<Instant>,
    current_mult: f64,
    expand_anim: Option<Animation>,
    decay_anim: Option<Animation>,
    last_expand_instant: Option<Instant>,
    params: CursorScaleParams,
    clock: Clock,
}

/// Checks if two movement deltas have the same sign (direction).
/// Movements smaller than TOLERANCE are considered direction-neutral.
fn same_sign(a: f64, b: f64) -> bool {
    (a >= -TOLERANCE && b >= -TOLERANCE) || (a <= TOLERANCE && b <= TOLERANCE)
}

impl CursorScaleTracker {
    pub fn new(clock: Clock, params: impl Into<CursorScaleParams>) -> Self {
        Self {
            history: Vec::new(),
            global_history: Vec::new(),
            last_motion_instant: None,
            current_mult: 1.0,
            expand_anim: None,
            decay_anim: None,
            last_expand_instant: None,
            params: params.into(),
            clock,
        }
    }

    pub fn reload(&mut self, params: impl Into<CursorScaleParams>) {
        self.params = params.into();
    }

    /// Updates the tracker with a new cursor position.
    pub fn on_motion(&mut self, is_global: bool, pos: Point<f64, Logical>) {
        if self.params.off {
            return;
        }
        let now = Instant::now();
        self.last_motion_instant = Some(now);

        if self.decay_anim.is_some() {
            self.decay_anim = None;
        }

        if self.expand_anim.is_some() {
            return;
        }

        let history = if is_global {
            &mut self.global_history
        } else {
            &mut self.history
        };
        let shake_interval = std::time::Duration::from_millis(self.params.shake_interval_ms);

        history
            .retain(|item| now.duration_since(item.timestamp) < shake_interval);

        if history.len() >= 2 {
            let last_idx = history.len() - 1;
            let last = &history[last_idx];
            let prev = &history[last_idx - 1];

            let same_x = same_sign(last.position.x - prev.position.x, pos.x - last.position.x);
            let same_y = same_sign(last.position.y - prev.position.y, pos.y - last.position.y);

            if same_x && same_y {
                // Movement continues in the same direction: update the endpoint.
                history[last_idx] = HistoryItem {
                    position: pos,
                    timestamp: now,
                };
            } else {
                history.push(HistoryItem {
                    position: pos,
                    timestamp: now,
                });
            }
        } else {
            history.push(HistoryItem {
                position: pos,
                timestamp: now,
            });
        }

        // Need at least 2 points to calculate shake.
        if history.len() < 2 {
            return;
        }

        // Calculate bounding box and total path distance.
        let mut left = history[0].position.x;
        let mut top = history[0].position.y;
        let mut right = history[0].position.x;
        let mut bottom = history[0].position.y;
        let mut distance = 0.0;

        for i in 1..history.len() {
            let delta_x = history[i].position.x - history[i - 1].position.x;
            let delta_y = history[i].position.y - history[i - 1].position.y;
            distance += (delta_x * delta_x + delta_y * delta_y).sqrt();

            left = left.min(history[i].position.x);
            top = top.min(history[i].position.y);
            right = right.max(history[i].position.x);
            bottom = bottom.max(history[i].position.y);
        }

        let bounds_width = right - left;
        let bounds_height = bottom - top;
        let diagonal = (bounds_width * bounds_width + bounds_height * bounds_height).sqrt();

        // Ignore very small movements.
        if diagonal < self.params.min_diagonal {
            return;
        }

        let shake_factor = distance / diagonal;

        if shake_factor > self.params.shake_sensitivity {
            let cooldown_ok = if let Some(last) = self.last_expand_instant {
                now.duration_since(last).as_millis() as u64 >= 100
            } else {
                true
            };

            if cooldown_ok {
                let anim = Animation::ease(
                    self.clock.clone(),
                    self.current_mult,
                    self.params.max_mult,
                    0.0,
                    self.params.expand_duration_ms,
                    Curve::EaseOutCubic,
                );
                self.expand_anim = Some(anim);
                self.last_expand_instant = Some(now);
            }

            history.clear();
        }
    }

    /// Advances animations and triggers decay after inactivity.
    ///
    /// Returns `true` if the cursor size changed (requires redraw).
    pub fn advance_animations(&mut self, cursor_manager: &mut CursorManager) -> bool {
        if self.params.off {
            return false;
        }
        let now = Instant::now();
        let mut changed = false;

        if let Some(anim) = &self.expand_anim {
            let value = anim.value();
            if (self.current_mult - value).abs() > 0.001 {
                self.current_mult = value;
                cursor_manager.set_size_multiplier(self.current_mult as f32);
                changed = true;
            }

            if anim.is_done() {
                self.expand_anim = None;
                self.last_expand_instant = Some(now);
            }

            return changed;
        }

        if let Some(anim) = &self.decay_anim {
            let value = anim.value();
            if (self.current_mult - value).abs() > 0.001 {
                self.current_mult = value;
                cursor_manager.set_size_multiplier(self.current_mult as f32);
                changed = true;
            }

            if anim.is_done() {
                self.decay_anim = None;
                self.current_mult = 1.0;
                cursor_manager.set_size_multiplier(1.0);
            }

            return changed;
        }

        if self.current_mult > 1.01 {
            if let Some(last_motion) = self.last_motion_instant {
                let elapsed_ms = now.duration_since(last_motion).as_millis() as u64;

                if elapsed_ms >= self.params.inactivity_timeout_ms {
                    let anim = Animation::ease(
                        self.clock.clone(),
                        self.current_mult,
                        1.0,
                        0.0,
                        self.params.decay_duration_ms,
                        Curve::EaseOutCubic,
                    );
                    self.decay_anim = Some(anim);
                    return true;
                }
            }
        }

        changed
    }
}
