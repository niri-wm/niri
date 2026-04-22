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
