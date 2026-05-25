use std::time::Duration;

use niri_config::ZoomMovementMode;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::animation::{Animation, Clock};
use crate::input::swipe_tracker::SwipeTracker;
use crate::utils::zoom::*;

/// Per-output zoom state. Layout writes these every animation tick;
/// external consumers read via `Layout`'s public API.
/// `level`/`focal`/`transition` are `pub(super)` (layout module only);
/// `locked` is `pub` (input-owned toggle).
#[derive(Debug, Clone)]
pub struct OutputZoomState {
    pub(super) level: f64,
    pub(super) focal: Point<f64, Logical>,
    pub locked: bool,
    pub(super) transition: Option<ZoomTransition>,
}

impl OutputZoomState {
    pub fn new_for_output(output: &Output) -> Self {
        let mode_size = output.current_mode().map_or((0, 0).into(), |m| m.size);
        let scale = output.current_scale().fractional_scale();
        let logical_size = mode_size.to_f64().to_logical(scale);
        Self {
            level: 1.0,
            focal: Point::from((logical_size.w / 2.0, logical_size.h / 2.0)),
            locked: false,
            transition: None,
        }
    }

    pub fn transitioning(&self) -> bool {
        self.transition
            .as_ref()
            .is_some_and(ZoomTransition::transitioning)
    }

    pub fn snapshot_at(&self, now: Duration) -> ZoomSnapshot {
        let level = self.transition.as_ref().map_or(self.level, |transition| {
            transition.current_level_at(self.level, now)
        });

        let focal = self.transition.as_ref().map_or(self.focal, |transition| {
            transition.current_focal_at(level, self.focal, now)
        });

        ZoomSnapshot {
            level,
            focal,
            locked: self.locked,
        }
    }

    pub fn apply_pending_transition_at(&mut self, now: Duration) {
        let Some(transition) = self.transition.take() else {
            return;
        };

        transition.apply_to_state_at(self, now);
        if !transition.is_done_at(now) {
            self.transition = Some(transition);
        }
    }

    pub fn is_active(&self) -> bool {
        self.level > 1.0
    }

    #[cfg(test)]
    pub fn test_new(
        level: f64,
        focal: Point<f64, Logical>,
        locked: bool,
        transition: Option<ZoomTransition>,
    ) -> Self {
        Self {
            level,
            focal,
            locked,
            transition,
        }
    }

    #[cfg(test)]
    pub fn test_level(&self) -> f64 {
        self.level
    }

    #[cfg(test)]
    pub fn test_focal(&self) -> Point<f64, Logical> {
        self.focal
    }

    #[cfg(test)]
    pub fn test_transition(&self) -> &Option<ZoomTransition> {
        &self.transition
    }

