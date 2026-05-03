use niri_config::ZoomMovementMode;
use smithay::output::Output;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Size};
pub fn apply_zoom_viewport(
    mut output_rect: Rectangle<f64, Logical>,
    focal_point: Point<f64, Logical>,
    zoom_factor: f64,
) -> Rectangle<f64, Logical> {
    output_rect.loc -= focal_point;
    output_rect = output_rect.downscale(zoom_factor);
    output_rect.loc += focal_point;
    output_rect
}

/// Canonical per-output display cursor position helper.
///
/// Given a global cursor position, the per-output origin and size, and the
/// current zoom state (level and focal point), return the global cursor position
/// where the cursor would be displayed on that output when zoom is active.
///
/// Semantics follow ZoomTransformInputs::display_position(): if the cursor is
/// outside the output, return None. Otherwise return the clamped position in
/// global coordinates, preserving the existing viewport clamping rules and
/// out-of-output semantics.
pub fn canonical_display_cursor_global_pos(
    global_pointer: Point<f64, Logical>,
    output_pos: Point<f64, Logical>,
    output_size: Size<f64, Logical>,
    zoom_level: f64,
    zoom_focal: Point<f64, Logical>,
) -> Option<Point<f64, Logical>> {
    // Reuse the existing transformation pathway to ensure consistent semantics.
    let inputs = ZoomTransformInputs::new(
        output_pos,
        global_pointer,
        output_size,
        zoom_level,
        zoom_focal,
    );
    inputs.display_position().map(|p| p + output_pos)
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

            let mut focal = viewport_loc.upscale(scale_factor);
            focal.x = focal.x.clamp(0.0, output_size.w - f64::EPSILON);
            focal.y = focal.y.clamp(0.0, output_size.h - f64::EPSILON);

            focal
        }
    }
}

pub fn compute_zoom_base_focal_update(
    output: &Output,
    output_geometry: Rectangle<f64, Logical>,
    cursor_position: Point<f64, Logical>,
    old_pos_global: Option<Point<f64, Logical>>,
    focal_point: Point<f64, Logical>,
    zoom_factor: f64,
    movement_mode: &ZoomMovementMode,
) -> Option<Point<f64, Logical>> {
    match movement_mode {
        ZoomMovementMode::CursorFollow => Some(cursor_position),
        ZoomMovementMode::Centered => Some(compute_focal_for_cursor(
            cursor_position,
            zoom_factor,
            output_geometry.size,
            &ZoomMovementMode::Centered,
        )),
        ZoomMovementMode::OnEdge => compute_on_edge_zoom_update(
            output,
            output_geometry,
            cursor_position,
            old_pos_global,
            focal_point,
            zoom_factor,
        ),
    }
}

fn compute_on_edge_zoom_update(
    output: &Output,
    output_geometry: Rectangle<f64, Logical>,
    cursor_position: Point<f64, Logical>,
    old_pos_global: Option<Point<f64, Logical>>,
    focal_point: Point<f64, Logical>,
    zoom_factor: f64,
) -> Option<Point<f64, Logical>> {
    let recentered = || {
        Some(compute_focal_for_cursor(
            cursor_position,
            zoom_factor,
            output_geometry.size,
            &ZoomMovementMode::Centered,
        ))
    };

    let Some(old_pos) = old_pos_global else {
        return recentered();
    };

    let focal_global = focal_point + output_geometry.loc;
    let zoomed_geometry_global = apply_zoom_viewport(output_geometry, focal_global, zoom_factor);

    let jump_threshold = (16.0 * output.current_scale().fractional_scale()) / zoom_factor;
    let jump_detect_size: Size<f64, Logical> = (jump_threshold, jump_threshold).into();
    let original_rect = Rectangle::new(
        old_pos - jump_detect_size.downscale(2.0).to_point(),
        jump_detect_size,
    );

    if !zoomed_geometry_global.overlaps_or_touches(original_rect) {
        return recentered();
    }

    if zoomed_geometry_global.contains(cursor_position + output_geometry.loc) {
        return None;
    }

    let scale = zoom_factor / (zoom_factor - 1.0);
    let viewport_size = output_geometry.size.downscale(zoom_factor);
    let output_rect = Rectangle::from_size(output_geometry.size);
    let zoomed_geometry_local = apply_zoom_viewport(output_rect, focal_point, zoom_factor);

    let mut new_focal = focal_point;
    let vp_left = zoomed_geometry_local.loc.x;
    let vp_right = vp_left + zoomed_geometry_local.size.w;
    let vp_top = zoomed_geometry_local.loc.y;
    let vp_bottom = vp_top + zoomed_geometry_local.size.h;

    if cursor_position.x < vp_left {
        new_focal.x = cursor_position.x * scale;
    } else if cursor_position.x > vp_right {
        new_focal.x = (cursor_position.x - viewport_size.w) * scale;
    }

    if cursor_position.y < vp_top {
        new_focal.y = cursor_position.y * scale;
    } else if cursor_position.y > vp_bottom {
        new_focal.y = (cursor_position.y - viewport_size.h) * scale;
    }

    new_focal.x = new_focal
        .x
        .clamp(0.0, output_geometry.size.w - f64::EPSILON);
    new_focal.y = new_focal
        .y
        .clamp(0.0, output_geometry.size.h - f64::EPSILON);

    Some(new_focal)
}

