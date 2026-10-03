use super::*;

#[test]
fn set_fullscreen_on_removed_output_does_not_panic() {
    let mut f = Fixture::new();
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

    // Grab the second output's wl_output proxy on the client side.
    let wl_output = f.client(id).output("headless-2");

    // Remove the output on the niri side. Its wl_output global is disabled but not yet
    // destroyed, so the client's wl_output resource is still valid and usable.
    let output = f.niri_output(2);
    f.niri().remove_output(&output);

    // Request fullscreen on the now-removed wl_output. niri must not panic.
    let window = f.client(id).window(&surface);
    window.set_fullscreen(Some(&wl_output));
    f.double_roundtrip(id);
}

#[test]
fn relative_output_position_is_applied_at_runtime() {
    use niri_config::{
        Align, Config, Direction, Output as OutputConfig, OutputName, Outputs, Position,
    };

    let mut config = Config::default();
    config.outputs = Outputs(vec![OutputConfig {
        name: "headless-2".to_owned(),
        position: Some(Position::Relative {
            relative_to: "headless-1".to_owned(),
            direction: Direction::LeftOf,
            align: Align::Beginning,
        }),
        ..Default::default()
    }]);

    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));

    let niri = f.niri();
    let mut g1 = None;
    let mut g2 = None;
    for output in niri.global_space.outputs() {
        let connector = output.user_data().get::<OutputName>().unwrap().connector.clone();
        let geo = niri.global_space.output_geometry(output).unwrap();
        match connector.as_str() {
            "headless-1" => g1 = Some(geo),
            "headless-2" => g2 = Some(geo),
            _ => {}
        }
    }
    let g1 = g1.expect("headless-1 not placed");
    let g2 = g2.expect("headless-2 not placed");

    // headless-2 is left-of headless-1: its right edge touches headless-1's left edge, top-aligned.
    // If relative positioning were ignored, headless-2 would auto-place to the RIGHT instead.
    assert_eq!(g2.loc.x + g2.size.w, g1.loc.x, "right edge should touch anchor's left edge");
    assert_eq!(g2.loc.y, g1.loc.y, "should be top-aligned");
}
