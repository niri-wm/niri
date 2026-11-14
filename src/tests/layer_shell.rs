use std::net::Shutdown;
use std::os::unix::net::UnixStream;

use insta::assert_snapshot;
use niri_config::Config;
use smithay::backend::input::{InputEvent, InputTime, KeyState};
use smithay::desktop::layer_map_for_output;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity,
};

use super::*;
use crate::tests::client::{LayerConfigureProps, LayerMargin};
use crate::tests::test_input_backend::{TestInputBackend, TestKeyboardKeyEvent};

#[test]
fn remove_output_cleans_up_layers_before_client_response() {
    // Exercise both explicit destruction orders, and disconnect without destroying the layers.
    for cleanup in 0..3 {
        let mut f = Fixture::new();
        f.add_output(1, (1920, 1080));
        f.add_output(2, (1280, 720));
        let removed_output = f.niri_output(1);
        let remaining_output = f.niri_output(2);
        let id = f.add_client();
        f.double_roundtrip(id);
        let mut surfaces = Vec::new();

        for output_name in ["headless-1", "headless-2"] {
            let output = f.client(id).output(output_name);
            for mapped in [true, false] {
                let layer = f.client(id).create_layer(Some(&output), Layer::Top, "");
                let surface = layer.surface.clone();
                layer.set_configure_props(LayerConfigureProps {
                    size: Some((100, 50)),
                    kb_interactivity: Some(if output_name == "headless-1" {
                        KeyboardInteractivity::OnDemand
                    } else {
                        KeyboardInteractivity::None
                    }),
                    ..Default::default()
                });
                layer.commit();
                f.double_roundtrip(id);

                if mapped {
                    let layer = f.client(id).layer(&surface);
                    layer.attach_new_buffer();
                    layer.set_size(100, 50);
                    layer.ack_last_and_commit();
                    f.double_roundtrip(id);
                }
                surfaces.push(surface);
            }
        }

        assert_eq!(f.niri().mapped_layer_surfaces.len(), 2);
        assert_eq!(f.niri().unmapped_layer_surfaces.len(), 2);
        assert!(f.niri().layer_shell_on_demand_focus.is_some());

        f.niri().remove_output(&removed_output);

        // No client dispatch has happened: cleanup must not depend on handling closed.
        assert!(!f.client(id).layer(&surfaces[0]).close_requested);
        assert_eq!(layer_map_for_output(&removed_output).layers().count(), 0);
        assert_eq!(layer_map_for_output(&remaining_output).layers().count(), 2);
        assert_eq!(f.niri().mapped_layer_surfaces.len(), 1);
        assert_eq!(f.niri().unmapped_layer_surfaces.len(), 1);
        assert!(f.niri().layer_shell_on_demand_focus.is_none());
        assert!(f
            .niri()
            .mapped_layer_surfaces
            .keys()
            .all(|layer| layer_map_for_output(&remaining_output)
                .layers()
                .any(|l| l == layer)));
        let remaining_unmapped = layer_map_for_output(&remaining_output)
            .layers()
            .find(|layer| !f.niri().mapped_layer_surfaces.contains_key(*layer))
            .unwrap()
            .wl_surface()
            .clone();
        assert!(f
            .niri()
            .unmapped_layer_surfaces
            .contains(&remaining_unmapped));

        f.double_roundtrip(id);
        for surface in &surfaces[..2] {
            assert!(f.client(id).layer(surface).close_requested);
        }
        for surface in &surfaces[2..] {
            assert!(!f.client(id).layer(surface).close_requested);
        }

        // Late commits must not reinsert a closed layer into either tracking collection.
        f.client(id).layer(&surfaces[0]).commit();
        f.double_roundtrip(id);
        f.client(id).layer(&surfaces[0]).attach_null();
        f.client(id).layer(&surfaces[0]).commit();
        f.double_roundtrip(id);
        assert_eq!(f.niri().mapped_layer_surfaces.len(), 1);
        assert_eq!(f.niri().unmapped_layer_surfaces.len(), 1);

        if cleanup == 2 {
            // Only dispatch the server after disconnecting: the fixture's client source is
            // deliberately still registered, but must not try to read the closed connection.
            let fd = f
                .client(id)
                .connection
                .backend()
                .poll_fd()
                .try_clone_to_owned()
                .unwrap();
            UnixStream::from(fd).shutdown(Shutdown::Both).unwrap();
            f.state.server.dispatch();
            assert!(f.niri().mapped_layer_surfaces.is_empty());
            assert!(f.niri().unmapped_layer_surfaces.is_empty());
            assert_eq!(layer_map_for_output(&remaining_output).layers().count(), 0);
        } else {
            for surface in &surfaces[..2] {
                let role = f.client(id).layer(surface).layer_surface.clone();
                if cleanup == 0 {
                    role.destroy();
                    f.double_roundtrip(id);
                    surface.destroy();
                } else {
                    surface.destroy();
                    f.double_roundtrip(id);
                    role.destroy();
                }
                f.double_roundtrip(id);
            }
            assert_eq!(f.niri().mapped_layer_surfaces.len(), 1);
            assert_eq!(f.niri().unmapped_layer_surfaces.len(), 1);
            assert_eq!(layer_map_for_output(&remaining_output).layers().count(), 2);
        }
        assert_eq!(layer_map_for_output(&removed_output).layers().count(), 0);
    }
}

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

#[test]
fn keyboard_interactivity_none_then_on_demand() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let layer = f.client(id).create_layer(None, Layer::Overlay, "");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        size: Some((100, 100)),
        kb_interactivity: Some(KeyboardInteractivity::None),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    // Start with None; switch to OnDemand immediately after mapping.
    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();

    layer.set_configure_props(LayerConfigureProps {
        kb_interactivity: Some(KeyboardInteractivity::OnDemand),
        ..Default::default()
    });
    layer.commit();

    f.double_roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"");
}

