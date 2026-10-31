use super::*;

#[test]
fn virtual_output_custom_mode_does_not_accumulate_modes() {
    let mut f = Fixture::new();

    let name = {
        let state = f.niri_state();
        state
            .backend
            .create_virtual_output(&mut state.niri, 1920, 1080, 60, Some("sunshine".to_owned()))
            .unwrap()
    };

    let output = f
        .niri()
        .global_space
        .outputs()
        .find(|o| o.name() == name)
        .unwrap()
        .clone();

    // Sanity: single initial mode.
    {
        let modes = output.modes();
        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0].size.w, 1920);
        assert_eq!(modes[0].size.h, 1080);
    }

    // 1080p -> 3200x1800
    {
        let state = f.niri_state();
        state.apply_transient_output_config(
            &name,
            niri_ipc::OutputAction::CustomMode {
                mode: niri_ipc::ConfiguredMode {
                    width: 3200,
                    height: 1800,
                    refresh: Some(60.0),
                },
            },
        );
    }

    {
        let modes = output.modes();
        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0].size.w, 3200);
        assert_eq!(modes[0].size.h, 1800);
    }

    // 3200x1800 -> 1080p
    {
        let state = f.niri_state();
        state.apply_transient_output_config(
            &name,
            niri_ipc::OutputAction::CustomMode {
                mode: niri_ipc::ConfiguredMode {
                    width: 1920,
                    height: 1080,
                    refresh: Some(60.0),
                },
            },
        );
    }

    {
        let modes = output.modes();
        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0].size.w, 1920);
        assert_eq!(modes[0].size.h, 1080);
    }
}

#[test]
fn removing_off_virtual_output_does_not_panic() {
    let mut f = Fixture::new();

    let name = {
        let state = f.niri_state();
        state
            .backend
            .create_virtual_output(&mut state.niri, 1920, 1080, 60, Some("virt".to_owned()))
            .unwrap()
    };

    {
        let state = f.niri_state();
        state.modify_output_config(&name, |config| {
            config.off = true;
        });
    }

    {
        let state = f.niri_state();
        state
            .backend
            .remove_virtual_output(&mut state.niri, &name)
            .unwrap();
    }
}

#[test]
fn config_remove_virtual_output_fails_while_configured() {
    let mut f = Fixture::new();
    let name = "virt";

    {
        let state = f.niri_state();
        state.modify_output_config(name, |config| {
            config.create_virtual = true;
        });
    }

    let result = {
        let state = f.niri_state();
        state.backend.remove_virtual_output(&mut state.niri, name)
    };

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("is configured"));
}

#[test]
fn unnamed_virtual_outputs_do_not_advance_counter_for_named_outputs() {
    let mut f = Fixture::new();

    {
        let state = f.niri_state();
        state.modify_output_config("virt", |_| {});
    }

    let headless_1 = {
        let state = f.niri_state();
        state
            .backend
            .create_virtual_output(&mut state.niri, 1920, 1080, 60, None)
            .unwrap()
    };

    assert_eq!(headless_1, "HEADLESS-1");

    let headless_2 = {
        let state = f.niri_state();
        state
            .backend
            .create_virtual_output(&mut state.niri, 1920, 1080, 60, None)
            .unwrap()
    };

    assert_eq!(headless_2, "HEADLESS-2");
}

#[test]
fn touch_input_targets_virtual_output_when_focused() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));

    let name = {
        let state = f.niri_state();
        state
            .backend
            .create_virtual_output(&mut state.niri, 1920, 1080, 60, Some("virt".to_owned()))
            .unwrap()
    };

    let virt = f
        .niri()
        .global_space
        .outputs()
        .find(|o| o.name() == name)
        .unwrap()
        .clone();

    f.niri().layout.focus_output(&virt);

    // With no explicit `input.touch.map-to-output` configured,
    // touch should follow the active output (which may be virtual).
    let touch_output = f.niri().output_for_touch().unwrap().clone();
    assert_eq!(touch_output, virt);
}
