use client::ClientId;
use insta::assert_snapshot;
use niri_config::Config;
use niri_ipc::SizeChange;
use smithay::utils::Point;
use wayland_client::protocol::wl_surface::WlSurface;

use super::*;

// Sets up a fixture with two outputs and 100×100 window.
fn set_up() -> (Fixture, ClientId, WlSurface) {
    set_up_with_config(Config::default())
}

fn set_up_with_config(config: Config) -> (Fixture, ClientId, WlSurface) {
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(100, 100);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    (f, id, surface)
}

#[test]
fn unfocus_preserves_current_size() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.roundtrip(id);

    // Change window size while it's floating.
    let window = f.client(id).window(&surface);
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Focus a different output which should drop the Activated state.
    f.niri_focus_output(2);

    f.double_roundtrip(id);

    // This should request 200 × 200 because that's the current window size.
    let window = f.client(id).window(&surface);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 200 × 200, bounds: 1920 × 1080, states: []"
    );

    // Change window size again.
    let window = f.client(id).window(&surface);
    window.set_size(300, 300);
    window.ack_last_and_commit();
    f.roundtrip(id);

    // Focus the first output which should add back the Activated state.
    f.niri_focus_output(1);

    f.double_roundtrip(id);

    // This should request 300 × 300 because that's the current window size.
    let window = f.client(id).window(&surface);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 300 × 300, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn resize_to_different_size() {
    let (mut f, id, surface) = set_up();
    let _ = f.client(id).window(&surface).recent_configures();

    // Commit in response to the Activated state change configure.
    f.client(id).window(&surface).ack_last_and_commit();
    f.double_roundtrip(id);

    f.niri().layout.toggle_window_floating(None);
    f.niri().layout.set_column_width(SizeChange::SetFixed(500));
    f.double_roundtrip(id);

    // This should request the new size, 500 × 100.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 100, bounds: 1920 × 1080, states: [Activated]"
    );

    // Focus a different output which should drop the Activated state.
    f.niri_focus_output(2);
    f.double_roundtrip(id);
    // This should request the new size since the window hasn't committed yet.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 100, bounds: 1920 × 1080, states: []"
    );

    // Ack but don't commit yet.
    let window = f.client(id).window(&surface);
    window.ack_last();
    f.roundtrip(id);
    // Add the activated state.
    f.niri_focus_output(1);
    f.double_roundtrip(id);
    // This should request the new size since the window hasn't committed yet.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 100, bounds: 1920 × 1080, states: [Activated]"
    );

    // Commit but with some different size.
    let window = f.client(id).window(&surface);
    window.set_size(200, 200);
    window.commit();
    f.double_roundtrip(id);
    // This shouldn't request anything.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @""
    );

    // Drop the Activated state.
    f.niri_focus_output(2);
    f.double_roundtrip(id);
    // This should request the current window size rather than keep requesting 500 × 100.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 200 × 200, bounds: 1920 × 1080, states: []"
    );
}

