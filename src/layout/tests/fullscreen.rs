use insta::assert_snapshot;

use super::*;

#[test]
fn fullscreen() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FullscreenWindow(1),
    ];

    check_ops(ops);
}

#[test]
fn unfullscreen_window_in_column() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::SetFullscreenWindow {
            window: 2,
            is_fullscreen: false,
        },
    ];

    check_ops(ops);
}

#[test]
fn unfullscreen_view_offset_not_reset_on_removal() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::FullscreenWindow(0),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::ConsumeOrExpelWindowRight { id: None },
    ];

    check_ops(ops);
}

#[test]
fn unfullscreen_view_offset_not_reset_on_consume() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::FullscreenWindow(0),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::ConsumeWindowIntoColumn,
    ];

    check_ops(ops);
}

#[test]
fn unfullscreen_view_offset_not_reset_on_quick_double_toggle() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::FullscreenWindow(0),
        Op::FullscreenWindow(0),
    ];

    check_ops(ops);
}

#[test]
fn unfullscreen_view_offset_set_on_fullscreening_inactive_tile_in_column() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::FullscreenWindow(0),
    ];

    check_ops(ops);
}

#[test]
fn unfullscreen_view_offset_not_reset_on_gesture() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FullscreenWindow(1),
        Op::ViewOffsetGestureBegin {
            output_idx: 1,
            workspace_idx: None,
            is_touchpad: true,
        },
        Op::ViewOffsetGestureEnd {
            is_touchpad: Some(true),
        },
    ];

    check_ops(ops);
}

#[test]
fn one_window_in_column_becomes_weight_1_after_fullscreen() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        Op::Communicate(2),
        Op::FocusWindowUp,
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        Op::Communicate(1),
        Op::CloseWindow(0),
        Op::FullscreenWindow(1),
    ];

    check_ops(ops);
}

#[test]
fn disable_tabbed_mode_in_fullscreen() {
    let ops = [
        Op::AddOutput(0),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::ToggleColumnTabbedDisplay,
        Op::FullscreenWindow(0),
        Op::ToggleColumnTabbedDisplay,
    ];

    check_ops(ops);
}

#[test]
fn unfullscreen_with_large_border() {
    let ops = [
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::FullscreenWindow(0),
        Op::Communicate(0),
        Op::FullscreenWindow(0),
    ];

    let options = Options {
        layout: niri_config::Layout {
            border: niri_config::Border {
                off: false,
                width: 10000.,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    check_ops_with_options(options, ops);
}

#[test]
fn fullscreen_to_windowed_fullscreen() {
    let ops = [
        Op::AddOutput(0),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::FullscreenWindow(0),
        Op::Communicate(0), // Make sure it goes into fullscreen.
        Op::ToggleWindowedFullscreen(0),
    ];

    check_ops(ops);
}

#[test]
fn windowed_fullscreen_to_fullscreen() {
    let ops = [
        Op::AddOutput(0),
        Op::AddWindow {
            params: TestWindowParams::new(0),
        },
        Op::FullscreenWindow(0),
        Op::Communicate(0),              // Commit fullscreen state.
        Op::ToggleWindowedFullscreen(0), // Switch is_fullscreen() to false.
        Op::FullscreenWindow(0),         // Switch is_fullscreen() back to true.
    ];

    check_ops(ops);
}

#[test]
fn move_pending_unfullscreen_window_out_of_active_column() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FullscreenWindow(1),
        Op::Communicate(1),
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::ConsumeWindowIntoColumn,
        // Window 1 is now pending unfullscreen.
        // Moving it out should reset view_offset_before_fullscreen.
        Op::MoveWindowToWorkspaceDown(true),
    ];

    check_ops(ops);
}

#[test]
fn move_unfocused_pending_unfullscreen_window_out_of_active_column() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::FullscreenWindow(1),
        Op::Communicate(1),
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::ConsumeWindowIntoColumn,
        // Window 1 is now pending unfullscreen.
        // Moving it out should reset view_offset_before_fullscreen.
        Op::FocusWindowDown,
        Op::MoveWindowToWorkspace {
            window_id: Some(1),
            workspace_idx: 1,
        },
    ];

    check_ops(ops);
}

