use std::time::Duration;

use niri_config::animations::{Animation, Curve, EasingParams, Kind};
use niri_config::{Config, ZoomMovementMode};
use proptest::prelude::*;
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::utils::{Point, Rectangle, Scale, Size};

use super::*;
use crate::layout::ZoomFocalAnimation;
use crate::utils::zoom::compute_focal_for_cursor;

fn complete_animations(layout: &mut Layout<TestWindow>) {
    let mut clock = layout.clock().clone();
    clock.set_complete_instantly(true);
    layout.advance_animations();
    clock.set_complete_instantly(false);
}

fn make_output(name: &str, w: i32, h: i32) -> Output {
    let output = Output::new(
        name.to_string(),
        PhysicalProperties {
            size: Size::from((w, h)),
            subpixel: Subpixel::Unknown,
            make: String::new(),
            model: String::new(),
            serial_number: String::new(),
        },
    );
    output.change_current_state(
        Some(Mode {
            size: Size::from((w, h)),
            refresh: 60000,
        }),
        None,
        None,
        None,
    );
    output.user_data().insert_if_missing(|| OutputName {
        connector: name.to_string(),
        make: None,
        model: None,
        serial: None,
    });
    output
}

/// Lock preserves focal when level changes; unlock restores cursor tracking.
#[test]
fn locked_zoom_level_change_preserves_focal() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);

    layout.set_zoom_level(
        &output,
        2.0,
        Point::from((0.0, 0.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    complete_animations(&mut layout);
    layout.toggle_zoom_lock(&output);
    complete_animations(&mut layout);
    let focal_before = layout.zoom_focal_for_output(&output);

    layout.set_zoom_level(
        &output,
        5.0,
        Point::from((0.0, 0.0)),
        &ZoomMovementMode::CursorFollow,
        true,
    );
    complete_animations(&mut layout);

    let level_after = layout.zoom_level_for_output(&output);
    let focal_after = layout.zoom_focal_for_output(&output);

    assert!((level_after - 5.0).abs() < 1e-6);
    assert!((focal_after.x - focal_before.x).abs() < 1e-6);
    assert!((focal_after.y - focal_before.y).abs() < 1e-6);
    assert!(layout.zoom_locked_for_output(&output));
}

#[test]
fn layout_zoom_store_is_removed_on_remove_output() {
    let mut layout = Layout::<TestWindow>::default();
    let output1 = make_output("o1", 1920, 1080);
    let output2 = make_output("o2", 1280, 720);
    layout.add_output(output1.clone(), None);
    layout.add_output(output2.clone(), None);

    assert!(layout.zoom_state_for_output(&output1).is_some());
    assert!(layout.zoom_state_for_output(&output2).is_some());

    layout.remove_output(&output2);
    assert!(layout.zoom_state_for_output(&output2).is_none());
    assert!(layout.zoom_state_for_output(&output1).is_some());

    layout.remove_output(&output1);
    assert!(layout.zoom_state_for_output(&output1).is_none());
}

#[test]
fn zoom_levels_are_independent_per_output() {
    let mut layout = Layout::<TestWindow>::default();
    let output1 = make_output("o1", 1920, 1080);
    let output2 = make_output("o2", 1280, 720);
    layout.add_output(output1.clone(), None);
    layout.add_output(output2.clone(), None);

    layout.set_zoom_level(
        &output1,
        2.0,
        Point::from((100.0, 100.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    complete_animations(&mut layout);

    assert!((layout.zoom_level_for_output(&output1) - 2.0).abs() < 1e-6);
    assert!((layout.zoom_level_for_output(&output2) - 1.0).abs() < 1e-6);
}

#[test]
fn removing_one_output_does_not_change_other_output_zoom() {
    let mut layout = Layout::<TestWindow>::default();
    let output1 = make_output("o1", 1920, 1080);
    let output2 = make_output("o2", 1280, 720);
    layout.add_output(output1.clone(), None);
    layout.add_output(output2.clone(), None);

    layout.set_zoom_level(
        &output1,
        2.0,
        Point::from((50.0, 50.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    complete_animations(&mut layout);
    let before = layout.zoom_level_for_output(&output1);

    layout.remove_output(&output2);
    let after = layout.zoom_level_for_output(&output1);

    assert!((after - before).abs() < 1e-6);
}

#[test]
fn centered_zoom_level_change_animates_when_target_is_edge_constrained() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);
    let cursor_local = Point::from((10.0, 10.0));
    let output_size = Size::from((1920.0, 1080.0));

    layout.set_zoom_level(
        &output,
        2.0,
        cursor_local,
        &ZoomMovementMode::Centered,
        false,
    );

    assert!(layout
        .zoom_state_for_output(&output)
        .unwrap()
        .transitioning());

    complete_animations(&mut layout);
    let state = layout.zoom_state_for_output(&output).unwrap();
    let expected_focal =
        compute_focal_for_cursor(cursor_local, 2.0, output_size, &ZoomMovementMode::Centered);

    assert!(!state.transitioning());
    assert!((state.level - 2.0).abs() < 1e-6);
    assert!((state.focal.x - expected_focal.x).abs() < 1e-6);
    assert!((state.focal.y - expected_focal.y).abs() < 1e-6);
}

#[test]
fn zoom_snapshot_reports_consistent_level_focal_locked() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);

    let snapshot = layout.zoom_snapshot_for_output(&output);
    assert!((snapshot.level - 1.0).abs() < 1e-6);
    assert!((snapshot.focal.x - 960.0).abs() < 1e-3);
    assert!((snapshot.focal.y - 540.0).abs() < 1e-3);
    assert!(!snapshot.locked);

    layout.set_zoom_level(
        &output,
        2.0,
        Point::from((100.0, 100.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    complete_animations(&mut layout);

    let snapshot = layout.zoom_snapshot_for_output(&output);
    assert!((snapshot.level - 2.0).abs() < 1e-6);
    assert!((snapshot.focal.x - 100.0).abs() < 1e-3);
    assert!((snapshot.focal.y - 100.0).abs() < 1e-3);
    assert!(!snapshot.locked);

    layout.toggle_zoom_lock(&output);
    complete_animations(&mut layout);

    let snapshot = layout.zoom_snapshot_for_output(&output);
    assert!((snapshot.level - 2.0).abs() < 1e-6);
    assert!(snapshot.locked);
}

#[test]
fn on_edge_set_zoom_level_creates_animating_transition() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);
    let cursor_local = Point::from((500.0, 400.0));

    layout.set_zoom_level(&output, 2.0, cursor_local, &ZoomMovementMode::OnEdge, false);

    complete_animations(&mut layout);
    let snapshot = layout.zoom_snapshot_for_output(&output);
    assert!((snapshot.level - 2.0).abs() < 1e-6);
    assert!(
        (snapshot.focal.x - 500.0).abs() < 1.0,
        "OnEdge focal.x should be near cursor.x=500, got {}",
        snapshot.focal.x,
    );
}

#[test]
fn on_edge_gesture_focal_uses_anchor_when_cursor_within_viewport() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);
    let cursor_local = Point::from((500.0, 400.0));
    let output_size = Size::from((1920.0, 1080.0));

    layout.zoom_gesture_begin(
        &output,
        Some(cursor_local),
        Some(output_size),
        Some(ZoomMovementMode::OnEdge),
    );

    let _ = layout.zoom_gesture_update(
        &output,
        1.0,
        1.0,
        Duration::from_millis(16),
        Some(cursor_local),
        Some(output_size),
    );

    let result = layout.zoom_gesture_update(
        &output,
        2.0,
        1.0,
        Duration::from_millis(32),
        Some(cursor_local),
        Some(output_size),
    );
    assert!(result.is_some(), "gesture update should succeed");

    let snapshot = layout.zoom_snapshot_for_output(&output);
    assert!(
        snapshot.level > 1.0,
        "gesture level should increase above 1.0, got {}",
        snapshot.level,
    );
    assert!(
        (snapshot.focal.x - cursor_local.x).abs() < (960.0 - cursor_local.x).abs(),
        "OnEdge focal should track cursor (focal.x={}, cursor.x={})",
        snapshot.focal.x,
        cursor_local.x,
    );
    assert!(
        (snapshot.focal.y - cursor_local.y).abs() < (540.0 - cursor_local.y).abs(),
        "OnEdge focal should track cursor (focal.y={}, cursor.y={})",
        snapshot.focal.y,
        cursor_local.y,
    );
    assert!(
        snapshot.focal.x >= 0.0 && snapshot.focal.x <= 1920.0,
        "focal.x {} out of bounds",
        snapshot.focal.x
    );
    assert!(
        snapshot.focal.y >= 0.0 && snapshot.focal.y <= 1080.0,
        "focal.y {} out of bounds",
        snapshot.focal.y
    );
}

#[test]
fn on_edge_gesture_tracks_cursor_pos_within_viewport() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);
    let cursor_local = Point::from((500.0, 400.0));
    let output_size = Size::from((1920.0, 1080.0));

    layout.zoom_gesture_begin(
        &output,
        Some(cursor_local),
        Some(output_size),
        Some(ZoomMovementMode::OnEdge),
    );

    let new_cursor = Point::from((700.0, 500.0));
    let result = layout.zoom_gesture_update(
        &output,
        2.0,
        1.0,
        Duration::from_millis(16),
        Some(new_cursor),
        Some(output_size),
    );
    assert!(result.is_some());

    let snapshot = layout.zoom_snapshot_for_output(&output);
    assert!(
        snapshot.focal.x >= 0.0 && snapshot.focal.x <= 1920.0,
        "focal.x {} out of bounds",
        snapshot.focal.x
    );
    assert!(
        snapshot.focal.y >= 0.0 && snapshot.focal.y <= 1080.0,
        "focal.y {} out of bounds",
        snapshot.focal.y
    );
    assert!(
        (snapshot.focal.x - new_cursor.x).abs() < (960.0 - new_cursor.x).abs(),
        "focal.x {} should be closer to cursor.x={} than to center",
        snapshot.focal.x,
        new_cursor.x,
    );
}

#[test]
fn update_zoom_movement_mode_recomputes_on_edge_anchor() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);
    let cursor_local = Point::from((500.0, 400.0));

    layout.set_zoom_level(
        &output,
        2.0,
        cursor_local,
        &ZoomMovementMode::CursorFollow,
        false,
    );
    complete_animations(&mut layout);
    let snapshot = layout.zoom_snapshot_for_output(&output);
    assert!((snapshot.focal.x - 500.0).abs() < 1.0);
    assert!((snapshot.focal.y - 400.0).abs() < 1.0);

    layout.update_zoom_movement_mode(&output, ZoomMovementMode::OnEdge);

    let snapshot = layout.zoom_snapshot_for_output(&output);
    assert!(
        (snapshot.focal.x - 500.0).abs() < 2.0,
        "After mode change to OnEdge, focal.x should be near cursor.x=500, got {}",
        snapshot.focal.x,
    );
}