#[test]
fn set_window_width_uses_current_height() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);
    let _ = f.client(id).window(&surface).recent_configures();

    // Resize to something different on both axes.
    let window = f.client(id).window(&surface);
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.roundtrip(id);

    // Request a width change.
    f.niri().layout.set_column_width(SizeChange::SetFixed(500));

    f.double_roundtrip(id);

    // This should use the current window height (200), rather than the initial window height (100).
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 200, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn set_window_height_uses_current_width() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);
    let _ = f.client(id).window(&surface).recent_configures();

    // Resize to something different on both axes.
    let window = f.client(id).window(&surface);
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.roundtrip(id);

    // Request a width change.
    f.niri()
        .layout
        .set_window_height(None, SizeChange::SetFixed(500));

    f.double_roundtrip(id);

    // This should use the current window width (200), rather than the initial window width (100).
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 200 × 500, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn resize_to_same_size() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);
    let _ = f.client(id).window(&surface).recent_configures();

    // Resize to something different.
    let window = f.client(id).window(&surface);
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.roundtrip(id);

    // Request a size change to the same size.
    f.niri().layout.set_column_width(SizeChange::SetFixed(200));

    f.double_roundtrip(id);

    // This needn't request anything because we're already that size; the size in the current
    // server state matches the requested size.
    //
    // FIXME: However, currently it will request the size anyway because the code checks the
    // current server state, and the last size niri requested of the window was 100×100 (even if
    // the window already acked and committed in response).
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 200 × 200, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn resize_to_different_then_same() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);
    let _ = f.client(id).window(&surface).recent_configures();

    // Commit in response to any configure from the floating change.
    let window = f.client(id).window(&surface);
    window.ack_last_and_commit();
    f.roundtrip(id);

    // Request a size change to a different size.
    f.niri().layout.set_column_width(SizeChange::SetFixed(500));

    f.double_roundtrip(id);

    // This should request the new size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 100, bounds: 1920 × 1080, states: [Activated]"
    );

    // Before the window has a chance to respond, request a size change to the same, new size.
    f.niri().layout.set_column_width(SizeChange::SetFixed(500));

    // And also drop the Activated state to have some pending change.
    f.niri_focus_output(2);

    f.double_roundtrip(id);

    // This should keep requesting the new size (500 × 100) since the window has not responded to
    // it yet.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 100, bounds: 1920 × 1080, states: []"
    );

    // Commit in response to the size change request.
    let window = f.client(id).window(&surface);
    window.set_size(300, 300);
    window.ack_last_and_commit();
    f.roundtrip(id);

    // And also add the Activated state to have some pending change.
    f.niri_focus_output(1);

    f.double_roundtrip(id);

    // This should request the current window size (300 × 300) since the window has committed in
    // response to the size change.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 300 × 300, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn restore_floating_size() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Change size while we're floating and commit in response to the floating configure.
    let window = f.client(id).window(&surface);
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Change back to tiling.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // We should get a tiling size configure.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 200 × 1048, bounds: 1888 × 1048, states: [Activated]"
    );

    // Resize as requested.
    let window = f.client(id).window(&surface);
    let (_, configure) = window.configures_received.last().unwrap();
    window.set_size(configure.size.0 as u16, configure.size.1 as u16);
    window.ack_last_and_commit();
    f.roundtrip(id);

    // Change back to floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // We should get a configure restoring out previous 200 × 200 size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 200 × 200, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn moving_across_workspaces_doesnt_cancel_resize() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Change size while we're floating and commit in response to the floating configure.
    let window = f.client(id).window(&surface);
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Request a size change to a different size.
    f.niri().layout.set_column_width(SizeChange::SetFixed(500));
    f.double_roundtrip(id);

    // This should request the new size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 200, bounds: 1920 × 1080, states: [Activated]"
    );

    // Move to a different workspace before the window has a chance to respond. This will remove it
    // from one floating layout and add into a different one, potentially causing a size request.
    f.niri().layout.move_to_workspace_down(true);
    // Drop the Activated state to force a configure.
    f.niri_focus_output(2);
    f.double_roundtrip(id);

    // This should request the new size again (500 × 200) since the window hasn't responded to it.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 200, bounds: 1920 × 1080, states: []"
    );

    // Respond to the resize with a different size.
    let window = f.client(id).window(&surface);
    window.set_size(300, 300);
    window.ack_last_and_commit();
    f.roundtrip(id);

    // Focus, adding Activated, and move to workspace down, causing removing and adding to a
    // floating layout.
    f.niri_focus_output(1);
    f.niri().layout.move_to_workspace_down(true);
    f.double_roundtrip(id);

    // This should request the current size (300 × 300) since the window responded to the change.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 300 × 300, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn moving_to_floating_doesnt_cancel_resize() {
    let (mut f, id, surface) = set_up();
    let _ = f.client(id).window(&surface).recent_configures();

    // Request a size change to a different size.
    f.niri().layout.set_column_width(SizeChange::SetFixed(500));
    f.double_roundtrip(id);

    // This should request the new size (500 ×).
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 1048, bounds: 1888 × 1048, states: [Activated]"
    );

    // Before the window has a chance to respond, make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // This should keep requesting the new size (500 ×).
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 1048, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn interactive_move_unfullscreen_to_floating_restores_size() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Change size while we're floating and commit.
    let window = f.client(id).window(&surface);
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    let niri = f.niri();
    let mapped = niri.layout.windows().next().unwrap().1;
    let window = mapped.window.clone();
    niri.layout.set_fullscreen(&window, true);
    f.double_roundtrip(id);

    // This should request a fullscreen size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1888 × 1048, states: [Activated, Fullscreen]"
    );

    // Start an interactive move which causes an unfullscreen into floating.
    let output = f.niri_output(1);
    let niri = f.niri();
    let mapped = niri.layout.windows().next().unwrap().1;
    let window = mapped.window.clone();
    niri.layout
        .interactive_move_begin(window.clone(), &output, Point::default());
    niri.layout.interactive_move_update(
        &window,
        Point::from((1000., 0.)),
        output,
        Point::default(),
    );
    f.double_roundtrip(id);

    // This should request the stored floating size (200 × 200).
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 200 × 200, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn interactive_move_unmaximize_to_floating_restores_size() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Change size while we're floating and commit.
    let window = f.client(id).window(&surface);
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    let niri = f.niri();
    let mapped = niri.layout.windows().next().unwrap().1;
    let window = mapped.window.clone();
    niri.layout.set_maximized(&window, true);
    f.double_roundtrip(id);

    // This should request a maximized size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1888 × 1048, states: [Activated, Maximized]"
    );

    // Start an interactive move which causes an unmaximize into floating.
    let output = f.niri_output(1);
    let niri = f.niri();
    let mapped = niri.layout.windows().next().unwrap().1;
    let window = mapped.window.clone();
    niri.layout
        .interactive_move_begin(window.clone(), &output, Point::default());
    niri.layout.interactive_move_update(
        &window,
        Point::from((1000., 0.)),
        output,
        Point::default(),
    );
    f.double_roundtrip(id);

    // This should request the stored floating size (200 × 200).
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 200 × 200, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn resize_during_interactive_move_propagates_to_floating() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Change size while we're floating and commit.
    let window = f.client(id).window(&surface);
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Start an interactive move.
    let output = f.niri_output(1);
    let niri = f.niri();
    let mapped = niri.layout.windows().next().unwrap().1;
    let window_id = mapped.window.clone();
    niri.layout
        .interactive_move_begin(window_id.clone(), &output, Point::default());
    niri.layout.interactive_move_update(
        &window_id,
        Point::from((1000., 0.)),
        output,
        Point::default(),
    );
    f.double_roundtrip(id);

    // This shouldn't request any new size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @""
    );

    // Change size while we're being interactively moved.
    let window = f.client(id).window(&surface);
    window.set_size(300, 300);
    window.commit();
    f.double_roundtrip(id);

    // This shouldn't request any new size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @""
    );

    // End the interactive move, placing the window into floating.
    f.niri().layout.interactive_move_end(&window_id);
    f.double_roundtrip(id);

    // This should keep the new 300 × 300 size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 300 × 300, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn resize_in_steps() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);
    let _ = f.client(id).window(&surface).recent_configures();

    // Commit in response to the floating bounds state change configure.
    f.client(id).window(&surface).ack_last_and_commit();
    f.double_roundtrip(id);

    // Request a size change to a different size in two steps.
    f.niri().layout.set_column_width(SizeChange::SetFixed(500));
    f.niri()
        .layout
        .set_window_height(None, SizeChange::SetFixed(500));
    f.double_roundtrip(id);

    // This should request the full new size (500 × 500) once.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 500, bounds: 1920 × 1080, states: [Activated]"
    );

    let window = f.client(id).window(&surface);
    let serial = window.configures_received.last().unwrap().0;

    // Request a size change now that the previous one is pending-but-not-acked.
    f.niri().layout.set_column_width(SizeChange::SetFixed(600));
    // Drop Activated to work around resize throttling.
    f.niri_focus_output(2);
    f.double_roundtrip(id);

    // This should request the new size (600 × 500) once.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 600 × 500, bounds: 1920 × 1080, states: []"
    );

    // Commit in response to the previous configure.
    let window = f.client(id).window(&surface);
    window.xdg_surface.ack_configure(serial);
    window.set_size(500, 500);
    window.commit();

    f.double_roundtrip(id);

    // This shouldn't request anything.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @""
    );

    // Request a height change now that the first one is committed-to, but the second isn't.
    let niri = f.niri();
    let mapped = niri.layout.windows().next().unwrap().1;
    let window = mapped.window.clone();
    f.niri()
        .layout
        .set_window_height(Some(&window), SizeChange::SetFixed(600));
    // Add Activated to work around resize throttling.
    f.niri_focus_output(1);
    f.double_roundtrip(id);

    // This should request the latest sizes (600 × 600).
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 600 × 600, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn state_change_doesnt_break_use_window_size() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);
    let _ = f.client(id).window(&surface).recent_configures();

    // Commit in response to the bounds change that comes with toggling floating.
    f.client(id).window(&surface).ack_last_and_commit();
    f.roundtrip(id);

    // Request a size change to a different size.
    f.niri().layout.set_column_width(SizeChange::SetFixed(500));
    f.double_roundtrip(id);

    // This should request the new size (500 × 100).
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 100, bounds: 1920 × 1080, states: [Activated]"
    );

    let window = f.client(id).window(&surface);
    let serial = window.configures_received.last().unwrap().0;

    // Request a state change by dropping Activated.
    f.niri_focus_output(2);
    f.double_roundtrip(id);

    // This should request the new size (500 × 100).
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 500 × 100, bounds: 1920 × 1080, states: []"
    );

    // Commit in response to the previous configure with a different size.
    let window = f.client(id).window(&surface);
    window.xdg_surface.ack_configure(serial);
    window.set_size(300, 300);
    window.commit();

    f.double_roundtrip(id);

    // This shouldn't request anything.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @""
    );

    // Request a height change now that the first one is committed-to, but the second isn't.
    let niri = f.niri();
    let mapped = niri.layout.windows().next().unwrap().1;
    let window = mapped.window.clone();
    f.niri()
        .layout
        .set_window_height(Some(&window), SizeChange::SetFixed(600));
    // Add Activated state to force a configure.
    f.niri_focus_output(1);
    f.double_roundtrip(id);

    // This should already request the current width (300 × 600) rather than the pending previous
    // width (500 × 600) from the state change configure.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 300 × 600, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn interactive_move_restores_floating_size_when_set_to_floating() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Change size while we're floating and commit to make niri remember it.
    let window = f.client(id).window(&surface);
    window.set_size(200, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Change back to tiling.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // We should get a tiled size configure.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 200 × 1048, bounds: 1888 × 1048, states: [Activated]"
    );

    // Resize as requested.
    let window = f.client(id).window(&surface);
    let (_, configure) = window.configures_received.last().unwrap();
    window.set_size(configure.size.0 as u16, configure.size.1 as u16);
    window.ack_last_and_commit();
    f.roundtrip(id);

    // Start an interactive move.
    let output = f.niri_output(1);
    let niri = f.niri();
    let mapped = niri.layout.windows().next().unwrap().1;
    let window_id = mapped.window.clone();
    niri.layout
        .interactive_move_begin(window_id.clone(), &output, Point::default());
    niri.layout.interactive_move_update(
        &window_id,
        Point::from((1000., 0.)),
        output,
        Point::default(),
    );
    f.double_roundtrip(id);

    // This shouldn't request any new size because interactive move targets tiling.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 200 × 1048, bounds: 1920 × 1080, states: [Activated]"
    );

    // Change interactive move to target floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // This should restore the floating window size (200 × 200).
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 200 × 200, bounds: 1920 × 1080, states: [Activated]"
    );

    // End the interactive move, placing the window into floating.
    f.niri().layout.interactive_move_end(&window_id);
    f.double_roundtrip(id);

    // This should keep the floating window size (200 × 200).
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @""
    );
}

