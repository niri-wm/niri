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

    pub fn current_level(&self) -> f64 {
        self.transition.as_ref().map_or(self.level, |transition| {
            transition.current_level(self.level)
        })
    }

    pub fn current_focal(&self) -> Point<f64, Logical> {
        self.transition
            .as_ref()
            .and_then(|transition| {
                if transition.is_done() {
                    // Transition is done but not yet cleared; return the state's focal
                    // which should have been updated by apply_pending_transition().
                    // This prevents stale transition focal from being used.
                    None
                } else {
                    Some(transition.current_focal(self.current_level(), self.focal))
                }
            })
            .unwrap_or(self.focal)
    }

    pub fn apply_pending_transition(&mut self) {
        let Some(transition) = self.transition.take() else {
            return;
        };

        transition.apply_to_state(self);
        if !transition.is_done() {
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
        Point::from((
            pos.x.clamp(vp.loc.x, vp.loc.x + vp.size.w - f64::EPSILON),
            pos.y.clamp(vp.loc.y, vp.loc.y + vp.size.h - f64::EPSILON),
        ))
    }
}

/// Animation for zoom level changes.
#[derive(Debug, Clone)]
pub struct ZoomLevelAnimation {
    anim: Animation,
    target: f64,
    cursor_pos: Option<Point<f64, Logical>>,
    output_size: Option<Size<f64, Logical>>,
    movement_mode: Option<ZoomMovementMode>,
    on_edge_cursor_anchor: Option<Point<f64, Logical>>,
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
            cursor_pos: None,
            output_size: None,
            movement_mode: None,
            on_edge_cursor_anchor: None,
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
        self.cursor_pos = cursor_pos;
        self.output_size = output_size;
        self.movement_mode = movement_mode;
        self.on_edge_cursor_anchor =
            self.compute_on_edge_tracking_anchor(current_level, current_focal);
        self
    }

    pub fn should_use_dynamic_focal_tracking(&self, locked: bool, level_changed: bool) -> bool {
        level_changed
            && !locked
            && self.target > 1.0
            && self.cursor_pos.is_some()
            && self.output_size.is_some()
            && matches!(self.movement_mode.as_ref(), Some(ZoomMovementMode::OnEdge))
    }

    pub fn compute_on_edge_tracking_anchor(
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

    fn compute_focal_with_cursor_policy(
        &self,
        level: f64,
        fallback: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        let (Some(cursor), Some(size), Some(mode)) = (
            self.cursor_pos,
            self.output_size,
            self.movement_mode.as_ref(),
        ) else {
            return fallback;
        };

        if matches!(mode, ZoomMovementMode::OnEdge) {
            if let Some(anchor) = self.on_edge_cursor_anchor {
                return compute_focal_for_on_edge_anchor(cursor, level, size, anchor);
            }
        }

        compute_focal_for_cursor(cursor, level, size, mode)
    }

    pub fn set_cursor_pos(&mut self, cursor_pos: Point<f64, Logical>) {
        self.cursor_pos = Some(cursor_pos);
    }

    pub fn compute_focal_or(
        &self,
        level: f64,
        fallback: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        if matches!(self.movement_mode, Some(ZoomMovementMode::CursorFollow)) {
            return fallback;
        }
        self.compute_focal_with_cursor_policy(level, fallback)
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
    pub cursor_pos: Option<Point<f64, Logical>>,
    pub output_size: Option<Size<f64, Logical>>,
    pub movement_mode: Option<ZoomMovementMode>,
    pub on_edge_cursor_anchor: Option<Point<f64, Logical>>,
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
            cursor_pos,
            output_size,
            movement_mode,
            on_edge_cursor_anchor,
        }
    }

    pub fn should_use_dynamic_focal_tracking(
        &self,
        target_level: f64,
        level_changed: bool,
    ) -> bool {
        level_changed
            && target_level > 1.0
            && self.cursor_pos.is_some()
            && self.output_size.is_some()
            && matches!(self.movement_mode.as_ref(), Some(ZoomMovementMode::OnEdge))
    }

    fn compute_focal_with_cursor_policy(
        &self,
        level: f64,
        fallback: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        let (Some(cursor), Some(size), Some(mode)) = (
            self.cursor_pos,
            self.output_size,
            self.movement_mode.as_ref(),
        ) else {
            return fallback;
        };

        if matches!(mode, ZoomMovementMode::OnEdge) {
            if let Some(anchor) = self.on_edge_cursor_anchor {
                return compute_focal_for_on_edge_anchor(cursor, level, size, anchor);
            }
        }

        compute_focal_for_cursor(cursor, level, size, mode)
    }

    pub fn compute_focal_or(
        &self,
        level: f64,
        fallback: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        self.compute_focal_with_cursor_policy(level, fallback)
    }
}

/// Progress of zoom level changes - either animating or in a gesture.
#[derive(Debug, Clone)]
pub enum ZoomLevelProgress {
    Animation(ZoomLevelAnimation),
    Gesture(ZoomLevelGesture),
}

impl ZoomLevelProgress {
    pub fn level(&self) -> f64 {
        match self {
            ZoomLevelProgress::Animation(anim) => anim.value(),
            ZoomLevelProgress::Gesture(gesture) => gesture.current_level,
        }
    }

    pub fn focal_point(
        &self,
        current_level: f64,
        current_focal: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        match self {
            ZoomLevelProgress::Animation(anim) => {
                anim.compute_focal_or(current_level, current_focal)
            }
            ZoomLevelProgress::Gesture(gesture) => gesture.current_focal,
        }
    }