pub fn zoom_display_cursor_logical(
    pointer_local: Point<f64, Logical>,
    output_size: Size<f64, Logical>,
    zoom_level: f64,
    zoom_focal: Point<f64, Logical>,
) -> Point<f64, Logical> {
    if zoom_level <= 1.0 {
        return pointer_local;
    }

    let output_rect = Rectangle::from_size(output_size);
    let viewport = apply_zoom_viewport(output_rect, zoom_focal, zoom_level);
    Point::from((
        pointer_local.x.clamp(
            viewport.loc.x,
            viewport.loc.x + viewport.size.w - f64::EPSILON,
        ),
        pointer_local.y.clamp(
            viewport.loc.y,
            viewport.loc.y + viewport.size.h - f64::EPSILON,
        ),
    ))
}

pub struct ZoomTransformInputs {
    pub output_pos: Point<f64, Logical>,
    pub pointer_local: Point<f64, Logical>,
    pub output_sz: Size<f64, Logical>,
    pub zoom_level: f64,
    pub zoom_focal: Point<f64, Logical>,
}

impl ZoomTransformInputs {
    pub fn new(
        output_pos: Point<f64, Logical>,
        pointer_pos: Point<f64, Logical>,
        output_sz: Size<f64, Logical>,
        zoom_level: f64,
        zoom_focal: Point<f64, Logical>,
    ) -> Self {
        let pointer_local = pointer_pos - output_pos;
        Self {
            output_pos,
            pointer_local,
            output_sz,
            zoom_level,
            zoom_focal,
        }
    }

    pub fn display_position(&self) -> Option<Point<f64, Logical>> {
        let output_rect: Rectangle<f64, Logical> = Rectangle::from_size(self.output_sz);
        if !output_rect.contains(self.pointer_local) {
            return None;
        }

        Some(zoom_display_cursor_logical(
            self.pointer_local,
            self.output_sz,
            self.zoom_level,
            self.zoom_focal,
        ))
    }
}

pub(crate) fn compute_on_edge_cursor_anchor(
    cursor_local: Point<f64, Logical>,
    zoom_level: f64,
    focal: Point<f64, Logical>,
    output_size: Size<f64, Logical>,
) -> Point<f64, Logical> {
    let output_rect: Rectangle<f64, Logical> = Rectangle::from_size(output_size);
    let viewport = apply_zoom_viewport(output_rect, focal, zoom_level);

    let anchor_x = if viewport.size.w.abs() < f64::EPSILON {
        0.5
    } else {
        ((cursor_local.x - viewport.loc.x) / viewport.size.w).clamp(0.0, 1.0)
    };
    let anchor_y = if viewport.size.h.abs() < f64::EPSILON {
        0.5
    } else {
        ((cursor_local.y - viewport.loc.y) / viewport.size.h).clamp(0.0, 1.0)
    };

    Point::from((anchor_x, anchor_y))
}

pub fn compute_focal_for_zoom_level(
    cursor_pos: Option<Point<f64, Logical>>,
    output_size: Option<Size<f64, Logical>>,
    movement_mode: Option<&ZoomMovementMode>,
    on_edge_cursor_anchor: Option<Point<f64, Logical>>,
    level: f64,
    fallback: Point<f64, Logical>,
) -> Point<f64, Logical> {
    let (Some(cursor), Some(size), Some(mode)) = (cursor_pos, output_size, movement_mode) else {
        return fallback;
    };

    if matches!(mode, ZoomMovementMode::OnEdge) {
        if let Some(anchor) = on_edge_cursor_anchor {
            return compute_focal_for_on_edge_anchor(cursor, level, size, anchor);
        }
    }

    compute_focal_for_cursor(cursor, level, size, mode)
}

pub(crate) fn compute_focal_for_on_edge_anchor(
    cursor_local: Point<f64, Logical>,
    zoom_level: f64,
    output_size: Size<f64, Logical>,
    cursor_anchor: Point<f64, Logical>,
) -> Point<f64, Logical> {
    if zoom_level <= 1.0 {
        return cursor_local;
    }

    let viewport_size = output_size.downscale(zoom_level);
    let anchor_offset = Point::from((
        viewport_size.w * cursor_anchor.x,
        viewport_size.h * cursor_anchor.y,
    ));
    let viewport_loc: Point<f64, Logical> = cursor_local - anchor_offset;
    let scale_factor = zoom_level / (zoom_level - 1.0).max(0.001);

    let mut focal = viewport_loc.upscale(scale_factor);
    focal.x = focal.x.clamp(0.0, output_size.w - f64::EPSILON);
    focal.y = focal.y.clamp(0.0, output_size.h - f64::EPSILON);
    focal
}

pub fn zoom_subpixel_correction(
    zoom_focal: Point<f64, Logical>,
    zoom_level: f64,
    output_scale: Scale<f64>,
) -> Point<i32, Physical> {
    let focal_i32: Point<i32, Physical> = zoom_focal.to_physical_precise_round(output_scale);
    let focal_f64 = zoom_focal.to_physical(output_scale);
    (focal_i32.to_f64() - focal_f64)
        .upscale(Scale::from(zoom_level - 1.0))
        .to_i32_round::<i32>()
}

pub fn zoom_transform_physical_point(
    point: Point<i32, Physical>,
    zoom_level: f64,
    zoom_focal: Point<f64, Logical>,
    output_scale: Scale<f64>,
) -> Point<i32, Physical> {
    let correction = zoom_subpixel_correction(zoom_focal, zoom_level, output_scale);
    let focal_physical: Point<i32, Physical> = zoom_focal.to_physical_precise_round(output_scale);
    let p = point.to_f64();
    let rounded = p.upscale(Scale::from(zoom_level))
        - focal_physical
            .to_f64()
            .upscale(Scale::from(zoom_level - 1.0));
    rounded.to_i32_round::<i32>() + correction
}
