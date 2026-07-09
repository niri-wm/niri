use std::fmt::Write as _;

use insta::assert_snapshot;
use niri_config::animations::{Curve, EasingParams, Kind};

use super::*;

fn format_tiles(layout: &Layout<TestWindow>) -> String {
    let mut buf = String::new();
    let ws = layout.active_workspace().unwrap();
    let mut tiles: Vec<_> = ws.tiles_with_render_positions().collect();

    // We sort by id since that gives us a consistent order (from first opened to last), but we
    // don't print the id since it's nondeterministic (the id is a global counter across all
    // running tests in the same binary).
    tiles.sort_by_key(|(tile, _, _)| tile.window().id());
    for (tile, pos, _visible) in tiles {
        let Size { w, h, .. } = tile.animated_tile_size();
        let Point { x, y, .. } = pos;
        writeln!(&mut buf, "{w:>3.0} × {h:>3.0} at x:{x:>3.0} y:{y:>3.0}").unwrap();
    }
    buf
}

fn make_options() -> Options {
    const LINEAR: Kind = Kind::Easing(EasingParams {
        duration_ms: 1000,
        curve: Curve::Linear,
    });

    let mut options = Options {
        layout: niri_config::Layout {
            gaps: 0.0,
            ..Default::default()
        },
        ..Options::default()
    };
    options.animations.window_resize.anim.kind = LINEAR;
    options.animations.window_movement.0.kind = LINEAR;

    options
}

fn set_up_two_in_column() -> Layout<TestWindow> {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::ConsumeWindowIntoColumn,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];

    check_ops_with_options(make_options(), ops)
}

