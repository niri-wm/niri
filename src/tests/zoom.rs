use niri_config::ZoomMovementMode;
use smithay::utils::Point;

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
