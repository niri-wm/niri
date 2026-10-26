use std::fmt::Write as _;

use insta::assert_snapshot;
use niri_config::{Action, BoundAction, Config};
use smithay::backend::input::{InputEvent, InputTime, KeyState, Keycode};
use smithay::input::keyboard::xkb::Keymap;
use wayland_client::protocol::wl_pointer;
use wayland_client::protocol::wl_surface::WlSurface;

use crate::tests::client::ClientId;
use crate::tests::fixture::Fixture;
use crate::tests::test_input_backend::{TestInputBackend, TestKeyboardKeyEvent};

#[derive(Clone, Copy)]
enum Op {
    Press(Keycode),
    Release(Keycode),
    MouseButton {
        name: &'static str,
        code: u32,
        state: wl_pointer::ButtonState,
    },
    /// A scroll tick. Scrolls are momentary events, so they have no press or release.
    Scroll {
        name: &'static str,
        source: wl_pointer::AxisSource,
        /// Positive scrolls down.
        amount: f64,
    },
}

/// Mouse buttons that can be pressed and released, named after the binds that trigger them.
const MOUSE_BUTTONS: &[(&str, u32)] = &[
    ("MouseLeft", 0x110),
    ("MouseRight", 0x111),
    ("MouseMiddle", 0x112),
    ("MouseBack", 0x113),
    ("MouseForward", 0x114),
];

/// Scrolls, named after the binds that trigger them.
const SCROLLS: &[(&str, wl_pointer::AxisSource, f64)] = &[
    ("WheelScrollDown", wl_pointer::AxisSource::Wheel, 10.),
    ("WheelScrollUp", wl_pointer::AxisSource::Wheel, -10.),
];

fn parse(keymap: &Keymap, input: &str) -> Vec<Op> {
    let mut ops = Vec::new();
    for part in input.split_ascii_whitespace() {
        // Scrolls are momentary, so they are written without a prefix.
        let (pressed, name) = match part.as_bytes().first() {
            Some(b'+') => (Some(true), &part[1..]),
            Some(b'-') => (Some(false), &part[1..]),
            _ => (None, part),
        };

        if let Some((name, source, amount)) = SCROLLS.iter().copied().find(|(n, _, _)| *n == name) {
            assert!(
                pressed.is_none(),
                "scrolls have no press or release: {part}"
            );
            ops.push(Op::Scroll {
                name,
                source,
                amount,
            });
            continue;
        }

        if let Some((name, code)) = MOUSE_BUTTONS.iter().copied().find(|(n, _)| *n == name) {
            let state = match pressed {
                Some(true) => wl_pointer::ButtonState::Pressed,
                Some(false) => wl_pointer::ButtonState::Released,
                None => panic!("a mouse button needs a + or - prefix: {part}"),
            };
            ops.push(Op::MouseButton { name, code, state });
            continue;
        }

        let Some(key) = keymap.key_by_name(name) else {
            panic!("unknown key {name}");
        };

        let op = match pressed {
            Some(true) => Op::Press(key),
            Some(false) => Op::Release(key),
            None => panic!("a key needs a + or - prefix: {part}"),
        };

        ops.push(op);
    }
    ops
}

fn set_up(config: &str) -> (Fixture, ClientId, WlSurface) {
    let mut config = Config::parse_mem(config).unwrap();

    for bind in &mut config.binds.0 {
        let actions = match &mut bind.action {
            BoundAction::Press(press) => [Some(press), None],
            BoundAction::Release(release) => [None, Some(release)],
            BoundAction::Both { press, release } => [Some(press), Some(release)],
        };
        for action in actions.into_iter().flatten() {
            // knuffel doesn't understand #[cfg(test)]...
            if !matches!(action, Action::Screenshot(_, _)) {
                *action = Action::TestAction;
            }
        }
    }

    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.roundtrip(id);

    let _ = f.client(id).state.recent_keyboard_events(&surface);

    (f, id, surface)
}

