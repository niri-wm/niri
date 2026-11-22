use std::time::{Duration, Instant};

use niri_config::{Action, Config};
use niri_ipc::DwellClickMode;
use smithay::backend::input::InputTime;
use wayland_client::protocol::wl_pointer;

use super::client::{ClientId, PointerEvent};
use super::*;
use crate::input::dwell::{BTN_LEFT, BTN_RIGHT};

fn setup_dwell_fixture(
    config_str: &str,
) -> (
    Fixture,
    client::ClientId,
    wayland_client::protocol::wl_surface::WlSurface,
) {
    let config = Config::parse_mem(config_str).unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1280, 720));

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(600, 600);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.niri_complete_animations();
    f.roundtrip(id);

    (f, id, surface)
}

fn button_events(
    f: &mut Fixture,
    id: ClientId,
    surface: &wayland_client::protocol::wl_surface::WlSurface,
) -> Vec<(u32, wl_pointer::ButtonState, u32)> {
    f.client(id)
        .state
        .recent_pointer_events(surface)
        .filter_map(|e| match *e {
            PointerEvent::Button {
                button,
                state,
                time,
            } => Some((button, state, time)),
            _ => None,
        })
        .collect()
}

fn wait_for_buttons(
    f: &mut Fixture,
    id: ClientId,
    surface: &wayland_client::protocol::wl_surface::WlSurface,
    expected: usize,
) -> Vec<(u32, wl_pointer::ButtonState, u32)> {
    let mut collected = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline && collected.len() < expected {
        let remaining = deadline.saturating_duration_since(Instant::now());
        f.event_loop
            .dispatch(Some(remaining), &mut f.state)
            .unwrap();
        f.roundtrip(id);
        collected.extend(button_events(f, id, surface));
    }
    assert_eq!(collected.len(), expected);
    collected
}

fn assert_no_buttons_for(
    f: &mut Fixture,
    id: ClientId,
    surface: &wayland_client::protocol::wl_surface::WlSurface,
    duration: Duration,
) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        f.event_loop
            .dispatch(Some(remaining), &mut f.state)
            .unwrap();
        f.roundtrip(id);
        assert!(button_events(f, id, surface).is_empty());
    }
}

fn btn_pairs(
    events: &[(u32, wl_pointer::ButtonState, u32)],
) -> Vec<(u32, wl_pointer::ButtonState)> {
    events.iter().map(|&(b, s, _)| (b, s)).collect()
}

#[test]
fn virtual_pointer_dwell_triggers_wayland_click_without_looping() {
    let (mut f, id, surface) = setup_dwell_fixture(
        r#"
        input {
            dwell-click {
                on
                delay-ms 0
                dwell-ms 25
                motion-threshold 5
            }
        }
        "#,
    );

    let vp = f.client(id).state.virtual_pointer();

    // First absolute motion sets anchor without starting dwell.
    vp.motion_absolute(0, 200, 200, 1280, 720);
    vp.frame();
    f.roundtrip(id);
    assert!(button_events(&mut f, id, &surface).is_empty());

    // Move beyond threshold (5px) -> dwell timer starts and fires left click.
    vp.motion_absolute(1, 220, 200, 1280, 720);
    vp.frame();
    f.roundtrip(id);

    let buttons = wait_for_buttons(&mut f, id, &surface, 2);
    assert_eq!(
        btn_pairs(&buttons),
        [
            (BTN_LEFT, wl_pointer::ButtonState::Pressed),
            (BTN_LEFT, wl_pointer::ButtonState::Released),
        ]
    );

    // Staying at the same spot (or jittering inside threshold) does NOT re-click,
    // even after ClickFeedback (100ms) finishes.
    vp.motion_absolute(2, 222, 201, 1280, 720);
    vp.frame();
    f.roundtrip(id);
    assert_no_buttons_for(&mut f, id, &surface, Duration::from_millis(160));
}

#[test]
fn right_click_reverts_and_drag_cancels_cleanly() {
    let (mut f, id, surface) = setup_dwell_fixture(
        r#"
        input {
            dwell-click {
                on
                delay-ms 0
                dwell-ms 20
                motion-threshold 5
            }
        }
        "#,
    );

    let vp = f.client(id).state.virtual_pointer();

    // Switch to one-shot Right click mode.
    f.niri_state().do_action(
        Action::SetDwellClickMode(DwellClickMode::Right),
        true,
        InputTime::now(),
    );
    assert_eq!(f.niri().dwell_click.state().mode, DwellClickMode::Right);

    vp.motion_absolute(0, 200, 200, 1280, 720);
    vp.frame();
    f.roundtrip(id);
    vp.motion_absolute(1, 220, 200, 1280, 720);
    vp.frame();
    f.roundtrip(id);

    let buttons = wait_for_buttons(&mut f, id, &surface, 2);
    assert_eq!(
        btn_pairs(&buttons),
        [
            (BTN_RIGHT, wl_pointer::ButtonState::Pressed),
            (BTN_RIGHT, wl_pointer::ButtonState::Released),
        ]
    );
    assert_eq!(f.niri().dwell_click.state().mode, DwellClickMode::Left);

    // Switch to Drag mode, start drag, then cancel via Action::CancelDwellDrag.
    f.niri_state().do_action(
        Action::SetDwellClickMode(DwellClickMode::Drag),
        true,
        InputTime::now(),
    );
    vp.motion_absolute(2, 240, 200, 1280, 720);
    vp.frame();
    f.roundtrip(id);
    vp.motion_absolute(3, 260, 200, 1280, 720);
    vp.frame();
    f.roundtrip(id);

    let buttons = wait_for_buttons(&mut f, id, &surface, 1);
    assert_eq!(
        btn_pairs(&buttons),
        [(BTN_LEFT, wl_pointer::ButtonState::Pressed)]
    );
    assert!(f.niri().dwell_click.state().dragging);

    f.niri_state()
        .do_action(Action::CancelDwellDrag, true, InputTime::now());
    f.roundtrip(id);

    let buttons = button_events(&mut f, id, &surface);
    assert_eq!(
        btn_pairs(&buttons),
        [(BTN_LEFT, wl_pointer::ButtonState::Released)]
    );
    assert!(!f.niri().dwell_click.state().dragging);
    assert_eq!(f.niri().dwell_click.state().mode, DwellClickMode::Left);
}

#[test]
fn double_click_uses_increasing_timestamps() {
    let (mut f, id, surface) = setup_dwell_fixture(
        r#"
        input {
            dwell-click {
                on
                delay-ms 0
                dwell-ms 20
                motion-threshold 5
                default-mode "double"
            }
        }
        "#,
    );

    let vp = f.client(id).state.virtual_pointer();
    vp.motion_absolute(0, 200, 200, 1280, 720);
    vp.frame();
    f.roundtrip(id);
    vp.motion_absolute(1, 220, 200, 1280, 720);
    vp.frame();
    f.roundtrip(id);

    let buttons = wait_for_buttons(&mut f, id, &surface, 4);
    assert_eq!(
        btn_pairs(&buttons),
        [
            (BTN_LEFT, wl_pointer::ButtonState::Pressed),
            (BTN_LEFT, wl_pointer::ButtonState::Released),
            (BTN_LEFT, wl_pointer::ButtonState::Pressed),
            (BTN_LEFT, wl_pointer::ButtonState::Released),
        ]
    );
    assert!(buttons[2].2 > buttons[0].2);
}