#[test]
fn floating_doesnt_store_fullscreen_size() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));

    // Open a window fullscreen.
    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.set_fullscreen(None);
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(1920, 1080);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // This should request 0 × 0 to unfullscreen.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 0 × 0, bounds: 1920 × 1080, states: [Activated]"
    );

    // Without committing, make it tiling again. We never committed while floating, so there's no
    // floating size to remember.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // This should request the tiled size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1048, bounds: 1888 × 1048, states: [Activated]"
    );

    // Commit in response.
    let window = f.client(id).window(&surface);
    window.set_size(100, 100);
    window.ack_last_and_commit();
    f.roundtrip(id);

    // Make the window floating again.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // This shouldn't request any size change, particularly not the fullscreen size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 100 × 100, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn floating_doesnt_store_maximized_size() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));

    // Open a window maximized.
    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.set_maximized();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(1920, 1080);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // This should request 0 × 0 to unmaximize.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 0 × 0, bounds: 1920 × 1080, states: [Activated]"
    );

    // Without committing, make it tiling again. We never committed while floating, so there's no
    // floating size to remember.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // This should request the tiled size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1048, bounds: 1888 × 1048, states: [Activated]"
    );

    // Commit in response.
    let window = f.client(id).window(&surface);
    window.set_size(100, 100);
    window.ack_last_and_commit();
    f.roundtrip(id);

    // Make the window floating again.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // This shouldn't request any size change, particularly not the maximized size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 100 × 100, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn floating_respects_non_fixed_min_max_rule() {
    let config = r##"