#[test]
fn update_zoom_movement_mode_noop_when_no_transition() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);

    layout.update_zoom_movement_mode(&output, ZoomMovementMode::OnEdge);

    let snapshot = layout.zoom_snapshot_for_output(&output);
    assert!((snapshot.level - 1.0).abs() < 1e-6);
}

#[test]
fn off_true_zoom_level_change_skips_transition() {
    let mut config = Config::default();
    config.animations.zoom_level_change.0.off = true;
    let mut layout = Layout::<TestWindow>::new(Clock::with_time(Duration::ZERO), &config);
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);

    layout.set_zoom_level(
        &output,
        2.0,
        Point::from((100.0, 100.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );

    // Animation::new() with config.off=true sets from=to, so value_at
    // returns to immediately, and the transition is_done_at returns true,
    // so it gets cleared once advance_animations runs.
    complete_animations(&mut layout);
    let state = layout.zoom_state_for_output(&output).unwrap();
    assert!(
        !state.transitioning(),
        "off=true should not leave a pending transition"
    );
    assert!(
        (state.level - 2.0).abs() < 1e-6,
        "off=true should snap to target level immediately"
    );
}

#[test]
fn zero_duration_zoom_level_change_skips_transition() {
    use niri_config::animations::{Animation as AnimConf, Curve as C, EasingParams, Kind as K};

    let mut config = Config::default();
    config.animations.zoom_level_change.0 = AnimConf {
        off: false,
        kind: K::Easing(EasingParams {
            duration_ms: 0,
            curve: C::Linear,
        }),
    };
    let mut layout = Layout::<TestWindow>::new(Clock::with_time(Duration::ZERO), &config);
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);

    layout.set_zoom_level(
        &output,
        2.0,
        Point::from((100.0, 100.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );

    complete_animations(&mut layout);
    let state = layout.zoom_state_for_output(&output).unwrap();
    assert!(
        !state.transitioning(),
        "zero-duration animation should not leave a pending transition"
    );
    assert!(
        (state.level - 2.0).abs() < 1e-6,
        "zero-duration should snap to target level immediately"
    );
}

#[test]
fn zoom_gesture_on_one_output_does_not_affect_other() {
    let mut layout = Layout::<TestWindow>::default();
    let output1 = make_output("o1", 1920, 1080);
    let output2 = make_output("o2", 1920, 1080);
    layout.add_output(output1.clone(), None);
    layout.add_output(output2.clone(), None);
    let output_size = Size::from((1920.0, 1080.0));
    let cursor_local = Point::from((500.0, 400.0));

    let initial_level2 = layout.zoom_level_for_output(&output2);

    layout.zoom_gesture_begin(
        &output1,
        Some(cursor_local),
        Some(output_size),
        Some(ZoomMovementMode::CursorFollow),
    );

    let _ = layout.zoom_gesture_update(
        &output1,
        2.0,
        1.0,
        Duration::from_millis(16),
        Some(cursor_local),
        Some(output_size),
    );

    let level2_during = layout.zoom_level_for_output(&output2);
    assert!(
        (level2_during - initial_level2).abs() < 1e-6,
        "output 2 level should not change during output 1 gesture"
    );

    layout.zoom_gesture_end(&output1, false);

    let level2_after = layout.zoom_level_for_output(&output2);
    assert!(
        (level2_after - initial_level2).abs() < 1e-6,
        "output 2 level should not change after output 1 gesture ends"
    );
}

#[test]
fn zoom_gesture_cursor_moves_between_outputs() {
    let mut layout = Layout::<TestWindow>::default();
    let output1 = make_output("o1", 1920, 1080);
    let output2 = make_output("o2", 1920, 1080);
    layout.add_output(output1.clone(), None);
    layout.add_output(output2.clone(), None);
    let output_size = Size::from((1920.0, 1080.0));

    let cursor_on_output1 = Point::from((500.0, 400.0));
    let cursor_on_output2 = Point::from((2500.0, 500.0));

    let initial_level2 = layout.zoom_level_for_output(&output2);

    layout.zoom_gesture_begin(
        &output1,
        Some(cursor_on_output1),
        Some(output_size),
        Some(ZoomMovementMode::CursorFollow),
    );

    let _ = layout.zoom_gesture_update(
        &output1,
        2.0,
        1.0,
        Duration::from_millis(16),
        Some(cursor_on_output2),
        Some(output_size),
    );
    let _ = layout.zoom_gesture_update(
        &output1,
        4.0,
        1.0,
        Duration::from_millis(32),
        Some(cursor_on_output2),
        Some(output_size),
    );

    let level1 = layout.zoom_level_for_output(&output1);
    assert!(
        level1 > 1.0,
        "output 1 level should increase during pinch gesture"
    );

    let level2_during = layout.zoom_level_for_output(&output2);
    assert!(
        (level2_during - initial_level2).abs() < 1e-6,
        "output 2 level should not change when cursor moves to output 2 during output 1 gesture"
    );

    layout.zoom_gesture_end(&output1, false);
    complete_animations(&mut layout);

    let level2_after = layout.zoom_level_for_output(&output2);
    assert!(
        (level2_after - initial_level2).abs() < 1e-6,
        "output 2 level should not change after gesture ends"
    );
}

#[test]
fn zoom_transform_physical_point_f64_preserves_fractional() {
    use crate::utils::zoom::zoom_transform_physical_point_f64;

    let point = Point::from((100.0, 100.0));
    let zoom_level = 2.0;
    let zoom_focal = Point::from((960.5, 540.5));
    let output_scale = Scale::from(1.0);

    let result = zoom_transform_physical_point_f64(point, zoom_level, zoom_focal, output_scale);

    // point * zoom_level - focal * (zoom_level - 1)
    // = (100, 100) * 2.0 - (960.5, 540.5) * 1.0 = (-760.5, -340.5)
    assert!((result.x - (-760.5)).abs() < 1e-6);
    assert!((result.y - (-340.5)).abs() < 1e-6);
}

#[test]
fn zoom_transform_physical_rect_equivalent_to_edge_transform() {
    use crate::utils::zoom::{zoom_transform_physical_point_f64, zoom_transform_physical_rect};

    let rect = Rectangle::new(Point::from((10, 20)), Size::from((100, 80)));
    let zoom_level = 1.5;
    let zoom_focal = Point::from((500.0, 400.0));
    let output_scale = Scale::from(1.0);

    let result = zoom_transform_physical_rect(rect, zoom_level, zoom_focal, output_scale);

    let top_left = zoom_transform_physical_point_f64(
        Point::from((10.0, 20.0)),
        zoom_level,
        zoom_focal,
        output_scale,
    );
    let bottom_right = zoom_transform_physical_point_f64(
        Point::from((110.0, 100.0)),
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

#[test]
fn zoom_gesture_update_accepts_cursor_local_and_updates_focal() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);
    let output_size = Size::from((1920.0, 1080.0));

    layout.zoom_gesture_begin(
        &output,
        Some(Point::from((100.0, 100.0))),
        Some(output_size),
        Some(ZoomMovementMode::CursorFollow),
    );

    let cursor_local = Point::from((500.0, 500.0));
    let result = layout.zoom_gesture_update(
        &output,
        2.0,
        1.0,
        Duration::from_millis(16),
        Some(cursor_local),
        Some(output_size),
    );
    assert!(result.is_some());

    let snapshot = layout.zoom_snapshot_for_output(&output);
    assert!(
        (snapshot.focal.x - 500.0).abs() < 1e-6,
        "CursorFollow focal.x {} != 500.0",
        snapshot.focal.x
    );
    assert!(
        (snapshot.focal.y - 500.0).abs() < 1e-6,
        "CursorFollow focal.y {} != 500.0",
        snapshot.focal.y
    );
}

#[test]
fn zoom_gesture_end_maintains_level_with_no_animation() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);
    let output_size = Size::from((1920.0, 1080.0));
    let cursor_local = Point::from((500.0, 400.0));

    layout.zoom_gesture_begin(
        &output,
        Some(cursor_local),
        Some(output_size),
        Some(ZoomMovementMode::CursorFollow),
    );

    assert!(layout
        .zoom_gesture_update(
            &output,
            1.0,
            1.0,
            Duration::from_millis(16),
            Some(cursor_local),
            Some(output_size),
        )
        .is_some());
    assert!(layout
        .zoom_gesture_update(
            &output,
            2.0,
            1.0,
            Duration::from_millis(32),
            Some(cursor_local),
            Some(output_size),
        )
        .is_some());

    assert_eq!(layout.zoom_gesture_end(&output, false), Some(true));
    let state = layout.zoom_state_for_output(&output).unwrap();
    assert!(!state.transitioning());
    assert!((state.level - 2.0).abs() < 1e-6);
    assert!((state.focal.x - 500.0).abs() < 1e-6);
    assert!((state.focal.y - 400.0).abs() < 1e-6);
}

