use niri_config::animations::{Animation, Curve, EasingParams, Kind};
use niri_config::{Config, ZoomMovementMode};
use proptest::prelude::*;
use smithay::utils::{Point, Rectangle, Scale, Size};

use super::*;
use crate::layout::{ZoomFocalAnimation, ZoomLevelAnimation, ZoomTransition};
use crate::utils::zoom::compute_focal_for_cursor;

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

/// Locked zoom accepts level changes but preserves the focal point.
///
/// Locking zoom blocks cursor-tracking focal recomputation, not level
/// changes.  When locked, `set_zoom_level` changes the magnification but
/// keeps the viewport center where it is.  Unlocking zoom later restores
/// cursor tracking (e.g. `animate_zoom_unlock` recomputes the focal from
/// the cursor position).
///
/// This distinction matters: a user who locks zoom to browse a specific
/// part of the screen can still adjust the magnification level without
/// the viewport jumping to follow the cursor.
#[test]
fn locked_zoom_level_change_preserves_focal() {
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
    let focal_before = f.niri().layout.zoom_focal_for_output(&output);

    f.niri().layout.set_zoom_level(
        &output,
        5.0,
        Point::from((0.0, 0.0)),
        &ZoomMovementMode::CursorFollow,
        true,
    );
    f.niri_complete_animations();

    let level_after = f.niri().layout.zoom_level_for_output(&output);
    let focal_after = f.niri().layout.zoom_focal_for_output(&output);

    assert!((level_after - 5.0).abs() < 1e-6);
    assert!((focal_after.x - focal_before.x).abs() < 1e-6);
    assert!((focal_after.y - focal_before.y).abs() < 1e-6);
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

#[test]
fn centered_zoom_level_change_animates_when_target_is_edge_constrained() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    let cursor_local = Point::from((10.0, 10.0));
    let output_size = Size::from((1920.0, 1080.0));

    f.niri().layout.set_zoom_level(
        &output,
        2.0,
        cursor_local,
        &ZoomMovementMode::Centered,
        false,
    );

    // Dynamic focal tracking now handles Centered mode smoothly,
    // so the transition animates rather than snapping.
    assert!(f
        .niri()
        .layout
        .zoom_state_for_output(&output)
        .unwrap()
        .transition
        .is_some());

    f.niri_complete_animations();
    let zoom_state = f.niri().layout.zoom_state_for_output(&output).unwrap();
    let expected_focal =
        compute_focal_for_cursor(cursor_local, 2.0, output_size, &ZoomMovementMode::Centered);

    assert!(zoom_state.transition.is_none());
    assert!((zoom_state.level - 2.0).abs() < 1e-6);
    assert!((zoom_state.focal.x - expected_focal.x).abs() < 1e-6);
    assert!((zoom_state.focal.y - expected_focal.y).abs() < 1e-6);
}

#[test]
fn centered_zoom_level_change_still_animates_when_focal_is_unchanged() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);

    f.niri().layout.set_zoom_level(
        &output,
        2.0,
        Point::from((960.0, 540.0)),
        &ZoomMovementMode::Centered,
        false,
    );

    assert!(f
        .niri()
        .layout
        .zoom_state_for_output(&output)
        .unwrap()
        .transition
        .is_some());
}

#[test]
fn centered_zoom_level_change_animates_when_focal_must_move() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    let output_size = Size::from((1920.0, 1080.0));
    let cursor_local = Point::from((700.0, 400.0));

    f.niri().layout.set_zoom_level(
        &output,
        2.0,
        cursor_local,
        &ZoomMovementMode::Centered,
        false,
    );

    // Dynamic focal tracking smoothly interpolates the focal during level animation.
    assert!(f
        .niri()
        .layout
        .zoom_state_for_output(&output)
        .unwrap()
        .transition
        .is_some());

    f.niri_complete_animations();
    let snapshot = f.niri().layout.zoom_snapshot_for_output(&output);
    let expected_focal =
        compute_focal_for_cursor(cursor_local, 2.0, output_size, &ZoomMovementMode::Centered);

    assert!((snapshot.level - 2.0).abs() < 1e-6);
    assert!((snapshot.focal.x - expected_focal.x).abs() < 1e-6);
    assert!((snapshot.focal.y - expected_focal.y).abs() < 1e-6);
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

