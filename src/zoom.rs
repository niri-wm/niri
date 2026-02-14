use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use niri_config::ZoomMovementMode;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};

use crate::rubber_band::RubberBand;

/// Minimum zoom level change before re-sending surface scales.
/// Prevents excessive protocol traffic during continuous zoom gestures.
pub const SCALE_CHANGE_THRESHOLD: f64 = 0.25;

/// Zoom rubber-banding constants (matching OVERVIEW_GESTURE_RUBBER_BAND pattern)
pub const ZOOM_GESTURE_RUBBER_BAND: RubberBand = RubberBand {
    stiffness: 0.5,
    limit: 0.05,
};

/// Convert log-space position to zoom level.
/// start_level * exp(log_pos) gives the new zoom level.
pub fn log_pos_to_zoom_level(start_level: f64, log_pos: f64) -> f64 {
    start_level * log_pos.exp()
}

/// Compute clamped zoom level with rubber-banding in log-space.
/// min_level and max_level define the zoom bounds (typically 1.0 and some max like 10.0).
pub fn clamp_zoom_level_with_rubber_band(level: f64, min_level: f64, max_level: f64) -> f64 {
    let log_level = level.ln();
    let log_min = min_level.ln();
    let log_max = max_level.ln();
    let clamped_log = ZOOM_GESTURE_RUBBER_BAND.clamp(log_min, log_max, log_level);
    clamped_log.exp()
}

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
    /// Surfaces currently in zoomed viewport
    pub zoomed_surfaces: HashMap<WlSurface, f64>, // Surface -> zoom level
    /// Last zoom level at which surface scales were sent (for debouncing)
    pub last_scale_update_level: Option<f64>,
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
            zoomed_surfaces: HashMap::new(),
            last_scale_update_level: None,
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
        zoomed_viewport(output_geometry, focal_global, self.level)
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

    /// Returns the zoom factor at which a surface was told to render via elevated
    /// preferred_scale. If the surface is not in `zoomed_surfaces` (i.e., it did not
    /// receive elevated scale), returns 1.0 — meaning it needs full compositor zoom.
    pub fn get_surface_zoom_factor(&self, surface: &WlSurface) -> f64 {
        self.zoomed_surfaces.get(surface).copied().unwrap_or(1.0)
    }

    /// Returns the rescale factor to apply during zoom rendering for a given surface.
    ///
    /// `rescale = zoom_factor / surface_zoom`:
    /// - If a surface rendered at 2.0× and zoom is 2.0×, rescale is 1.0 (no compositor upscale
    ///   needed — the surface already has the pixels).
    /// - If a surface didn't respond to elevated scale (1.0), rescale is the full `zoom_factor`.
    pub fn get_rescale_factor(&self, surface: &WlSurface, zoom_factor: f64) -> f64 {
        zoom_factor / self.get_surface_zoom_factor(surface)
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
            zoomed_surfaces: HashMap::new(),
            last_scale_update_level: None,
        }
    }
}

pub trait OutputZoomExt {
    fn zoom_state(&self) -> Option<MutexGuard<'_, OutputZoomState>>;
}

impl OutputZoomExt for Output {
    fn zoom_state(&self) -> Option<MutexGuard<'_, OutputZoomState>> {
        self.user_data()
            .get::<Mutex<OutputZoomState>>()?
            .lock()
            .ok()
    }
}

pub fn zoomed_viewport(
    output_rect: Rectangle<f64, Logical>,
    focal_point: Point<f64, Logical>,
    zoom_factor: f64,
) -> Rectangle<f64, Logical> {
    let mut geo = output_rect;
    geo.loc -= focal_point;
    geo = geo.downscale(zoom_factor);
    geo.loc += focal_point;
    geo
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

/// Compute the ratio of window area intersecting the zoom viewport (0.0–1.0).
pub fn calculate_visibility(
    window_geo: Rectangle<f64, Logical>,
    zoom_rect: Rectangle<f64, Logical>,
) -> f64 {
    let Some(intersect) = window_geo.intersection(zoom_rect) else {
        return 0.0;
    };

    let intersect_area = intersect.size.w * intersect.size.h;
    let window_area = window_geo.size.w * window_geo.size.h;

    if window_area <= 0.0 {
        return 0.0;
    }

    (intersect_area / window_area).clamp(0.0, 1.0)
}
