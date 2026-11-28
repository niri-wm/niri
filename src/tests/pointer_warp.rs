use niri_config::{Action, Config, WorkspaceReference};
use smithay::backend::input::InputTime;
use smithay::utils::Point;

use super::*;

fn set_up() -> Fixture {
    let config = r##"
output "headless-1" {
    position x=0 y=0
}
output "headless-2" {
    position x=1920 y=0
}
workspace "two" {
    open-on-output "headless-2"
}
"##;
    let config = Config::parse_mem(config).unwrap();

    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    f.niri_focus_output(1);
    f
}

fn pointer_location(f: &mut Fixture) -> Point<f64, smithay::utils::Logical> {
    f.niri().seat.get_pointer().unwrap().current_location()
}

#[test]
fn focus_workspace_on_other_output_keeps_pointer_already_there() {
    let mut f = set_up();

    // The pointer moved onto output 2 without focusing it.
    f.niri_state().move_cursor(Point::from((2000., 10.)));

    f.niri_state().do_action(
        Action::FocusWorkspace(WorkspaceReference::Name("two".into())),
        false,
        InputTime::now(),
    );

    assert_eq!(pointer_location(&mut f), Point::from((2000., 10.)));
}

#[test]
fn focus_monitor_moves_pointer_to_other_output() {
    let mut f = set_up();
    f.niri_state().move_cursor(Point::from((100., 10.)));

    f.niri_state()
        .do_action(Action::FocusMonitorRight, false, InputTime::now());

    assert_eq!(pointer_location(&mut f), Point::from((2880., 540.)));
}