#[test]
fn interactive_resize_on_pending_unfullscreen_column() {
    let ops = [
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FullscreenWindow(2),
        Op::Communicate(2),
        Op::SetFullscreenWindow {
            window: 2,
            is_fullscreen: false,
        },
        Op::InteractiveResizeBegin {
            window: 2,
            edges: ResizeEdge::RIGHT,
        },
        Op::Communicate(2),
    ];

    check_ops(ops);
}

#[test]
fn interactive_move_unfullscreen_to_floating_stops_dnd_scroll() {
    let ops = [
        Op::AddOutput(3),
        Op::AddWindow {
            params: TestWindowParams {
                is_floating: true,
                ..TestWindowParams::new(4)
            },
        },
        // This moves the window to tiling.
        Op::SetFullscreenWindow {
            window: 4,
            is_fullscreen: true,
        },
        // This starts a DnD scroll since we're dragging a tiled window.
        Op::InteractiveMoveBegin {
            window: 4,
            output_idx: 3,
            px: 0.0,
            py: 0.0,
        },
        // This will cause the window to unfullscreen to floating, and should stop the DnD scroll
        // since we're no longer dragging a tiled window, but rather a floating one.
        Op::InteractiveMoveUpdate {
            window: 4,
            dx: 0.0,
            dy: 15035.31210741684,
            output_idx: 3,
            px: 0.0,
            py: 0.0,
        },
        Op::InteractiveMoveEnd { window: 4 },
    ];

    check_ops(ops);
}

#[test]
fn interactive_move_restore_to_floating_animates_view_offset() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        // Toggle window 1 to floating.
        Op::FocusWindow(1),
        Op::ToggleWindowFloating { id: None },
        // Fullscreen window 1 - it moves to scrolling with restore_to_floating = true.
        Op::FullscreenWindow(1),
        Op::Communicate(1),
        Op::CompleteAnimations,
    ];

    let mut layout = check_ops(ops);

    // Verify window 1 is in scrolling and has restore_to_floating = true.
    let scrolling = layout.active_workspace().unwrap().scrolling();
    let tile1 = scrolling.tiles().find(|t| *t.window().id() == 1).unwrap();
    assert!(
        tile1.restore_to_floating,
        "window 1 should have restore_to_floating = true"
    );

    let ops = [
        // Start interactive move on window 1.
        Op::InteractiveMoveBegin {
            window: 1,
            output_idx: 1,
            px: 100.,
            py: 100.,
        },
        // Update with a large delta to trigger the unmaximize.
        Op::InteractiveMoveUpdate {
            window: 1,
            dx: 1000.,
            dy: 1000.,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Window 1 should now be removed from the workspace (in the interactive move state).
    // Window 2 should be the only window in the scrolling space.
    let scrolling = layout.active_workspace().unwrap().scrolling();
    assert_eq!(scrolling.tiles().count(), 1);
    assert!(scrolling.tiles().next().unwrap().window().id() == &2);

    // The view offset should be animating to show window 2.
    assert!(scrolling.view_offset().is_animation_ongoing());
}

#[test]
fn unfullscreen_view_offset_not_reset_during_dnd_gesture() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::FullscreenWindow(3),
        Op::Communicate(3),
        Op::DndUpdate {
            output_idx: 1,
            px: 0.0,
            py: 0.0,
        },
        Op::FullscreenWindow(3),
        Op::Communicate(3),
    ];

    check_ops(ops);
}

#[test]
fn unfullscreen_view_offset_not_reset_during_gesture() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::FullscreenWindow(3),
        Op::Communicate(3),
        Op::ViewOffsetGestureBegin {
            output_idx: 1,
            workspace_idx: None,
            is_touchpad: false,
        },
        Op::FullscreenWindow(3),
        Op::Communicate(3),
    ];

    check_ops(ops);
}

#[test]
fn unfullscreen_view_offset_not_reset_during_ongoing_gesture() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::ViewOffsetGestureBegin {
            output_idx: 1,
            workspace_idx: None,
            is_touchpad: false,
        },
        Op::FullscreenWindow(3),
        Op::Communicate(3),
        Op::FullscreenWindow(3),
        Op::Communicate(3),
    ];

    check_ops(ops);
}

