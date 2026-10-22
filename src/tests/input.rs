use niri_config::{Action, Bind, Key, Modifiers, Trigger};
use smithay::backend::input::Keycode;
use smithay::input::keyboard::Keysym;

use super::*;

fn repeat_bind(keysym: Keysym, action: Action) -> Bind {
    Bind {
        key: Key {
            trigger: Trigger::Keysym(keysym),
            modifiers: Modifiers::COMPOSITOR | Modifiers::SHIFT,
        },
        action,
        repeat: true,
        cooldown: None,
        allow_when_locked: false,
        allow_inhibiting: false,
        hotkey_overlay_title: None,
    }
}

#[test]
fn bind_repeats_stop_independently() {
    let mut fixture = Fixture::new();
    let state = fixture.niri_state();
    let down = Keycode::from(108_u32);
    let right = Keycode::from(106_u32);

    state.start_key_repeat(down, repeat_bind(Keysym::Down, Action::MoveWindowDown));
    state.start_key_repeat(right, repeat_bind(Keysym::Right, Action::MoveColumnRight));

    assert!(state.niri.bind_repeat_timers.contains_key(&down));
    assert!(state.niri.bind_repeat_timers.contains_key(&right));

    state.update_key_repeats(down, false, Keysym::Down);
    assert!(!state.niri.bind_repeat_timers.contains_key(&down));
    assert!(state.niri.bind_repeat_timers.contains_key(&right));

    state.update_key_repeats(right, false, Keysym::Right);
    assert!(state.niri.bind_repeat_timers.is_empty());
}

#[test]
fn modifier_release_stops_all_bind_repeats() {
    let mut fixture = Fixture::new();
    let state = fixture.niri_state();
    let down = Keycode::from(108_u32);
    let right = Keycode::from(106_u32);
    let shift = Keycode::from(42_u32);

    state.start_key_repeat(down, repeat_bind(Keysym::Down, Action::MoveWindowDown));
    state.start_key_repeat(right, repeat_bind(Keysym::Right, Action::MoveColumnRight));
    assert_eq!(state.niri.bind_repeat_timers.len(), 2);

    state.update_key_repeats(shift, false, Keysym::Shift_L);
    assert!(state.niri.bind_repeat_timers.is_empty());
}