#[test]
fn zoom_gesture_cancel_animates_back_to_start_level() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);
    let output_size = Size::from((1920.0, 1080.0));
    let cursor_local = Point::from((500.0, 400.0));

    layout.zoom_gesture_begin(
        &output,
        Some(cursor_local),
        Some(output_size),
        Some(ZoomMovementMode::CursorFollow),
    );

    assert!(layout
        .zoom_gesture_update(
            &output,
            1.0,
            1.0,
            Duration::from_millis(16),
            Some(cursor_local),
            Some(output_size),
        )
        .is_some());
    assert!(layout
        .zoom_gesture_update(
            &output,
            2.0,
            1.0,
            Duration::from_millis(32),
            Some(cursor_local),
            Some(output_size),
        )
        .is_some());

    assert_eq!(layout.zoom_gesture_end(&output, true), Some(true));
    let state_before = layout.zoom_state_for_output(&output).unwrap();
    assert!(state_before.transitioning());
    assert!((state_before.focal.x - 500.0).abs() < 1e-6);
    assert!((state_before.focal.y - 400.0).abs() < 1e-6);

    complete_animations(&mut layout);
    let state_after = layout.zoom_state_for_output(&output).unwrap();
    assert!((state_after.level - 1.0).abs() < 1e-6);
    assert!(!state_after.transitioning());
    assert!((state_after.focal.x - 500.0).abs() < 1e-6);
    assert!((state_after.focal.y - 400.0).abs() < 1e-6);
}

