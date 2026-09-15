// Inspiration: https://invent.kde.org/plasma/kwin/-/blob/96bc84d33d2a5913bfec17b96686b5f4bb4e41c8/src/plugins/shakecursor/shakedetector.cpp
//
// This module implements cursor scaling based on shake detection, inspired by KDE's shake cursor
// plugin. The algorithm detects when a user is "shaking" their mouse (moving it rapidly back and
// forth) to temporarily enlarge the cursor, making it easier to locate on screen - similar to
// macOS's cursor shake-to-find feature.
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

use std::time::{Duration, Instant};

use niri_config::{Animations, ShakeConfig};
use smithay::utils::{Logical, Point};
use tracing::{debug, trace};

use crate::animation::{Animation, Clock};
use crate::cursor::CursorManager;

/// Tolerance in pixels for direction change detection.
/// Movements smaller than this are considered direction-neutral.
const TOLERANCE: f64 = 1.0;

/// Behaviour modes for the shake feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShakeBehavior {
    /// Keep the cursor enlarged while the pointer is moving; start decay when pointer stops.
    HoldWhileMoving,
    /// Start decay when the shake intensity decreases.
    IntensityBased,
}

impl Default for ShakeBehavior {
    fn default() -> Self {
        ShakeBehavior::HoldWhileMoving
    }
}

#[derive(Debug, Clone)]
pub struct CursorScaleParams {
    off: bool,
    max_mult: f64,
    shake_interval_ms: u64,
    shake_sensitivity: f64,
    min_diagonal: f64,
    post_expand_delay_ms: u64,
    cooldown_ms: u64,
    behavior: ShakeBehavior,
    stopped_threshold_ms: u64,
    shake_relax_ms: u64,
    expand_anim_config: niri_config::animations::CursorExpandAnim,
    shrink_anim_config: niri_config::animations::CursorShrinkAnim,
}