fn run_f(f: &mut Fixture, id: ClientId, surface: &WlSurface, input: &str) -> String {
    let state = f.niri_state();
    let keyboard = state.niri.seat.get_keyboard().unwrap();
    let ops = keyboard.with_xkb_state(state, |xkb| {
        let xkb = xkb.xkb().lock().unwrap();
        let keymap = unsafe { xkb.keymap() };
        parse(keymap, input)
    });

    let mut rv = String::new();
    for event in f.client(id).state.recent_keyboard_events(surface) {
        let _ = writeln!(&mut rv, "    surface {event}");
    }

    for op in ops {
        let prev = f.niri().test_action_count;

        match op {
            Op::Press(code) | Op::Release(code) => {
                let key_state = match op {
                    Op::Press(_) => KeyState::Pressed,
                    Op::Release(_) => KeyState::Released,
                    _ => unreachable!(),
                };

                let state = f.niri_state();
                let keyboard = state.niri.seat.get_keyboard().unwrap();
                keyboard.with_xkb_state(state, |xkb| {
                    let xkb = xkb.xkb().lock().unwrap();
                    let xkb_state = unsafe { xkb.state() };
                    let keymap = xkb_state.get_keymap();

                    let c = match key_state {
                        KeyState::Pressed => "+",
                        KeyState::Released => "-",
                    };

                    let name = keymap.key_get_name(code).unwrap_or("None");
                    let keysym = xkb_state.key_get_one_sym(code);

                    let _ = writeln!(&mut rv, "{c}{name} {:>3} {keysym:?}", code.raw());
                });

                let state = f.niri_state();
                state.process_input_event(InputEvent::<TestInputBackend>::Keyboard {
                    event: TestKeyboardKeyEvent {
                        time: InputTime::from_micros(0),
                        code,
                        state: key_state,
                        count: 1, // niri doesn't use this
                    },
                });
            }
            Op::MouseButton {
                name,
                code,
                state: button_state,
            } => {
                let c = match button_state {
                    wl_pointer::ButtonState::Pressed => "+",
                    wl_pointer::ButtonState::Released => "-",
                    _ => unreachable!(),
                };
                let _ = writeln!(&mut rv, "{c}{name}");

                let pointer = f.client(id).state.virtual_pointer();
                pointer.button(0, code, button_state);
                pointer.frame();
            }
            Op::Scroll {
                name,
                source,
                amount,
            } => {
                let _ = writeln!(&mut rv, "{name}");

                let pointer = f.client(id).state.virtual_pointer();
                pointer.axis_source(source);
                pointer.axis(0, wl_pointer::Axis::VerticalScroll, amount);
                pointer.frame();
            }
        }

        f.roundtrip(id);

        let diff = f.niri().test_action_count - prev;
        for _ in 0..diff {
            let _ = writeln!(&mut rv, "    niri test-action");
        }

        for event in f.client(id).state.recent_keyboard_events(surface) {
            let _ = writeln!(&mut rv, "    surface {event}");
        }
    }

    rv
}

fn run(config: &str, input: &str) -> String {
    let (mut f, id, surface) = set_up(config);
    run_f(&mut f, id, &surface, input)
}

#[test]
fn combos() {
    let c = "
    binds {
        Mod+Ctrl+Q { close-window; }
        Mod+Ctrl+W { close-window; }
    }
    ";

    // Action press/release.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatQ -LatQ -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    +AD01  24 XK_q
        niri test-action
    -AD01  24 XK_q
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    "
    );

    // Two actions interleaved.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatQ +LatW -LatQ -LatW -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    +AD01  24 XK_q
        niri test-action
    +AD02  25 XK_w
        niri test-action
    -AD01  24 XK_q
    -AD02  25 XK_w
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    "
    );

    // Extra Alt = no action.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LALT +LatQ -LALT -LatQ -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=76, latched=0, locked=0, group=0
    +AD01  24 XK_q
        surface key pressed: 16
    -LALT  64 XK_Alt_L
        surface key released: 56
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    -AD01  24 XK_q
        surface key released: 16
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    "
    );

    // Key that doesn't correspond to any bind.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatA -LatA -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    "
    );

    // Press action, press arbitrary, release action, release arbitrary.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatQ +LatA -LatQ -LatA -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    +AD01  24 XK_q
        niri test-action
    +AC01  38 XK_a
        surface key pressed: 30
    -AD01  24 XK_q
    -AC01  38 XK_a
        surface key released: 30
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    "
    );

    // Press arbitrary, press action, release arbitrary, release action.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatA +LatQ -LatA -LatQ -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    +AC01  38 XK_a
        surface key pressed: 30
    +AD01  24 XK_q
        niri test-action
    -AC01  38 XK_a
        surface key released: 30
    -AD01  24 XK_q
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    "
    );

    // Trigger action then release mods.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatQ -LCTL -LWIN -LatQ"),
        @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    +AD01  24 XK_q
        niri test-action
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    -AD01  24 XK_q
    "
    );

    // Modifiers after trigger key don't trigger the action.
    assert_snapshot!(
        run(c, "+LWIN +LatQ +LCTL -LCTL -LatQ -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
        surface key pressed: 16
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -AD01  24 XK_q
        surface key released: 16
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    "
    );
}

#[test]
fn inhibiting() {
    let config = "
    binds {
        Q { close-window; }
        U allow-inhibiting=false { close-window; }
    }
    ";

    let (mut f, id, surface) = set_up(config);

    let inhibitor = f.client(id).state.inhibit_shortcuts(&surface);
    f.roundtrip(id);

    // While inhibiting, we don't intercept the shortcut.
    assert_snapshot!(
        run_f(&mut f, id, &surface, "+LatQ -LatQ"),
        @"
    +AD01  24 XK_q
        surface key pressed: 16
    -AD01  24 XK_q
        surface key released: 16
    "
    );

    // allow-inhibiting=false still triggers.
    assert_snapshot!(
        run_f(&mut f, id, &surface, "+LatU -LatU"),
        @"
    +AD07  30 XK_u
        niri test-action
    -AD07  30 XK_u
    "
    );

    // Toggle it off after pressing the shortcut.
    assert_snapshot!(
        run_f(&mut f, id, &surface, "+LatQ"),
        @"
    +AD01  24 XK_q
        surface key pressed: 16
    "
    );

    inhibitor.destroy();
    f.roundtrip(id);

    // The surface must get key release since it got the key press.
    assert_snapshot!(
        run_f(&mut f, id, &surface, "-LatQ"),
        @"
    -AD01  24 XK_q
        surface key released: 16
    "
    );

    // Toggle it on after pressing the shortcut.
    assert_snapshot!(
        run_f(&mut f, id, &surface, "+LatQ"),
        @"
    +AD01  24 XK_q
        niri test-action
    "
    );

    let _inhibitor = f.client(id).state.inhibit_shortcuts(&surface);
    f.roundtrip(id);

    // The surface must not get key release since there was no key press.
    assert_snapshot!(
        run_f(&mut f, id, &surface, "-LatQ"),
        @"-AD01  24 XK_q"
    );
}

