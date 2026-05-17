use std::time::Duration;

use niri_config::animations::{Curve, EasingParams, Kind};
use niri_config::ZoomMovementMode;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::animation::{Animation, Clock};
use crate::input::swipe_tracker::SwipeTracker;
use crate::utils::zoom::*;

/// Per-output zoom state.
///
/// This struct holds the effective zoom values that external consumers (backends,
/// input, niri rendering) read each frame. Layout writes these values every
/// animation tick, so they always reflect the current animation/gesture state.
///
/// Animation and gesture tracking live in the owned transition state.
///
/// Mutable ownership boundaries:
/// - Layout owns animated `level` / `focal` / `transition`.
/// - Input owns `locked` toggling.
#[derive(Debug, Clone)]
pub struct OutputZoomState {
    /// Current effective zoom level (layout-owned, updated each frame).
    pub level: f64,
    /// Current effective focal point in output-local logical coordinates
    /// (layout-owned, updated each frame).
    pub focal: Point<f64, Logical>,
    /// Whether focal point is locked (input-owned toggle).
    pub locked: bool,
    /// In-progress zoom transition, if any.
    pub transition: Option<ZoomTransition>,
}

impl OutputZoomState {
    /// Create a new zoom state centered on the given output.
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

    pub fn apply_pending_transition(&mut self) {
        let now = self
            .transition
            .as_ref()
            .and_then(ZoomTransition::sample_time)
            .unwrap_or(Duration::ZERO);
        self.apply_pending_transition_at(now);
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

    pub fn viewport_global(
        &self,
        output_geometry: Rectangle<f64, Logical>,
    ) -> Rectangle<f64, Logical> {
        let focal_global = self.focal + output_geometry.loc;
        apply_zoom_viewport(output_geometry, focal_global, self.level)
    }

    pub fn clamp_to_viewport(
        &self,
        pos: Point<f64, Logical>,
        output_geometry: Rectangle<f64, Logical>,
    ) -> Point<f64, Logical> {
        let vp = self.viewport_global(output_geometry);
        pos.constrain(Rectangle::new(
            vp.loc,
            vp.size - Size::from((f64::EPSILON, f64::EPSILON)),
        ))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ZoomSnapshot {
    pub level: f64,
    pub focal: Point<f64, Logical>,
    pub locked: bool,
}

impl ZoomSnapshot {
    pub fn inactive() -> Self {
        Self {
            level: 1.0,
            focal: Point::from((0.0, 0.0)),
            locked: false,
        }
    }

    pub fn is_active(self) -> bool {
        self.level > 1.0
    }
}

/// Shared cursor-tracking context for focal point computation.
///
/// Used by both `ZoomLevelAnimation` and `ZoomLevelGesture` to avoid
/// duplicating the focal tracking logic.
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
            && matches!(
                self.movement_mode.as_ref(),
                Some(ZoomMovementMode::OnEdge | ZoomMovementMode::CursorFollow)
            )
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

/// Animation for zoom level changes.
#[derive(Debug, Clone)]
pub struct ZoomLevelAnimation {
    anim: Animation,
    target: f64,
    tracking: FocalTrackingContext,
}

impl ZoomLevelAnimation {
    pub fn new(clock: Clock, from: f64, to: f64) -> Self {
        let config = niri_config::Animation {
            off: false,
            kind: Kind::Easing(EasingParams {
                duration_ms: 250,
                curve: Curve::EaseOutExpo,
            }),
        };

        Self {
            anim: Animation::new(clock, from, to, 0.0, config),
            target: to,
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

    pub fn value(&self) -> f64 {
        if self.anim.is_done() {
            self.target
        } else {
            self.anim.value()
        }
    }

    pub fn is_done(&self) -> bool {
        self.anim.is_done()
    }

    pub fn value_at(&self, now: Duration) -> f64 {
        if self.anim.is_done_at(now) {
            self.target
        } else {
            self.anim.value_at(now)
        }
    }

    pub fn is_done_at(&self, now: Duration) -> bool {
        self.anim.is_done_at(now)
    }

    pub fn sample_time(&self) -> Duration {
        self.anim.clock_now()
    }
}

/// Gesture tracking for zoom level changes.
#[derive(Debug, Clone)]
pub struct ZoomLevelGesture {
    pub tracker: SwipeTracker,
    pub start_level: f64,
    pub current_level: f64,
    pub current_focal: Point<f64, Logical>,
    /// Last log-scale value for computing log-space deltas from Wayland pinch events.
    /// Wayland provides absolute scale since gesture begin; we convert to log-space deltas.
    /// `None` means the first update hasn't been received yet.
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

/// Animation for focal point panning.
/// Uses separate X and Y animations to handle Point interpolation.
#[derive(Debug, Clone)]
pub struct ZoomFocalAnimation {
    pub x_anim: Animation,
    pub y_anim: Animation,
    pub target: Point<f64, Logical>,
    pub start: Point<f64, Logical>,
}

impl ZoomFocalAnimation {
    pub fn new(clock: Clock, from: Point<f64, Logical>, to: Point<f64, Logical>) -> Self {
        let config = niri_config::Animation {
            off: false,
            kind: Kind::Easing(EasingParams {
                duration_ms: 250,
                curve: Curve::CubicBezier(0.05, 0.7, 0.1, 1.0),
            }),
        };
        Self {
            x_anim: Animation::new(clock.clone(), from.x, to.x, 0.0, config),
            y_anim: Animation::new(clock, from.y, to.y, 0.0, config),
            target: to,
            start: from,
        }
    }

    /// Get the current focal point value.
    /// When both animations are done, returns the target.
    pub fn value(&self) -> Point<f64, Logical> {
        if self.is_done() {
            self.target
        } else {
            Point::from((self.x_anim.value(), self.y_anim.value()))
        }
    }

    pub fn value_at(&self, now: Duration) -> Point<f64, Logical> {
        if self.is_done_at(now) {
            self.target
        } else {
            Point::from((self.x_anim.value_at(now), self.y_anim.value_at(now)))
        }
    }

    pub fn is_done(&self) -> bool {
        self.x_anim.is_done() && self.y_anim.is_done()
    }

    pub fn is_done_at(&self, now: Duration) -> bool {
        self.x_anim.is_done_at(now) && self.y_anim.is_done_at(now)
    }

    pub fn sample_time(&self) -> Duration {
        self.x_anim.clock_now()
    }
}

/// In-progress zoom transition.
///
/// Holds either a level animation, a level gesture, or neither (focal-only animation).
#[derive(Debug, Clone, Default)]
pub struct ZoomTransition {
    level_anim: Option<ZoomLevelAnimation>,
    level_gesture: Option<ZoomLevelGesture>,
    focal_anim: Option<ZoomFocalAnimation>,
}

impl ZoomTransition {
    pub fn current_level(&self, fallback: f64) -> f64 {
        self.level_anim.as_ref().map_or(fallback, |a| a.value())
    }

    pub fn current_level_at(&self, fallback: f64, now: Duration) -> f64 {
        if let Some(anim) = &self.level_anim {
            return anim.value_at(now);
        }
        if let Some(g) = &self.level_gesture {
            return g.current_level;
        }
        fallback
    }

    pub fn current_focal(
        &self,
        current_level: f64,
        fallback: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        if let Some(anim) = &self.focal_anim {
            anim.value()
        } else if let Some(anim) = &self.level_anim {
            anim.tracking.compute_focal(current_level, fallback)
        } else if let Some(gesture) = &self.level_gesture {
            gesture.compute_focal_or(current_level, gesture.current_focal)
        } else {
            fallback
        }
    }

    pub fn current_focal_at(
        &self,
        current_level: f64,
        fallback: Point<f64, Logical>,
        now: Duration,
    ) -> Point<f64, Logical> {
        if let Some(anim) = &self.focal_anim {
            anim.value_at(now)
        } else if let Some(anim) = &self.level_anim {
            anim.tracking.compute_focal(current_level, fallback)
        } else if let Some(gesture) = &self.level_gesture {
            gesture.compute_focal_or(current_level, gesture.current_focal)
        } else {
            fallback
        }
    }

    pub fn level_is_animation(&self) -> bool {
        self.level_anim.is_some()
    }

    pub fn is_animation_ongoing(&self) -> bool {
        self.level_anim.is_some() || self.focal_anim.is_some()
    }

    pub fn level_gesture_mut(&mut self) -> Option<&mut ZoomLevelGesture> {
        self.level_gesture.as_mut()
    }

    pub fn take_level_gesture(&mut self) -> Option<ZoomLevelGesture> {
        self.level_gesture.take()
    }

    /// Start a new level animation from current state.
    pub fn start_level_animation(&mut self, anim: ZoomLevelAnimation) {
        self.level_anim = Some(anim);
        self.level_gesture = None;
    }

    /// Start a new gesture from current state.
    pub fn start_gesture(&mut self, gesture: ZoomLevelGesture) {
        self.level_gesture = Some(gesture);
        self.level_anim = None;
        self.focal_anim = None;
    }

    /// Start a focal-only animation (for unlock).
    pub fn start_focal_animation(&mut self, focal_anim: ZoomFocalAnimation) {
        self.focal_anim = Some(focal_anim);
    }

    /// End gesture, converting to deceleration animation.
    pub fn cancel_gesture(&mut self, level_anim: ZoomLevelAnimation, clear_focal: bool) {
        self.level_anim = Some(level_anim);
        self.level_gesture = None;
        if clear_focal {
            self.focal_anim = None;
        }
    }

    /// End gesture, optionally starting level and/or focal animations.
    pub fn finalize_gesture(
        &mut self,
        level_anim: Option<ZoomLevelAnimation>,
        focal_anim: Option<ZoomFocalAnimation>,
    ) {
        self.level_anim = level_anim;
        self.level_gesture = None;
        if let Some(focal_anim) = focal_anim {
            self.focal_anim = Some(focal_anim);
        }
    }

    pub fn set_cursor_pos(&mut self, pos: Point<f64, Logical>) {
        if let Some(anim) = &mut self.level_anim {
            anim.set_cursor_pos(pos);
        }
        if let Some(gesture) = &mut self.level_gesture {
            gesture.set_cursor_pos(pos);
        }
    }

    pub fn apply_to_state(&self, zoom_state: &mut OutputZoomState) {
        let now = self.sample_time().unwrap_or(Duration::ZERO);
        self.apply_to_state_at(zoom_state, now);
    }

    pub fn apply_to_state_at(&self, zoom_state: &mut OutputZoomState, now: Duration) {
        let current_level = self.current_level_at(zoom_state.level, now);
        zoom_state.level = current_level;
        if self.focal_anim.is_none() {
            zoom_state.focal = self.current_focal_at(current_level, zoom_state.focal, now);
        }

        if let Some(anim) = &self.focal_anim {
            zoom_state.focal = anim.value_at(now);
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
        let level_done = self.level_anim.as_ref().is_none_or(|a| a.is_done_at(now))
            && self.level_gesture.is_none();
        let focal_done = self.focal_anim.as_ref().is_none_or(|a| a.is_done_at(now));
        level_done && focal_done
    }

    pub fn sample_time(&self) -> Option<Duration> {
        self.level_anim
            .as_ref()
            .map(ZoomLevelAnimation::sample_time)
            .or_else(|| {
                self.focal_anim
                    .as_ref()
                    .map(ZoomFocalAnimation::sample_time)
            })
    }

    pub fn clear_if_done(&mut self) {
        if self.is_done() {
            self.level_anim = None;
            self.level_gesture = None;
            self.focal_anim = None;
        }
    }
}
