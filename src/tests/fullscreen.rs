use client::ClientId;
use insta::assert_snapshot;
use niri_config::Config;
use smithay::utils::Point;
use wayland_client::protocol::wl_surface::WlSurface;

use super::*;
use crate::layout::LayoutElement as _;

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
fn windowed_fullscreen() {
    let (mut f, id, surface) = set_up();

    let _ = f.client(id).window(&surface).recent_configures();

    let niri = f.niri();
    let mapped = niri.layout.windows().next().unwrap().1;
    let window_id = mapped.window.clone();

    // Enable windowed fullscreen.
    niri.layout.toggle_windowed_fullscreen(&window_id);
    f.double_roundtrip(id);

    // Should request fullscreen state with the tiled size.
    let window = f.client(id).window(&surface);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 936 × 1048, bounds: 1888 × 1048, states: [Activated, Fullscreen]"
    );

    let mapped = f.niri().layout.windows().next().unwrap().1;
    // Not committed yet.
    assert!(!mapped.is_windowed_fullscreen());

    // Commit in response.
    let window = f.client(id).window(&surface);
    window.ack_last_and_commit();
    f.roundtrip(id);

    let mapped = f.niri().layout.windows().next().unwrap().1;
    // Now it is committed.
    assert!(mapped.is_windowed_fullscreen());

    // Disable windowed fullscreen.
    f.niri().layout.toggle_windowed_fullscreen(&window_id);
    f.double_roundtrip(id);

    // Should request without fullscreen state with the tiled size.
    let window = f.client(id).window(&surface);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 936 × 1048, bounds: 1888 × 1048, states: [Activated]"
    );

    let mapped = f.niri().layout.windows().next().unwrap().1;
    // Not committed yet.
    assert!(mapped.is_windowed_fullscreen());

    // Commit in response.
    let window = f.client(id).window(&surface);
    window.ack_last_and_commit();
    f.roundtrip(id);

    let mapped = f.niri().layout.windows().next().unwrap().1;
    // Now it is committed.
    assert!(!mapped.is_windowed_fullscreen());
}

#[test]
fn windowed_fullscreen_chain() {
    let (mut f, id, surface) = set_up();

    let _ = f.client(id).window(&surface).recent_configures();

    let mapped = f.niri().layout.windows().next().unwrap().1;
    let window_id = mapped.window.clone();

    f.niri().layout.toggle_windowed_fullscreen(&window_id);
    f.roundtrip(id);
    f.niri().layout.toggle_windowed_fullscreen(&window_id);
    f.roundtrip(id);
    f.niri().layout.toggle_windowed_fullscreen(&window_id);
    f.roundtrip(id);
    f.niri().layout.toggle_windowed_fullscreen(&window_id);
    f.double_roundtrip(id);

    // Should be four configures matching the four requests.
    let window = f.client(id).window(&surface);
    assert_snapshot!(
        window.format_recent_configures(),
        @r"
    size: 936 × 1048, bounds: 1888 × 1048, states: [Activated, Fullscreen]
    size: 936 × 1048, bounds: 1888 × 1048, states: [Activated]
    size: 936 × 1048, bounds: 1888 × 1048, states: [Activated, Fullscreen]
    size: 936 × 1048, bounds: 1888 × 1048, states: [Activated]
    "
    );

    let window = f.client(id).window(&surface);
    let serials = Vec::from_iter(
        window.configures_received[window.configures_received.len() - 4..]
            .iter()
            .map(|(s, _c)| *s),
    );

    let get_state = |f: &mut Fixture| {
        let mapped = f.niri().layout.windows().next().unwrap().1;
        format!(
            "fs {}, wfs {}",
            mapped.sizing_mode().is_fullscreen(),
            mapped.is_windowed_fullscreen()
        )
    };

    let mut states = vec![get_state(&mut f)];
    for serial in serials {
        let window = f.client(id).window(&surface);
        window.xdg_surface.ack_configure(serial);
        window.commit();
        f.roundtrip(id);
        states.push(get_state(&mut f));
    }

    // We expect fs to always be false (because each Fullscreen state request corresponded to a
    // windowed fullscreen), and wfs to toggle on and off.
    assert_snapshot!(
        states.join("\n"),
        @r"
    fs false, wfs false
    fs false, wfs true
    fs false, wfs false
    fs false, wfs true
    fs false, wfs false
    "
    );
}

#[test]
fn unfullscreen_before_fullscreen_ack_doesnt_prevent_view_offset_save_restore() {
    let (mut f, id, _surface) = set_up();

    let window2 = f.client(id).create_window();
    let surface2 = window2.surface.clone();
    window2.commit();
    f.roundtrip(id);

    let window2 = f.client(id).window(&surface2);
    window2.attach_new_buffer();
    window2.set_size(200, 200);
    window2.ack_last_and_commit();
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface2).recent_configures();

    let niri = f.niri();
    let mapped2 = niri.layout.windows().last().unwrap().1;
    let window2_id = mapped2.window.clone();

    // The view position is at the first window.
    assert_snapshot!(niri.layout.active_workspace().unwrap().scrolling().view_pos(), @"-16");

    // Fullscreen window2 and send the configure so we can clear pending.
    niri.layout.set_fullscreen(&window2_id, true);
    f.double_roundtrip(id);

    // Before acking, unfullscreen the column, clearing the pending fullscreen flag.
    f.niri().layout.set_fullscreen(&window2_id, false);

    // Now, window2 receives the fullscreen configure and resizes in response.
    let window2 = f.client(id).window(&surface2);
    assert_snapshot!(
        window2.format_recent_configures(),
        @"size: 1920 × 1080, bounds: 1888 × 1048, states: [Activated, Fullscreen]"
    );
    let (_, configure) = window2.configures_received.last().unwrap();
    window2.set_size(configure.size.0 as u16, configure.size.1 as u16);
    window2.ack_last_and_commit();
    f.double_roundtrip(id);
    f.niri_complete_animations();

    // The view position is now at the fullscreen-sized window2.
    assert_snapshot!(f.niri().layout.active_workspace().unwrap().scrolling().view_pos(), @"116");

    // Now, window2 receives the unfullscreen configure and resizes in response.
    let window2 = f.client(id).window(&surface2);
    assert_snapshot!(
        window2.format_recent_configures(),
        @"size: 936 × 1048, bounds: 1888 × 1048, states: [Activated]"
    );
    window2.set_size(200, 200);
    window2.ack_last_and_commit();
    f.roundtrip(id);
    f.niri_complete_animations();

    // The view position should restore to the first window.
    assert_snapshot!(f.niri().layout.active_workspace().unwrap().scrolling().view_pos(), @"-16");
}

