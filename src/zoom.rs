use std::sync::{Mutex, MutexGuard};

use niri_config::ZoomMovementMode;
use smithay::backend::renderer::element::utils::{RelocateRenderElement, RescaleRenderElement};
use smithay::output::Output;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};

// Define a type alias for the common zoom wrapper: Relocate(Rescale(T))
pub type ZoomWrapper<T> = RelocateRenderElement<RescaleRenderElement<T>>;

/// Per-output zoom snapshot.
///
/// This struct holds the effective zoom values that external consumers (backends,
/// input, niri rendering) read each frame. Layout writes these values every
/// animation tick, so they always reflect the current animation/gesture state.
///
/// Animation and gesture tracking live in `Monitor` inside the layout module.
#[derive(Debug, Clone)]
pub struct OutputZoomState {
    /// Current effective zoom level (updated by layout each frame).
    pub level: f64,
    /// Current effective focal point in output-local logical coordinates
    /// (updated by layout each frame).
    pub focal: Point<f64, Logical>,
    /// Whether focal point is locked.
    pub locked: bool,
    /// Whether a zoom animation or gesture is currently in progress.
    pub transitioning: bool,
    /// Cursor position used to compute focal_point, in output-local logical coords.
    /// When Some, render computes visual position as: focal + (cursor - focal) * zoom.
    /// When None, render uses pointer element position for the calculation.
    pub cursor_logical_pos: Option<Point<f64, Logical>>,
    /// Cursor hotspot in physical pixels. This is the offset from the cursor element's top-left to
    /// the hotspot point. Used for precise cursor positioning during zoom to avoid jitter.
    pub cursor_hotspot: Option<Point<i32, Physical>>,
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
            transitioning: false,
            cursor_logical_pos: None,
            cursor_hotspot: None,
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

    pub fn store_hotspot(&mut self, hotspot: Point<i32, Physical>) {
        if self.is_active() {
            self.cursor_hotspot = Some(hotspot);
        }
    }
}

impl Default for OutputZoomState {
    fn default() -> Self {
        Self {
            level: 1.0,
            focal: Point::from((0.0, 0.0)),
            locked: false,
            transitioning: false,
            cursor_logical_pos: None,
            cursor_hotspot: None,
        }
    }
}

pub trait OutputZoomExt {
    fn zoom_state(&self) -> Option<MutexGuard<'_, OutputZoomState>>;

    /// Returns true if zoom state has been initialized on this output.
    fn has_zoom_state(&self) -> bool {
        self.zoom_state().is_some()
    }

    /// Returns true if zoom is active (level > 1.0), false if no zoom or not available.
    fn zoom_is_active(&self) -> bool {
        self.zoom_state().is_some_and(|z| z.is_active())
    }

    /// Get current zoom level, returns 1.0 if not available.
    fn zoom_level(&self) -> f64 {
        self.zoom_state().map(|z| z.level).unwrap_or(1.0)
    }

    /// Get current focal point, returns origin if not available.
    fn zoom_focal(&self) -> Point<f64, Logical> {
        self.zoom_state()
            .map(|z| z.focal)
            .unwrap_or_else(|| Point::from((0.0, 0.0)))
    }

    /// Get locked state, returns false if not available.
    fn zoom_locked(&self) -> bool {
        self.zoom_state().map(|z| z.locked).unwrap_or(false)
    }

    /// Get transitioning state, returns false if not available.
    fn zoom_transitioning(&self) -> bool {
        self.zoom_state().map(|z| z.transitioning).unwrap_or(false)
    }

    /// Get cursor logical position, returns None if not available.
    fn zoom_cursor_logical_pos(&self) -> Option<Point<f64, Logical>> {
        self.zoom_state().and_then(|z| z.cursor_logical_pos)
    }

    /// Get cursor hotspot, returns None if not available.
    fn zoom_cursor_hotspot(&self) -> Option<Point<i32, Physical>> {
        self.zoom_state().and_then(|z| z.cursor_hotspot)
    }

    /// Compute viewport for the given output geometry if zoom is active.
    fn zoom_viewport_global(
        &self,
        output_geometry: Rectangle<f64, Logical>,
    ) -> Option<Rectangle<f64, Logical>> {
        let zoom_state = self.zoom_state()?;
        if zoom_state.is_active() {
            Some(zoom_state.viewport_global(output_geometry))
        } else {
            None
        }
    }

    /// Compute zoomed viewport for arbitrary focal/zoom.
    fn zoomed_geometry(
        &self,
        output_rect: Rectangle<f64, Logical>,
        focal_point: Point<f64, Logical>,
        zoom_factor: f64,
    ) -> Rectangle<f64, Logical> {
        apply_zoom_viewport(output_rect, focal_point, zoom_factor)
    }

    /// Clamp a position to the zoom viewport if zoom is active.
    fn zoom_clamp_to_viewport(
        &self,
        pos: Point<f64, Logical>,
        output_geometry: Rectangle<f64, Logical>,
    ) -> Option<Point<f64, Logical>> {
        let zoom_state = self.zoom_state()?;
        if zoom_state.is_active() {
            Some(zoom_state.clamp_to_viewport(pos, output_geometry))
        } else {
            None
        }
    }

    /// Store cursor hotspot if zoom is active. Returns true if stored.
    fn zoom_store_hotspot(&self, hotspot: Point<i32, Physical>) -> bool {
        if let Some(mut zoom_state) = self.zoom_state() {
            zoom_state.store_hotspot(hotspot);
            true
        } else {
            false
        }
    }
}

impl OutputZoomExt for Output {
    fn zoom_state(&self) -> Option<MutexGuard<'_, OutputZoomState>> {
        self.user_data()
            .get::<Mutex<OutputZoomState>>()?
            .lock()
            .ok()
    }
}

fn apply_zoom_viewport(
    mut output_rect: Rectangle<f64, Logical>,
    focal_point: Point<f64, Logical>,
    zoom_factor: f64,
) -> Rectangle<f64, Logical> {
    output_rect.loc -= focal_point;
    output_rect = output_rect.downscale(zoom_factor);
    output_rect.loc += focal_point;
    output_rect
}

pub fn compute_focal_for_cursor(
    cursor_local: Point<f64, Logical>,
    zoom_level: f64,
    output_size: Size<f64, Logical>,
    movement_mode: &ZoomMovementMode,
) -> Point<f64, Logical> {
    if zoom_level <= 1.0 {
        return cursor_local;
    }

    match movement_mode {
        ZoomMovementMode::CursorFollow => cursor_local,
        ZoomMovementMode::Centered | ZoomMovementMode::OnEdge => {
            let viewport_size = output_size.downscale(zoom_level);
            let viewport_loc = cursor_local - viewport_size.downscale(2.0).to_point();
            let scale_factor = zoom_level / (zoom_level - 1.0).max(0.001);
            let mut focal =
                Point::from((viewport_loc.x * scale_factor, viewport_loc.y * scale_factor));
            focal.x = focal.x.clamp(0.0, output_size.w - f64::EPSILON);
            focal.y = focal.y.clamp(0.0, output_size.h - f64::EPSILON);
            focal
        }
    }
}
