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
