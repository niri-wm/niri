use smithay::reexports::wayland_protocols_wlr::output_power_management::v1::client::zwlr_output_power_v1;

use super::*;
use crate::niri::LockState;
use crate::protocols::output_power_management::Mode;

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

#[test]
fn mode_change_fans_out_to_all_controls() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    f.double_roundtrip(id);

    let output = f.client(id).output("headless-1");
    f.client(id).create_output_power_control(&output);
    f.client(id).create_output_power_control(&output);
    f.double_roundtrip(id);

    // Both controls should have received the initial On mode event.
    {
        let (data0_arc, data1_arc) = {
            let controls = &f.client(id).state.output_powers[&output];
            (controls[0].data.clone(), controls[1].data.clone())
        };
        let data0 = data0_arc.lock().unwrap();
        let data1 = data1_arc.lock().unwrap();
        assert_eq!(
            *data0.mode_events.last().unwrap(),
            zwlr_output_power_v1::Mode::On,
            "control 0: expected initial On"
        );
        assert_eq!(
            *data1.mode_events.last().unwrap(),
            zwlr_output_power_v1::Mode::On,
            "control 1: expected initial On"
        );
    }

    // Set mode Off — both controls must receive the event.
    f.set_output_power_mode(1, Mode::Off);
    f.double_roundtrip(id);

    {
        let (data0_arc, data1_arc) = {
            let controls = &f.client(id).state.output_powers[&output];
            (controls[0].data.clone(), controls[1].data.clone())
        };
        let data0 = data0_arc.lock().unwrap();
        let data1 = data1_arc.lock().unwrap();
        assert_eq!(
            *data0.mode_events.last().unwrap(),
            zwlr_output_power_v1::Mode::Off,
            "control 0: expected Off after set_output_power_mode(Off)"
        );
        assert_eq!(
            *data1.mode_events.last().unwrap(),
            zwlr_output_power_v1::Mode::Off,
            "control 1: expected Off after set_output_power_mode(Off)"
        );
    }

    // Set mode On — both controls must receive the event.
    f.set_output_power_mode(1, Mode::On);
    f.double_roundtrip(id);

    {
        let (data0_arc, data1_arc) = {
            let controls = &f.client(id).state.output_powers[&output];
            (controls[0].data.clone(), controls[1].data.clone())
        };
        let data0 = data0_arc.lock().unwrap();
        let data1 = data1_arc.lock().unwrap();
        assert_eq!(
            *data0.mode_events.last().unwrap(),
            zwlr_output_power_v1::Mode::On,
            "control 0: expected On after set_output_power_mode(On)"
        );
        assert_eq!(
            *data1.mode_events.last().unwrap(),
            zwlr_output_power_v1::Mode::On,
            "control 1: expected On after set_output_power_mode(On)"
        );
    }
}

#[test]
fn deactivate_reactivate_preserves_per_output_mode_events() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    let id = f.add_client();
    f.double_roundtrip(id);

    let output1 = f.client(id).output("headless-1");
    let output2 = f.client(id).output("headless-2");

    f.client(id).create_output_power_control(&output1);
    f.client(id).create_output_power_control(&output2);
    f.double_roundtrip(id);

    // Set output-1 to Off, leave output-2 On.
    f.set_output_power_mode(1, Mode::Off);
    f.double_roundtrip(id);

    // Simulate a monitor deactivate/reactivate cycle (e.g. lid-close + lid-open).
    f.deactivate_monitors();
    for _ in 0..3 {
        f.dispatch();
    }
    f.activate_monitors();
    for _ in 0..3 {
        f.dispatch();
    }
    f.double_roundtrip(id);

    // After reactivation:
    //   - output-1 was set to Off before the cycle → last mode event must be Off
    //   - output-2 was never changed → last mode event must be On
    let last1 = {
        let control = f.client(id).state.output_power_control(&output1);
        let data = control.data.lock().unwrap();
        *data
            .mode_events
            .last()
            .expect("no mode events for output-1")
    };
    let last2 = {
        let control = f.client(id).state.output_power_control(&output2);
        let data = control.data.lock().unwrap();
        *data
            .mode_events
            .last()
            .expect("no mode events for output-2")
    };

    assert_eq!(
        last1,
        zwlr_output_power_v1::Mode::Off,
        "output-1: expected last mode Off after deactivate/reactivate cycle, got {last1:?}"
    );
    assert_eq!(
        last2,
        zwlr_output_power_v1::Mode::On,
        "output-2: expected last mode On after deactivate/reactivate cycle, got {last2:?}"
    );
}

