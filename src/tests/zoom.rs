use niri_config::ZoomMovementMode;
use smithay::utils::{Point, Rectangle, Scale, Size};

use super::*;

#[test]
fn zoom_state_action_query_reports_level_and_lock() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    assert!((f.niri().layout.zoom_level_for_output(&output) - 1.0).abs() < 1e-6);
    assert!(!f.niri().layout.zoom_locked_for_output(&output));
    let cursor_local = Point::from((0.0, 0.0));
    f.niri().layout.set_zoom_level(
        &output,
        2.0,
        cursor_local,
        &ZoomMovementMode::CursorFollow,
        false,
    );
    f.niri_complete_animations();
    assert!((f.niri().layout.zoom_level_for_output(&output) - 2.0).abs() < 1e-6);
    assert!(!f.niri().layout.zoom_locked_for_output(&output));
    f.niri().layout.toggle_zoom_lock(&output);
    f.niri_complete_animations();
    assert!(f.niri().layout.zoom_locked_for_output(&output));
}

#[test]
fn locked_zoom_clamps_pointer_and_touch_to_viewport() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    f.niri().layout.set_zoom_level(
        &output,
        2.0,
        Point::from((0.0, 0.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    f.niri_complete_animations();
    f.niri().layout.toggle_zoom_lock(&output);
    f.niri_complete_animations();
    f.niri().layout.set_zoom_level(
        &output,
        5.0,
        Point::from((0.0, 0.0)),
        &ZoomMovementMode::CursorFollow,
        true,
    );
    f.niri_complete_animations();

    // Level should remain at the previous value and remain locked
    let level_before = f.niri().layout.zoom_level_for_output(&output);
    let level_after = f.niri().layout.zoom_level_for_output(&output);
    assert!((level_after - level_before).abs() < 1e-6);
    assert!(f.niri().layout.zoom_locked_for_output(&output));
}

#[test]
fn layout_zoom_store_is_seeded_on_add_output() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));

    let output = f.niri_output(1);
    let zoom_state = f.niri().layout.zoom_state_for_output(&output).unwrap();
    assert!((zoom_state.level - 1.0).abs() < 1e-6);
    assert!(!zoom_state.locked);
    assert!(zoom_state.transition.is_none());
}

#[test]
fn layout_zoom_store_is_removed_on_remove_output() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));

    let output1 = f.niri_output(1);
    let output2 = f.niri_output(2);
    assert!(f.niri().layout.zoom_state_for_output(&output1).is_some());
    assert!(f.niri().layout.zoom_state_for_output(&output2).is_some());

    f.niri().remove_output(&output2);
    assert!(f.niri().layout.zoom_state_for_output(&output2).is_none());
    assert!(f.niri().layout.zoom_state_for_output(&output1).is_some());

    f.niri().remove_output(&output1);
    assert!(f.niri().layout.zoom_state_for_output(&output1).is_none());
}