proptest! {
    /// Invariant: viewport_global output is within valid bounds for various
    /// zoom levels and focal points.
    #[test]
    fn zoom_state_viewport_bounds(
        level in 1.0f64..=5.0f64,
        focal_x in 0.0f64..1920.0f64,
        focal_y in 0.0f64..1080.0f64,
    ) {
        let state = OutputZoomState {
            level,
            focal: Point::from((focal_x, focal_y)),
            locked: false,
            level_transition: ZoomLevelTransition::Idle,
            focal_animation: None,
        };
        let output_geo = Rectangle::new(
            Point::from((0.0f64, 0.0f64)),
            Size::from((1920.0f64, 1080.0f64)),
        );
        let viewport = state.viewport_global(output_geo, Duration::ZERO);

        prop_assert!(viewport.size.w > 0.0, "viewport width must be positive");
        prop_assert!(viewport.size.h > 0.0, "viewport height must be positive");
        prop_assert!(
            viewport.size.w <= output_geo.size.w + 1e-9,
            "viewport width {} exceeds output width {}",
            viewport.size.w,
            viewport.size.h
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
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);
    let output_size = Size::from((1920.0, 1080.0));
    let cursor_local = Point::from((700.0, 400.0));
    let target_level = 2.0;
    let target_focal = compute_focal_for_cursor(
        cursor_local,
        target_level,
        output_size,
        &ZoomMovementMode::Centered,
    );

    complete_animations(&mut layout);

    let clock = layout.clock().clone();
    let focal_init = Point::from((960.0, 540.0));

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
    let focal_anim = ZoomFocalAnimation::new(clock, focal_init, target_focal, focal_config);

    layout.zoom_set_state_for_test(
        &output,
        1.0,
        focal_init,
        ZoomLevelTransition::Animating(level_anim),
        Some(focal_anim),
    );

    complete_animations(&mut layout);

    let snapshot = layout.zoom_snapshot_for_output(&output);
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

#[test]
fn zoom_transition_snapshot_values() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);

    let start_level = 1.0;
    let target_level = 2.0;
    let center = Point::from((960.0, 540.0));

    layout.set_zoom_level(
        &output,
        target_level,
        center,
        &ZoomMovementMode::Centered,
        false,
    );

    let snap = layout.zoom_snapshot_for_output(&output);
    assert!(
        (snap.level - start_level).abs() < 1e-6,
        "before animation: level should be {} (start), got {}",
        start_level,
        snap.level,
    );
    assert!(
        (snap.focal.x - 960.0).abs() < 1.0,
        "before animation: Centered focal.x should be at output center (960), got {}",
        snap.focal.x,
    );

    complete_animations(&mut layout);
    let snap = layout.zoom_snapshot_for_output(&output);
    assert!(
        (snap.level - target_level).abs() < 1e-6,
        "after completion: level should be {} (target), got {}",
        target_level,
        snap.level,
    );
    assert!(
        (snap.focal.x - 960.0).abs() < 1.0,
        "after completion: Centered focal.x should stay at output center (960), got {}",
        snap.focal.x,
    );
}