#[test]
fn layouts() {
    let c = r#"
    input {
        keyboard {
            xkb {
                layout "us,ru"
                options "grp:lalt_toggle"
            }
        }
    }

    binds {
        Q { close-window; }
        Shift+Slash { close-window; }
    }
    "#;

    // On a cyrillic layout (ru), an ascii bind is searched in the ascii layout (us).
    assert_snapshot!(
        run(c, "+LALT -LALT +LatQ -LatQ"),
        @"
    +LALT  64 XK_ISO_Next_Group
        surface key pressed: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=1
    -LALT  64 XK_ISO_Next_Group
        surface key released: 56
    +AD01  24 XK_Cyrillic_shorti
        niri test-action
    -AD01  24 XK_Cyrillic_shorti
    "
    );

    // The slash key has . , in ru, and pressing those shouldn't search another layout.
    assert_snapshot!(
        run(
            c,
            "
            +LFSH +AB10 -AB10 -LFSH \
            +LALT -LALT \
            +LFSH +AB10 -AB10 -LFSH
            "
        ),
        @"
    +LFSH  50 XK_Shift_L
        surface key pressed: 42
        surface modifiers: depressed=1, latched=0, locked=0, group=0
    +AB10  61 XK_question
        niri test-action
    -AB10  61 XK_question
    -LFSH  50 XK_Shift_L
        surface key released: 42
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    +LALT  64 XK_ISO_Next_Group
        surface key pressed: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=1
    -LALT  64 XK_ISO_Next_Group
        surface key released: 56
    +LFSH  50 XK_Shift_L
        surface key pressed: 42
        surface modifiers: depressed=1, latched=0, locked=0, group=1
    +AB10  61 XK_comma
        surface key pressed: 53
    -AB10  61 XK_comma
        surface key released: 53
    -LFSH  50 XK_Shift_L
        surface key released: 42
        surface modifiers: depressed=0, latched=0, locked=0, group=1
    "
    );

    // In ru, / is on the \ / key (so, Shift + \). So, arguably, it would make sense for Shift + /
    // to trigger it, but it currently doesn't (niri requires an unshifted trigger key, despite
    // working fine with capital case alphabetic keys).
    assert_snapshot!(
        run(c, "+LALT -LALT +LFSH +BKSL -BKSL -LFSH"),
        @"
    +LALT  64 XK_ISO_Next_Group
        surface key pressed: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=1
    -LALT  64 XK_ISO_Next_Group
        surface key released: 56
    +LFSH  50 XK_Shift_L
        surface key pressed: 42
        surface modifiers: depressed=1, latched=0, locked=0, group=1
    +BKSL  51 XK_slash
        surface key pressed: 43
    -BKSL  51 XK_slash
        surface key released: 43
    -LFSH  50 XK_Shift_L
        surface key released: 42
        surface modifiers: depressed=0, latched=0, locked=0, group=1
    "
    );
}

