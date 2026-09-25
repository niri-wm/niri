use smithay::utils::Point;
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
    for (border, focus_ring) in [(false, false), (false, true), (true, false), (true, true)] {
        for draw_background in [false, true] {
            for top_active in [false, true] {
                let config = format!(
                    r#"
layout {{
    focus-ring {{ {focus_ring}; }}
    border {{ {border}; width 4; }}
}}

window-rule {{
    match app-id="^input-region-test$"
    open-floating true
    default-floating-position x=100 y=100
    draw-border-with-background {draw_background}
}}
"#,
                    focus_ring = if focus_ring { "on" } else { "off" },
                    border = if border { "on" } else { "off" },
                );
                let config = niri_config::Config::parse_mem(&config).unwrap();
                let mut f = Fixture::with_config(config);
                f.add_output(1, (800, 600));
                let client_id = f.add_client();

                let _bottom = map_window(&mut f, client_id, false);
                let bottom_id = LayoutElement::id(f.niri().layout.focus().unwrap()).clone();
                let _top = map_window(&mut f, client_id, true);
                let top_id = LayoutElement::id(f.niri().layout.focus().unwrap()).clone();

                // Changing focus must not change the stacking order.
                let initially_focused = if top_active { &top_id } else { &bottom_id };
                f.niri()
                    .layout
                    .activate_window_without_raising(initially_focused);

                let expected = if draw_background && (border || (focus_ring && top_active)) {
                    &top_id
                } else {
                    &bottom_id
                };
                let output = f.niri().layout.outputs().next().unwrap().clone();
                let hit = f.niri().layout.window_under(&output, (150., 150.).into());
                assert_eq!(
            hit.map(|(window, _)| LayoutElement::id(window)),
            Some(expected),
            "border={border}, focus_ring={focus_ring}, draw_background={draw_background}, top_active={top_active}"
        );

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

                assert_eq!(
                    LayoutElement::id(f.niri().layout.focus().unwrap()),
                    expected
                );
            }
        }
    }
}

#[test]
fn left_press_resizes_visible_floating_border_only() {
    for (border, draw_background, offset, should_resize) in [
        (true, false, (2., 50.), true),
        (true, false, (50., 50.), false),
        (true, false, (10., 10.), false),
        (false, true, (50., 50.), false),
    ] {
        let config = format!(
            r#"
layout {{
    border {{ {border}; width 4; }}
    focus-ring {{ on; }}
}}
window-rule {{
    match app-id="^input-region-test$"
    open-floating true
    default-floating-position x=100 y=100
    draw-border-with-background {draw_background}
}}
"#,
            border = if border { "on" } else { "off" },
        );
        let mut f = Fixture::with_config(niri_config::Config::parse_mem(&config).unwrap());
        f.add_output(1, (800, 600));
        let client_id = f.add_client();
        let surface = map_window(&mut f, client_id, true);

        let (tile, tile_pos, _) = f
            .niri()
            .layout
            .active_workspace()
            .unwrap()
            .tiles_with_render_positions()
            .next()
            .unwrap();
        assert_eq!(tile.window_size(), (100., 100.).into());
        let pos = tile_pos + Point::from(offset);
        let pointer = {
            let client = f.client(client_id);
            client
                .state
                .virtual_pointer_manager
                .as_ref()
                .unwrap()
                .create_virtual_pointer(None, &client.qh, ())
        };
        pointer.motion_absolute(0, pos.x as u32, pos.y as u32, 800, 600);
        pointer.frame();
        f.roundtrip(client_id);
        pointer.button(1, 0x110, wl_pointer::ButtonState::Pressed);
        pointer.frame();
        f.double_roundtrip(client_id);

        let resizing = f
            .niri()
            .layout
            .focus()
            .unwrap()
            .interactive_resize_data()
            .is_some();
        assert_eq!(
            resizing, should_resize,
            "border={border}, draw_background={draw_background}, offset={offset:?}"
        );

        if should_resize {
            f.client(client_id).window(&surface).ack_last_and_commit();
            f.double_roundtrip(client_id);
            pointer.motion_absolute(2, (pos.x - 20.) as u32, pos.y as u32, 800, 600);
            pointer.frame();
            f.double_roundtrip(client_id);
            let size = f
                .client(client_id)
                .window(&surface)
                .configures_received
                .last()
                .unwrap()
                .1
                .size;
            let requested = f.niri().layout.focus().unwrap().requested_size();
            let pointer_pos = f.niri().seat.get_pointer().unwrap().current_location();
            assert!(
                size.0 > 100,
                "left-border drag did not increase width: configure={size:?}, requested={requested:?}, pointer={pointer_pos:?}, start={pos:?}"
            );
        }
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