#[test]
fn last_writer_wins_across_clients() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id_a = f.add_client();
    let id_b = f.add_client();
    f.double_roundtrip(id_a);
    f.double_roundtrip(id_b);

    let output_a = f.client(id_a).output("headless-1");
    let output_b = f.client(id_b).output("headless-1");
    f.client(id_a).create_output_power_control(&output_a);
    f.client(id_b).create_output_power_control(&output_b);
    f.double_roundtrip(id_a);
    f.double_roundtrip(id_b);

    // Client A sets Off — both controls must see Off (fan-out).
    let power_a = f
        .client(id_a)
        .state
        .output_power_control(&output_a)
        .power
        .clone();
    power_a.set_mode(zwlr_output_power_v1::Mode::Off);
    f.client(id_a).connection.flush().unwrap();
    f.double_roundtrip(id_a);
    f.double_roundtrip(id_b);

    assert_eq!(
        *f.client(id_a)
            .state
            .output_power_control(&output_a)
            .data
            .lock()
            .unwrap()
            .mode_events
            .last()
            .unwrap(),
        zwlr_output_power_v1::Mode::Off,
        "client A: expected Off after A sets Off"
    );
    assert_eq!(
        *f.client(id_b)
            .state
            .output_power_control(&output_b)
            .data
            .lock()
            .unwrap()
            .mode_events
            .last()
            .unwrap(),
        zwlr_output_power_v1::Mode::Off,
        "client B: expected Off after A sets Off (fan-out)"
    );

    // Client B sets On — last writer wins, both controls must see On.
    let power_b = f
        .client(id_b)
        .state
        .output_power_control(&output_b)
        .power
        .clone();
    power_b.set_mode(zwlr_output_power_v1::Mode::On);
    f.client(id_b).connection.flush().unwrap();
    f.double_roundtrip(id_a);
    f.double_roundtrip(id_b);

    assert_eq!(
        *f.client(id_a)
            .state
            .output_power_control(&output_a)
            .data
            .lock()
            .unwrap()
            .mode_events
            .last()
            .unwrap(),
        zwlr_output_power_v1::Mode::On,
        "client A: expected On after B sets On (last writer wins)"
    );
    assert_eq!(
        *f.client(id_b)
            .state
            .output_power_control(&output_b)
            .data
            .lock()
            .unwrap()
            .mode_events
            .last()
            .unwrap(),
        zwlr_output_power_v1::Mode::On,
        "client B: expected On after B sets On"
    );
}

#[test]
fn disconnect_of_one_client_does_not_break_other_controls() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id_a = f.add_client();
    let id_b = f.add_client();
    f.double_roundtrip(id_a);
    f.double_roundtrip(id_b);

    let output_a = f.client(id_a).output("headless-1");
    let output_b = f.client(id_b).output("headless-1");
    f.client(id_a).create_output_power_control(&output_a);
    f.client(id_b).create_output_power_control(&output_b);
    f.double_roundtrip(id_a);
    f.double_roundtrip(id_b);

    // Client A sets Off — both see it.
    let power_a = f
        .client(id_a)
        .state
        .output_power_control(&output_a)
        .power
        .clone();
    power_a.set_mode(zwlr_output_power_v1::Mode::Off);
    f.client(id_a).connection.flush().unwrap();
    f.double_roundtrip(id_a);
    f.double_roundtrip(id_b);

    // Destroy client A's control object — server removes it from the list.
    let power_a = f
        .client(id_a)
        .state
        .output_power_control(&output_a)
        .power
        .clone();
    power_a.destroy();
    f.client(id_a).connection.flush().unwrap();
    f.double_roundtrip(id_a);
    f.double_roundtrip(id_b);

    // Client B sets On after A's control is gone — B's control must still work.
    let power_b = f
        .client(id_b)
        .state
        .output_power_control(&output_b)
        .power
        .clone();
    power_b.set_mode(zwlr_output_power_v1::Mode::On);
    f.client(id_b).connection.flush().unwrap();
    f.double_roundtrip(id_b);

    assert_eq!(
        *f.client(id_b)
            .state
            .output_power_control(&output_b)
            .data
            .lock()
            .unwrap()
            .mode_events
            .last()
            .unwrap(),
        zwlr_output_power_v1::Mode::On,
        "client B: expected On after B sets On (A's control was destroyed)"
    );
}

#[test]
fn per_output_mode_survives_monitor_reactivate() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));

    // Explicitly turn output-1 off while output-2 stays on.
    f.set_output_power_mode(1, Mode::Off);
    assert_eq!(
        f.get_output_power_mode(1),
        Mode::Off,
        "precondition: output-1 is Off"
    );

    // Simulate the system going to sleep and waking up.
    f.deactivate_monitors();
    f.dispatch();
    f.activate_monitors();
    f.dispatch();

    // output-1 must still be Off — the user's choice must be preserved.
    assert_eq!(
        f.get_output_power_mode(1),
        Mode::Off,
        "output-1 power mode should survive monitor reactivation"
    );
}

