use niri_config::Config;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::xdg_activation::XdgActivationHandler;

use super::client::ClientId;
use super::Fixture;
use crate::protocols::foreign_toplevel::ForeignToplevelHandler;
use crate::utils::{center, center_f64};

#[derive(Clone, Copy)]
enum Activation {
    ForeignToplevel,
    Xdg,
}

impl Activation {
    fn activate(self, f: &mut Fixture, surface: WlSurface) {
        let state = f.niri_state();
        match self {
            Self::ForeignToplevel => ForeignToplevelHandler::activate(state, surface),
            Self::Xdg => {
                let (token, data) = state.niri.activation_state.create_external_token(None);
                let (token, data) = (token.clone(), data.clone());
                XdgActivationHandler::request_activation(state, token, data, surface);
            }
        }
    }
}

fn map_window(f: &mut Fixture, id: ClientId) -> WlSurface {
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.niri_complete_animations();

    f.niri()
        .layout
        .focus()
        .unwrap()
        .toplevel()
        .wl_surface()
        .clone()
}

fn set_up(warp: bool, cross_output: bool, rule: Option<&str>) -> (Fixture, WlSurface) {
    let input = if warp {
        "input { warp-mouse-to-focus mode=\"center-xy-always\"; }"
    } else {
        ""
    };
    let rule = rule
        .map(|rule| format!("window-rule {{ on-xdg-activate \"{rule}\"; }}"))
        .unwrap_or_default();
    let config = Config::parse_mem(&format!("{input}\n{rule}")).unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));
    f.niri_focus_output(1);
    let id = f.add_client();
    let target = map_window(&mut f, id);

    if cross_output {
        f.niri_focus_output(2);
    }
    let other = map_window(&mut f, id);
    assert_ne!(target, other);
    f.niri_state().update_keyboard_focus();
    let output = f.niri().layout.active_output().unwrap().clone();
    let output_geo = f.niri().global_space.output_geometry(&output).unwrap();
    let location = output_geo.loc.to_f64() + Point::from((20., 20.));
    f.niri_state().move_cursor(location);
    (f, target)
}

fn pointer_location(f: &mut Fixture) -> Point<f64, Logical> {
    f.niri().seat.get_pointer().unwrap().current_location()
}

fn check_focus(activation: Activation, warp: bool, cross_output: bool, rule: Option<&str>) {
    let (mut f, target) = set_up(warp, cross_output, rule);
    let before = pointer_location(&mut f);
    activation.activate(&mut f, target.clone());

    let target_output = f.niri_output(1);
    let niri = f.niri();
    let (focused, output) = niri.layout.focus_with_output().unwrap();
    assert_eq!(focused.toplevel().wl_surface(), &target);
    assert_eq!(output, &target_output);
    let output_geo = niri.global_space.output_geometry(output).unwrap();
    let expected = if warp {
        let monitor = niri.layout.monitor_for_output(output).unwrap();
        let rect = monitor.active_window_visual_rectangle().unwrap();
        center_f64(rect) + output_geo.loc.to_f64()
    } else if cross_output {
        center(output_geo).to_f64()
    } else {
        before
    };
    if warp || cross_output {
        assert_ne!(before, expected);
    }
    assert_eq!(pointer_location(&mut f), expected);
}

#[test]
fn foreign_toplevel_activation_warps_to_window_center() {
    for cross_output in [false, true] {
        check_focus(Activation::ForeignToplevel, true, cross_output, None);
    }
}

#[test]
fn foreign_toplevel_activation_without_warp() {
    for cross_output in [false, true] {
        check_focus(Activation::ForeignToplevel, false, cross_output, None);
    }
}

#[test]
fn xdg_activation_default_warps_to_window_center() {
    for cross_output in [false, true] {
        check_focus(Activation::Xdg, true, cross_output, None);
    }
}

#[test]
fn xdg_activation_explicit_focus_warps_to_window_center() {
    for cross_output in [false, true] {
        check_focus(Activation::Xdg, true, cross_output, Some("focus"));
    }
}

#[test]
fn xdg_activation_without_warp() {
    for rule in [None, Some("focus")] {
        for cross_output in [false, true] {
            check_focus(Activation::Xdg, false, cross_output, rule);
        }
    }
}

#[test]
fn xdg_activation_ignore_and_set_urgent_preserve_focus_and_pointer() {
    for rule in ["ignore", "set-urgent"] {
        for cross_output in [false, true] {
            let (mut f, target) = set_up(true, cross_output, Some(rule));
            let before = pointer_location(&mut f);
            let focused = f.niri().layout.focus().unwrap().window.clone();
            Activation::Xdg.activate(&mut f, target.clone());

            assert_eq!(f.niri().layout.focus().unwrap().window, focused);
            assert_eq!(pointer_location(&mut f), before);
            let (mapped, _) = f.niri().layout.find_window_and_output(&target).unwrap();
            assert_eq!(mapped.is_urgent(), rule == "set-urgent");
        }
    }
}