impl CursorScaleParams {
    pub fn from_config(config: ShakeConfig, animations: &Animations) -> Self {
        Self {
            off: config.off,
            max_mult: config.max_multiplier,
            shake_interval_ms: config.shake_interval_ms,
            shake_sensitivity: config.sensitivity,
            min_diagonal: config.min_diagonal,
            post_expand_delay_ms: config.post_expand_delay_ms,
            cooldown_ms: config.cooldown_ms.unwrap_or(400),
            behavior: match config.behavior.as_deref() {
                Some("intensity") => ShakeBehavior::IntensityBased,
                _ => ShakeBehavior::HoldWhileMoving,
            },
            stopped_threshold_ms: config.stopped_threshold_ms.unwrap_or(50),
            shake_relax_ms: config.shake_relax_ms.unwrap_or(150),
            expand_anim_config: animations.cursor_expand,
            shrink_anim_config: animations.cursor_shrink,
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

    last_motion_instant: Option<Instant>,
    last_shake_factor: f64,

    current_mult: f64,
    expand_anim: Option<Animation>,
    decay_anim: Option<Animation>,

    last_expand_completed: Option<Instant>,
    pending_decay_at: Option<Instant>,
    cooldown_until: Option<Instant>,

    relax_start: Option<Instant>,

    params: CursorScaleParams,
    clock: Clock,
}

/// Checks if two movement deltas have the same sign (direction).
/// Movements smaller than TOLERANCE are considered direction-neutral.
fn same_sign(a: f64, b: f64) -> bool {
    (a >= -TOLERANCE && b >= -TOLERANCE) || (a <= TOLERANCE && b <= TOLERANCE)
}

impl CursorScaleTracker {
    pub fn new(clock: Clock, params: CursorScaleParams) -> Self {
        Self {
            history: Vec::new(),
            last_motion_instant: None,
            last_shake_factor: 0.0,
            current_mult: 1.0,
            expand_anim: None,
            decay_anim: None,
            last_expand_completed: None,
            pending_decay_at: None,
            cooldown_until: None,
            relax_start: None,
            params,
            clock,
        }
    }

    pub fn reload(&mut self, params: CursorScaleParams) {
        self.params = params;
    }

    /// Updates the tracker with a new cursor position.
    pub fn on_motion(&mut self, pos: Point<f64, Logical>) {
        if self.params.off {
            return;
        }
        let now = Instant::now();

        self.last_motion_instant = Some(now);

        // Track motion history even during decay animation to prepare for next shake
        let shake_interval = Duration::from_millis(self.params.shake_interval_ms);
        self.history.retain(|item| now.duration_since(item.timestamp) < shake_interval);

        // Update history with new position
        if self.history.len() >= 2 {
            let last_idx = self.history.len() - 1;
            let last = &self.history[last_idx];
            let prev = &self.history[last_idx - 1];

            let same_x = same_sign(last.position.x - prev.position.x, pos.x - last.position.x);
            let same_y = same_sign(last.position.y - prev.position.y, pos.y - last.position.y);

            if same_x && same_y {
                // Movement continues in the same direction: update the endpoint.
                self.history[last_idx] = HistoryItem {
                    position: pos,
                    timestamp: now,
                };
            } else {
                self.history.push(HistoryItem {
                    position: pos,
                    timestamp: now,
                });
            }
        } else {
            self.history.push(HistoryItem {
                position: pos,
                timestamp: now,
            });
        }

        // If a decay animation (shrink) is already running, do not interrupt it.
        if self.decay_anim.is_some() {
            trace!("Motion during decay animation, not interrupting");
            return;
        }

        // If an expansion animation is running, let it continue (but still track motion).
        if self.expand_anim.is_some() {
            trace!("Motion during expansion animation, continuing");
            return;
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

        // If movement area too small, treat as relaxed.
        if diagonal < self.params.min_diagonal {
            self.last_shake_factor = 0.0;
            trace!("Movement area too small: diagonal={:.2}, min={:.2}", diagonal, self.params.min_diagonal);

            // Intensity mode: start (or continue) relax timer.
            if self.params.behavior == ShakeBehavior::IntensityBased && self.current_mult > 1.01 {
                if self.relax_start.is_none() {
                    self.relax_start = Some(now);
                    debug!("Started relax timer (IntensityBased mode)");
                }
                // schedule decay only after sustained relaxation handled below
            }

            return;
        }

        let shake_factor = distance / diagonal;
        self.last_shake_factor = shake_factor;
        trace!("Shake factor: {:.2} (distance={:.2}, diagonal={:.2})", shake_factor, distance, diagonal);

        // If we're in cooldown, do not start a new expansion.
        if let Some(until) = self.cooldown_until {
            if now < until {
                trace!("In cooldown, ignoring shake");
                // Also reset relax state because we're ignoring expansions during cooldown.
                self.relax_start = None;
                return;
            }
        }

        // Expand if shake detected.
        if shake_factor > self.params.shake_sensitivity {
            debug!("Shake detected! Factor: {:.2} > sensitivity: {:.2}", shake_factor, self.params.shake_sensitivity);
            // If we were relaxing, cancel it because shake resumed strongly.
            self.relax_start = None;
            // cooldown for repeated expansions
            let cooldown_ok = if let Some(last) = self.last_expand_completed {
                now.duration_since(last).as_millis() as u64 >= 100
            } else {
                true
            };

            if cooldown_ok {
                debug!("Starting expansion animation: {:.2} -> {:.2}", self.current_mult, self.params.max_mult);
                let anim = Animation::new(
                    self.clock.clone(),
                    self.current_mult,
                    self.params.max_mult,
                    0.0,
                    self.params.expand_anim_config.0,
                );
                self.expand_anim = Some(anim);
                // clear any previously scheduled pending decay
                self.pending_decay_at = None;
            } else {
                trace!("Expansion blocked by cooldown");
            }

            self.history.clear();
        } else {
            // Relaxed (shake_factor <= sensitivity)
            if self.params.behavior == ShakeBehavior::IntensityBased && self.current_mult > 1.01 {
                // start or continue relax timer
                if self.relax_start.is_none() {
                    self.relax_start = Some(now);
                    debug!("Started relax timer (shake factor below sensitivity)");
                } else {
                    // if relaxed long enough, schedule decay (if not already pending)
                    let since = now.duration_since(self.relax_start.unwrap()).as_millis() as u64;
                    if since >= self.params.shake_relax_ms && self.pending_decay_at.is_none() {
                        debug!("Scheduling decay after {}ms of relaxation", since);
                        self.pending_decay_at =
                            Some(now + Duration::from_millis(self.params.post_expand_delay_ms));
                    }
                }
            }
            // For HoldWhileMoving we rely on the stop detection in advance_animations.
        }
    }

    /// Advances animations and triggers decay according to configured behavior.
    ///
    /// Returns `true` if the cursor size changed (requires redraw).
    /// Returns `true` if there are active or pending animations.
    pub fn has_animations(&self) -> bool {
        if self.params.off {
            return false;
        }

        self.expand_anim.is_some()
            || self.decay_anim.is_some()
            || self.pending_decay_at.is_some()
    }

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

            // When expansion finishes, record completion and (for HoldWhileMoving)
            // we may start scheduling decay from motion state in the next step.
            if anim.is_done() {
                debug!("Expansion animation completed");
                self.expand_anim = None;
                self.last_expand_completed = Some(now);
                // cancel any pending decay (we'll schedule based on behavior below)
                self.pending_decay_at = None;
                self.relax_start = None;
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
                debug!("Decay animation completed, entering cooldown for {}ms", self.params.cooldown_ms);
                self.decay_anim = None;
                // mark cooldown so we don't immediately re-expand
                self.cooldown_until = Some(now + Duration::from_millis(self.params.cooldown_ms));
                if (self.current_mult - 1.0).abs() > 0.0001 {
                    self.current_mult = 1.0;
                    cursor_manager.set_size_multiplier(1.0);
                    changed = true;
                }
            }

            return changed;
        }

        if self.current_mult > 1.01 {
            match self.params.behavior {
                ShakeBehavior::HoldWhileMoving => {
                    if let Some(last_motion) = self.last_motion_instant {
                        let elapsed_ms = now.duration_since(last_motion).as_millis() as u64;
                        if elapsed_ms >= self.params.stopped_threshold_ms {
                            if self.pending_decay_at.is_none() {
                                debug!("Pointer stopped for {}ms, scheduling decay (HoldWhileMoving)", elapsed_ms);
                                self.pending_decay_at =
                                    Some(now + Duration::from_millis(self.params.post_expand_delay_ms));
                            }
                        }
                    }
                }
                ShakeBehavior::IntensityBased => {
                    // If there is no recent motion, or we have a relax_start, compute whether relax
                    // has been sustained long enough to schedule decay.
                    if self.pending_decay_at.is_none() {
                        // If we have an explicit relax_start from on_motion, honor it.
                        if let Some(rs) = self.relax_start {
                            let since = now.duration_since(rs).as_millis() as u64;
                            if since >= self.params.shake_relax_ms {
                                self.pending_decay_at =
                                    Some(now + Duration::from_millis(self.params.post_expand_delay_ms));
                            }
                        } else if let Some(last_motion) = self.last_motion_instant {
                            // No relax_start recorded: if there's been no motion and last_shake_factor is relaxed,
                            // treat it as if relax_start happened at (now - stopped_threshold_ms).
                            let elapsed_ms = now.duration_since(last_motion).as_millis() as u64;
                            if elapsed_ms >= self.params.stopped_threshold_ms
                                && self.last_shake_factor <= self.params.shake_sensitivity
                            {
                                // pretend relax started stopped_threshold_ms ago
                                if self.params.shake_relax_ms <= elapsed_ms {
                                    self.pending_decay_at =
                                        Some(now + Duration::from_millis(self.params.post_expand_delay_ms));
                                } else {
                                    // start relax_start so future frames can count it
                                    self.relax_start =
                                        Some(now - Duration::from_millis(self.params.stopped_threshold_ms));
                                }
                            }
                        }
                    }
                }
            }

            // If a decay was scheduled and its time arrived, start it.
            if let Some(at) = self.pending_decay_at {
                if now >= at {
                    debug!("Starting decay animation: {:.2} -> 1.0", self.current_mult);
                    let anim = Animation::new(
                        self.clock.clone(),
                        self.current_mult,
                        1.0,
                        0.0,
                        self.params.shrink_anim_config.0,
                    );
                    self.decay_anim = Some(anim);
                    // clear pending and apply first frame
                    self.pending_decay_at = None;
                    let value = self.decay_anim.as_ref().unwrap().value();
                    if (self.current_mult - value).abs() > 0.001 {
                        self.current_mult = value;
                        cursor_manager.set_size_multiplier(self.current_mult as f32);
                        changed = true;
                    }
                }
            }
        }

        changed
    }
}