window-rule {
    min-width 200
    max-width 300
}
"##;
    let config = Config::parse_mem(config).unwrap();
    let (mut f, id, surface) = set_up_with_config(config);

    // Commit to the Activated state configure.
    f.client(id).window(&surface).ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // This should clamp to min-width and request 200 × 100.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 200 × 100, bounds: 1920 × 1080, states: [Activated]"
    );

    // Commit with a bigger width than max.
    let window = f.client(id).window(&surface);
    window.set_size(400, 100);
    window.ack_last_and_commit();
    f.roundtrip(id);

    // Make it tiling.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    f.client(id).window(&surface).ack_last_and_commit();
    f.roundtrip(id);

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // This should clamp to max-width and request 300 × 100.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 300 × 100, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn unmap_from_floating() {
    let (mut f, id, surface) = set_up();

    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);
    let _ = f.client(id).window(&surface).recent_configures();

    // Resize to something different on both axes.
    let window = f.client(id).window(&surface);
    window.attach_null();
    window.commit();

    // Shouldn't panic.
    f.double_roundtrip(id);
}

#[test]
fn unfullscreen_to_floating_doesnt_send_extra_configure() {
    let (mut f, id, surface) = set_up();

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.roundtrip(id);

    // Fullscreen.
    let window = f.client(id).window(&surface);
    window.set_fullscreen(None);
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Unfullscreen via the window request which requires a configure response.
    let window = f.client(id).window(&surface);
    window.unset_fullscreen();
    f.double_roundtrip(id);

    // This should configure only once and not twice.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 936 × 1048, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn unmaximize_to_floating_doesnt_send_extra_configure() {
    let (mut f, id, surface) = set_up();

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.roundtrip(id);

    // Maximize.
    let window = f.client(id).window(&surface);
    window.set_maximized();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Unmaximzie via the window request which requires a configure response.
    let window = f.client(id).window(&surface);
    window.unset_maximized();
    f.double_roundtrip(id);

    // This should configure only once and not twice.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 936 × 1048, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn unfullscreen_to_same_size_floating() {
    let (mut f, id, surface) = set_up();

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Change size to the same as fullscreen, make niri remember it.
    let window = f.client(id).window(&surface);
    window.set_size(1920, 1080);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Fullscreen.
    let window = f.client(id).window(&surface);
    window.set_fullscreen(None);
    f.double_roundtrip(id);

    // The fullscreen configure.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1888 × 1048, states: [Activated, Fullscreen]"
    );

    // Unfullscreen into floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // We should see a configure with the same size and no Fullscreen state.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn unmaximize_to_same_size_floating() {
    let (mut f, id, surface) = set_up();

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Change size to the same as maximized, make niri remember it.
    let window = f.client(id).window(&surface);
    window.set_size(1920, 1080);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Maximize.
    let window = f.client(id).window(&surface);
    window.set_maximized();
    f.double_roundtrip(id);

    // The maximize configure.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1888 × 1048, states: [Activated, Maximized]"
    );

    // Unmaximize into floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // We should see a configure with the same size and no maximized state.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn unfullscreen_to_same_size_windowed_fullscreen_floating() {
    let (mut f, id, surface) = set_up();

    let mapped = f.niri().layout.windows().next().unwrap().1;
    let window_id = mapped.window.clone();

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Change size to the same as fullscreen, make niri remember it.
    let window = f.client(id).window(&surface);
    window.set_size(1920, 1080);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Fullscreen.
    let window = f.client(id).window(&surface);
    window.set_fullscreen(None);
    f.double_roundtrip(id);

    // The fullscreen configure.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1888 × 1048, states: [Activated, Fullscreen]"
    );

    // Unfullscreen into windowed-fullscreen floating.
    f.niri().layout.toggle_windowed_fullscreen(&window_id);
    f.double_roundtrip(id);

    // Should send configure because the bounds have changed.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1920 × 1080, states: [Activated, Fullscreen]"
    );
}

