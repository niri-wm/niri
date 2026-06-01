use std::time::Duration;

use niri_config::ZoomMovementMode;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::animation::{Animation, Clock};
use crate::input::swipe_tracker::SwipeTracker;
use crate::utils::zoom::*;

/// Per-output zoom state. Layout writes these every animation tick;
/// external consumers read via `Layout`'s public API.
///
/// Level and focal transitions are stored independently — they share a clock
/// and config for synchronization but have separate lifecycles.
#[derive(Debug, Clone)]
pub struct OutputZoomState {
    pub level: f64,
    pub focal: Point<f64, Logical>,
    pub locked: bool,
    pub level_transition: ZoomLevelTransition,
    pub focal_animation: Option<ZoomFocalAnimation>,
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
            level_transition: ZoomLevelTransition::Idle,
            focal_animation: None,
        }
    }

    /// True when any transition (level or focal) is active and not yet done.
    pub fn transitioning(&self) -> bool {
        self.level_transition.is_active() || self.focal_animation.is_some()
    }

    /// Returns true when any `Animating` transition is active (not `Gesturing`).
    ///
    /// Used by `are_animations_ongoing()` to avoid driving the render loop
    /// during gestures — gesture updates already call `queue_redraw()`
    /// explicitly, so the VBlank-driven redraw loop is unnecessary and
    /// creates a render storm that can starve input processing.
    pub fn is_animating(&self) -> bool {
        matches!(self.level_transition, ZoomLevelTransition::Animating(_))
            || self.focal_animation.is_some()
    }

    pub fn snapshot_at(&self, now: Duration) -> ZoomSnapshot {
        let level = match &self.level_transition {
            ZoomLevelTransition::Animating(a) => a.value_at(now),
            ZoomLevelTransition::Gesturing(g) => g.current_level,
            ZoomLevelTransition::Idle => self.level,
        };

        let focal = match &self.focal_animation {
            Some(a) => a.value_at(now),
            None => {
                // When no focal animation is active, compute focal from the
                // active level transition's tracking context.
                match &self.level_transition {
                    ZoomLevelTransition::Animating(a) => {
                        a.tracking.compute_focal(level, self.focal)
                    }
                    ZoomLevelTransition::Gesturing(g) => g.compute_focal_or(level, g.current_focal),
                    ZoomLevelTransition::Idle => self.focal,
                }
            }
        };

        ZoomSnapshot {
            level,
            focal,
            locked: self.locked,
        }
    }

    pub fn apply_pending_transition_at(&mut self, now: Duration) {
        // Delegate to snapshot_at for the canonical level/focal computation,
        // then commit and sweep. This avoids duplicating the match logic.
        let snap = self.snapshot_at(now);
        self.level = snap.level;
        self.focal = snap.focal;
        self.level_transition.sweep_at(now);
        if let Some(a) = &self.focal_animation {
            if a.is_done_at(now) {
                self.focal_animation = None;
            }
        }
    }

    /// True when zoom is currently active. Reads the current animated level
    /// via `snapshot_at` so mid-transition calls return the correct state.
    pub fn is_active(&self, now: Duration) -> bool {
        self.snapshot_at(now).level > 1.0
    }

    /// Update cursor position on active transitions for focal tracking.
    pub fn set_cursor_pos(&mut self, pos: Point<f64, Logical>) {
        match &mut self.level_transition {
            ZoomLevelTransition::Animating(a) => a.set_cursor_pos(pos),
            ZoomLevelTransition::Gesturing(g) => g.set_cursor_pos(pos),
            ZoomLevelTransition::Idle => {}
        }
    }

    /// Update the movement mode on the active level transition's tracking
    /// context. The OnEdge anchor is recomputed for the new mode so that
    /// subsequent focal computations use the correct mode.
    ///
    /// Does nothing when no level transition is active — the movement mode
    /// is read fresh from config by `update_zoom_base_focal` in that case.
    pub fn update_movement_mode(&mut self, mode: ZoomMovementMode) {
        match &mut self.level_transition {
            ZoomLevelTransition::Animating(a) => {
                let level = a.value_at(a.sample_time());
                // Compute focal from the tracking context rather than
                // using self.focal directly — self.focal may be stale
                // (it's not updated until apply_pending_transition_at).
                let focal = a.tracking.compute_focal(level, self.focal);
                a.set_movement_mode(mode, level, focal);
            }
            ZoomLevelTransition::Gesturing(g) => {
                g.set_movement_mode(mode, g.current_level, g.current_focal);
            }
            ZoomLevelTransition::Idle => {}
        }
    }

    /// Viewport for the current animated zoom state.
    ///
    /// Reads the current level/focal via `snapshot_at` so mid-transition
    /// calls return the correct state.
    pub fn viewport_global(
        &self,
        output_geometry: Rectangle<f64, Logical>,
        now: Duration,
    ) -> Rectangle<f64, Logical> {
        let snap = self.snapshot_at(now);
        let focal_global = snap.focal + output_geometry.loc;
        apply_zoom_viewport(output_geometry, focal_global, snap.level)
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

    /// Update the movement mode. If the new mode is OnEdge, the cursor anchor
    /// is recomputed from the current cursor/level/focal so subsequent
    /// `compute_focal()` calls use the new mode's logic.
    ///
    /// `current_level` and `current_focal` are the zoom state at the time of
    /// the mode change — used to compute the OnEdge anchor.
    pub fn set_movement_mode(
        &mut self,
        mode: ZoomMovementMode,
        current_level: f64,
        current_focal: Point<f64, Logical>,
    ) {
        self.movement_mode = Some(mode);
        self.on_edge_cursor_anchor = self.compute_on_edge_anchor(current_level, current_focal);
    }
}

