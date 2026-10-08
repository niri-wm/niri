use calloop::timer::{TimeoutAction, Timer};
use smithay::backend::input::{KeyState, Keycode};
use smithay::input::keyboard::FilterResult;
use smithay::utils::SERIAL_COUNTER;

use super::Fixture;
use crate::dbus::freedesktop_login1::Login1ToNiri;

#[test]
fn preparing_for_sleep_releases_physical_keys() {
    let mut fixture = Fixture::new();
    let keyboard = fixture.niri().seat.get_keyboard().unwrap();
    let keycode = Keycode::from(28u32);

    keyboard.input(
        fixture.niri_state(),
        keycode,
        KeyState::Pressed,
        SERIAL_COUNTER.next_serial(),
        0,
        |_, _, _| FilterResult::<()>::Forward,
    );
    fixture.niri().suppressed_keys.insert(keycode);
    let token = fixture
        .niri()
        .event_loop
        .insert_source(Timer::immediate(), |_, _, _| TimeoutAction::Drop)
        .unwrap();
    fixture.niri().bind_repeat_timer = Some(token);

    fixture
        .niri_state()
        .on_login1_msg(Login1ToNiri::PrepareForSleep(false));
    assert_eq!(keyboard.pressed_keys(), [keycode].into());

    fixture
        .niri_state()
        .on_login1_msg(Login1ToNiri::PrepareForSleep(true));
    assert!(keyboard.pressed_keys().is_empty());
    assert!(fixture.niri().suppressed_keys.is_empty());
    assert!(fixture.niri().bind_repeat_timer.is_none());

    // The delayed physical release after resume is absorbed.
    let _: Option<()> = keyboard.input(
        fixture.niri_state(),
        keycode,
        KeyState::Released,
        SERIAL_COUNTER.next_serial(),
        0,
        |_, _, _| panic!("duplicate release reached the input filter"),
    );
    assert!(keyboard.pressed_keys().is_empty());
}