#[test]
fn active_output_handoff_on_power_off() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));

    // output-1 is active by default (first added).
    // Power off the active output-1 while output-2 is still on.
    f.set_output_power_mode(1, Mode::Off);
    f.dispatch();

    // The active output should now be output-2 (handoff occurred),
    // not output-1 which is powered off.
    let active = f.niri().layout.active_output().cloned();
    let output1 = f.niri_output(1);
    assert_ne!(
        active.as_ref(),
        Some(&output1),
        "active output should not be the powered-off output-1 after handoff"
    );
    let output2 = f.niri_output(2);
    assert_eq!(
        active.as_ref(),
        Some(&output2),
        "active output should be output-2 after output-1 is powered off"
    );
}

/// Lock must complete when one output is powered off and only on-outputs have surfaces.
#[test]
fn lock_completes_with_one_output_powered_off() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    f.set_output_power_mode(2, Mode::Off);
    let id = f.add_client();
    f.double_roundtrip(id);

    f.client(id).start_lock();
    f.double_roundtrip(id);

    let output1 = f.client(id).output("headless-1");
    f.client(id).create_lock_surface(&output1);
    f.double_roundtrip(id);

    f.client(id).commit_lock_surfaces();
    f.double_roundtrip(id);

    for _ in 0..20 {
        f.dispatch();
    }
    f.double_roundtrip(id);

    assert!(
        f.niri().is_locked(),
        "lock should complete even when one output is powered off"
    );
}

/// Lock must complete immediately when all outputs are powered off (no surfaces needed).
#[test]
fn lock_completes_when_all_outputs_powered_off() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.set_output_power_mode(1, Mode::Off);
    let id = f.add_client();
    f.double_roundtrip(id);

    f.client(id).start_lock();
    f.double_roundtrip(id);

    for _ in 0..20 {
        f.dispatch();
    }
    f.double_roundtrip(id);

    assert!(
        f.niri().is_locked(),
        "lock should complete immediately when all outputs are powered off"
    );
}

/// Powering off an output during Locking state must not stall the lock.
/// When an output is powered off while in LockState::Locking, the compositor should
/// re-check whether all remaining on-outputs have completed rendering and complete the lock.
#[test]
fn power_off_during_lock_does_not_stall() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    let id = f.add_client();
    f.double_roundtrip(id);

    // Start lock and create surfaces for both outputs.
    f.client(id).start_lock();
    f.double_roundtrip(id);

    let output1 = f.client(id).output("headless-1");
    let output2 = f.client(id).output("headless-2");
    f.client(id).create_lock_surface(&output1);
    f.client(id).create_lock_surface(&output2);
    f.double_roundtrip(id);

    f.client(id).commit_lock_surfaces();
    f.double_roundtrip(id);

    // We are now in Locking state (renders have been queued).
    // Before the lock fully completes (before all redraws), power off output-1.
    f.set_output_power_mode(1, Mode::Off);

    // Dispatch to process the power-off and any queued redraws.
    for _ in 0..20 {
        f.dispatch();
    }
    f.double_roundtrip(id);

    assert!(
        f.niri().is_locked(),
        "lock should complete when an output is powered off during Locking state"
    );
}

/// Powered-off output must not block lock, but powered-on output must still commit a surface.
///
/// With output-2 off, the lock should only need output-1's surface. It must not transition to
/// Locked before that surface is committed and rendered, but must complete afterwards.
#[test]
fn late_lock_surface_on_powered_on_output_progresses() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    let id = f.add_client();
    f.double_roundtrip(id);

    // Power off output-2 before starting the lock.
    f.set_output_power_mode(2, Mode::Off);
    f.dispatch();

    // Start lock and create surface only for output-1 (the only powered-on output).
    f.client(id).start_lock();
    f.double_roundtrip(id);

    let output1 = f.client(id).output("headless-1");
    f.client(id).create_lock_surface(&output1);
    f.double_roundtrip(id);

    // Surface exists but has not been committed yet — lock must not be Locked.
    assert!(
        !matches!(f.niri().lock_state, LockState::Locked(_)),
        "lock must not be Locked before the on-output surface is committed"
    );

    // Commit the lock surface.
    f.client(id).commit_lock_surfaces();
    f.double_roundtrip(id);

    for _ in 0..20 {
        f.dispatch();
    }
    f.double_roundtrip(id);

    assert!(
        matches!(f.niri().lock_state, LockState::Locked(_)),
        "lock should complete after the powered-on output's surface commits and renders"
    );
}