    pub fn is_animation(&self) -> bool {
        matches!(self, ZoomLevelProgress::Animation(_))
    }

    pub fn is_gesture(&self) -> bool {
        matches!(self, ZoomLevelProgress::Gesture(_))
    }

    pub fn is_done(&self) -> bool {
        match self {
            ZoomLevelProgress::Animation(anim) => anim.is_done(),
            ZoomLevelProgress::Gesture(_) => false,
        }
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

    pub fn is_done(&self) -> bool {
        self.x_anim.is_done() && self.y_anim.is_done()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ZoomTransition {
    level_progress: Option<ZoomLevelProgress>,
    focal_anim: Option<ZoomFocalAnimation>,
}

impl ZoomTransition {
    pub fn current_level(&self, fallback: f64) -> f64 {
        self.level_progress.as_ref().map_or(fallback, |p| p.level())
    }

    pub fn current_focal(
        &self,
        current_level: f64,
        fallback: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        if let Some(anim) = self.focal_anim.as_ref() {
            anim.value()
        } else if let Some(progress) = self.level_progress.as_ref() {
            progress.focal_point(current_level, fallback)
        } else {
            fallback
        }
    }

    pub fn level_is_animation(&self) -> bool {
        self.level_progress
            .as_ref()
            .is_some_and(|progress| progress.is_animation())
    }

    pub fn is_animation_ongoing(&self) -> bool {
        self.level_is_animation() || self.focal_anim.is_some()
    }

    pub fn set_level_animation(&mut self, anim: ZoomLevelAnimation) {
        self.level_progress = Some(ZoomLevelProgress::Animation(anim));
    }

    pub fn set_level_gesture(&mut self, gesture: ZoomLevelGesture) {
        self.level_progress = Some(ZoomLevelProgress::Gesture(gesture));
    }

    pub fn take_level_gesture(&mut self) -> Option<ZoomLevelGesture> {
        match self.level_progress.take() {
            Some(ZoomLevelProgress::Gesture(gesture)) => Some(gesture),
            Some(other) => {
                self.level_progress = Some(other);
                None
            }
            None => None,
        }
    }

    pub fn level_gesture_mut(&mut self) -> Option<&mut ZoomLevelGesture> {
        match self.level_progress.as_mut() {
            Some(ZoomLevelProgress::Gesture(gesture)) => Some(gesture),
            _ => None,
        }
    }

    pub fn set_focal_animation(&mut self, focal_anim: Option<ZoomFocalAnimation>) {
        self.focal_anim = focal_anim;
    }

    pub fn clear_focal_animation(&mut self) {
        self.focal_anim = None;
    }

    pub fn set_cursor_pos(&mut self, pos: Point<f64, Logical>) {
        if let Some(progress) = &mut self.level_progress {
            match progress {
                ZoomLevelProgress::Gesture(gesture) => {
                    gesture.cursor_pos = Some(pos);
                    gesture.current_focal =
                        gesture.compute_focal_or(gesture.current_level, gesture.current_focal);
                }
                ZoomLevelProgress::Animation(anim) => anim.set_cursor_pos(pos),
            }
        }
    }

    pub fn mark_transitioning(&self, zoom_state: &mut OutputZoomState) {
        zoom_state.transition = self.transitioning().then_some(self.clone());
    }

    pub fn begin_transition_from_state(
        &self,
        zoom_state: &mut OutputZoomState,
        level: f64,
        focal: Point<f64, Logical>,
    ) {
        zoom_state.level = level;
        zoom_state.focal = focal;
        self.mark_transitioning(zoom_state);
    }

    pub fn cancel_gesture_to_animation(
        &mut self,
        level_anim: ZoomLevelAnimation,
        clear_focal_animation: bool,
    ) {
        self.set_level_animation(level_anim);
        if clear_focal_animation {
            self.clear_focal_animation();
        }
    }

    pub fn finalize_gesture_to_animation(
        &mut self,
        level_anim: Option<ZoomLevelAnimation>,
        focal_anim: Option<ZoomFocalAnimation>,
        clear_focal_animation: bool,
    ) {
        if let Some(level_anim) = level_anim {
            self.set_level_animation(level_anim);
        }

        if let Some(focal_anim) = focal_anim {
            self.set_focal_animation(Some(focal_anim));
        } else if clear_focal_animation {
            self.clear_focal_animation();
        }
    }

    pub fn apply_to_state(&self, zoom_state: &mut OutputZoomState) {
        if let Some(progress) = self.level_progress.as_ref() {
            let current_level = progress.level();
            zoom_state.level = current_level;
            if self.focal_anim.is_none() {
                zoom_state.focal = progress.focal_point(current_level, zoom_state.focal);
            }
        }

        if let Some(anim) = self.focal_anim.as_ref() {
            zoom_state.focal = anim.value();
        }

        self.mark_transitioning(zoom_state);
    }

    pub fn transitioning(&self) -> bool {
        !self.is_done()
    }

    pub fn is_done(&self) -> bool {
        let level_done = self
            .level_progress
            .as_ref()
            .is_none_or(|progress| progress.is_done());
        let focal_done = self.focal_anim.as_ref().is_none_or(|anim| anim.is_done());

        level_done && focal_done
    }

    pub fn clear_if_done(&mut self) {
        if self.is_done() {
            self.level_progress = None;
            self.focal_anim = None;
        }
    }
}
