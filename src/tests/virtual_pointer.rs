use wayland_client::protocol::wl_pointer;
use wayland_client::protocol::wl_surface::WlSurface;

use super::*;
use crate::layout::LayoutElement;

#[test]
fn axis_discrete_overflow() {
    let mut f = Fixture::new();
    let id = f.add_client();

    let client = f.client(id);
    let manager = client.state.virtual_pointer_manager.as_ref().unwrap();
    let pointer = manager.create_virtual_pointer(None, &client.qh, ());
    pointer.axis_discrete(0, wl_pointer::Axis::VerticalScroll, 0., i32::MAX);
    f.roundtrip(id);
}

#[test]
fn input_region_hole_respects_decoration_background() {
    for draw_background in [false, true] {
        let config = format!(
            r#"
layout {{
    focus-ring {{ off; }}
    border {{ on; width 4; }}
}}

window-rule {{
    match app-id="^input-region-test$"
    open-floating true
    default-floating-position x=100 y=100
    draw-border-with-background {draw_background}
}}
"#
        );
        let config = niri_config::Config::parse_mem(&config).unwrap();
        let mut f = Fixture::with_config(config);
        f.add_output(1, (800, 600));
        let client_id = f.add_client();

        let _bottom = map_window(&mut f, client_id, false);
        let bottom_id = LayoutElement::id(f.niri().layout.focus().unwrap()).clone();
        let _top = map_window(&mut f, client_id, true);
        let top_id = LayoutElement::id(f.niri().layout.focus().unwrap()).clone();

        // Keep the top window above the bottom one while setting the opposite focus state from
        // what the click should produce.
        let initially_focused = if draw_background { &bottom_id } else { &top_id };
        f.niri()
            .layout
            .activate_window_without_raising(initially_focused);

        let pointer = {
            let client = f.client(client_id);
            client
                .state
                .virtual_pointer_manager
                .as_ref()
                .unwrap()
                .create_virtual_pointer(None, &client.qh, ())
        };
        pointer.motion_absolute(0, 150, 150, 800, 600);
        pointer.button(1, 0x110, wl_pointer::ButtonState::Pressed);
        pointer.button(2, 0x110, wl_pointer::ButtonState::Released);
        pointer.frame();
        f.double_roundtrip(client_id);

        let expected = if draw_background { &top_id } else { &bottom_id };
        assert_eq!(
            LayoutElement::id(f.niri().layout.focus().unwrap()),
            expected
        );
    }
}

#[test]
fn left_button_starts_resize_on_compositor_decoration() {
    for (decoration, pointer_x) in [("border", 102), ("focus-ring", 98)] {
        let config = format!(
            r#"
layout {{
    focus-ring {{ {}; width 4; }}
    border {{ {}; width 4; }}
}}

window-rule {{
    match app-id="^input-region-test$"
    open-floating true
    default-floating-position x=100 y=100
    draw-border-with-background false
}}
"#,
            if decoration == "focus-ring" {
                "on"
            } else {
                "off"
            },
            if decoration == "border" { "on" } else { "off" },
        );
        let config = niri_config::Config::parse_mem(&config).unwrap();
        let mut f = Fixture::with_config(config);
        f.add_output(1, (800, 600));
        let client_id = f.add_client();
        let _window = map_window(&mut f, client_id, true);

        let pointer = {
            let client = f.client(client_id);
            client
                .state
                .virtual_pointer_manager
                .as_ref()
                .unwrap()
                .create_virtual_pointer(None, &client.qh, ())
        };
        pointer.motion_absolute(0, pointer_x, 150, 800, 600);
        pointer.button(1, 0x110, wl_pointer::ButtonState::Pressed);
        pointer.frame();
        f.double_roundtrip(client_id);

        assert!(
            f.niri()
                .layout
                .focus()
                .unwrap()
                .interactive_resize_data()
                .is_some(),
            "left click did not start a resize from the {decoration}"
        );

        pointer.button(2, 0x110, wl_pointer::ButtonState::Released);
        pointer.frame();
        f.double_roundtrip(client_id);
    }
}

fn map_window(f: &mut Fixture, client_id: client::ClientId, input_hole: bool) -> WlSurface {
    let window = f.client(client_id).create_window();
    let surface = window.surface.clone();
    window.xdg_toplevel.set_app_id("input-region-test".into());
    window.commit();
    f.roundtrip(client_id);

    if input_hole {
        f.client(client_id)
            .set_input_region(&surface, [(0, 0, 20, 20)]);
    }
    let window = f.client(client_id).window(&surface);
    window.attach_new_buffer();
    window.set_size(100, 100);
    window.ack_last_and_commit();
    f.double_roundtrip(client_id);
    surface
}
