use smithay::desktop::layer_map_for_output;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::KeyboardInteractivity;

use super::client::LayerConfigureProps;
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
fn remove_output_cleans_up_layer_surfaces() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));

    let id = f.add_client();
    f.double_roundtrip(id);
    let surviving_wl_output = f.client(id).output("headless-1");
    let wl_output = f.client(id).output("headless-2");

    let surviving_layer =
        f.client(id)
            .create_layer(Some(&surviving_wl_output), Layer::Top, "surviving");
    let surviving_surface = surviving_layer.surface.clone();
    surviving_layer.set_configure_props(LayerConfigureProps {
        size: Some((100, 100)),
        ..Default::default()
    });
    surviving_layer.commit();
    f.double_roundtrip(id);

    let surviving_layer = f.client(id).layer(&surviving_surface);
    surviving_layer.attach_new_buffer();
    surviving_layer.set_size(100, 100);
    surviving_layer.ack_last_and_commit();
    f.double_roundtrip(id);

    let layer = f
        .client(id)
        .create_layer(Some(&wl_output), Layer::Top, "removed");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        size: Some((100, 100)),
        kb_interactivity: Some(KeyboardInteractivity::OnDemand),
        ..Default::default()
    });
    layer.commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    let unmapped_layer = f
        .client(id)
        .create_layer(Some(&wl_output), Layer::Top, "unmapped");
    let unmapped_surface = unmapped_layer.surface.clone();
    f.double_roundtrip(id);

    assert_eq!(f.niri().mapped_layer_surfaces.len(), 2);
    assert_eq!(f.niri().unmapped_layer_surfaces.len(), 1);
    assert!(f.niri().layer_shell_on_demand_focus.is_some());

    let output = f.niri_output(2);
    f.niri().remove_output(&output);

    let niri = f.niri();
    assert_eq!(niri.mapped_layer_surfaces.len(), 1);
    assert!(niri
        .mapped_layer_surfaces
        .keys()
        .any(|layer| layer.namespace() == "surviving"));
    assert!(niri.unmapped_layer_surfaces.is_empty());
    assert!(niri.layer_shell_on_demand_focus.is_none());
    assert_eq!(layer_map_for_output(&output).layers().count(), 0);

    f.double_roundtrip(id);

    assert!(!f.client(id).layer(&surviving_surface).close_requested);
    assert!(f.client(id).layer(&surface).close_requested);
    assert!(f.client(id).layer(&unmapped_surface).close_requested);

    f.client(id).layer(&surface).destroy();
    f.client(id).layer(&unmapped_surface).destroy();
    f.double_roundtrip(id);

    assert_eq!(f.niri().mapped_layer_surfaces.len(), 1);
}