// ── OnEdge, animation-disable, and multi-output gesture tests ────────────

/// OnEdge `set_zoom_level` creates an animation with dynamic focal tracking.
///
/// When `set_zoom_level` is called with OnEdge mode, the level animation's
/// tracking context should compute an on-edge cursor anchor from the current
/// focal/level, so the focal during animation is computed relative to that
/// anchor.
#[test]
fn on_edge_set_zoom_level_creates_animating_transition() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    let cursor_local = Point::from((500.0, 400.0));

    f.niri()
        .layout
        .set_zoom_level(&output, 2.0, cursor_local, &ZoomMovementMode::OnEdge, false);

    // Check transition exists and is Animating with dynamic focal tracking.
    {
        let state = f.niri().layout.zoom_state_for_output(&output).unwrap();
        assert!(
            state.transition.is_some(),
            "OnEdge level change should create a transition"
        );
        assert!(
            matches!(
                state.transition.as_ref().unwrap(),
                ZoomTransition::Animating { .. }
            ),
            "transition should be Animating for set_zoom_level"
        );
        if let ZoomTransition::Animating { focal, .. } = state.transition.as_ref().unwrap() {
            assert!(
                focal.is_none(),
                "OnEdge should use dynamic focal tracking, not a separate focal animation"
            );
        }
    }

    f.niri_complete_animations();
    let snapshot = f.niri().layout.zoom_snapshot_for_output(&output);
    assert!((snapshot.level - 2.0).abs() < 1e-6);

    // With OnEdge, the anchor preserves the cursor's relative position in
    // the viewport, so the focal should be near the cursor position.
    assert!(
        (snapshot.focal.x - 500.0).abs() < 1.0,
        "OnEdge focal.x should be near cursor.x=500, got {}",
        snapshot.focal.x,
    );
}

/// OnEdge gesture computes an anchor at gesture start and preserves it.
///
/// When the cursor stays within the viewport during an OnEdge gesture, the
/// focal should be computed relative to the cursor anchor that was captured
/// at gesture begin time.
#[test]
fn on_edge_gesture_focal_uses_anchor_when_cursor_within_viewport() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    let cursor_local = Point::from((500.0, 400.0));
    let output_size = Size::from((1920.0, 1080.0));

    // Begin gesture with OnEdge mode and a cursor position.
    f.niri().layout.zoom_gesture_begin(
        &output,
        Some(cursor_local),
        Some(output_size),
        Some(ZoomMovementMode::OnEdge),
    );

    // First update initializes the internal log-scale tracker.
    let _ = f.niri().layout.zoom_gesture_update(
        &output,
        1.0, // initial scale (no actual change)
        1.0, // sensitivity
        std::time::Duration::from_millis(16),
        Some(cursor_local),
        Some(output_size),
    );

    // Second update applies a zoom to the gesture level.
    let result = f.niri().layout.zoom_gesture_update(
        &output,
        2.0, // scale factor
        1.0, // sensitivity
        std::time::Duration::from_millis(32),
        Some(cursor_local),
        Some(output_size),
    );
    assert!(result.is_some(), "gesture update should succeed");

    // The focal should be near the cursor, computed via OnEdge anchor logic.
    let snapshot = f.niri().layout.zoom_snapshot_for_output(&output);
    assert!(
        snapshot.level > 1.0,
        "gesture level should increase above 1.0, got {}",
        snapshot.level,
    );
    // Focal should be within a reasonable range of the cursor (viewport-sized area).
    assert!(
        (snapshot.focal.x - 500.0).abs() < 500.0,
        "focal.x should be near cursor x"
    );
    assert!(
        (snapshot.focal.y - 400.0).abs() < 500.0,
        "focal.y should be near cursor y"
    );
}

