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

/// Given a global cursor position, the per-output origin and size, and the
/// current zoom state (level and focal point), return the output-local logical
/// cursor position where the cursor would be displayed on that output.
///
/// If the cursor is outside the output, return None. Otherwise return the
/// clamped position in output-local logical coordinates, preserving the
/// existing viewport clamping rules and out-of-output semantics.
pub fn display_cursor_local_for_output(
    global_pointer: Point<f64, Logical>,
    output_pos: Point<f64, Logical>,
    output_size: Size<f64, Logical>,
    zoom_level: f64,
    zoom_focal: Point<f64, Logical>,
) -> Option<Point<f64, Logical>> {
    let pointer_local = global_pointer - output_pos;
    let output_rect: Rectangle<f64, Logical> = Rectangle::from_size(output_size);
    if !output_rect.contains(pointer_local) {
        return None;
    }

    Some(zoom_display_cursor_logical(
        pointer_local,
        output_size,
        zoom_level,
        zoom_focal,
    ))
}

/// Canonical per-output display cursor position helper in global coordinates.
pub fn canonical_display_cursor_global_pos(
    global_pointer: Point<f64, Logical>,
    output_pos: Point<f64, Logical>,
    output_size: Size<f64, Logical>,
    zoom_level: f64,
    zoom_focal: Point<f64, Logical>,
) -> Option<Point<f64, Logical>> {
    display_cursor_local_for_output(
        global_pointer,
        output_pos,
        output_size,
        zoom_level,
        zoom_focal,
    )
    .map(|p| p + output_pos)
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

            viewport_loc
                .upscale(scale_factor)
                .constrain(Rectangle::from_size(
                    output_size - Size::from((f64::EPSILON, f64::EPSILON)),
                ))
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

    let scale = zoom_factor / (zoom_factor - 1.0);
    let viewport_size = output_geometry.size.downscale(zoom_factor);
    let output_rect = Rectangle::from_size(output_geometry.size);
    let zoomed_geometry_local = apply_zoom_viewport(output_rect, focal_point, zoom_factor);

    let mut new_focal = focal_point;
    let vp_top_left = zoomed_geometry_local.loc;
    let vp_bottom_right = vp_top_left + zoomed_geometry_local.size;

    let mut needs_update = false;

    if cursor_position.y < vp_top_left.y {
        new_focal.y = cursor_position.y * scale;
        needs_update = true;
    } else if cursor_position.y > vp_bottom_right.y {
        new_focal.y = (cursor_position.y - viewport_size.h) * scale;
        needs_update = true;
    }

    if cursor_position.x < vp_top_left.x {
        new_focal.x = cursor_position.x * scale;
        needs_update = true;
    } else if cursor_position.x > vp_bottom_right.x {
        new_focal.x = (cursor_position.x - viewport_size.w) * scale;
        needs_update = true;
    }

    if !needs_update {
        return None;
    }

    Some(new_focal.constrain(Rectangle::from_size(
        output_geometry.size - Size::from((f64::EPSILON, f64::EPSILON)),
    )))
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
    pointer_local.constrain(Rectangle::new(
        viewport.loc,
        viewport.size - Size::from((f64::EPSILON, f64::EPSILON)),
    ))
}

pub(crate) fn compute_on_edge_cursor_anchor(
    cursor_local: Point<f64, Logical>,
    zoom_level: f64,
    focal: Point<f64, Logical>,
    output_size: Size<f64, Logical>,
) -> Point<f64, Logical> {
    let output_rect: Rectangle<f64, Logical> = Rectangle::from_size(output_size);
    let viewport = apply_zoom_viewport(output_rect, focal, zoom_level);
    // Clamp cursor to viewport first — ensures normalized coords are already in [0,1]
    let constrained = cursor_local.constrain(Rectangle::new(
        viewport.loc,
        viewport.size - Size::from((f64::EPSILON, f64::EPSILON)),
    ));
    let delta = constrained - viewport.loc;
    let anchor_x = if viewport.size.w.abs() < f64::EPSILON {
        0.5
    } else {
        delta.x / viewport.size.w
    };
    let anchor_y = if viewport.size.h.abs() < f64::EPSILON {
        0.5
    } else {
        delta.y / viewport.size.h
    };
    (anchor_x, anchor_y).into()
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

    // The scaled and clamped viewport loc is the new focal point that will keep the cursor anchored
    // at the same relative position within the viewport.
    viewport_loc
        .upscale(scale_factor)
        .constrain(Rectangle::from_size(
            output_size - Size::from((f64::EPSILON, f64::EPSILON)),
        ))
}

pub fn zoom_transform_physical_point(
    point: Point<i32, Physical>,
    zoom_level: f64,
    zoom_focal: Point<f64, Logical>,
    output_scale: Scale<f64>,
) -> Point<i32, Physical> {
    zoom_transform_physical_point_f64(point.to_f64(), zoom_level, zoom_focal, output_scale)
        .to_i32_round::<i32>()
}

pub fn zoom_transform_physical_point_f64(
    point: Point<f64, Physical>,
    zoom_level: f64,
    zoom_focal: Point<f64, Logical>,
    output_scale: Scale<f64>,
) -> Point<f64, Physical> {
    let focal_physical: Point<f64, Physical> = zoom_focal.to_physical(output_scale);
    point.upscale(Scale::from(zoom_level)) - focal_physical.upscale(Scale::from(zoom_level - 1.0))
}

pub fn zoom_transform_physical_rect(
    rect: Rectangle<i32, Physical>,
    zoom_level: f64,
    zoom_focal: Point<f64, Logical>,
    output_scale: Scale<f64>,
) -> Rectangle<i32, Physical> {
    let loc =
        zoom_transform_physical_point_f64(rect.loc.to_f64(), zoom_level, zoom_focal, output_scale);
    let bottom_right = zoom_transform_physical_point_f64(
        (rect.loc + rect.size).to_f64(),
        zoom_level,
        zoom_focal,
        output_scale,
    );

    let loc = loc.to_i32_round::<i32>();
    let bottom_right = bottom_right.to_i32_round::<i32>();
    Rectangle::new(loc, (bottom_right - loc).to_size())
}
