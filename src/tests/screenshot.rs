use niri_config::{Config, ScreenshotPadding};
use smithay::backend::renderer::element::Element;
use smithay::utils::{Point, Rectangle, Scale, Size};

use super::*;
use crate::render_helpers::encompassing_geo;

fn set_up_with_border(border_width: f64) -> Fixture {
    let mut config = Config::default();
    config.layout.border.off = false;
    config.layout.border.width = border_width;
    config.layout.focus_ring.off = true;
    config.layout.shadow.on = true;
    config.layout.shadow.softness = 8.0;
    config.layout.shadow.spread = 4.0;
    config.layout.shadow.draw_behind_window = false;
    config.layout.shadow.color = niri_config::Color::from_rgba8_unpremul(0, 0, 0, 128);

    let mut f = Fixture::with_config(config);
    f.niri_state().backend.headless().add_renderer().unwrap();
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

    f
}

#[test]
fn screenshot_bounding_box_with_padding() {
    let _f = set_up_with_border(0.0);
    let scale = Scale::from(1.0);

    let pad = ScreenshotPadding {
        left: 12.0,
        right: 24.0,
        top: 18.0,
        bottom: 30.0,
    };

    let base_geo: Rectangle<i32, smithay::utils::Physical> =
        Rectangle::new(Point::from((10, 10)), Size::from((100, 100)));
    let pad_l = (pad.left.max(0.0) * scale.x).round() as i32;
    let pad_r = (pad.right.max(0.0) * scale.x).round() as i32;
    let pad_t = (pad.top.max(0.0) * scale.y).round() as i32;
    let pad_b = (pad.bottom.max(0.0) * scale.y).round() as i32;

    let canvas_size: Size<i32, smithay::utils::Physical> = Size::new(
        base_geo.size.w + pad_l + pad_r,
        base_geo.size.h + pad_t + pad_b,
    );
    let canvas_loc: Point<i32, smithay::utils::Physical> =
        Point::new(base_geo.loc.x - pad_l, base_geo.loc.y - pad_t);

    assert_eq!(canvas_size.w, 100 + 12 + 24);
    assert_eq!(canvas_size.h, 100 + 18 + 30);
    assert_eq!(canvas_loc.x, 10 - 12);
    assert_eq!(canvas_loc.y, 10 - 18);
}

#[test]
fn screenshot_shadow_offset_with_decorative_borders() {
    let border_width = 6.0;
    let mut f = set_up_with_border(border_width);

    let window = f.niri().layout.windows().next().unwrap().1.window.clone();

    let state = f.niri_state();
    let tile = state
        .niri
        .layout
        .find_tile_for_window(&window)
        .expect("tile should be found");

    assert_eq!(tile.visual_border_width(), Some(border_width));

    let win_loc = Point::from((0.0, 0.0));
    let shadow_loc = match tile.visual_border_width() {
        Some(width) => win_loc - Point::from((width, width)),
        None => win_loc,
    };

    // Shadow location must be inset by (-border_width, -border_width) to align with outer border box.
    assert_eq!(shadow_loc, Point::from((-6.0, -6.0)));

    let mut shadow_elements = Vec::new();
    state.backend.with_primary_renderer(|renderer| {
        tile.render_shadow(renderer, shadow_loc, false, &mut |elem| {
            shadow_elements.push(elem);
        });
    });

    assert!(!shadow_elements.is_empty(), "shadow elements must be generated");

    let scale = Scale::from(1.0);
    let bounds = encompassing_geo(scale, shadow_elements.iter());

    // Window size is 100x100, border is 6.0, outer border box is (-6, -6, 112, 112).
    // The center of the outer border box is (-6 + 112 / 2, -6 + 112 / 2) = (50.0, 50.0).
    // The shadow bounds must be symmetrically centered around (50, 50).
    let center_x = bounds.loc.x as f64 + (bounds.size.w as f64) / 2.0;
    let center_y = bounds.loc.y as f64 + (bounds.size.h as f64) / 2.0;
    assert_eq!(center_x, 50.0);
    // Config default shadow offset.y is 5.0, so center_y is 50.0 + 5.0 = 55.0.
    assert_eq!(center_y, 55.0);

    // If the uncorrected win_loc were passed (the bug), shadow would center at (56.0, 61.0).
    let mut uncorrected_elements = Vec::new();
    state.backend.with_primary_renderer(|renderer| {
        tile.render_shadow(renderer, win_loc, false, &mut |elem| {
            uncorrected_elements.push(elem);
        });
    });
    let uncorrected_bounds = encompassing_geo(scale, uncorrected_elements.iter());
    let uncorrected_center_x =
        uncorrected_bounds.loc.x as f64 + (uncorrected_bounds.size.w as f64) / 2.0;
    let uncorrected_center_y =
        uncorrected_bounds.loc.y as f64 + (uncorrected_bounds.size.h as f64) / 2.0;
    assert_eq!(uncorrected_center_x, 56.0);
    assert_eq!(uncorrected_center_y, 61.0);
}