/// OnEdge gesture focal shifts when the cursor moves outside the viewport.
///
/// Cursor outside the viewport while in OnEdge mode does not trigger edge-shift
/// at the gesture update level (edge-shift is handled by input's
/// `update_zoom_base_focal`).  This test verifies the gesture correctly tracks
/// the cursor within the viewport area.
#[test]
fn on_edge_gesture_tracks_cursor_pos_within_viewport() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    let cursor_local = Point::from((500.0, 400.0));
    let output_size = Size::from((1920.0, 1080.0));

    f.niri().layout.zoom_gesture_begin(
        &output,
        Some(cursor_local),
        Some(output_size),
        Some(ZoomMovementMode::OnEdge),
    );

    // Move cursor to a different position and update gesture.
    let new_cursor = Point::from((700.0, 500.0));
    let result = f.niri().layout.zoom_gesture_update(
        &output,
        2.0,
        1.0,
        std::time::Duration::from_millis(16),
        Some(new_cursor),
        Some(output_size),
    );
    assert!(result.is_some());

    // The focal should reflect the new cursor position.
    let snapshot = f.niri().layout.zoom_snapshot_for_output(&output);
    assert!(
        (snapshot.focal.x - 700.0).abs() < 500.0,
        "focal.x should track cursor x"
    );
    assert!(
        (snapshot.focal.y - 500.0).abs() < 500.0,
        "focal.y should track cursor y"
    );
}