#[test]
fn interactive_move_unfullscreen_to_scrolling_restores_size() {
    let (mut f, id, surface) = set_up();

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

    // Start an interactive move which causes an unfullscreen.
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

    // This should request the tiled size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 936 × 1048, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn interactive_move_unmaximize_to_scrolling_restores_size() {
    let (mut f, id, surface) = set_up();

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

    // Start an interactive move which causes an unmaximize.
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

    // This should request the tiled size.
    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"size: 936 × 1048, bounds: 1920 × 1080, states: [Activated]"
    );
}

#[test]
fn fullscreen_max_size() {
    let config = r#"
    window-rule {
        open-fullscreen true
        max-width 500
        max-height 700
    }
    "#;
    let config = Config::parse_mem(config).unwrap();
    let (mut f, id, surface) = set_up_with_config(config);

    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"
    size: 500 × 700, bounds: 1888 × 1048, states: [Fullscreen]
    size: 500 × 700, bounds: 1888 × 1048, states: [Fullscreen, Activated]
    "
    );
}

#[test]
fn fullscreen_min_size() {
    let config = r#"
    window-rule {
        open-fullscreen true
        min-width 2000
        min-height 3000
    }
    "#;
    let config = Config::parse_mem(config).unwrap();
    let (mut f, id, surface) = set_up_with_config(config);

    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"
    size: 2000 × 3000, bounds: 1888 × 1048, states: [Fullscreen]
    size: 2000 × 3000, bounds: 1888 × 1048, states: [Fullscreen, Activated]
    "
    );
}

#[test]
fn maximized_max_size() {
    let config = r#"
    window-rule {
        open-maximized-to-edges true
        max-width 500
        max-height 700
    }
    "#;
    let config = Config::parse_mem(config).unwrap();
    let (mut f, id, surface) = set_up_with_config(config);

    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"
    size: 500 × 700, bounds: 1888 × 1048, states: [Maximized]
    size: 500 × 700, bounds: 1888 × 1048, states: [Maximized, Activated]
    "
    );
}

#[test]
fn maximized_min_size() {
    let config = r#"
    window-rule {
        open-maximized-to-edges true
        min-width 2000
        min-height 3000
    }
    "#;
    let config = Config::parse_mem(config).unwrap();
    let (mut f, id, surface) = set_up_with_config(config);

    assert_snapshot!(
        f.client(id).window(&surface).format_recent_configures(),
        @"
    size: 2000 × 3000, bounds: 1888 × 1048, states: [Maximized]
    size: 2000 × 3000, bounds: 1888 × 1048, states: [Maximized, Activated]
    "
    );
}

#[test]
fn fullscreen_backdrop_color_and_draw_background() {
    let config = r##"
    layout {
        draw-fullscreen-background true
    }
    window-rule {
        opacity 0.75
    }
    "##;
    let config = Config::parse_mem(config).unwrap();
    let (mut f, id, surface) = set_up_with_config(config);

    let niri = f.niri();
    let mapped = niri.layout.windows().next().unwrap().1;
    let window_id = mapped.window.clone();

    // Toggle fullscreen.
    niri.layout.toggle_fullscreen(&window_id);
    f.double_roundtrip(id);

    let window = f.client(id).window(&surface);
    window.ack_last_and_commit();
    f.roundtrip(id);

    let tile = f.niri().layout.find_tile_for_window(&window_id).unwrap();

    // With draw-fullscreen-background true and no custom backdrop, no solid quad is drawn.
    assert!(!tile.renders_fullscreen_backdrop());
    // Window maintains its configured opacity since wallpaper is rendered underneath.
    assert_eq!(tile.effective_window_alpha(), 0.75);
}

#[test]
fn fullscreen_custom_backdrop_color_with_draw_background_false() {
    let config = r##"
    layout {
        draw-fullscreen-background false
    }
    window-rule {
        draw-fullscreen-background false
        fullscreen-backdrop-color "#112233"
        opacity 0.60
    }
    "##;
    let config = Config::parse_mem(config).unwrap();
    let (mut f, id, surface) = set_up_with_config(config);

    let niri = f.niri();
    let mapped = niri.layout.windows().next().unwrap().1;
    let window_id = mapped.window.clone();

    // Toggle fullscreen.
    niri.layout.toggle_fullscreen(&window_id);
    f.double_roundtrip(id);

    let window = f.client(id).window(&surface);
    window.ack_last_and_commit();
    f.roundtrip(id);

    let tile = f.niri().layout.find_tile_for_window(&window_id).unwrap();

    // With draw-fullscreen-background false, solid backdrop quad is rendered to block wallpaper.
    assert!(tile.renders_fullscreen_backdrop());
    // Since custom backdrop has opaque alpha (0xff), alpha is interpolated to 1.0.
    assert_eq!(tile.effective_window_alpha(), 1.0);
}
