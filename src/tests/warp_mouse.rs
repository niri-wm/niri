use niri_config::{Action, Config};
use smithay::utils::{Logical, Point};

use super::Fixture;
use crate::utils::center_f64;

fn set_up() -> Fixture {
    let config =
        Config::parse_mem(r#"input { warp-mouse-to-focus mode="cross-output"; }"#).unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));
    let id = f.add_client();

    for output in [1, 1, 2] {
        f.niri_focus_output(output);
        let window = f.client(id).create_window();
        let surface = window.surface.clone();
        window.commit();
        f.roundtrip(id);
        let window = f.client(id).window(&surface);
        window.attach_new_buffer();
        window.set_size(100, 100);
        window.ack_last_and_commit();
        f.double_roundtrip(id);
    }

    f.niri_focus_output(1);
    f.niri_complete_animations();
    f.niri_state().update_keyboard_focus();
    f.niri_state().move_cursor((10., 10.).into());
    f
}

fn cursor(f: &mut Fixture) -> Point<f64, Logical> {
    f.niri().seat.get_pointer().unwrap().current_location()
}

fn focused_center(f: &mut Fixture) -> Point<f64, Logical> {
    let niri = f.niri();
    let output = niri.layout.active_output().unwrap();
    let monitor = niri.layout.monitor_for_output(output).unwrap();
    let rect = monitor.active_window_visual_rectangle().unwrap();
    center_f64(rect)
        + niri
            .global_space
            .output_geometry(output)
            .unwrap()
            .loc
            .to_f64()
}

#[test]
fn cross_output_keeps_cursor_on_same_monitor() {
    let mut f = set_up();
    let original = cursor(&mut f);
    for action in [
        Action::FocusColumnLeft,
        Action::FocusColumnRight,
        Action::FocusWorkspaceDown,
        Action::FocusWorkspaceUp,
    ] {
        f.niri_state().do_action(action, false);
        assert_eq!(cursor(&mut f), original);
    }
}

#[test]
fn cross_output_centers_on_monitor_change() {
    let mut f = set_up();
    for action in [Action::FocusMonitorRight, Action::FocusMonitorLeft] {
        f.niri_state().do_action(action, false);
        assert_eq!(cursor(&mut f), focused_center(&mut f));
    }
}

#[test]
fn cross_output_moves_to_empty_monitor() {
    let mut f = set_up();
    f.add_output(3, (1280, 720));
    let output = f.niri_output(3);
    let center = center_f64(
        f.niri()
            .global_space
            .output_geometry(&output)
            .unwrap()
            .to_f64(),
    );
    f.niri_state()
        .do_action(Action::FocusMonitor(output.name()), false);
    assert_eq!(cursor(&mut f), center);
}

#[test]
fn cross_output_does_not_recenter_current_monitor() {
    let mut f = set_up();
    let original = cursor(&mut f);
    let name = f.niri_output(1).name();
    f.niri_state().do_action(Action::FocusMonitor(name), false);
    assert_eq!(cursor(&mut f), original);
}

#[test]
fn cross_output_centers_even_when_cursor_is_in_target_window() {
    let mut f = set_up();
    f.niri_focus_output(2);
    let target = focused_center(&mut f);
    f.niri_focus_output(1);
    f.niri_state().move_cursor(target + Point::from((1., 1.)));
    f.niri_state().do_action(Action::FocusMonitorRight, false);
    assert_eq!(cursor(&mut f), target);
}

#[test]
fn cross_output_follows_window_on_another_monitor() {
    let mut f = set_up();
    let output = f.niri_output(2);
    let window = f
        .niri()
        .layout
        .monitor_for_output(&output)
        .unwrap()
        .active_window()
        .unwrap()
        .window
        .clone();
    f.niri_state().focus_window(&window);
    assert_eq!(cursor(&mut f), focused_center(&mut f));
    let offset = cursor(&mut f) + Point::from((1., 1.));
    f.niri_state().move_cursor(offset);
    f.niri_state().focus_window(&window);
    assert_eq!(cursor(&mut f), offset);
}
