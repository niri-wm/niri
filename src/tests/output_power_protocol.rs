use smithay::reexports::wayland_protocols_wlr::output_power_management::v1::client::zwlr_output_power_v1;

use super::*;

#[test]
fn protocol_global_is_advertised() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let globals = &f.client(id).state.globals;
    let found = globals
        .iter()
        .any(|g| g.interface == "zwlr_output_power_manager_v1");
    assert!(
        found,
        "zwlr_output_power_manager_v1 not advertised in globals"
    );

    assert!(
        f.client(id).state.output_power_manager.is_some(),
        "output_power_manager not bound by registry handler"
    );
}

#[test]
fn initial_mode_event_is_reported() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    f.double_roundtrip(id);

    let output = f.client(id).output("headless-1");
    f.client(id).create_output_power_control(&output);
    f.double_roundtrip(id);

    let control = f.client(id).state.output_power_control(&output);
    let data = control.data.lock().unwrap();
    assert!(
        !data.mode_events.is_empty(),
        "no mode events received after creating power control"
    );
    let last_mode = *data.mode_events.last().unwrap();
    assert_eq!(
        last_mode,
        zwlr_output_power_v1::Mode::On,
        "expected initial mode On, got {last_mode:?}"
    );
}

#[test]
fn control_fails_on_output_remove() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    f.double_roundtrip(id);

    let output = f.client(id).output("headless-1");
    f.client(id).create_output_power_control(&output);
    f.double_roundtrip(id);

    {
        let control = f.client(id).state.output_power_control(&output);
        let data = control.data.lock().unwrap();
        assert!(
            !data.failed_received,
            "failed event received before output removal"
        );
    }

    let server_output = f.niri_output(1);
    f.niri().remove_output(&server_output);
    f.double_roundtrip(id);

    let control = f.client(id).state.output_power_control(&output);
    let data = control.data.lock().unwrap();
    assert!(
        data.failed_received,
        "failed event not received after output removal"
    );
}