#[test]
fn release_binds() {
    let c = "
    binds {
        Mod {
            release { toggle-overview; }
        }
        Mod+Q {
            release { close-window; }
        }
        Mod+U { center-column; }
        Ctrl+Alt_L {
            release { switch-layout \"next\"; }
        }
        Ctrl+Alt+U { close-window; }
    }
    ";

    // Releasing Mod by itself runs its release action.
    assert_snapshot!(run(c, "+LWIN -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        niri test-action
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Pressing another key in between cancels a modifier-only release bind.
    assert_snapshot!(run(c, "+LWIN +LatA -LatA -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Pressing a key that triggers its own bind cancels the pending release bind.
    assert_snapshot!(run(c, "+LWIN +LatU -LatU -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD07  30 XK_u
        niri test-action
    -AD07  30 XK_u
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // A key that was already held when the bind was pressed doesn't cancel it: only input in
    // between does.
    assert_snapshot!(run(c, "+LatA +LWIN -LatA -LWIN"), @"
    +AC01  38 XK_a
        surface key pressed: 30
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -AC01  38 XK_a
        surface key released: 30
    -LWIN 133 XK_Super_L
        niri test-action
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // A release-only bind on a regular key.
    assert_snapshot!(run(c, "+LWIN +LatQ -LatQ -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
    -AD01  24 XK_q
        niri test-action
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // The release action runs even if the modifiers are released before the trigger key.
    assert_snapshot!(run(c, "+LWIN +LatQ -LWIN -LatQ"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    -AD01  24 XK_q
        niri test-action
    ");

    // Another key in between cancels it, no matter the trigger.
    assert_snapshot!(run(c, "+LWIN +LatQ +LatA -LatA -LWIN -LatQ"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    -AD01  24 XK_q
    ");

    // Ctrl+Alt_L is a modifier-only bind: its whole key consists of modifiers, so releasing the
    // combo on its own runs its release action.
    assert_snapshot!(run(c, "+LCTL +LALT -LALT -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=12, latched=0, locked=0, group=0
    -LALT  64 XK_Alt_L
        niri test-action
        surface key released: 56
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Pressing another key in between cancels it, so using Ctrl+Alt+A as an app shortcut does not
    // additionally switch the layout.
    assert_snapshot!(run(c, "+LCTL +LALT +LatA -LatA -LALT -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=12, latched=0, locked=0, group=0
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -LALT  64 XK_Alt_L
        surface key released: 56
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Pressing a key that triggers its own bind cancels the Ctrl+Alt_L release bind.
    assert_snapshot!(run(c, "+LCTL +LALT +LatU -LatU -LALT -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=12, latched=0, locked=0, group=0
    +AD07  30 XK_u
        niri test-action
    -AD07  30 XK_u
    -LALT  64 XK_Alt_L
        surface key released: 56
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn press_and_release_binds() {
    let c = "
    binds {
        Mod+Q {
            press { close-window; }
            release { center-column; }
        }
    }
    ";

    // Both actions run, and the key never reaches the surface.
    assert_snapshot!(run(c, "+LWIN +LatQ -LatQ -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
        niri test-action
    -AD01  24 XK_q
        niri test-action
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // The release action runs even if the modifiers are released first.
    assert_snapshot!(run(c, "+LWIN +LatQ -LWIN -LatQ"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
        niri test-action
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    -AD01  24 XK_q
        niri test-action
    ");

    // Extra modifiers held at release don't stop the release action either.
    assert_snapshot!(run(c, "+LWIN +LatQ +LCTL +LFSH -LatQ -LFSH -LCTL -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
        niri test-action
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    +LFSH  50 XK_Shift_L
        surface key pressed: 42
        surface modifiers: depressed=69, latched=0, locked=0, group=0
    -AD01  24 XK_Q
        niri test-action
    -LFSH  50 XK_Shift_L
        surface key released: 42
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Push-to-talk: other input in between doesn't cancel the release action.
    assert_snapshot!(run(c, "+LWIN +LatQ +LatA -LatA -LatQ -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
        niri test-action
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -AD01  24 XK_q
        niri test-action
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Pressing the key without the mod key held doesn't run either action.
    assert_snapshot!(run(c, "+LatQ +LWIN -LatQ -LWIN"), @"
    +AD01  24 XK_q
        surface key pressed: 16
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -AD01  24 XK_q
        surface key released: 16
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn generic_modifier_press_release() {
    let c = "
    binds {
        Super+Shift {
            press { close-window; }
            release { center-column; }
        }

        Ctrl+Shift {
            release { close-window; }
        }
    }
    ";

    // Press Super and then two Shifts.
    assert_snapshot!(run(c, "+LWIN +LFSH +RTSH -RTSH -LFSH -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LFSH  50 XK_Shift_L
        niri test-action
        surface modifiers: depressed=65, latched=0, locked=0, group=0
    +RTSH  62 XK_Shift_R
        niri test-action
    -RTSH  62 XK_Shift_R
        niri test-action
    -LFSH  50 XK_Shift_L
        niri test-action
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Other release order.
    assert_snapshot!(run(c, "+LWIN +LFSH +RTSH -LFSH -RTSH -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LFSH  50 XK_Shift_L
        niri test-action
        surface modifiers: depressed=65, latched=0, locked=0, group=0
    +RTSH  62 XK_Shift_R
        niri test-action
    -LFSH  50 XK_Shift_L
        niri test-action
    -RTSH  62 XK_Shift_R
        niri test-action
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Same but release-only.
    assert_snapshot!(run(c, "+LCTL +LFSH +RTSH -RTSH -LFSH -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    +LFSH  50 XK_Shift_L
        surface key pressed: 42
        surface modifiers: depressed=5, latched=0, locked=0, group=0
    +RTSH  62 XK_Shift_R
        surface key pressed: 54
    -RTSH  62 XK_Shift_R
        niri test-action
        surface key released: 54
    -LFSH  50 XK_Shift_L
        surface key released: 42
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Other release order (release-only).
    assert_snapshot!(run(c, "+LCTL +LFSH +RTSH -LFSH -RTSH -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    +LFSH  50 XK_Shift_L
        surface key pressed: 42
        surface modifiers: depressed=5, latched=0, locked=0, group=0
    +RTSH  62 XK_Shift_R
        surface key pressed: 54
    -LFSH  50 XK_Shift_L
        surface key released: 42
    -RTSH  62 XK_Shift_R
        niri test-action
        surface key released: 54
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Now, Shift first.
    assert_snapshot!(run(c, "+LFSH +LWIN -LWIN -LFSH"), @"
    +LFSH  50 XK_Shift_L
        surface key pressed: 42
        surface modifiers: depressed=1, latched=0, locked=0, group=0
    +LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=65, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=1, latched=0, locked=0, group=0
    -LFSH  50 XK_Shift_L
        surface key released: 42
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Two Supers.
    assert_snapshot!(run(c, "+LFSH +LWIN +RWIN -RWIN -LWIN -LFSH"), @"
    +LFSH  50 XK_Shift_L
        surface key pressed: 42
        surface modifiers: depressed=1, latched=0, locked=0, group=0
    +LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=65, latched=0, locked=0, group=0
    +RWIN 134 XK_Super_R
        niri test-action
    -RWIN 134 XK_Super_R
        niri test-action
    -LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=1, latched=0, locked=0, group=0
    -LFSH  50 XK_Shift_L
        surface key released: 42
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Unrelated modifier in the middle.
    assert_snapshot!(run(c, "+LWIN +LCTL +LFSH -LCTL -LFSH -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    +LFSH  50 XK_Shift_L
        surface key pressed: 42
        surface modifiers: depressed=69, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=65, latched=0, locked=0, group=0
    -LFSH  50 XK_Shift_L
        surface key released: 42
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Regressions.
    let c = "
    binds {
        Shift+Mod { close-window; }
        Ctrl+Super { close-window; }
    }
    ";

    assert_snapshot!(run(c, "+LFSH -LFSH"), @"
    +LFSH  50 XK_Shift_L
        surface key pressed: 42
        surface modifiers: depressed=1, latched=0, locked=0, group=0
    -LFSH  50 XK_Shift_L
        surface key released: 42
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    assert_snapshot!(run(c, "+LFSH +LWIN -LWIN -LFSH"), @"
    +LFSH  50 XK_Shift_L
        surface key pressed: 42
        surface modifiers: depressed=1, latched=0, locked=0, group=0
    +LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=65, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=1, latched=0, locked=0, group=0
    -LFSH  50 XK_Shift_L
        surface key released: 42
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    assert_snapshot!(run(c, "+LCTL -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    assert_snapshot!(run(c, "+LCTL +LWIN -LWIN -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    +LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn mod_key_press_and_release_bind() {
    let c = "
    binds {
        Mod {
            press { close-window; }
            release { center-column; }
        }
    }
    ";

    // Both actions run, and the window doesn't see the key press or release themselves, only the
    // modifier state.
    assert_snapshot!(run(c, "+LWIN -LWIN"), @"
    +LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Other input in between doesn't cancel the release action.
    assert_snapshot!(run(c, "+LWIN +LatA -LatA -LWIN"), @"
    +LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn mod_key_press_and_release_bind_focus_change() {
    let c = "
    binds {
        Mod {
            press { close-window; }
            release { center-column; }
        }
    }
    ";

    let (mut f, id, surface) = set_up(c);

    // Hold mod: the press action runs, and the window sees the modifier state.
    run_f(&mut f, id, &surface, "+LWIN");

    // A new window takes the keyboard focus while mod is held.
    let window = f.client(id).create_window();
    let surface2 = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface2);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.roundtrip(id);

    // The new window doesn't see the key press or release, but the focus change and the
    // intercepted release still update its modifier state, so it doesn't get stuck with mod held.
    assert_snapshot!(run_f(&mut f, id, &surface2, "-LWIN +LatA -LatA"), @"
        surface enter: []
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    ");
}

#[test]
fn modifier_key_trigger_binds() {
    let c = "
    binds {
        Mod+Control_L {
            press { close-window; }
            release { center-column; }
        }
        Alt+Control_L { close-window; }
    }
    ";

    // A modifier key used as the trigger is intercepted, so the window doesn't see the key press
    // and release, only the modifier state; both actions run.
    assert_snapshot!(run(c, "+LWIN +LCTL -LCTL -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        niri test-action
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        niri test-action
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Without the mod key held the bind doesn't match.
    assert_snapshot!(run(c, "+LCTL -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // With Alt held instead, the Control_L press is intercepted, and only the press action runs.
    assert_snapshot!(run(c, "+LALT +LCTL -LCTL -LALT"), @"
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        niri test-action
        surface modifiers: depressed=12, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    -LALT  64 XK_Alt_L
        surface key released: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn release_bind_on_modifier_key() {
    let c = "
    binds {
        Mod+Control_L {
            release { close-window; }
        }
    }
    ";

    // The press is forwarded so that apps see the modifier, and the release action runs when the
    // modifier is released on its own.
    assert_snapshot!(run(c, "+LWIN +LCTL -LCTL -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        niri test-action
        surface key released: 29
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Pressing another key in between cancels the release action.
    assert_snapshot!(run(c, "+LWIN +LCTL +LFSH -LCTL -LFSH -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    +LFSH  50 XK_Shift_L
        surface key pressed: 42
        surface modifiers: depressed=69, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=65, latched=0, locked=0, group=0
    -LFSH  50 XK_Shift_L
        surface key released: 42
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn modifier_key_and_keysym_triggers() {
    // `Ctrl` is not the mod key, so it is matched by keysym directly: both Control_L and
    // Control_R trigger it.
    let by_modifier = "
    binds {
        Ctrl {
            release { close-window; }
        }
    }
    ";

    assert_snapshot!(run(by_modifier, "+LCTL -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        niri test-action
        surface key released: 29
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    assert_snapshot!(run(by_modifier, "+RCTL -RCTL"), @"
    +RCTL 105 XK_Control_R
        surface key pressed: 97
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -RCTL 105 XK_Control_R
        niri test-action
        surface key released: 97
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // `Control_L` is a specific key instead, so it doesn't trigger on Control_R, which reaches
    // the window instead.
    let by_keysym = "
    binds {
        Control_L {
            release { close-window; }
        }
    }
    ";

    assert_snapshot!(run(by_keysym, "+LCTL -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        niri test-action
        surface key released: 29
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    assert_snapshot!(run(by_keysym, "+RCTL -RCTL"), @"
    +RCTL 105 XK_Control_R
        surface key pressed: 97
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -RCTL 105 XK_Control_R
        surface key released: 97
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn mod_key_trigger_with_own_modifier() {
    // The trigger is the configured mod key, and has Mod at a modifier.
    let bind_by_mod = "
    binds {
        Mod+Super_L { close-window; }
    }
    ";

    assert_snapshot!(run(bind_by_mod, "+LWIN -LWIN"), @"
    +LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    let bind_by_keysym = "
    binds {
        Super_L { close-window; }
    }
    ";

    assert_snapshot!(run(bind_by_keysym, "+LWIN -LWIN"), @"
    +LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    let bind_by_generic = "
    binds {
        Mod+Super { close-window; }
    }
    ";

    assert_snapshot!(run(bind_by_generic, "+LWIN -LWIN"), @"
    +LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn release_bind_requires_matching_modifiers_on_press() {
    let c = "
    binds {
        Mod+Ctrl+Q {
            release { close-window; }
        }
    }
    ";

    // Pressing Q without the mod key held forwards it and doesn't arm the release action, even if
    // the mod key is held by the time Q is released.
    assert_snapshot!(run(c, "+LCTL +LatQ +LWIN -LatQ -LWIN -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface key pressed: 29
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    +AD01  24 XK_q
        surface key pressed: 16
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    -AD01  24 XK_q
        surface key released: 16
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

/// `Mod` and `Mod+Q` release-only binds, used by the inhibitor tests below.
const INHIBITING_BINDS: &str = "
    binds {
        Mod {
            release { toggle-overview; }
        }
        Mod+Q {
            release { close-window; }
        }
    }
";

#[test]
fn inhibiting_release_binds() {
    let (mut f, id, surface) = set_up(INHIBITING_BINDS);
    let _inhibitor = f.client(id).state.inhibit_shortcuts(&surface);
    f.roundtrip(id);

    // While inhibiting, release binds don't run, and the whole key is forwarded to the window.
    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN +LatQ -LatQ -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
        surface key pressed: 16
    -AD01  24 XK_q
        surface key released: 16
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn release_binds_run_after_the_inhibitor_is_gone() {
    let (mut f, id, surface) = set_up(INHIBITING_BINDS);
    let inhibitor = f.client(id).state.inhibit_shortcuts(&surface);
    f.roundtrip(id);
    inhibitor.destroy();
    f.roundtrip(id);

    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN +LatQ -LatQ -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
    -AD01  24 XK_q
        niri test-action
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn deactivating_the_inhibitor_does_not_arm_a_release_bind_retroactively() {
    let (mut f, id, surface) = set_up(INHIBITING_BINDS);
    let inhibitor = f.client(id).state.inhibit_shortcuts(&surface);
    f.roundtrip(id);

    // The press while inhibiting is forwarded and arms nothing.
    run_f(&mut f, id, &surface, "+LWIN +LatQ");

    inhibitor.destroy();
    f.roundtrip(id);

    // The release is still forwarded, and still runs no action.
    assert_snapshot!(run_f(&mut f, id, &surface, "-LatQ -LWIN"), @"
    -AD01  24 XK_q
        surface key released: 16
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn activating_the_inhibitor_keeps_an_armed_release_bind() {
    let (mut f, id, surface) = set_up(INHIBITING_BINDS);

    // The press while not inhibiting is intercepted and arms the release action.
    run_f(&mut f, id, &surface, "+LWIN +LatQ");

    let _inhibitor = f.client(id).state.inhibit_shortcuts(&surface);
    f.roundtrip(id);

    // The release stays intercepted, and still runs the armed release action.
    assert_snapshot!(run_f(&mut f, id, &surface, "-LatQ -LWIN"), @"
    -AD01  24 XK_q
        niri test-action
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn inhibiting_a_modifier_press_still_forwards_it() {
    let (mut f, id, surface) = set_up(INHIBITING_BINDS);
    let _inhibitor = f.client(id).state.inhibit_shortcuts(&surface);
    f.roundtrip(id);

    // A modifier press is forwarded either way; the inhibitor only decides whether it arms the
    // release action, and here it doesn't.
    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    ");
}

#[test]
fn armed_modifier_release_runs_after_the_inhibitor_activates() {
    let (mut f, id, surface) = set_up(INHIBITING_BINDS);

    // The modifier press while not inhibiting arms its release action.
    run_f(&mut f, id, &surface, "+LWIN");

    let _inhibitor = f.client(id).state.inhibit_shortcuts(&surface);
    f.roundtrip(id);

    assert_snapshot!(run_f(&mut f, id, &surface, "-LWIN"), @"
    -LWIN 133 XK_Super_L
        niri test-action
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn egl_release_action_runs_when_press_opens_screenshot_ui() {
    // Release actions must always run if their press action ran,
    // including if the screenshot UI is opened before the key is released.
    let c = "
    binds {
        Mod+Q {
            press { screenshot; }
            release { close-window; }
        }
    }
    ";

    let (mut f, id, surface) = set_up(c);
    f.niri_state().backend.headless().add_renderer().unwrap();

    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN +LatQ -LatQ -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
        surface leave
    -AD01  24 XK_q
        niri test-action
    -LWIN 133 XK_Super_L
    ");
    assert!(f.niri().screenshot_ui.is_open());
}

#[test]
fn cooldown_skips_whole_bind() {
    // A bind on cooldown is skipped entirely: its press runs no action, and no release action is
    // recorded, so its release runs no action either.
    let c = "
    binds {
        Mod+MouseLeft cooldown-ms=1000 {
            press { close-window; }
            release { center-column; }
        }
    }
    ";

    assert_snapshot!(run(c, "+LWIN +MouseLeft -MouseLeft +MouseLeft -MouseLeft -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +MouseLeft
        niri test-action
    -MouseLeft
        niri test-action
    +MouseLeft
    -MouseLeft
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn cooldown_only_skips_the_action() {
    let c = "
    binds {
        Mod+Control_L cooldown-ms=1000 { close-window; }
    }
    ";

    // The cooldown only skips the action: the second press runs no action, but it is still
    // intercepted and hidden from the window, only advertising the modifier state.
    assert_snapshot!(run(c, "+LWIN +LCTL -LCTL +LCTL"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        niri test-action
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
    ");
}

#[test]
fn cooldown_applies_to_release_action_of_release_only_bind() {
    // The release action is what starts this bind, so the cooldown rate-limits it: it runs for the
    // first release, and not for the second while the bind is on cooldown.
    let c = "
    binds {
        Mod cooldown-ms=1000 {
            release { toggle-overview; }
        }
    }
    ";

    assert_snapshot!(run(c, "+LWIN -LWIN +LWIN -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        niri test-action
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn cooldown_applies_to_press_action_only() {
    // The cooldown applies to the press action only, and the release action runs only if its press
    // action ran, so the second press runs no action at all.
    let c = "
    binds {
        Mod+Q cooldown-ms=1000 {
            press { close-window; }
            release { center-column; }
        }
    }
    ";

    assert_snapshot!(run(c, "+LWIN +LatQ -LatQ +LatQ -LatQ -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
        niri test-action
    -AD01  24 XK_q
        niri test-action
    +AD01  24 XK_q
    -AD01  24 XK_q
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn release_bind_runs_when_mru_closes() {
    // `Alt` is a push-to-talk bind: the default Alt+Tab opens the MRU, and releasing Alt closes it
    // instead of going through the regular bind lookup, so its release action must still run.
    let c = "
    binds {
        Alt {
            press { spawn \"ptt-toggle\"; }
            release { spawn \"ptt-toggle\"; }
        }
    }
    ";

    assert_snapshot!(run(c, "+LALT +TAB -LALT"), @"
    +LALT  64 XK_Alt_L
        niri test-action
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    +TAB  23 XK_Tab
        surface leave
    -LALT  64 XK_Alt_L
        niri test-action
        surface enter: []
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn mru_close_forwards_key_release_without_bind() {
    // No bind is recorded for Alt, so releasing it while the MRU is open closes the MRU and the
    // release is forwarded to the window like any other unsuppressed key.
    let c = "
    binds {
        Mod+Q { close-window; }
    }
    ";

    assert_snapshot!(run(c, "+LALT +TAB -LALT"), @"
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    +TAB  23 XK_Tab
        surface leave
    -LALT  64 XK_Alt_L
        surface enter: [56]
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn mru_ignores_generic_modifier_binds() {
    let c = "
    binds {
        Alt+Ctrl { screenshot; }
    }
    ";

    // If a press runs the screenshot action,
    // we will get surface enter after it.
    assert_snapshot!(run(c, "+LALT +TAB -TAB +LFSH -LFSH -LALT"), @"
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    +TAB  23 XK_Tab
        surface leave
    -TAB  23 XK_Tab
    +LFSH  50 XK_Shift_L
    -LFSH  50 XK_Shift_L
    -LALT  64 XK_Alt_L
        surface enter: [56]
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    assert_snapshot!(run(c, "+LALT +TAB -TAB +LCTL -LCTL -LALT"), @"
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    +TAB  23 XK_Tab
        surface leave
    -TAB  23 XK_Tab
    +LCTL  37 XK_Control_L
    -LCTL  37 XK_Control_L
    -LALT  64 XK_Alt_L
        surface enter: [56]
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    let c = "
    binds {
        Ctrl+Shift { screenshot; }
    }
    ";

    assert_snapshot!(run(c, "+LALT +TAB -TAB +LFSH -LFSH -LALT"), @"
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    +TAB  23 XK_Tab
        surface leave
    -TAB  23 XK_Tab
    +LFSH  50 XK_Shift_L
    -LFSH  50 XK_Shift_L
    -LALT  64 XK_Alt_L
        surface enter: [56]
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    assert_snapshot!(run(c, "+LALT +TAB -TAB +LCTL +LFSH -LFSH -LCTL -LALT"), @"
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    +TAB  23 XK_Tab
        surface leave
    -TAB  23 XK_Tab
    +LCTL  37 XK_Control_L
    +LFSH  50 XK_Shift_L
    -LFSH  50 XK_Shift_L
    -LCTL  37 XK_Control_L
    -LALT  64 XK_Alt_L
        surface enter: [56]
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Now with a normal trigger.
    let c = "
    binds {
        Alt+Control_L { screenshot; }
    }
    ";

    assert_snapshot!(run(c, "+LALT +TAB -TAB +LFSH -LFSH -LALT"), @"
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    +TAB  23 XK_Tab
        surface leave
    -TAB  23 XK_Tab
    +LFSH  50 XK_Shift_L
    -LFSH  50 XK_Shift_L
    -LALT  64 XK_Alt_L
        surface enter: [56]
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    assert_snapshot!(run(c, "+LALT +TAB -TAB +LCTL -LCTL -LALT"), @"
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    +TAB  23 XK_Tab
        surface leave
    -TAB  23 XK_Tab
    +LCTL  37 XK_Control_L
        surface enter: [56]
        surface modifiers: depressed=12, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    -LALT  64 XK_Alt_L
        surface key released: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    let c = "
    binds {
        Ctrl+Shift_L { screenshot; }
    }
    ";

    assert_snapshot!(run(c, "+LALT +TAB -TAB +LFSH -LFSH -LALT"), @"
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    +TAB  23 XK_Tab
        surface leave
    -TAB  23 XK_Tab
    +LFSH  50 XK_Shift_L
        surface enter: [56]
        surface modifiers: depressed=9, latched=0, locked=0, group=0
    -LFSH  50 XK_Shift_L
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    -LALT  64 XK_Alt_L
        surface key released: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    assert_snapshot!(run(c, "+LALT +TAB -TAB +LCTL +LFSH -LFSH -LCTL -LALT"), @"
    +LALT  64 XK_Alt_L
        surface key pressed: 56
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    +TAB  23 XK_Tab
        surface leave
    -TAB  23 XK_Tab
    +LCTL  37 XK_Control_L
    +LFSH  50 XK_Shift_L
        surface enter: [29, 56]
        surface modifiers: depressed=13, latched=0, locked=0, group=0
    -LFSH  50 XK_Shift_L
        surface modifiers: depressed=12, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        surface key released: 29
        surface modifiers: depressed=8, latched=0, locked=0, group=0
    -LALT  64 XK_Alt_L
        surface key released: 56
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn vt_switch_runs_release_actions_of_held_binds() {
    // A VT switch can take the input away without ever delivering the releases of everything held
    // down, so the held binds are treated as released rather than forgotten.
    let c = "
    binds {
        Mod+Q {
            press { close-window; }
            release { center-column; }
        }
        Mod+MouseLeft {
            press { close-window; }
            release { center-column; }
        }
    }
    ";

    let (mut f, id, surface) = set_up(c);

    // Hold a key and a mouse button, both with press and release actions.
    run_f(&mut f, id, &surface, "+LWIN +LatQ +MouseLeft");
    assert_eq!(f.niri().test_action_count, 2);

    // The VT switch runs both release actions.
    f.niri_state().do_action(Action::ChangeVt(1), false);
    assert_eq!(f.niri().test_action_count, 4);

    // If the releases arrive anyway, they run nothing.
    run_f(&mut f, id, &surface, "-LatQ -MouseLeft");
    assert_eq!(f.niri().test_action_count, 4);
}

#[test]
fn other_input_cancels_release_only_mouse_bind() {
    // A release-only mouse bind is cancelled by other input in between, just like a release-only
    // keyboard bind.
    let c = "
    binds {
        Mod+MouseLeft {
            release { close-window; }
        }
    }
    ";

    // Pressing a key in between cancels it.
    assert_snapshot!(run(c, "+LWIN +MouseLeft +LatA -LatA -MouseLeft -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +MouseLeft
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -MouseLeft
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Pressing another mouse button in between cancels it.
    assert_snapshot!(run(c, "+LWIN +MouseLeft +MouseRight -MouseRight -MouseLeft -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +MouseLeft
    +MouseRight
    -MouseRight
    -MouseLeft
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // With nothing else in between, the release action runs.
    assert_snapshot!(run(c, "+LWIN +MouseLeft -MouseLeft -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +MouseLeft
    -MouseLeft
        niri test-action
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn mouse_button_press_cancels_release_only_key_bind() {
    // A mouse button press is other input, so it interrupts a pending release-only keyboard bind,
    // including one on a bare modifier. Only the mouse bind's press action runs.
    let c = "
    binds {
        Mod {
            release { center-column; }
        }
        Mod+MouseLeft { close-window; }
    }
    ";

    assert_snapshot!(run(c, "+LWIN +MouseLeft -MouseLeft -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +MouseLeft
        niri test-action
    -MouseLeft
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn scroll_cancels_release_only_key_bind() {
    // Scrolling is input, so it cancels a pending release-only bind. Here there is no scroll bind
    // for the held modifiers.
    let c = "
    binds {
        Mod {
            release { center-column; }
        }
    }
    ";

    // The scroll in between cancels the pending release bind, so the release runs no action.
    assert_snapshot!(run(c, "+LWIN WheelScrollDown -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    WheelScrollDown
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Without a scroll in between, the release action runs.
    assert_snapshot!(run(c, "+LWIN -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        niri test-action
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn redundant_modifier_binds() {
    // Binds should behave identically with or without redundant modifiers.
    let input = "+LALT -LALT";
    assert_eq!(
        run("binds { Alt_L { release { close-window; }; }; }", input),
        run("binds { Alt+Alt_L { release { close-window; }; }; }", input)
    );

    let input = "+LALT +LatQ -LatQ -LALT";
    assert_eq!(
        run("binds { Alt+Q { release { close-window; }; }; }", input),
        run("binds { Alt+Alt+Q { release { close-window; }; }; }", input)
    );
}