/// `off=true` on `zoom_level_change` skips the transition entirely.
///
/// When `zoom_level_change.off` is set to true in the config, the zoom level
/// must snap immediately without creating a pending transition or animating.
#[test]
fn off_true_zoom_level_change_skips_transition() {
    let mut config = Config::default();
    config.animations.zoom_level_change.0.off = true;
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);

    f.niri().layout.set_zoom_level(
        &output,
        2.0,
        Point::from((100.0, 100.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );

    // Animation::new() with config.off=true sets from=to, so value_at
    // returns to immediately, and the transition is_done_at returns true,
    // so it gets cleared once advance_animations runs.
    f.niri_complete_animations();
    let state = f.niri().layout.zoom_state_for_output(&output).unwrap();
    assert!(
        state.transition.is_none(),
        "off=true should not leave a pending transition"
    );
    assert!(
        (state.level - 2.0).abs() < 1e-6,
        "off=true should snap to target level immediately"
    );
}

/// `duration-ms: 0` on `zoom_level_change` skips the transition.
///
/// Zero-duration easing animations should complete instantly, just like
/// `off=true`.
#[test]
fn zero_duration_zoom_level_change_skips_transition() {
    use niri_config::animations::{Animation as AnimConf, Curve, EasingParams, Kind};

    let mut config = Config::default();
    config.animations.zoom_level_change.0 = AnimConf {
        off: false,
        kind: Kind::Easing(EasingParams {
            duration_ms: 0,
            curve: Curve::Linear,
        }),
    };
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);

    f.niri().layout.set_zoom_level(
        &output,
        2.0,
        Point::from((100.0, 100.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );

    f.niri_complete_animations();
    let state = f.niri().layout.zoom_state_for_output(&output).unwrap();
    assert!(
        state.transition.is_none(),
        "zero-duration animation should not leave a pending transition"
    );
    assert!(
        (state.level - 2.0).abs() < 1e-6,
        "zero-duration should snap to target level immediately"
    );
}

/// Zoom gesture on one output does not affect another output.
///
/// When a gesture is active on output 1, output 2's zoom state must remain
/// independent.  Beginning, updating, and ending a gesture on output 1 should
/// not change output 2's level or focal.
#[test]
fn zoom_gesture_on_one_output_does_not_affect_other() {
    let mut f = Fixture::new();
    // Two outputs side by side (both 1920x1080).
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    let output1 = f.niri_output(1);
    let output2 = f.niri_output(2);
    let output_size = Size::from((1920.0, 1080.0));
    let cursor_local = Point::from((500.0, 400.0));

    // Capture output 2's initial zoom state.
    let initial_level2 = f.niri().layout.zoom_level_for_output(&output2);

    // Begin and update gesture on output 1.
    f.niri().layout.zoom_gesture_begin(
        &output1,
        Some(cursor_local),
        Some(output_size),
        Some(ZoomMovementMode::CursorFollow),
    );

    let _ = f.niri().layout.zoom_gesture_update(
        &output1,
        2.0,
        1.0,
        std::time::Duration::from_millis(16),
        Some(cursor_local),
        Some(output_size),
    );

    // Output 2's level should remain unchanged.
    let level2_during = f.niri().layout.zoom_level_for_output(&output2);
    assert!(
        (level2_during - initial_level2).abs() < 1e-6,
        "output 2 level should not change during output 1 gesture"
    );

    f.niri().layout.zoom_gesture_end(&output1, false);

    // Output 2's level should still be unchanged.
    let level2_after = f.niri().layout.zoom_level_for_output(&output2);
    assert!(
        (level2_after - initial_level2).abs() < 1e-6,
        "output 2 level should not change after output 1 gesture ends"
    );
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

#[test]
fn zoom_gesture_end_maintains_level_with_no_animation() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    let output_size = Size::from((1920.0, 1080.0));
    let cursor_local = Point::from((500.0, 400.0));

    f.niri().layout.zoom_gesture_begin(
        &output,
        Some(cursor_local),
        Some(output_size),
        Some(ZoomMovementMode::CursorFollow),
    );

    assert!(f
        .niri()
        .layout
        .zoom_gesture_update(
            &output,
            1.0,
            1.0,
            std::time::Duration::from_millis(16),
            Some(cursor_local),
            Some(output_size),
        )
        .is_some());
    assert!(f
        .niri()
        .layout
        .zoom_gesture_update(
            &output,
            2.0,
            1.0,
            std::time::Duration::from_millis(32),
            Some(cursor_local),
            Some(output_size),
        )
        .is_some());

    // Non-cancelled gesture end — when rubber-banding/clamping don't
    // diverge the target, target equals current level, so no animation
    // transition is created.  The level and focal must already be at
    // their final values without waiting for any animation.
    assert_eq!(f.niri().layout.zoom_gesture_end(&output, false), Some(true));
    let state = f.niri().layout.zoom_state_for_output(&output).unwrap();
    assert!(state.transition.is_none());
    assert!((state.level - 2.0).abs() < 1e-6);
    assert!((state.focal.x - 500.0).abs() < 50.0);
    assert!((state.focal.y - 400.0).abs() < 50.0);
}

#[test]
fn zoom_gesture_cancel_animates_back_to_start_level() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    let output_size = Size::from((1920.0, 1080.0));
    let cursor_local = Point::from((500.0, 400.0));

    f.niri().layout.zoom_gesture_begin(
        &output,
        Some(cursor_local),
        Some(output_size),
        Some(ZoomMovementMode::CursorFollow),
    );

    assert!(f
        .niri()
        .layout
        .zoom_gesture_update(
            &output,
            1.0,
            1.0,
            std::time::Duration::from_millis(16),
            Some(cursor_local),
            Some(output_size),
        )
        .is_some());
    assert!(f
        .niri()
        .layout
        .zoom_gesture_update(
            &output,
            2.0,
            1.0,
            std::time::Duration::from_millis(32),
            Some(cursor_local),
            Some(output_size),
        )
        .is_some());

    // Cancelled end — level should animate back to start level.
    assert_eq!(f.niri().layout.zoom_gesture_end(&output, true), Some(true));
    let state_before = f.niri().layout.zoom_state_for_output(&output).unwrap();
    assert!(state_before.transition.is_some());
    // Focal should still be near the cursor at cancel time.
    assert!((state_before.focal.x - 500.0).abs() < 50.0);
    assert!((state_before.focal.y - 400.0).abs() < 50.0);

    f.niri_complete_animations();
    let state_after = f.niri().layout.zoom_state_for_output(&output).unwrap();
    assert!((state_after.level - 1.0).abs() < 1e-6);
    assert!(state_after.transition.is_none());
    // In CursorFollow mode cancel always clears focal (gesture start always
    // had no prior focal animation), so the focal snaps to cursor position.
    assert!((state_after.focal.x - 500.0).abs() < 50.0);
    assert!((state_after.focal.y - 400.0).abs() < 50.0);
}

// ── Proptest invariants ─────────────────────────────────────────────────

/// Invariant: zoom level is always ≥ 1.0.
proptest! {
    #[test]
    fn zoom_state_level_invariant(
        level in 1.0f64..=10.0f64,
        focal_x in 0.0f64..1920.0f64,
        focal_y in 0.0f64..1080.0f64,
        locked in proptest::bool::ANY,
    ) {
        let state = crate::layout::OutputZoomState {
            level,
            focal: Point::from((focal_x, focal_y)),
            locked,
            transition: None,
        };
        prop_assert!(state.level >= 1.0, "level must be >= 1.0");
        prop_assert!(
            state.focal.x >= 0.0 && state.focal.x <= 1920.0,
            "focal.x {} out of [0, 1920]", state.focal.x
        );
        prop_assert!(
            state.focal.y >= 0.0 && state.focal.y <= 1080.0,
            "focal.y {} out of [0, 1080]", state.focal.y
        );
        prop_assert_eq!(state.locked, locked);
        prop_assert!(!state.transitioning(), "Idle state must not transition");
    }
}

/// Invariant: snapshot_at is consistent with direct field access for idle states.
proptest! {
    #[test]
    fn zoom_state_snapshot_consistency(
        level in 1.0f64..=10.0f64,
        focal_x in 0.0f64..1920.0f64,
        focal_y in 0.0f64..1080.0f64,
        locked in proptest::bool::ANY,
    ) {
        let state = crate::layout::OutputZoomState {
            level,
            focal: Point::from((focal_x, focal_y)),
            locked,
            transition: None,
        };
        let now = std::time::Duration::ZERO;
        let snapshot = state.snapshot_at(now);

        prop_assert!((snapshot.level - level).abs() < 1e-9, "snapshot level mismatch");
        prop_assert!(
            (snapshot.focal.x - focal_x).abs() < 1e-9,
            "snapshot focal.x mismatch"
        );
        prop_assert!(
            (snapshot.focal.y - focal_y).abs() < 1e-9,
            "snapshot focal.y mismatch"
        );
        prop_assert_eq!(snapshot.locked, locked);
    }
}

/// Invariant: viewport_global output is within valid bounds for various
/// zoom levels and focal points.
proptest! {
    #[test]
    fn zoom_state_viewport_bounds(
        level in 1.0f64..=5.0f64,
        focal_x in 0.0f64..1920.0f64,
        focal_y in 0.0f64..1080.0f64,
    ) {
        let state = crate::layout::OutputZoomState {
            level,
            focal: Point::from((focal_x, focal_y)),
            locked: false,
            transition: None,
        };
        let output_geo = Rectangle::new(
            Point::from((0.0f64, 0.0f64)),
            Size::from((1920.0f64, 1080.0f64)),
        );
        let viewport = state.viewport_global(output_geo);

        // Viewport must be non-empty.
        prop_assert!(viewport.size.w > 0.0, "viewport width must be positive");
        prop_assert!(viewport.size.h > 0.0, "viewport height must be positive");

        // Viewport size must not exceed the output size (zoom ≤ 1 means
        // viewport == output; zoom > 1 means viewport < output).
        prop_assert!(
            viewport.size.w <= output_geo.size.w + 1e-9,
            "viewport width {} exceeds output width {}",
            viewport.size.w,
            output_geo.size.w
        );
        prop_assert!(
            viewport.size.h <= output_geo.size.h + 1e-9,
            "viewport height {} exceeds output height {}",
            viewport.size.h,
            output_geo.size.h
        );
    }
}

#[test]
fn composed_level_focal_animation_completes_to_targets() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    let output_size = Size::from((1920.0, 1080.0));
    let cursor_local = Point::from((700.0, 400.0));
    let target_level = 2.0;
    let target_focal = compute_focal_for_cursor(
        cursor_local,
        target_level,
        output_size,
        &ZoomMovementMode::Centered,
    );

    // Complete initial setup animations.
    f.niri_complete_animations();

    // Manually construct a composed Animating { level, focal: Some(focal) } transition,
    // exercising the same variant that the defensive compose path in set_zoom_level
    // produces.
    let clock = f.niri().clock.clone();
    {
        let state = f.niri().layout.zoom_state_mut(&output).unwrap();
        state.focal = Point::from((960.0, 540.0));

        let level_config = Animation {
            off: false,
            kind: Kind::Easing(EasingParams {
                duration_ms: 250,
                curve: Curve::EaseOutExpo,
            }),
        };
        let focal_config = Animation {
            off: false,
            kind: Kind::Easing(EasingParams {
                duration_ms: 250,
                curve: Curve::CubicBezier(0.05, 0.7, 0.1, 1.0),
            }),
        };

        let level_anim = ZoomLevelAnimation::new(clock.clone(), 1.0, target_level, level_config);

        let focal_anim = ZoomFocalAnimation::new(clock, state.focal, target_focal, focal_config);

        state.transition = Some(ZoomTransition::Animating {
            level: level_anim,
            focal: Some(focal_anim),
        });
    }

    // Complete all animations.
    f.niri_complete_animations();

    let snapshot = f.niri().layout.zoom_snapshot_for_output(&output);
    assert!(
        (snapshot.level - target_level).abs() < 1e-6,
        "level {} != {}",
        snapshot.level,
        target_level,
    );
    assert!(
        (snapshot.focal.x - target_focal.x).abs() < 1e-6,
        "focal.x {} != {}",
        snapshot.focal.x,
        target_focal.x,
    );
    assert!(
        (snapshot.focal.y - target_focal.y).abs() < 1e-6,
        "focal.y {} != {}",
        snapshot.focal.y,
        target_focal.y,
    );
}

/// Invariant: focal computation returns points within output bounds for
/// all movement modes.
proptest! {
    #[test]
    fn compute_focal_bounds(
        cursor_x in 0.0f64..1920.0f64,
        cursor_y in 0.0f64..1080.0f64,
        level in 1.0f64..=10.0f64,
        mode in prop_oneof![
            Just(ZoomMovementMode::CursorFollow),
            Just(ZoomMovementMode::Centered),
            Just(ZoomMovementMode::OnEdge),
        ],
    ) {
        let output_size = Size::from((1920.0f64, 1080.0f64));
        let cursor = Point::from((cursor_x, cursor_y));
        let focal = compute_focal_for_cursor(cursor, level, output_size, &mode);

        prop_assert!(
            focal.x >= 0.0 && focal.x <= 1920.0,
            "focal.x {} out of [0, 1920] for mode {:?}", focal.x, mode
        );
        prop_assert!(
            focal.y >= 0.0 && focal.y <= 1080.0,
            "focal.y {} out of [0, 1080] for mode {:?}", focal.y, mode
        );
    }
}
