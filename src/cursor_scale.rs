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
//
// The tracker is a small state machine (`Idle -> Expanding -> Expanded -> Shrinking -> Idle`).
// All timestamps come from the animation `Clock`, so the shake timers follow the same time base
// (and slowdown rate) as the expand/shrink animations. While `Expanded` nothing animates, so the
// tracker exposes the next deadline through `next_wakeup()` and the compositor arms a timer for
// it; otherwise an idle pointer would never wake the event loop to start the shrink.

use std::time::Duration;

use niri_config::{Animations, ShakeConfig};
use smithay::utils::{Logical, Point};
use tracing::{debug, trace};

use crate::animation::{Animation, Clock};
use crate::cursor::CursorManager;

/// Tolerance in pixels for direction change detection.
/// Movements smaller than this are considered direction-neutral.
const TOLERANCE: f64 = 1.0;

/// Behaviour modes for the shake feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShakeBehavior {
    /// Keep the cursor enlarged while the pointer is moving; start decay when pointer stops.
    #[default]
    HoldWhileMoving,
    /// Start decay when the shake intensity decreases.
    IntensityBased,
}

#[derive(Debug, Clone)]
pub struct CursorScaleParams {
    off: bool,
    max_mult: f64,
    shake_interval: Duration,
    shake_sensitivity: f64,
    min_diagonal: f64,
    post_expand_delay: Duration,
    cooldown: Duration,
    behavior: ShakeBehavior,
    stopped_threshold: Duration,
    shake_relax: Duration,
    expand_anim_config: niri_config::animations::CursorExpandAnim,
    shrink_anim_config: niri_config::animations::CursorShrinkAnim,
}

impl CursorScaleParams {
    pub fn from_config(config: ShakeConfig, animations: &Animations) -> Self {
        Self {
            off: config.off,
            max_mult: config.max_multiplier,
            shake_interval: Duration::from_millis(config.shake_interval_ms),
            shake_sensitivity: config.sensitivity,
            min_diagonal: config.min_diagonal,
            post_expand_delay: Duration::from_millis(config.post_expand_delay_ms),
            cooldown: Duration::from_millis(config.cooldown_ms.unwrap_or(400)),
            behavior: match config.behavior.as_deref() {
                Some("intensity") => ShakeBehavior::IntensityBased,
                _ => ShakeBehavior::HoldWhileMoving,
            },
            stopped_threshold: Duration::from_millis(config.stopped_threshold_ms.unwrap_or(50)),
            shake_relax: Duration::from_millis(config.shake_relax_ms.unwrap_or(150)),
            expand_anim_config: animations.cursor_expand,
            shrink_anim_config: animations.cursor_shrink,
        }
    }
}

#[derive(Debug, Clone)]
struct HistoryItem {
    position: Point<f64, Logical>,
    timestamp: Duration,
}

#[derive(Debug, Default)]
enum ScaleState {
    /// Normal size, watching for shakes.
    #[default]
    Idle,
    /// Growing towards `max_mult`.
    Expanding(Animation),
    /// Fully grown, waiting for the decay deadline.
    Expanded {
        /// Clock time when the expansion finished.
        since: Duration,
    },
    /// Shrinking back to normal size.
    Shrinking(Animation),
}

/// Tracks cursor movement and detects shake gestures to scale the cursor.
#[derive(Debug)]
pub struct CursorScaleTracker {
    history: Vec<HistoryItem>,
    state: ScaleState,
    current_mult: f64,

    last_motion: Option<Duration>,
    /// Intensity mode: when the shake factor dropped below the sensitivity while expanded.
    relax_start: Option<Duration>,
    cooldown_until: Option<Duration>,

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
            state: ScaleState::Idle,
            current_mult: 1.0,
            last_motion: None,
            relax_start: None,
            cooldown_until: None,
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
        let now = self.clock.now();
        self.last_motion = Some(now);
        self.push_history(pos, now);