#[derive(Debug, Clone)]
pub struct ZoomLevelAnimation {
    pub(super) anim: Animation,
    pub(super) tracking: FocalTrackingContext,
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

    pub fn set_movement_mode(
        &mut self,
        mode: ZoomMovementMode,
        current_level: f64,
        current_focal: Point<f64, Logical>,
    ) {
        self.tracking
            .set_movement_mode(mode, current_level, current_focal);
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
        let mut result = Self {
            tracker: SwipeTracker::new(),
            start_level,
            current_level: start_level,
            current_focal,
            last_log_scale: None,
            tracking: FocalTrackingContext {
                cursor_pos,
                output_size,
                movement_mode,
                on_edge_cursor_anchor: None,
            },
        };
        result.tracking.on_edge_cursor_anchor = result
            .tracking
            .compute_on_edge_anchor(start_level, current_focal);
        result
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

    pub fn set_movement_mode(
        &mut self,
        mode: ZoomMovementMode,
        current_level: f64,
        current_focal: Point<f64, Logical>,
    ) {
        self.tracking
            .set_movement_mode(mode, current_level, current_focal);
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

/// Level transition: idle, scripted animation, or user-driven gesture.
#[derive(Debug, Clone, Default)]
pub enum ZoomLevelTransition {
    #[default]
    Idle,
    Animating(ZoomLevelAnimation),
    Gesturing(ZoomLevelGesture),
}

impl ZoomLevelTransition {
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    pub fn is_done_at(&self, now: Duration) -> bool {
        match self {
            Self::Animating(a) => a.is_done_at(now),
            Self::Gesturing(_) => false,
            Self::Idle => true,
        }
    }

    /// Clear completed `Animating` transitions.
    pub fn sweep_at(&mut self, now: Duration) {
        if let Self::Animating(a) = self {
            if a.is_done_at(now) {
                *self = Self::Idle;
            }
        }
    }

    pub fn take_gesture(&mut self) -> Option<ZoomLevelGesture> {
        match std::mem::take(self) {
            Self::Gesturing(g) => Some(g),
            other => {
                *self = other;
                None
            }
        }
    }

    pub fn gesture_mut(&mut self) -> Option<&mut ZoomLevelGesture> {
        match self {
            Self::Gesturing(g) => Some(g),
            _ => None,
        }
    }
}
