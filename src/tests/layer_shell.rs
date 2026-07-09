use insta::assert_snapshot;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity,
};

use super::*;
use crate::tests::client::{LayerConfigureProps, LayerMargin};

#[test]
fn simple_top_anchor() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let layer = f.client(id).create_layer(None, Layer::Top, "");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top),
        size: Some((0, 50)),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    assert_snapshot!(layer.format_recent_configures(), @"size: 1920 × 50");
}

#[test]
fn margin_overflow() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let layer = f.client(id).create_layer(None, Layer::Top, "");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top | Anchor::Bottom),
        margin: Some(LayerMargin {
            top: i32::MAX,
            right: i32::MAX,
            bottom: i32::MAX,
            left: i32::MAX,
        }),
        exclusive_zone: Some(i32::MAX),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    assert_snapshot!(layer.format_recent_configures(), @"size: 0 × 0");

    // Add a second one for good measure.
    let layer = f.client(id).create_layer(None, Layer::Top, "");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top | Anchor::Bottom),
        margin: Some(LayerMargin {
            top: i32::MAX,
            right: i32::MAX,
            bottom: i32::MAX,
            left: i32::MAX,
        }),
        exclusive_zone: Some(i32::MAX),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    assert_snapshot!(layer.format_recent_configures(), @"size: 0 × 0");
}

#[test]
fn unmap_through_null_buffer() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let layer = f.client(id).create_layer(None, Layer::Top, "");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top),
        size: Some((0, 50)),
        ..Default::default()
    });
    layer.commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    assert_snapshot!(layer.format_recent_configures(), @"size: 1920 × 50");

    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    // No new configure since nothing changed.
    assert_snapshot!(layer.format_recent_configures(), @"");

    // Unmap by attaching a null buffer. This moves the surface back to pre-initial-commit stage.
    layer.attach_null();
    layer.commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    // Configures must be empty because we haven't done an initial commit yet.
    assert_snapshot!(layer.format_recent_configures(), @"");

    // Do the initial commit again.
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top),
        size: Some((0, 100)),
        ..Default::default()
    });
    layer.commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    // This is the new initial configure.
    assert_snapshot!(layer.format_recent_configures(), @"size: 1920 × 100");

    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    assert_snapshot!(layer.format_recent_configures(), @"");
}

#[test]
fn multiple_commits_before_mapping() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let layer = f.client(id).create_layer(None, Layer::Top, "");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top),
        size: Some((0, 50)),
        ..Default::default()
    });
    layer.commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    assert_snapshot!(layer.format_recent_configures(), @"size: 1920 × 50");

    // Change something that won't cause a configure.
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top),
        size: Some((0, 50)),
        kb_interactivity: Some(KeyboardInteractivity::OnDemand),
        ..Default::default()
    });
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    // No new configure since the size hasn't changed.
    assert_snapshot!(layer.format_recent_configures(), @"");

    // Change something that will cause a configure.
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top),
        size: Some((0, 100)),
        ..Default::default()
    });
    layer.commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    // Configure with new size.
    assert_snapshot!(layer.format_recent_configures(), @"size: 1920 × 100");

    // Map.
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    // No new configure since nothing changed.
    assert_snapshot!(layer.format_recent_configures(), @"");

    // Unmap by attaching a null buffer. This moves the surface back to pre-initial-commit stage.
    layer.attach_null();
    layer.commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    // Configures must be empty because we haven't done an initial commit yet.
    assert_snapshot!(layer.format_recent_configures(), @"");

    // Same configure props as before, but since we unmapped, we should get a new initial
    // configure (that will happen to match the previous configure we had got while mapped).
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top),
        size: Some((0, 100)),
        ..Default::default()
    });
    layer.commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    assert_snapshot!(layer.format_recent_configures(), @"size: 1920 × 100");

    // Change something that won't cause a configure.
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top),
        size: Some((0, 100)),
        kb_interactivity: Some(KeyboardInteractivity::OnDemand),
        ..Default::default()
    });
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    // No new configure since the size hasn't changed.
    assert_snapshot!(layer.format_recent_configures(), @"");

    // Change something that will cause a configure.
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top),
        size: Some((0, 50)),
        ..Default::default()
    });
    layer.commit();
    f.double_roundtrip(id);

    let layer = f.client(id).layer(&surface);
    // Configure with new size.
    assert_snapshot!(layer.format_recent_configures(), @"size: 1920 × 50");
}