proptest! {
    /// Invariant: focal computation returns points within output bounds for
    /// all movement modes.
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

#[test]
fn animation_interruption_restarts_to_new_target() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);

    layout.set_zoom_level(
        &output,
        2.0,
        Point::from((100.0, 100.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    layout.set_zoom_level(
        &output,
        3.0,
        Point::from((200.0, 200.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );

    assert!(
        layout
            .zoom_state_for_output(&output)
            .unwrap()
            .transitioning(),
        "interrupted animation should still have a transition"
    );

    complete_animations(&mut layout);
    let state = layout.zoom_state_for_output(&output).unwrap();
    assert!(
        (state.level - 3.0).abs() < 1e-6,
        "final level should be 3.0, got {}",
        state.level
    );
}

#[test]
fn set_zoom_level_during_gesture_clears_it() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);
    let cursor = Point::from((500.0, 400.0));
    let output_size = Size::from((1920.0, 1080.0));

    layout.zoom_gesture_begin(
        &output,
        Some(cursor),
        Some(output_size),
        Some(ZoomMovementMode::CursorFollow),
    );
    let _ = layout.zoom_gesture_update(
        &output,
        1.0,
        1.0,
        Duration::from_millis(16),
        Some(cursor),
        Some(output_size),
    );
    let _ = layout.zoom_gesture_update(
        &output,
        2.0,
        1.0,
        Duration::from_millis(32),
        Some(cursor),
        Some(output_size),
    );

    layout.set_zoom_level(&output, 3.0, cursor, &ZoomMovementMode::CursorFollow, false);

    assert_eq!(
        layout.zoom_gesture_end(&output, false),
        None,
        "set_zoom_level should clear the gesture",
    );

    assert!(
        layout
            .zoom_state_for_output(&output)
            .unwrap()
            .transitioning(),
        "set_zoom_level should create an animation"
    );

    complete_animations(&mut layout);
    assert!(
        (layout.zoom_level_for_output(&output) - 3.0).abs() < 1e-6,
        "final level should be 3.0",
    );
}

#[test]
fn toggle_zoom_lock_during_gesture_does_not_panic() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);
    let cursor = Point::from((500.0, 400.0));
    let output_size = Size::from((1920.0, 1080.0));

    layout.zoom_gesture_begin(
        &output,
        Some(cursor),
        Some(output_size),
        Some(ZoomMovementMode::CursorFollow),
    );

    let _ = layout.zoom_gesture_update(
        &output,
        2.0,
        1.0,
        Duration::from_millis(16),
        Some(cursor),
        Some(output_size),
    );

    layout.toggle_zoom_lock(&output);
    assert!(
        layout.zoom_locked_for_output(&output),
        "lock should be toggled on"
    );

    let result = layout.zoom_gesture_end(&output, false);
    assert!(
        result.is_some(),
        "gesture end after lock toggle should succeed"
    );
}

