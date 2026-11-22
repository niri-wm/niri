use insta::assert_snapshot;
use wayland_client::protocol::wl_pointer;

use super::*;

#[test]
fn axis_discrete_overflow() {
    let mut f = Fixture::new();
    let id = f.add_client();

    let client = f.client(id);
    let pointer = client.state.virtual_pointer();
    pointer.axis_discrete(0, wl_pointer::Axis::VerticalScroll, 0., i32::MAX);
    f.roundtrip(id);
}

#[test]
fn virtual_pointer_delivers_enter_motion_button_leave() {
    let mut f = Fixture::new();
    f.add_output(1, (1280, 720));

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(400, 400);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.niri_complete_animations();
    f.roundtrip(id);

    let pointer = f.client(id).state.virtual_pointer();
    pointer.motion_absolute(0, 200, 200, 1280, 720);
    pointer.frame();
    pointer.motion_absolute(1, 220, 200, 1280, 720);
    pointer.frame();
    pointer.button(2, 0x110, wl_pointer::ButtonState::Pressed);
    pointer.frame();
    pointer.button(3, 0x110, wl_pointer::ButtonState::Released);
    pointer.frame();
    pointer.motion_absolute(4, 1200, 200, 1280, 720);
    pointer.frame();
    f.roundtrip(id);

    assert_snapshot!(
        f.client(id).state.format_recent_pointer_events(&surface),
        @"
    enter: 184, 184
    motion: 204, 184
    button pressed: 272 (time=2)
    button released: 272 (time=3)
    leave
    "
    );
    assert_eq!(
        f.client(id).state.recent_pointer_events(&surface).count(),
        0
    );
}