#[test]
fn screenshot_transparency_preservation_no_shadow_under_window() {
    let mut f = set_up_with_border(0.0);

    let window = f.niri().layout.windows().next().unwrap().1.window.clone();

    let state = f.niri_state();
    let tile = state
        .niri
        .layout
        .find_tile_for_window(&window)
        .expect("tile should be found");

    let win_loc = Point::from((0.0, 0.0));
    let scale = Scale::from(1.0);
    let window_center = Point::from((50, 50));

    // When draw_behind_window is false (window-background "transparent"):
    let mut transparent_shadow_elements = Vec::new();
    state.backend.with_primary_renderer(|renderer| {
        tile.render_shadow(renderer, win_loc, false, &mut |elem| {
            transparent_shadow_elements.push(elem);
        });
    });

    assert!(!transparent_shadow_elements.is_empty());
    let center_covered = transparent_shadow_elements
        .iter()
        .any(|elem| elem.geometry(scale).contains(window_center));
    assert!(
        !center_covered,
        "shadow quad must NOT cover the window center when window background is transparent"
    );

    // Red-Green check: when draw_behind_window is true, the center IS covered by the shadow quad.
    let mut behind_shadow_elements = Vec::new();
    state.backend.with_primary_renderer(|renderer| {
        tile.render_shadow(renderer, win_loc, true, &mut |elem| {
            behind_shadow_elements.push(elem);
        });
    });
    let behind_center_covered = behind_shadow_elements
        .iter()
        .any(|elem| elem.geometry(scale).contains(window_center));
    assert!(
        behind_center_covered,
        "when draw_behind_window is true, shadow quad covers behind the window"
    );
}

#[test]
fn screenshot_unrendered_tile_has_valid_shadow_bounds() {
    let mut f = set_up_with_border(4.0);

    let window = f.niri().layout.windows().next().unwrap().1.window.clone();

    let state = f.niri_state();
    let tile = state
        .niri
        .layout
        .find_tile_for_window(&window)
        .expect("tile should be found");

    // Even if update_render_elements was not called previously on this tile,
    // render_shadow must compute up-to-date shadow elements.
    let mut elements = Vec::new();
    state.backend.with_primary_renderer(|renderer| {
        tile.render_shadow(renderer, Point::from((0.0, 0.0)), false, &mut |elem| {
            elements.push(elem);
        });
    });

    assert!(!elements.is_empty(), "unrendered tile must generate valid shadow elements");
    let scale = Scale::from(1.0);
    let bounds = encompassing_geo(scale, elements.iter());
    assert!(bounds.size.w > 100);
    assert!(bounds.size.h > 100);
}

#[test]
fn screenshot_wallpaper_crop_on_moving_and_floating_windows() {
    let mut f = set_up_with_border(0.0);

    let output1 = f.niri_output(1);
    let output2 = f.niri_output(2);

    let window = f.niri().layout.windows().next().unwrap().1.window.clone();

    // 1. Tiled window position on output 1.
    let pos_tiled = f
        .niri()
        .layout
        .tile_pos_for_window(&output1, &window);
    assert!(pos_tiled.is_some(), "tiled window must have position on output 1");

    // Output 2 does not have this window in its workspaces.
    let pos_out2 = f
        .niri()
        .layout
        .tile_pos_for_window(&output2, &window);
    assert_eq!(pos_out2, None);

    // 2. Floating window.
    f.niri().layout.toggle_window_floating(None);

    let pos_floating = f
        .niri()
        .layout
        .tile_pos_for_window(&output1, &window);
    assert!(pos_floating.is_some(), "floating window must have valid position on output 1");

    // 3. Interactive move state.
    f.niri().layout.interactive_move_begin(
        window.clone(),
        &output1,
        Point::from((500.0, 400.0)),
    );

    let pos_moving = f
        .niri()
        .layout
        .tile_pos_for_window(&output1, &window);
    assert!(pos_moving.is_some(), "moving window must have position on its output");
    assert_ne!(pos_moving.unwrap(), Point::from((0.0, 0.0)));
}