        match self.state {
            // Let running animations finish; the history keeps being tracked meanwhile.
            ScaleState::Expanding(_) | ScaleState::Shrinking(_) => (),
            ScaleState::Idle => self.maybe_expand(now),
            // HoldWhileMoving needs nothing here: the decay deadline follows `last_motion`.
            ScaleState::Expanded { .. } => {
                if self.params.behavior == ShakeBehavior::IntensityBased {
                    self.update_relax(now);
                }
            }
        }
    }

    fn push_history(&mut self, pos: Point<f64, Logical>, now: Duration) {
        let shake_interval = self.params.shake_interval;
        self.history
            .retain(|item| now.saturating_sub(item.timestamp) < shake_interval);

        let item = HistoryItem {
            position: pos,
            timestamp: now,
        };

        // Movement continuing in the same direction only moves the endpoint, so the history
        // keeps one point per direction change.
        if let [.., prev, last] = self.history.as_mut_slice() {
            let same_x = same_sign(last.position.x - prev.position.x, pos.x - last.position.x);
            let same_y = same_sign(last.position.y - prev.position.y, pos.y - last.position.y);
            if same_x && same_y {
                *last = item;
                return;
            }
        }

        self.history.push(item);
    }

    /// Ratio of travelled path length to bounding box diagonal of the recent history.
    ///
    /// Returns `None` when there is not enough movement to tell (too few points, or an area
    /// smaller than `min_diagonal`), which counts as not shaking.
    fn shake_factor(&self) -> Option<f64> {
        let first = self.history.first()?.position;
        let (mut left, mut top, mut right, mut bottom) = (first.x, first.y, first.x, first.y);
        let mut distance = 0.0;

        for pair in self.history.windows(2) {
            let (a, b) = (pair[0].position, pair[1].position);
            distance += (b.x - a.x).hypot(b.y - a.y);
            left = left.min(b.x);
            top = top.min(b.y);
            right = right.max(b.x);
            bottom = bottom.max(b.y);
        }

        let diagonal = (right - left).hypot(bottom - top);
        if self.history.len() < 2 || diagonal < self.params.min_diagonal {
            trace!(
                "movement area too small: diagonal={diagonal:.2}, min={:.2}",
                self.params.min_diagonal
            );
            return None;
        }

        let factor = distance / diagonal;
        trace!("shake factor: {factor:.2} (distance={distance:.2}, diagonal={diagonal:.2})");
        Some(factor)
    }

    fn is_shaking(&self) -> bool {
        self.shake_factor()
            .is_some_and(|factor| factor > self.params.shake_sensitivity)
    }

    fn maybe_expand(&mut self, now: Duration) {
        if self.cooldown_until.is_some_and(|until| now < until) {
            trace!("in cooldown, ignoring shake");
            return;
        }
        if !self.is_shaking() {
            return;
        }

        debug!(
            "shake detected, expanding cursor: {:.2} -> {:.2}",
            self.current_mult, self.params.max_mult
        );
        self.state = ScaleState::Expanding(Animation::new(
            self.clock.clone(),
            self.current_mult,
            self.params.max_mult,
            0.0,
            self.params.expand_anim_config.0,
        ));
        self.history.clear();
    }

    fn update_relax(&mut self, now: Duration) {
        if self.is_shaking() {
            self.relax_start = None;
        } else if self.relax_start.is_none() {
            debug!("shake relaxed, starting relax timer");
            self.relax_start = Some(now);
        }
    }

    /// Clock time at which an expanded cursor starts shrinking.
    fn decay_deadline(&self, since: Duration) -> Duration {
        let last_motion = self.last_motion.unwrap_or(since);
        let calm_since = match self.params.behavior {
            ShakeBehavior::HoldWhileMoving => last_motion + self.params.stopped_threshold,
            // Without a relax_start the pointer was still shaking when it stopped moving, so
            // relaxation counts from the last motion.
            ShakeBehavior::IntensityBased => {
                self.relax_start.unwrap_or(last_motion) + self.params.shake_relax
            }
        };
        calm_since.max(since) + self.params.post_expand_delay
    }

    /// Returns `true` while an animation needs a redraw every frame.
    #[must_use]
    pub fn has_animations(&self) -> bool {
        matches!(
            self.state,
            ScaleState::Expanding(_) | ScaleState::Shrinking(_)
        )
    }

    /// Multiplier the cursor is growing towards, while an expansion is running.
    ///
    /// The renderer uses it to prepare every texture size the animation will go through.
    #[must_use]
    pub fn expansion_target(&self) -> Option<f64> {
        match self.state {
            ScaleState::Expanding(_) => Some(self.params.max_mult),
            ScaleState::Idle | ScaleState::Expanded { .. } | ScaleState::Shrinking(_) => None,
        }
    }

    /// Clock time at which `advance_animations()` must run again even without any input.
    #[must_use]
    pub fn next_wakeup(&self) -> Option<Duration> {
        match self.state {
            ScaleState::Expanded { since } => Some(self.decay_deadline(since)),
            ScaleState::Idle | ScaleState::Expanding(_) | ScaleState::Shrinking(_) => None,
        }
    }

    /// Advances animations and triggers decay according to configured behavior.
    ///
    /// Returns `true` if the cursor size changed (requires redraw).
    pub fn advance_animations(&mut self, cursor_manager: &mut CursorManager) -> bool {
        if self.params.off {
            self.state = ScaleState::Idle;
            return self.set_mult(1.0, cursor_manager);
        }

        let now = self.clock.now();
        let mut changed = false;

        // Each step may transition into the next one within the same frame, so that a finished
        // expansion immediately evaluates its decay deadline instead of waiting for more input.
        if let ScaleState::Expanding(anim) = &self.state {
            let (value, done) = (anim.value(), anim.is_done());
            changed |= self.set_mult(value, cursor_manager);
            if done {
                debug!("cursor expansion completed");
                self.state = ScaleState::Expanded { since: now };
                self.relax_start = None;
            }
        }

        if let ScaleState::Expanded { since } = self.state {
            if now >= self.decay_deadline(since) {
                debug!("shrinking cursor: {:.2} -> 1.0", self.current_mult);
                self.state = ScaleState::Shrinking(Animation::new(
                    self.clock.clone(),
                    self.current_mult,
                    1.0,
                    0.0,
                    self.params.shrink_anim_config.0,
                ));
            }
        }

        if let ScaleState::Shrinking(anim) = &self.state {
            let (value, done) = (anim.value(), anim.is_done());
            changed |= self.set_mult(value, cursor_manager);
            if done {
                debug!(
                    "cursor shrink completed, cooldown for {:?}",
                    self.params.cooldown
                );
                changed |= self.set_mult(1.0, cursor_manager);
                self.state = ScaleState::Idle;
                self.cooldown_until = Some(now + self.params.cooldown);
            }
        }

        changed
    }

    fn set_mult(&mut self, value: f64, cursor_manager: &mut CursorManager) -> bool {
        if (self.current_mult - value).abs() <= 0.001 {
            return false;
        }
        self.current_mult = value;
        cursor_manager.set_size_multiplier(value as f32);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    struct Harness {
        clock: Clock,
        tracker: CursorScaleTracker,
        cursor_manager: CursorManager,
    }

    impl Harness {
        fn new(behavior: &str) -> Self {
            let clock = Clock::with_time(Duration::ZERO);
            let config = ShakeConfig {
                off: false,
                behavior: Some(behavior.to_owned()),
                ..ShakeConfig::default()
            };
            let params = CursorScaleParams::from_config(config, &Animations::default());
            Self {
                tracker: CursorScaleTracker::new(clock.clone(), params),
                clock,
                cursor_manager: CursorManager::new("default", 24),
            }
        }

        fn set_time(&mut self, time: Duration) {
            self.clock.set_unadjusted(time);
        }

        fn advance(&mut self) {
            self.tracker.advance_animations(&mut self.cursor_manager);
        }

        /// Shakes horizontally from `start`, one sample every 10 ms, and returns the time of the
        /// last sample.
        fn shake(&mut self, start: Duration, samples: u32) -> Duration {
            let mut time = start;
            for i in 0..samples {
                time = start + MS * 10 * i;
                self.set_time(time);
                let x = if i % 2 == 0 { 0. } else { 300. };
                self.tracker.on_motion(Point::from((x, 0.)));
            }
            time
        }

        fn mult(&self) -> f32 {
            self.cursor_manager.size_multiplier()
        }
    }

    #[test]
    fn shrinks_after_stopping_right_when_expansion_ends() {
        let mut h = Harness::new("hold");
        let last_motion = h.shake(Duration::ZERO, 6);
        assert!(h.tracker.has_animations());

        // The pointer stopped; the next event loop wakeup only comes once the expansion is over.
        h.set_time(last_motion + MS * 1000);
        h.advance();
        assert!(h.mult() > 1.);
        assert!(!h.tracker.has_animations());

        // Nothing animates now, so the compositor must be told when to wake up.
        let wakeup = h
            .tracker
            .next_wakeup()
            .expect("expanded cursor must schedule its decay");
        h.set_time(wakeup);
        h.advance();
        assert!(
            h.tracker.has_animations(),
            "decay must start without any input"
        );

        h.set_time(wakeup + MS * 1000);
        h.advance();
        assert_eq!(h.mult(), 1.);
        assert!(!h.tracker.has_animations());
        assert_eq!(h.tracker.next_wakeup(), None);
    }

    #[test]
    fn hold_keeps_cursor_expanded_while_moving() {
        let mut h = Harness::new("hold");
        let last_motion = h.shake(Duration::ZERO, 6);
        h.set_time(last_motion + MS * 1000);
        h.advance();
        let first_deadline = h.tracker.next_wakeup().unwrap();

        // Keep moving (no shake) past the original deadline.
        let mut time = last_motion + MS * 1000;
        while time <= first_deadline + MS * 200 {
            time += MS * 10;
            h.set_time(time);
            h.tracker
                .on_motion(Point::from((time.as_millis() as f64, 0.)));
            h.advance();
            assert!(!h.tracker.has_animations(), "must not shrink while moving");
        }
        assert!(h.tracker.next_wakeup().unwrap() > first_deadline);
    }

    #[test]
    fn intensity_shrinks_after_stopping_mid_shake() {
        let mut h = Harness::new("intensity");
        let last_motion = h.shake(Duration::ZERO, 6);
        h.set_time(last_motion + MS * 1000);
        h.advance();

        let wakeup = h.tracker.next_wakeup().unwrap();
        h.set_time(wakeup);
        h.advance();
        assert!(
            h.tracker.has_animations(),
            "decay must start without any input"
        );
    }
}