#[test]
fn unmaximize_to_same_size_windowed_fullscreen_floating() {
    let (mut f, id, surface) = set_up();

    let mapped = f.niri().layout.windows().next().unwrap().1;
    let window_id = mapped.window.clone();

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Change size to the same as maximized, make niri remember it.
    let window = f.client(id).window(&surface);
    window.set_size(1920, 1080);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Maximize.
    let window = f.client(id).window(&surface);
    window.set_maximized();
    f.double_roundtrip(id);

    // The maximize configure.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1888 × 1048, states: [Activated, Maximized]"
    );

    // Enable windowed-fullscreen.
    f.niri().layout.toggle_windowed_fullscreen(&window_id);
    f.double_roundtrip(id);

    // The windowed-fullscreen configure.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1888 × 1048, states: [Activated, Fullscreen]"
    );

    // Go back to windowed-fullscreen floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Should send configure because the bounds have changed.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1920 × 1080, states: [Activated, Fullscreen]"
    );

    // Disable windowed-fullscreen.
    f.niri().layout.toggle_windowed_fullscreen(&window_id);
    f.double_roundtrip(id);

    // Should send configure dropping the Fullscreen state.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn unfullscreen_to_same_size_same_bounds_floating() {
    let config = r##"
layout {
    gaps 0
}
"##;
    let config = Config::parse_mem(config).unwrap();
    let (mut f, id, surface) = set_up_with_config(config);

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Change size to the same as fullscreen, make niri remember it.
    let window = f.client(id).window(&surface);
    window.set_size(1920, 1080);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Fullscreen.
    let window = f.client(id).window(&surface);
    window.set_fullscreen(None);
    f.double_roundtrip(id);

    // The fullscreen configure.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1920 × 1080, states: [Activated, Fullscreen]"
    );

    // Unfullscreen into floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // We should see a configure with the same size and no Fullscreen state.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn unmaximize_to_same_size_same_bounds_floating() {
    let config = r##"