#[test]
fn zoom_level_clamps_below_minimum() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);

    layout.set_zoom_level(
        &output,
        0.5,
        Point::from((100.0, 100.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    complete_animations(&mut layout);

    let state = layout.zoom_state_for_output(&output).unwrap();
    assert!(
        (state.level - 1.0).abs() < 1e-6,
        "level {} should be clamped to min 1.0",
        state.level
    );
}

#[test]
fn zoom_level_clamps_above_maximum() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);

    layout.set_zoom_level(
        &output,
        30.0,
        Point::from((100.0, 100.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    complete_animations(&mut layout);

    let state = layout.zoom_state_for_output(&output).unwrap();
    assert!(
        (state.level - 10.0).abs() < 1e-6,
        "level {} should be clamped to max 10.0",
        state.level
    );
}

#[test]
fn focal_only_animation_updates_state_focal() {
    let mut layout = Layout::<TestWindow>::default();
    let output = make_output("o1", 1920, 1080);
    layout.add_output(output.clone(), None);

    layout.set_zoom_level(
        &output,
        2.0,
        Point::from((100.0, 100.0)),
        &ZoomMovementMode::CursorFollow,
        false,
    );
    complete_animations(&mut layout);

    let snapshot = layout.zoom_snapshot_for_output(&output);
    assert!((snapshot.level - 2.0).abs() < 1e-6);
    let old_focal = snapshot.focal;

    let clock = layout.clock().clone();
    let focal_init = old_focal;
    let focal_target = Point::from((100.0, 100.0));
    let focal_config = Animation {
        off: false,
        kind: Kind::Easing(EasingParams {
            duration_ms: 250,
            curve: Curve::EaseOutExpo,
        }),
    };
    let focal_anim = ZoomFocalAnimation::new(clock, focal_init, focal_target, focal_config);

    layout.zoom_set_state_for_test(
        &output,
        2.0,
        old_focal,
        ZoomLevelTransition::Idle,
        Some(focal_anim),
    );

    complete_animations(&mut layout);

    let snapshot = layout.zoom_snapshot_for_output(&output);
    assert!((snapshot.level - 2.0).abs() < 1e-6);
    assert!(
        (snapshot.focal.x - focal_target.x).abs() < 1e-6,
        "focal.x {} should be {} after focal-only animation",
        snapshot.focal.x,
        focal_target.x,
    );
    assert!(
        (snapshot.focal.y - focal_target.y).abs() < 1e-6,
        "focal.y {} should be {} after focal-only animation",
        snapshot.focal.y,
        focal_target.y,
    );

    let state = layout.zoom_state_for_output(&output).unwrap();
    assert!(matches!(state.level_transition, ZoomLevelTransition::Idle));
    assert!(state.focal_animation.is_none());
}

// ── Pure utility tests ─────────────────────────────────────────────────

/// Rubber-banding is smooth, not a hard clamp — levels within [min, max]
/// should pass through approximately unchanged.
#[test]
fn rubber_band_identity_within_bounds() {
    let level = clamp_zoom_level_with_rubber_band(2.0, 1.0, 10.0);
    assert!(
        (level - 2.0).abs() < 1e-6,
        "level within bounds should pass through, got {level}"
    );
}

/// Levels far below the minimum get pulled up toward it by the rubber band
/// (smooth transition, not a hard clamp).
#[test]
fn rubber_band_pulls_up_below_min() {
    let level = clamp_zoom_level_with_rubber_band(0.001, 1.0, 10.0);
    assert!(
        level > 0.001,
        "level {level} should be pulled above original 0.001"
    );
    assert!(
        level < 1.0 + 0.1,
        "level {level} should approach but not exceed min 1.0"
    );
}

/// Levels far above the maximum get pulled down toward it by the rubber band
/// (smooth transition, not a hard clamp).
#[test]
fn rubber_band_pulls_down_above_max() {
    let level = clamp_zoom_level_with_rubber_band(100.0, 1.0, 10.0);
    assert!(
        level < 100.0,
        "level {level} should be pulled below original 100.0"
    );
    assert!(level > 9.0, "level {level} should be near max 10.0");
}

/// Level exactly at the minimum boundary passes through unchanged.
#[test]
fn rubber_band_at_min_boundary() {
    let level = clamp_zoom_level_with_rubber_band(1.0, 1.0, 10.0);
    assert!(
        (level - 1.0).abs() < 1e-9,
        "level at min should pass through"
    );
}

/// Level exactly at the maximum boundary passes through unchanged.
#[test]
fn rubber_band_at_max_boundary() {
    let level = clamp_zoom_level_with_rubber_band(10.0, 1.0, 10.0);
    assert!(
        (level - 10.0).abs() < 1e-9,
        "level at max should pass through"
    );
}

/// log_pos = 0 should return the start level unchanged.
#[test]
fn log_pos_zero_returns_start() {
    assert!((log_pos_to_zoom_level(2.5, 0.0) - 2.5).abs() < 1e-9);
}

/// Positive log_pos increases the level exponentially.
#[test]
fn log_pos_positive_increases_level() {
    let level = log_pos_to_zoom_level(1.0, 2.0_f64.ln());
    assert!(
        (level - 2.0).abs() < 1e-9,
        "ln(2) from 1.0 should give 2.0, got {level}"
    );
}

/// Negative log_pos decreases the level exponentially.
#[test]
fn log_pos_negative_decreases_level() {
    let level = log_pos_to_zoom_level(2.0, 0.5_f64.ln());
    assert!(
        (level - 1.0).abs() < 1e-9,
        "ln(0.5) from 2.0 should give 1.0, got {level}"
    );
}