#[test]
fn height_resize_animates_next_y() {
    let mut layout = set_up_two_in_column();

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::AdjustFixed(-50),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 50)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed yet, so we're at the initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");

    // Advance the time halfway.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Top window is half-resized at 75 px tall, bottom window is at y=75 matching it.
    assert_snapshot!(format_tiles(&layout), @r"
    100 ×  75 at x:  0 y:  0
    200 × 200 at x:  0 y: 75
    ");

    // Advance the time to completion.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Final state at 50 px.
    assert_snapshot!(format_tiles(&layout), @r"
    100 ×  50 at x:  0 y:  0
    200 × 200 at x:  0 y: 50
    ");
}

#[test]
fn clientside_height_change_doesnt_animate() {
    let mut layout = set_up_two_in_column();

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");

    let ops = [
        // The top window shrinks by itself, without a niri-issued resize.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 50)),
        },
        // This does not start any animations.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed yet, but we are at the final state right away.
    assert_snapshot!(format_tiles(&layout), @r"
    100 ×  50 at x:  0 y:  0
    200 × 200 at x:  0 y: 50
    ");
}

#[test]
fn height_resize_and_back() {
    let mut layout = set_up_two_in_column();

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time halfway.
        Op::AdvanceAnimations { msec_delta: 500 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Top window is half-resized at 150 px tall, bottom window is at y=150 matching it.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 150 at x:  0 y:  0
    200 × 200 at x:  0 y:150
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This starts a new resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed yet, and we expect no animation jumps, so this state matches the last.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 150 at x:  0 y:  0
    200 × 200 at x:  0 y:150
    ");

    // Advance the time halfway.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Halfway through at 125px.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 125 at x:  0 y:  0
    200 × 200 at x:  0 y:125
    ");

    // Advance the time to completion.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Final state back at 100px.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn height_resize_and_cancel() {
    let mut layout = set_up_two_in_column();

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time slightly.
        Op::AdvanceAnimations { msec_delta: 50 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Top window is half-resized at 105 px tall, bottom window is at y=105 matching it.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 105 at x:  0 y:  0
    200 × 200 at x:  0 y:105
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 5 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // Since the resize animation is cancelled, the height goes to the new value immediately. The Y
    // position doesn't jump, instead the animation is offset to preserve the current position.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:105
    ");

    // Advance to the end of the move animation.
    Op::AdvanceAnimations { msec_delta: 950 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn height_resize_and_back_during_another_y_anim() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Consume second window into column, starting the X/Y move anim down.
    Op::ConsumeWindowIntoColumn.apply(&mut layout);

    // No time had passed, so no change in coordinates yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance the time halfway.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Second window halfway to the bottom.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x: 50 y: 50
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed, so no change in state yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x: 50 y: 50
    ");

    // Advance the time a bit.
    Op::AdvanceAnimations { msec_delta: 200 }.apply(&mut layout);

    // X changed by 20, but y changed by 30 since the Y movement from the resize compounds with the
    // Y movement from consume-into-column.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 120 at x:  0 y:  0
    200 × 200 at x: 30 y: 80
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed, so no change in state yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 120 at x:  0 y:  0
    200 × 200 at x: 30 y: 80
    ");

    // Advance the time a bit. Both resize and consume movement are still ongoing.
    Op::AdvanceAnimations { msec_delta: 200 }.apply(&mut layout);

    assert_snapshot!(format_tiles(&layout), @r"
    100 × 116 at x:  0 y:  0
    200 × 200 at x: 10 y: 84
    ");

    // Advance the time to complete the consume movement.
    Op::AdvanceAnimations { msec_delta: 100 }.apply(&mut layout);

    // The Y position is still lower than the height since the window started the resize-induced Y
    // movement high up.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 114 at x:  0 y:  0
    200 × 200 at x:  0 y: 86
    ");

    // Advance the time to complete the resize.
    Op::AdvanceAnimations { msec_delta: 700 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn height_resize_and_cancel_during_another_y_anim() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Consume second window into column, starting the X/Y move anim down.
    Op::ConsumeWindowIntoColumn.apply(&mut layout);

    // No time had passed, so no change in coordinates yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance the time halfway.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Second window halfway to the bottom.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x: 50 y: 50
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time slightly.
        Op::AdvanceAnimations { msec_delta: 50 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // X changed by 5, but y changed by 8 since the Y movement from the resize compounds with the Y
    // movement from consume-into-column.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 105 at x:  0 y:  0
    200 × 200 at x: 45 y: 58
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 5 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // Since the resize anim was cancelled, second window's Y anim is adjusted to preserve the
    // current position while targeting the new final position.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x: 45 y: 58
    ");

    // Advance the time to complete the consume movement.
    Op::AdvanceAnimations { msec_delta: 450 }.apply(&mut layout);

    // Since we don't cancel the resize-induced part of the anim (in fact the move Y anim isn't
    // split into parts, so there's no way to tell), it keeps going still.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y: 78
    ");

    // Advance the time to complete the resize-induced anim.
    Op::AdvanceAnimations { msec_delta: 550 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn height_resize_before_another_y_anim_then_back() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time a bit.
        Op::AdvanceAnimations { msec_delta: 200 },
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The resize is in progress.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 120 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Consume second window into column, starting the X/Y move anim down.
    Op::ConsumeWindowIntoColumn.apply(&mut layout);

    // No time had passed, so no change in coordinates yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 120 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance the time halfway.
    Op::AdvanceAnimations { msec_delta: 600 }.apply(&mut layout);

    // Second window halfway to the bottom. Since consume happened after the start of the first
    // window's resize, the second window's Y is unaffected by it and is animating towards the
    // final position right away.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 180 at x:  0 y:  0
    200 × 200 at x: 40 y:120
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed, so no change in state yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 180 at x:  0 y:  0
    200 × 200 at x: 40 y:120
    ");

    // Advance the time a bit. Both resize and consume movement are still ongoing.
    Op::AdvanceAnimations { msec_delta: 200 }.apply(&mut layout);

    assert_snapshot!(format_tiles(&layout), @r"
    100 × 164 at x:  0 y:  0
    200 × 200 at x: 20 y:116
    ");

    // Advance the time to complete the consume movement.
    Op::AdvanceAnimations { msec_delta: 200 }.apply(&mut layout);

    assert_snapshot!(format_tiles(&layout), @r"
    100 × 148 at x:  0 y:  0
    200 × 200 at x:  0 y:112
    ");

    // Advance the time to complete the resize.
    Op::AdvanceAnimations { msec_delta: 600 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn height_resize_before_another_y_anim_then_cancel() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time a bit.
        Op::AdvanceAnimations { msec_delta: 20 },
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The resize is in progress.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 102 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Consume second window into column, starting the X/Y move anim down.
    Op::ConsumeWindowIntoColumn.apply(&mut layout);

    // No time had passed, so no change in coordinates yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 102 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance the time a little.
    Op::AdvanceAnimations { msec_delta: 20 }.apply(&mut layout);

    // Second window on its way to the bottom.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 104 at x:  0 y:  0
    200 × 200 at x: 98 y:  4
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 4 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // The second window's trajectory readjusts to the new final position at 100 px, without jumps.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x: 98 y:  4
    ");

    // Advance the time to complete the consume movement.
    Op::AdvanceAnimations { msec_delta: 980 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn clientside_height_change_during_another_y_anim() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
        Op::ConsumeWindowIntoColumn,
        // Clear the animate next configure flag.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time a bit.
        Op::AdvanceAnimations { msec_delta: 200 },
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // Second window on its way to the bottom.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x: 80 y: 20
    ");

    let ops = [
        // The top window suddenly grows.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // The second window's trajectory readjusts to the new final position at 200 px, without jumps.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 200 at x:  0 y:  0
    200 × 200 at x: 80 y: 20
    ");

    // Advance the time to complete the consume movement.
    Op::AdvanceAnimations { msec_delta: 800 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 200 at x:  0 y:  0
    200 × 200 at x:  0 y:200
    ");
}

#[test]
fn height_resize_cancel_with_stationary_second_window() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time a bit.
        Op::AdvanceAnimations { msec_delta: 20 },
    ];
    let mut options = make_options();
    // Window movement will happen instantly.
    options.animations.window_movement.0.off = true;
    let mut layout = check_ops_with_options(options, ops);

    // The resize is in progress.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 102 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Consume second window into column, starting the X/Y move anim down.
    Op::ConsumeWindowIntoColumn.apply(&mut layout);

    // No time had passed, so no change in coordinates yet.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 102 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance the time a little.
    Op::AdvanceAnimations { msec_delta: 20 }.apply(&mut layout);

    // The window movement anim is off, so the second window is already at the bottom. Since
    // consume started after the resize, the second window is unaffected by the resize-induced Y
    // movement, and sits at the final position at 200 px.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 104 at x:  0 y:  0
    200 × 200 at x:  0 y:200
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 4 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // This causes the second window to jump down, which is correct because it hadn't been in an
    // animation, and as far as it's concerned, this is the same case as a window just deciding to
    // do a clientside resize on its own, which is not animated.
    //
    // Since the resize is also cancelled, this is the final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn width_resize_and_cancel() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowWidth {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The left window grows in response.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(200, 100)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time slightly.
        Op::AdvanceAnimations { msec_delta: 50 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Left window is half-resized at 105 px wide, right window is at x=105 matching it.
    assert_snapshot!(format_tiles(&layout), @r"
    105 × 100 at x:  0 y:  0
    200 × 200 at x:105 y:  0
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowWidth {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 5 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // Since the resize animation is cancelled, the width goes to the new value immediately. The X
    // position doesn't jump, instead the animation is restarted to preserve the current position.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:105 y:  0
    ");

    // Advance to the end of the move animation.
    Op::AdvanceAnimations { msec_delta: 1000 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");
}

#[test]
fn width_resize_and_cancel_of_column_to_the_left() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowWidth {
            id: Some(1),
            change: SizeChange::SetFixed(200),
        },
        // The left window grows in response.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(200, 100)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time slightly.
        Op::AdvanceAnimations { msec_delta: 50 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Left window is half-resized at 105 px wide, it's at x=-5 matching the right edge position.
    assert_snapshot!(format_tiles(&layout), @r"
    105 × 100 at x: -5 y:  0
    200 × 200 at x:100 y:  0
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowWidth {
            id: Some(1),
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 5 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // Since the resize animation is cancelled, the width goes to the new value immediately. The X
    // position doesn't jump, instead the animation is restarted to preserve the current position.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x: -5 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance to the end of the move animation.
    Op::AdvanceAnimations { msec_delta: 1000 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");
}
