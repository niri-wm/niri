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

use smithay::utils::{Logical, Point};

use crate::animation::{Animation, Clock, Curve};
use crate::cursor::CursorManager;

/// Tolerance in pixels for direction change detection.
/// Movements smaller than this are considered direction-neutral.
const TOLERANCE: f64 = 1.0;

#[derive(Debug, Clone)]
pub struct CursorScaleParams {
    /// Maximum scale multiplier for the cursor (e.g., 2.5 = 250% size).
    pub max_mult: f64,

    /// Time in milliseconds before cursor returns to normal size after movement stops.
    pub inactivity_timeout_ms: u64,

    /// Duration in milliseconds for the expansion animation.
    pub expand_duration_ms: u64,

    /// Duration in milliseconds for the decay (shrink) animation.
    pub decay_duration_ms: u64,

    /// Time window in milliseconds for collecting movement history.
    /// Positions older than this are discarded.
    pub shake_interval_ms: u64,

    /// Minimum shake factor (distance/diagonal ratio) to trigger cursor scaling.
    /// KDE uses 2.0: path must be at least 2x the bounding box diagonal.
    pub shake_sensitivity: f64,

    /// Minimum diagonal size in pixels of the bounding box to consider as a shake.
    /// Prevents tiny movements from triggering the effect.
    pub min_diagonal: f64,
}

impl Default for CursorScaleParams {
    fn default() -> Self {
        Self {
            max_mult: 4.5,
            inactivity_timeout_ms: 250,
            expand_duration_ms: 200,
            decay_duration_ms: 300,
            shake_interval_ms: 400,
            shake_sensitivity: 2.0,
            min_diagonal: 100.0,
        }
    }
}

/// A single point in the cursor movement history.
#[derive(Debug, Clone)]
struct HistoryItem {
    position: Point<f64, Logical>,
    timestamp: Instant,
}

/// Tracks cursor movement and detects shake gestures to scale the cursor.
#[derive(Debug)]
pub struct CursorScaleTracker {
    /// History of cursor positions within the shake detection window.
    history: Vec<HistoryItem>,

    /// Timestamp of the last cursor movement.
    last_motion_instant: Option<Instant>,

    /// Current cursor scale multiplier (1.0 = normal size).
    current_mult: f64,

    /// Active expansion animation (enlarging cursor).
    expand_anim: Option<Animation>,

    /// Active decay animation (shrinking cursor back to normal).
    decay_anim: Option<Animation>,

    /// Timestamp of the last cursor expansion (for cooldown).
    last_expand_instant: Option<Instant>,

    /// Configuration parameters.
    params: CursorScaleParams,

    /// Animation clock for creating timed animations.
    clock: Clock,
}

impl Default for CursorScaleTracker {
    fn default() -> Self {
        Self {
            history: Vec::new(),
            last_motion_instant: None,
            current_mult: 1.0,
            expand_anim: None,
            decay_anim: None,
            last_expand_instant: None,
            params: CursorScaleParams::default(),
            clock: Clock::default(),
        }
    }
}

impl CursorScaleTracker {
    pub fn new(clock: Clock, params: CursorScaleParams) -> Self {
        Self {
            history: Vec::new(),
            last_motion_instant: None,
            current_mult: 1.0,
            expand_anim: None,
            decay_anim: None,
            last_expand_instant: None,
            params,
            clock,
        }
    }

    /// Checks if two movement deltas have the same sign (direction).
    /// Movements smaller than TOLERANCE are considered direction-neutral.
    fn same_sign(a: f64, b: f64) -> bool {
        (a >= -TOLERANCE && b >= -TOLERANCE) || (a <= TOLERANCE && b <= TOLERANCE)
    }

    /// Updates the tracker with a new cursor position.
    //
    // This is the core of the shake detection algorithm. It:
    // 1. Maintains a history of positions (pruning old ones)
    // 2. Optimizes the history by only tracking direction changes
    // 3. Calculates the shake factor (path length / bounding box diagonal)
    // 4. Triggers cursor expansion when shake is detected
    pub fn on_motion(&mut self, pos: Point<f64, Logical>) {
        let now = Instant::now();
        self.last_motion_instant = Some(now);

        if self.decay_anim.is_some() {
            self.decay_anim = None;
        }

        if self.expand_anim.is_some() {
            return;
        }

        let shake_interval = std::time::Duration::from_millis(self.params.shake_interval_ms);

        self.history
            .retain(|item| now.duration_since(item.timestamp) < shake_interval);

        if self.history.len() >= 2 {
            let last_idx = self.history.len() - 1;
            let last = &self.history[last_idx];
            let prev = &self.history[last_idx - 1];

            let same_x =
                Self::same_sign(last.position.x - prev.position.x, pos.x - last.position.x);
            let same_y =
                Self::same_sign(last.position.y - prev.position.y, pos.y - last.position.y);

            if same_x && same_y {
                // Movement continues in the same direction: update the endpoint.
                self.history[last_idx] = HistoryItem {
                    position: pos,
                    timestamp: now,
                };
            } else {
                // Direction changed: add a new point.
                self.history.push(HistoryItem {
                    position: pos,
                    timestamp: now,
                });
            }
        } else {
            // First two points: always add.
            self.history.push(HistoryItem {
                position: pos,
                timestamp: now,
            });
        }

        // Need at least 2 points to calculate shake.
        if self.history.len() < 2 {
            return;
        }

        // Calculate bounding box and total path distance.
        let mut left = self.history[0].position.x;
        let mut top = self.history[0].position.y;
        let mut right = self.history[0].position.x;
        let mut bottom = self.history[0].position.y;
        let mut distance = 0.0;

        for i in 1..self.history.len() {
            let delta_x = self.history[i].position.x - self.history[i - 1].position.x;
            let delta_y = self.history[i].position.y - self.history[i - 1].position.y;
            distance += (delta_x * delta_x + delta_y * delta_y).sqrt();

            left = left.min(self.history[i].position.x);
            top = top.min(self.history[i].position.y);
            right = right.max(self.history[i].position.x);
            bottom = bottom.max(self.history[i].position.y);
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

            self.history.clear();
        }
    }

    /// Advances animations and triggers decay after inactivity.
    ///
    /// Returns `true` if the cursor size changed (requires redraw).
    pub fn advance_animations(&mut self, cursor_manager: &mut CursorManager) -> bool {
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