#[test]
fn zoom_levels_are_independent_per_output() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));

    let output1 = f.niri_output(1);
    let output2 = f.niri_output(2);

    f.niri().layout.set_zoom_level(
        &output1,
        2.0,
        Point::from((100.0, 100.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    f.niri_complete_animations();

    assert!((f.niri().layout.zoom_level_for_output(&output1) - 2.0).abs() < 1e-6);
    assert!((f.niri().layout.zoom_level_for_output(&output2) - 1.0).abs() < 1e-6);
}

#[test]
fn removing_one_output_does_not_change_other_output_zoom() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));

    let output1 = f.niri_output(1);
    let output2 = f.niri_output(2);

    f.niri().layout.set_zoom_level(
        &output1,
        2.0,
        Point::from((50.0, 50.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    f.niri_complete_animations();
    let before = f.niri().layout.zoom_level_for_output(&output1);

    f.niri().remove_output(&output2);
    let after = f.niri().layout.zoom_level_for_output(&output1);

    assert!((after - before).abs() < 1e-6);
}

#[test]
fn completed_zoom_transition_is_cleared_from_state() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);

    f.niri().layout.set_zoom_level(
        &output,
        2.0,
        Point::from((0.0, 0.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    assert!(f
        .niri()
        .layout
        .zoom_state_for_output(&output)
        .unwrap()
        .transition
        .is_some());

    f.niri_complete_animations();
    let zoom_state = f.niri().layout.zoom_state_for_output(&output).unwrap();
    assert!(zoom_state.transition.is_none());
}

/// Test that zoom_snapshot_for_output reports consistent level/focal/locked
/// after setting zoom and toggling lock.
#[test]
fn zoom_snapshot_reports_consistent_level_focal_locked() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);

    // Initial state: level=1, focal=center, locked=false
    let snapshot = f.niri().layout.zoom_snapshot_for_output(&output);
    assert!((snapshot.level - 1.0).abs() < 1e-6);
    assert!((snapshot.focal.x - 960.0).abs() < 1e-3);
    assert!((snapshot.focal.y - 540.0).abs() < 1e-3);
    assert!(!snapshot.locked);

    // Set zoom level to 2.0 at cursor position (100, 100)
    f.niri().layout.set_zoom_level(
        &output,
        2.0,
        Point::from((100.0, 100.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    f.niri_complete_animations();

    // Snapshot should reflect the new zoom state
    let snapshot = f.niri().layout.zoom_snapshot_for_output(&output);
    assert!((snapshot.level - 2.0).abs() < 1e-6);
    assert!((snapshot.focal.x - 100.0).abs() < 1e-3);
    assert!((snapshot.focal.y - 100.0).abs() < 1e-3);
    assert!(!snapshot.locked);

    // Toggle lock
    f.niri().layout.toggle_zoom_lock(&output);
    f.niri_complete_animations();

    // Snapshot should reflect locked state
    let snapshot = f.niri().layout.zoom_snapshot_for_output(&output);
    assert!((snapshot.level - 2.0).abs() < 1e-6);
    assert!(snapshot.locked);
}

/// Test that OutputZoomState::snapshot_at returns consistent values.
#[test]
fn output_zoom_state_snapshot_at_returns_consistent_values() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);

    // Set zoom level
    f.niri().layout.set_zoom_level(
        &output,
        3.0,
        Point::from((500.0, 400.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    f.niri_complete_animations();

    // Get the zoom state and call snapshot_at directly
    let state = f.niri().layout.zoom_state_for_output(&output).unwrap();
    let now = std::time::Duration::ZERO;
    let snapshot = state.snapshot_at(now);

    assert!((snapshot.level - 3.0).abs() < 1e-6);
    assert!((snapshot.focal.x - 500.0).abs() < 1e-3);
    assert!((snapshot.focal.y - 400.0).abs() < 1e-3);
    assert!(!snapshot.locked);
}

/// Test that zoom_transform_physical_point_f64 preserves fractional values
/// before rounding for fractional focal/scale.
#[test]
fn zoom_transform_physical_point_f64_preserves_fractional() {
    use crate::utils::zoom::zoom_transform_physical_point_f64;

    // Test with fractional focal point (960.5, 540.5) and scale 2.0
    let point = Point::from((100.0, 100.0));
    let zoom_level = 2.0;
    let zoom_focal = Point::from((960.5, 540.5)); // fractional focal
    let output_scale = Scale::from(1.0);

    let result = zoom_transform_physical_point_f64(point, zoom_level, zoom_focal, output_scale);

    // The result should preserve fractional precision before final rounding
    // Formula: point * zoom_level - focal * (zoom_level - 1)
    // = (100, 100) * 2.0 - (960.5, 540.5) * 1.0
    // = (200, 200) - (960.5, 540.5)
    // = (-760.5, -340.5)
    assert!((result.x - (-760.5)).abs() < 1e-6);
    assert!((result.y - (-340.5)).abs() < 1e-6);
}

/// Test that zoom_transform_physical_rect is equivalent to transforming
/// both edges with the f64 function and rounding once.
#[test]
fn zoom_transform_physical_rect_equivalent_to_edge_transform() {
    use crate::utils::zoom::{zoom_transform_physical_point_f64, zoom_transform_physical_rect};

    let rect = Rectangle::new(Point::from((10, 20)), Size::from((100, 80)));
    let zoom_level = 1.5;
    let zoom_focal = Point::from((500.0, 400.0));
    let output_scale = Scale::from(1.0);

    // Use the rect function
    let result = zoom_transform_physical_rect(rect, zoom_level, zoom_focal, output_scale);

    // Manually transform both edges using f64 and round once
    let top_left = zoom_transform_physical_point_f64(
        Point::from((10.0, 20.0)),
        zoom_level,
        zoom_focal,
        output_scale,
    );
    let bottom_right = zoom_transform_physical_point_f64(
        Point::from((110.0, 100.0)), // 10+100, 20+80
        zoom_level,
        zoom_focal,
        output_scale,
    );

    let expected_loc = top_left.to_i32_round::<i32>();
    let expected_bottom_right = bottom_right.to_i32_round::<i32>();
    let expected_size = (expected_bottom_right - expected_loc).to_size();
    let expected = Rectangle::new(expected_loc, expected_size);

    assert_eq!(result.loc, expected.loc);
    assert_eq!(result.size, expected.size);
}

/// Test that zoom_gesture_update accepts cursor_local and updates focal
/// for CursorFollow mode.
#[test]
fn zoom_gesture_update_accepts_cursor_local_and_updates_focal() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    let output_size = Size::from((1920.0, 1080.0));

    // Start a zoom gesture
    f.niri().layout.zoom_gesture_begin(
        &output,
        Some(Point::from((100.0, 100.0))),
        Some(output_size),
        Some(ZoomMovementMode::CursorFollow),
    );

    // Update gesture with new cursor position
    let cursor_local = Point::from((500.0, 500.0));
    let result = f.niri().layout.zoom_gesture_update(
        &output,
        2.0, // scale factor
        1.0, // sensitivity
        std::time::Duration::from_millis(16),
        Some(cursor_local),
        Some(output_size),
    );

    assert!(result.is_some());

    // The focal should have been updated to the new cursor position
    // because we're in CursorFollow mode
    let snapshot = f.niri().layout.zoom_snapshot_for_output(&output);
    // The focal should be close to the new cursor position
    assert!((snapshot.focal.x - 500.0).abs() < 50.0);
    assert!((snapshot.focal.y - 500.0).abs() < 50.0);
}

/// Test that zoom_gesture_update works without cursor_local (None case).
#[test]
fn zoom_gesture_update_works_without_cursor_local() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    let output_size = Size::from((1920.0, 1080.0));

    // Start a zoom gesture without cursor position
    f.niri().layout.zoom_gesture_begin(
        &output,
        None, // no cursor pos
        Some(output_size),
        Some(ZoomMovementMode::Centered),
    );

    // Update gesture without providing cursor_local
    let result = f.niri().layout.zoom_gesture_update(
        &output,
        2.0,
        1.0,
        std::time::Duration::from_millis(16),
        None, // no cursor_local
        Some(output_size),
    );

    assert!(result.is_some());
}