layout {
    gaps 0
}
"##;
    let config = Config::parse_mem(config).unwrap();
    let (mut f, id, surface) = set_up_with_config(config);

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // Change size to the same as fullscreen, make niri remember it.
    let window = f.client(id).window(&surface);
    window.set_size(1920, 1080);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface).recent_configures();

    // Maximize.
    let window = f.client(id).window(&surface);
    window.set_maximized();
    f.double_roundtrip(id);

    // The maximize configure.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1920 × 1080, states: [Activated, Maximized]"
    );

    // Unmaximize into floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // We should see a configure with the same size and no Maximized state.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn repeated_size_request() {
    let (mut f, id, surface) = set_up();
    let _ = f.client(id).window(&surface).recent_configures();

    // Make it floating.
    f.niri().layout.toggle_window_floating(None);
    f.double_roundtrip(id);

    // The floating configure.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 936 × 1048, bounds: 1920 × 1080, states: [Activated]"
    );

    // Request a different width (200x100).
    f.niri()
        .layout
        .set_window_width(None, SizeChange::SetFixed(200));
    f.niri()
        .layout
        .set_window_height(None, SizeChange::SetFixed(100));
    f.double_roundtrip(id);

    // The 200x100 request.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 200 × 100, bounds: 1920 × 1080, states: [Activated]"
    );

    // Request a size change to the same size as we have just requested.
    f.niri().layout.set_column_width(SizeChange::SetFixed(200));
    f.double_roundtrip(id);

    // Should request nothing as this is a repeated same-size request in floating and the surface
    // hasn't committed to it yet.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @""
    );

    // Ack but don't commit yet.
    let window = f.client(id).window(&surface);
    window.ack_last();
    f.double_roundtrip(id);

    // Request a size change to the same size as we have just requested.
    f.niri().layout.set_column_width(SizeChange::SetFixed(200));
    f.double_roundtrip(id);

    // Should request nothing as this is a repeated same-size request in floating and the surface
    // hasn't committed to it yet.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @""
    );

    // Commit.
    let window = f.client(id).window(&surface);
    window.commit();
    f.double_roundtrip(id);

    // Request the size change again.
    f.niri().layout.set_column_width(SizeChange::SetFixed(200));
    f.double_roundtrip(id);

    // This should send a new configure since the window had committed.
    //
    // FIXME: doesn't request that currently.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @""
    );
}