#[test]
fn unfullscreen_preserves_view_pos() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
    ];

    let mut layout = check_ops(ops);

    // View pos is looking at the first window.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"-16");

    let ops = [
        Op::FullscreenWindow(2),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    check_ops_on_layout(&mut layout, ops);

    // View pos = width of first window + gap.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"116");

    let ops = [
        Op::FullscreenWindow(2),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    check_ops_on_layout(&mut layout, ops);

    // View pos is back to showing the first window.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"-16");
}

#[test]
fn unfullscreen_of_tabbed_preserves_view_pos() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::SetColumnDisplay(ColumnDisplay::Tabbed),
        // Get view pos back on the first window.
        Op::FocusColumnLeft,
        Op::FocusColumnRight,
    ];

    let mut layout = check_ops(ops);

    // View pos is looking at the first window.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"-16");

    let ops = [
        Op::FullscreenWindow(2),
        Op::Communicate(2),
        Op::Communicate(3),
        Op::CompleteAnimations,
    ];
    check_ops_on_layout(&mut layout, ops);

    // View pos = width of first window + gap.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"116");

    let ops = [
        Op::FullscreenWindow(3),
        Op::Communicate(3),
        Op::CompleteAnimations,
    ];
    check_ops_on_layout(&mut layout, ops);

    // View pos is still on the second column because the second tile hasn't unfullscreened yet.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"116");

    let ops = [Op::Communicate(2), Op::CompleteAnimations];
    check_ops_on_layout(&mut layout, ops);

    // View pos is back to showing the first window.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"-16");
}

#[test]
fn unfullscreen_of_tabbed_via_change_to_normal_preserves_view_pos() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::AddWindow {
            params: TestWindowParams::new(3),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::SetColumnDisplay(ColumnDisplay::Tabbed),
        // Get view pos back on the first window.
        Op::FocusColumnLeft,
        Op::FocusColumnRight,
    ];

    let mut layout = check_ops(ops);

    // View pos is looking at the first window.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"-16");

    let ops = [
        Op::FullscreenWindow(2),
        Op::Communicate(2),
        Op::Communicate(3),
        Op::CompleteAnimations,
    ];
    check_ops_on_layout(&mut layout, ops);

    // View pos = width of first window + gap.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"116");

    let ops = [
        Op::SetColumnDisplay(ColumnDisplay::Normal),
        Op::Communicate(3),
        Op::CompleteAnimations,
    ];
    check_ops_on_layout(&mut layout, ops);

    // View pos is still on the second column because the second tile hasn't unfullscreened yet.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"116");

    let ops = [Op::Communicate(2), Op::CompleteAnimations];
    check_ops_on_layout(&mut layout, ops);

    // View pos is back to showing the first window.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"-16");
}

#[test]
fn removing_only_fullscreen_tile_updates_view_offset() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::ConsumeOrExpelWindowLeft { id: None },
        Op::SetColumnDisplay(ColumnDisplay::Tabbed),
        Op::CompleteAnimations,
    ];

    let mut layout = check_ops(ops);

    // View pos with gap.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"-16");

    let ops = [
        Op::FullscreenWindow(2),
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    check_ops_on_layout(&mut layout, ops);

    // View pos without gap because we went fullscreen.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"0");

    let ops = [
        Op::FullscreenWindow(2),
        // The active window responds, the other tabbed window doesn't yet.
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    check_ops_on_layout(&mut layout, ops);

    // View pos without gap because other tile is still fullscreen.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"0");

    let ops = [
        // Expel the fullscreen window from the column, changing the column to non-fullscreen.
        Op::ConsumeOrExpelWindowRight { id: Some(1) },
        Op::CompleteAnimations,
    ];
    check_ops_on_layout(&mut layout, ops);

    // View pos should include gap now that the column is no longer fullscreen.
    // FIXME: currently, removing a tile doesn't cause the view offset to update.
    assert_snapshot!(layout.active_workspace().unwrap().scrolling().view_pos(), @"0");
}