    pub fn viewport_global(
        &self,
        output_geometry: Rectangle<f64, Logical>,
    ) -> Rectangle<f64, Logical> {
        let focal_global = self.focal + output_geometry.loc;
        apply_zoom_viewport(output_geometry, focal_global, self.level)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ZoomSnapshot {
    pub level: f64,
    pub focal: Point<f64, Logical>,
    pub locked: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FocalTrackingContext {
    cursor_pos: Option<Point<f64, Logical>>,
    output_size: Option<Size<f64, Logical>>,
    movement_mode: Option<ZoomMovementMode>,
    on_edge_cursor_anchor: Option<Point<f64, Logical>>,
}

impl FocalTrackingContext {
    pub fn should_use_dynamic_focal_tracking(
        &self,
        target_level: f64,
        locked: bool,
        level_changed: bool,
    ) -> bool {
        level_changed
            && !locked
            && target_level > 1.0
            && self.cursor_pos.is_some()
            && self.output_size.is_some()
            && self.movement_mode.is_some()
    }

    pub fn compute_focal(&self, level: f64, fallback: Point<f64, Logical>) -> Point<f64, Logical> {
        compute_focal_for_zoom_level(
            self.cursor_pos,
            self.output_size,
            self.movement_mode.as_ref(),
            self.on_edge_cursor_anchor,
            level,
            fallback,
        )
    }

    pub fn compute_on_edge_anchor(
        &self,
        current_level: f64,
        current_focal: Point<f64, Logical>,
    ) -> Option<Point<f64, Logical>> {
        let (Some(cursor_local), Some(output_size)) = (self.cursor_pos, self.output_size) else {
            return None;
        };

        if matches!(self.movement_mode.as_ref(), Some(ZoomMovementMode::OnEdge)) {
            Some(compute_on_edge_cursor_anchor(
                cursor_local,
                current_level,
                current_focal,
                output_size,
            ))
        } else {
            None
        }
    }

    pub fn set_cursor_pos(&mut self, pos: Point<f64, Logical>) {
        self.cursor_pos = Some(pos);
    }
}

#[derive(Debug, Clone)]
pub struct ZoomLevelAnimation {
    anim: Animation,
    tracking: FocalTrackingContext,
}

impl ZoomLevelAnimation {
    pub fn new(clock: Clock, from: f64, to: f64, config: niri_config::Animation) -> Self {
        Self {
            anim: Animation::new(clock, from, to, 0.0, config),
            tracking: FocalTrackingContext::default(),
        }
    }

    pub fn with_tracking_context(
        mut self,
        cursor_pos: Option<Point<f64, Logical>>,
        output_size: Option<Size<f64, Logical>>,
        movement_mode: Option<ZoomMovementMode>,
        current_level: f64,
        current_focal: Point<f64, Logical>,
    ) -> Self {
        self.tracking.cursor_pos = cursor_pos;
        self.tracking.output_size = output_size;
        self.tracking.movement_mode = movement_mode;
        self.tracking.on_edge_cursor_anchor = self
            .tracking
            .compute_on_edge_anchor(current_level, current_focal);
        self
    }

    pub fn set_cursor_pos(&mut self, pos: Point<f64, Logical>) {
        self.tracking.set_cursor_pos(pos);
    }

    pub fn should_use_dynamic_focal_tracking(
        &self,
        target_level: f64,
        locked: bool,
        level_changed: bool,
    ) -> bool {
        self.tracking
            .should_use_dynamic_focal_tracking(target_level, locked, level_changed)
    }

    pub fn value_at(&self, now: Duration) -> f64 {
        self.anim.value_at(now)
    }

    pub fn is_done_at(&self, now: Duration) -> bool {
        self.anim.is_done_at(now)
    }

    pub fn sample_time(&self) -> Duration {
        self.anim.clock_now()
    }
}

#[derive(Debug, Clone)]
pub struct ZoomLevelGesture {
    pub tracker: SwipeTracker,
    pub start_level: f64,
    pub current_level: f64,
    pub current_focal: Point<f64, Logical>,
    pub last_log_scale: Option<f64>,
    tracking: FocalTrackingContext,
}

impl ZoomLevelGesture {
    pub fn new(
        start_level: f64,
        current_focal: Point<f64, Logical>,
        cursor_pos: Option<Point<f64, Logical>>,
        output_size: Option<Size<f64, Logical>>,
        movement_mode: Option<ZoomMovementMode>,
    ) -> Self {
        let on_edge_cursor_anchor =
            if let (Some(cursor_local), Some(output_size)) = (cursor_pos, output_size) {
                if matches!(movement_mode.as_ref(), Some(ZoomMovementMode::OnEdge)) {
                    Some(compute_on_edge_cursor_anchor(
                        cursor_local,
                        start_level,
                        current_focal,
                        output_size,
                    ))
                } else {
                    None
                }
            } else {
                None
            };

        Self {
            tracker: SwipeTracker::new(),
            start_level,
            current_level: start_level,
            current_focal,
            last_log_scale: None,
            tracking: FocalTrackingContext {
                cursor_pos,
                output_size,
                movement_mode,
                on_edge_cursor_anchor,
            },
        }
    }

    pub fn compute_focal_or(
        &self,
        level: f64,
        fallback: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        self.tracking.compute_focal(level, fallback)
    }

    pub fn cursor_pos(&self) -> Option<Point<f64, Logical>> {
        self.tracking.cursor_pos
    }

    pub fn output_size(&self) -> Option<Size<f64, Logical>> {
        self.tracking.output_size
    }

    pub fn movement_mode(&self) -> Option<&ZoomMovementMode> {
        self.tracking.movement_mode.as_ref()
    }

    pub fn set_cursor_pos(&mut self, pos: Point<f64, Logical>) {
        self.tracking.set_cursor_pos(pos);
    }

    pub fn set_output_size(&mut self, size: Size<f64, Logical>) {
        self.tracking.output_size = Some(size);
    }

    pub fn should_use_dynamic_focal_tracking(
        &self,
        target_level: f64,
        locked: bool,
        level_changed: bool,
    ) -> bool {
        self.tracking
            .should_use_dynamic_focal_tracking(target_level, locked, level_changed)
    }
}

#[derive(Debug, Clone)]
pub struct ZoomFocalAnimation {
    pub x_anim: Animation,
    pub y_anim: Animation,
}

impl ZoomFocalAnimation {
    pub fn new(
        clock: Clock,
        from: Point<f64, Logical>,
        to: Point<f64, Logical>,
        config: niri_config::Animation,
    ) -> Self {
        Self {
            x_anim: Animation::new(clock.clone(), from.x, to.x, 0.0, config),
            y_anim: Animation::new(clock, from.y, to.y, 0.0, config),
        }
    }

    pub fn value_at(&self, now: Duration) -> Point<f64, Logical> {
        Point::from((self.x_anim.value_at(now), self.y_anim.value_at(now)))
    }

    pub fn is_done_at(&self, now: Duration) -> bool {
        self.x_anim.is_done_at(now) && self.y_anim.is_done_at(now)
    }

    pub fn sample_time(&self) -> Duration {
        self.x_anim.clock_now()
    }
}

#[derive(Debug, Clone, Default)]
#[allow(clippy::large_enum_variant)]
pub enum ZoomTransition {
    #[default]
    Idle,
    Animating {
        level: Option<ZoomLevelAnimation>,
        focal: Option<ZoomFocalAnimation>,
    },
    /// Pinch gesture in progress.
    Gesturing(ZoomLevelGesture),
}

impl ZoomTransition {
    pub fn current_level_at(&self, fallback: f64, now: Duration) -> f64 {
        match self {
            Self::Animating {
                level: Some(level), ..
            } => level.value_at(now),
            Self::Gesturing(g) => g.current_level,
            _ => fallback,
        }
    }

    pub fn current_focal_at(
        &self,
        current_level: f64,
        fallback: Point<f64, Logical>,
        now: Duration,
    ) -> Point<f64, Logical> {
        match self {
            Self::Animating { focal: Some(f), .. } => f.value_at(now),
            Self::Animating {
                level: Some(level),
                focal: None,
            } => level.tracking.compute_focal(current_level, fallback),
            Self::Animating {
                level: None,
                focal: None,
            } => fallback,
            Self::Gesturing(g) => g.compute_focal_or(current_level, g.current_focal),
            Self::Idle => fallback,
        }
    }

    pub fn is_animation_ongoing(&self) -> bool {
        matches!(self, Self::Animating { .. })
    }

    pub fn level_gesture_mut(&mut self) -> Option<&mut ZoomLevelGesture> {
        match self {
            Self::Gesturing(g) => Some(g),
            _ => None,
        }
    }

    pub fn take_level_gesture(&mut self) -> Option<ZoomLevelGesture> {
        match std::mem::take(self) {
            Self::Gesturing(g) => Some(g),
            other => {
                *self = other;
                None
            }
        }
    }

    /// Start a new level animation from current state.
    pub fn start_level_animation(&mut self, anim: ZoomLevelAnimation) {
        *self = Self::Animating {
            level: Some(anim),
            focal: None,
        };
    }

    /// Start a new gesture from current state.
    pub fn start_gesture(&mut self, gesture: ZoomLevelGesture) {
        *self = Self::Gesturing(gesture);
    }

    /// Start a focal-only animation (for unlock or cursor-moved correction).
    /// The level stays stable while only the focal point animates.
    pub fn start_focal_animation(&mut self, focal_anim: ZoomFocalAnimation) {
        *self = Self::Animating {
            level: None,
            focal: Some(focal_anim),
        };
    }

    /// End gesture, converting to deceleration animation.
    /// Focal animation is always cleared because `start_gesture()` already
    /// cleared any prior focal animation when the gesture began.
    pub fn cancel_gesture(&mut self, level_anim: ZoomLevelAnimation) {
        *self = Self::Animating {
            level: Some(level_anim),
            focal: None,
        };
    }

    /// End gesture, optionally starting level and/or focal animations.
    pub fn finalize_gesture(
        &mut self,
        level_anim: Option<ZoomLevelAnimation>,
        focal_anim: Option<ZoomFocalAnimation>,
    ) {
        *self = match (level_anim, focal_anim) {
            (Some(level), Some(focal)) => Self::Animating {
                level: Some(level),
                focal: Some(focal),
            },
            (Some(level), None) => Self::Animating {
                level: Some(level),
                focal: None,
            },
            (None, Some(focal)) => Self::Animating {
                level: None,
                focal: Some(focal),
            },
            (None, None) => Self::Idle,
        };
    }

    pub fn set_cursor_pos(&mut self, pos: Point<f64, Logical>) {
        match self {
            Self::Animating {
                level: Some(level), ..
            } => level.set_cursor_pos(pos),
            Self::Gesturing(g) => g.set_cursor_pos(pos),
            _ => {}
        }
    }

    pub fn apply_to_state_at(&self, zoom_state: &mut OutputZoomState, now: Duration) {
        match self {
            Self::Idle => {}
            Self::Animating {
                level: Some(level),
                focal,
            } => {
                let current_level = level.value_at(now);
                zoom_state.level = current_level;
                if let Some(f) = focal {
                    zoom_state.focal = f.value_at(now);
                } else {
                    zoom_state.focal = level
                        .tracking
                        .compute_focal(current_level, zoom_state.focal);
                }
            }
            Self::Animating {
                level: None,
                focal: Some(f),
            } => {
                zoom_state.focal = f.value_at(now);
            }
            Self::Animating {
                level: None,
                focal: None,
            } => {}
            Self::Gesturing(g) => {
                zoom_state.level = g.current_level;
                zoom_state.focal = g.compute_focal_or(g.current_level, g.current_focal);
            }
        }
    }

    pub fn transitioning(&self) -> bool {
        !self.is_done()
    }

    pub fn is_done(&self) -> bool {
        let now = self.sample_time().unwrap_or(Duration::ZERO);
        self.is_done_at(now)
    }

    pub fn is_done_at(&self, now: Duration) -> bool {
        match self {
            Self::Animating { level, focal } => {
                let level_done = level.as_ref().is_none_or(|l| l.is_done_at(now));
                let focal_done = focal.as_ref().is_none_or(|f| f.is_done_at(now));
                level_done && focal_done
            }
            Self::Gesturing(_) => false,
            Self::Idle => true,
        }
    }

    pub fn sample_time(&self) -> Option<Duration> {
        match self {
            Self::Animating {
                level: Some(level), ..
            } => Some(level.sample_time()),
            Self::Animating {
                level: None,
                focal: Some(f),
            } => Some(f.sample_time()),
            _ => None,
        }
    }
}
