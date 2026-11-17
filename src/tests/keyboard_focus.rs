use insta::assert_snapshot;

use super::*;

#[test]
fn leave_on_focused_window_destroy() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"
    enter: []
    modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Destroy everything in one batch, like clients do when closing a window.
    let window = f.client(id).window(&surface);
    window.xdg_toplevel.destroy();
    window.xdg_surface.destroy();
    window.viewport.destroy();
    window.surface.destroy();
    f.double_roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"leave");
}