#[test]
fn keyboard_interactivity_on_demand_to_none_then_on_demand() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let layer = f.client(id).create_layer(None, Layer::Overlay, "");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        size: Some((100, 100)),
        kb_interactivity: Some(KeyboardInteractivity::OnDemand),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"
    enter: []
    modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Set to None and back to OnDemand within one compositor event loop iteration.
    let layer = f.client(id).layer(&surface);
    layer.set_configure_props(LayerConfigureProps {
        kb_interactivity: Some(KeyboardInteractivity::None),
        ..Default::default()
    });
    layer.commit();
    layer.set_configure_props(LayerConfigureProps {
        kb_interactivity: Some(KeyboardInteractivity::OnDemand),
        ..Default::default()
    });
    layer.commit();
    f.double_roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"leave");
}

#[test]
fn keyboard_interactivity_exclusive_to_on_demand() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let layer = f.client(id).create_layer(None, Layer::Overlay, "");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        size: Some((100, 100)),
        kb_interactivity: Some(KeyboardInteractivity::Exclusive),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"
    enter: []
    modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    let layer = f.client(id).layer(&surface);
    layer.set_configure_props(LayerConfigureProps {
        kb_interactivity: Some(KeyboardInteractivity::OnDemand),
        ..Default::default()
    });
    layer.commit();
    f.double_roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"leave");
}

#[test]
fn focus_on_type_in_overview() {
    let config = r#"
    input {
        keyboard {
            xkb {
                layout "us,ru"
                options "grp:lalt_toggle"
            }
        }
    }

    layer-rule {
        focus-on-type-in-overview true
    }
    "#;
    let config = Config::parse_mem(config).unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    // Open the overview.
    f.niri().layout.toggle_overview();

    // Spawn a None->OnDemand layer surface (no automatic keyboard focus).
    let layer = f.client(id).create_layer(None, Layer::Overlay, "");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        size: Some((100, 100)),
        kb_interactivity: Some(KeyboardInteractivity::None),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();

    layer.set_configure_props(LayerConfigureProps {
        kb_interactivity: Some(KeyboardInteractivity::OnDemand),
        ..Default::default()
    });
    layer.commit();

    f.double_roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"");

    let event = |f: &mut Fixture, key: &str, down: bool| {
        let state = f.niri_state();

        let keyboard = state.niri.seat.get_keyboard().unwrap();
        let code = keyboard.with_xkb_state(state, |xkb| {
            let xkb = xkb.xkb().lock().unwrap();
            let keymap = unsafe { xkb.keymap() };
            keymap.key_by_name(key).expect("unknown key")
        });

        let key_state = if down {
            KeyState::Pressed
        } else {
            KeyState::Released
        };

        state.process_input_event(InputEvent::<TestInputBackend>::Keyboard {
            event: TestKeyboardKeyEvent {
                time: InputTime::from_micros(0),
                code,
                state: key_state,
                count: 1, // niri doesn't use this
            },
        });
    };

    // Hardcoded overview binds don't put focus on the layer surface.
    event(&mut f, "LEFT", true);
    event(&mut f, "LEFT", false);
    f.roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"");

    // Other non-typing keys don't put focus on the layer surface.
    event(&mut f, "BKSP", true);
    event(&mut f, "BKSP", false);
    f.roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"");

    // Typing keys do.
    event(&mut f, "LatQ", true);
    event(&mut f, "LatQ", false);
    f.roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"
    enter: []
    modifiers: depressed=0, latched=0, locked=0, group=0
    key pressed: 16
    key released: 16
    ");

    // Drop keyboard focus.
    let layer = f.client(id).layer(&surface);
    layer.set_configure_props(LayerConfigureProps {
        kb_interactivity: Some(KeyboardInteractivity::None),
        ..Default::default()
    });
    layer.commit();
    layer.set_configure_props(LayerConfigureProps {
        kb_interactivity: Some(KeyboardInteractivity::OnDemand),
        ..Default::default()
    });
    layer.commit();
    f.double_roundtrip(id);
    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"leave");

    // Hit Shift first. By itself, it shouldn't focus the surface, but the letter should,
    // with correct modifiers.
    event(&mut f, "LFSH", true);
    event(&mut f, "LatQ", true);
    event(&mut f, "LatQ", false);
    event(&mut f, "LFSH", false);
    f.roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"
    enter: [42]
    modifiers: depressed=1, latched=0, locked=0, group=0
    key pressed: 16
    key released: 16
    key released: 42
    modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Drop keyboard focus.
    let layer = f.client(id).layer(&surface);
    layer.set_configure_props(LayerConfigureProps {
        kb_interactivity: Some(KeyboardInteractivity::None),
        ..Default::default()
    });
    layer.commit();
    layer.set_configure_props(LayerConfigureProps {
        kb_interactivity: Some(KeyboardInteractivity::OnDemand),
        ..Default::default()
    });
    layer.commit();
    f.double_roundtrip(id);
    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"leave");

    // Switch layout and try with a non-ASCII typing character.
    event(&mut f, "LALT", true);
    event(&mut f, "LALT", false);
    event(&mut f, "LatQ", true);
    event(&mut f, "LatQ", false);
    f.roundtrip(id);

    assert_snapshot!(f.client(id).state.format_recent_keyboard_events(&surface), @"
    enter: []
    modifiers: depressed=0, latched=0, locked=0, group=1
    key pressed: 16
    key released: 16
    ");
}
