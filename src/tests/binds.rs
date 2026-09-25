use std::fmt::Write as _;

use insta::assert_snapshot;
use niri_config::{Action, BoundAction, Config};
use smithay::backend::input::{InputEvent, InputTime, KeyState, Keycode};
use smithay::input::keyboard::xkb::Keymap;
use wayland_client::protocol::wl_surface::WlSurface;

use crate::tests::client::ClientId;
use crate::tests::fixture::Fixture;
use crate::tests::test_input_backend::{TestInputBackend, TestKeyboardKeyEvent};

enum Op {
    Press(Keycode),
    Release(Keycode),
}

fn parse(keymap: &Keymap, input: &str) -> Vec<Op> {
    let mut ops = Vec::new();
    for part in input.split_ascii_whitespace() {
        let name = &part[1..];
        let Some(key) = keymap.key_by_name(name) else {
            panic!("unknown key {name}");
        };

        let c = part.bytes().next().unwrap();
        let op = match c {
            b'+' => Op::Press(key),
            b'-' => Op::Release(key),
            _ => panic!("keys must begin with + or -, got {c}"),
        };

        ops.push(op);
    }
    ops
}

fn set_up(config: &str) -> (Fixture, ClientId, WlSurface) {
    let mut config = Config::parse_mem(config).unwrap();
    // knuffel doesn't understand #[cfg(test)]...
    for bind in &mut config.binds.0 {
        bind.action = match &bind.action {
            BoundAction::Press(_) => BoundAction::Press(Action::TestAction),
            BoundAction::Release(_) => BoundAction::Release(Action::TestAction),
            BoundAction::Both { .. } => BoundAction::Both {
                press: Action::TestAction,
                release: Action::TestAction,
            },
        };
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

    for op in ops {
        let (code, key_state) = match op {
            Op::Press(code) => (code, KeyState::Pressed),
            Op::Release(code) => (code, KeyState::Released),
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

        let prev = state.niri.test_action_count;

        state.process_input_event(InputEvent::<TestInputBackend>::Keyboard {
            event: TestKeyboardKeyEvent {
                time: InputTime::from_micros(0),
                code,
                state: key_state,
                count: 1, // niri doesn't use this
            },
        });

        let diff = f.niri().test_action_count - prev;
        for _ in 0..diff {
            let _ = writeln!(&mut rv, "    niri test-action");
        }

        f.roundtrip(id);
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
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +AD01  24 XK_q
        niri test-action
    -AD01  24 XK_q
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    "
    );

    // Two actions interleaved.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatQ +LatW -LatQ -LatW -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +AD01  24 XK_q
        niri test-action
    +AD02  25 XK_w
        niri test-action
    -AD01  24 XK_q
    -AD02  25 XK_w
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    "
    );

    // Extra Alt = no action.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LALT +LatQ -LALT -LatQ -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +LALT  64 XK_Alt_L
        surface modifiers: depressed=76, latched=0, locked=0, group=0
        surface key pressed: 56
    +AD01  24 XK_q
        surface key pressed: 16
    -LALT  64 XK_Alt_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key released: 56
    -AD01  24 XK_q
        surface key released: 16
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    "
    );

    // Key that doesn't correspond to any bind.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatA -LatA -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    "
    );

    // Press action, press arbitrary, release action, release arbitrary.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatQ +LatA -LatQ -LatA -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +AD01  24 XK_q
        niri test-action
    +AC01  38 XK_a
        surface key pressed: 30
    -AD01  24 XK_q
    -AC01  38 XK_a
        surface key released: 30
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    "
    );

    // Press arbitrary, press action, release arbitrary, release action.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatA +LatQ -LatA -LatQ -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +AC01  38 XK_a
        surface key pressed: 30
    +AD01  24 XK_q
        niri test-action
    -AC01  38 XK_a
        surface key released: 30
    -AD01  24 XK_q
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    "
    );

    // Trigger action then release mods.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatQ -LCTL -LWIN -LatQ"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +AD01  24 XK_q
        niri test-action
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    -AD01  24 XK_q
    "
    );

    // Modifiers after trigger key don't trigger the action.
    assert_snapshot!(
        run(c, "+LWIN +LatQ +LCTL -LCTL -LatQ -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +AD01  24 XK_q
        surface key pressed: 16
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -AD01  24 XK_q
        surface key released: 16
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
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
        surface modifiers: depressed=0, latched=0, locked=0, group=1
        surface key pressed: 56
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
        surface modifiers: depressed=1, latched=0, locked=0, group=0
        surface key pressed: 42
    +AB10  61 XK_question
        niri test-action
    -AB10  61 XK_question
    -LFSH  50 XK_Shift_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 42
    +LALT  64 XK_ISO_Next_Group
        surface modifiers: depressed=0, latched=0, locked=0, group=1
        surface key pressed: 56
    -LALT  64 XK_ISO_Next_Group
        surface key released: 56
    +LFSH  50 XK_Shift_L
        surface modifiers: depressed=1, latched=0, locked=0, group=1
        surface key pressed: 42
    +AB10  61 XK_comma
        surface key pressed: 53
    -AB10  61 XK_comma
        surface key released: 53
    -LFSH  50 XK_Shift_L
        surface modifiers: depressed=0, latched=0, locked=0, group=1
        surface key released: 42
    "
    );

    // In ru, / is on the \ / key (so, Shift + \). So, arguably, it would make sense for Shift + /
    // to trigger it, but it currently doesn't (niri requires an unshifted trigger key, despite
    // working fine with capital case alphabetic keys).
    assert_snapshot!(
        run(c, "+LALT -LALT +LFSH +BKSL -BKSL -LFSH"),
        @"
    +LALT  64 XK_ISO_Next_Group
        surface modifiers: depressed=0, latched=0, locked=0, group=1
        surface key pressed: 56
    -LALT  64 XK_ISO_Next_Group
        surface key released: 56
    +LFSH  50 XK_Shift_L
        surface modifiers: depressed=1, latched=0, locked=0, group=1
        surface key pressed: 42
    +BKSL  51 XK_slash
        surface key pressed: 43
    -BKSL  51 XK_slash
        surface key released: 43
    -LFSH  50 XK_Shift_L
        surface modifiers: depressed=0, latched=0, locked=0, group=1
        surface key released: 42
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
        Mod+L { center-column; }
        Ctrl+Alt_L {
            release { switch-layout \"next\"; }
        }
        Ctrl+Alt+L { close-window; }
    }
    ";

    // Releasing Mod by itself runs its release action.
    assert_snapshot!(run(c, "+LWIN -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    -LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    ");

    // Pressing another key in between cancels a modifier-only release bind.
    assert_snapshot!(run(c, "+LWIN +LatA -LatA -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    ");

    // The same applies if the other key triggered a bind.
    assert_snapshot!(run(c, "+LWIN +LatL -LatL -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +AC09  46 XK_l
        niri test-action
    -AC09  46 XK_l
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    ");

    // Release-only binds on regular keys are not cancelled by other input, and they run even if
    // the modifiers are released first.
    assert_snapshot!(run(c, "+LWIN +LatQ +LatA -LatA -LWIN -LatQ"), @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +AD01  24 XK_q
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    -AD01  24 XK_q
        niri test-action
    ");

    // Ctrl+Alt_L is a modifier-only bind too: its whole key consists of modifiers, so releasing the
    // combo on its own runs its release action.
    assert_snapshot!(run(c, "+LCTL +LALT -LALT -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=4, latched=0, locked=0, group=0
        surface key pressed: 29
    +LALT  64 XK_Alt_L
        surface modifiers: depressed=12, latched=0, locked=0, group=0
        surface key pressed: 56
    -LALT  64 XK_Alt_L
        niri test-action
        surface modifiers: depressed=4, latched=0, locked=0, group=0
        surface key released: 56
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 29
    ");

    // Pressing another key in between cancels it, just like for a bare modifier bind, so using
    // Ctrl+Alt+L as an app shortcut does not additionally switch the layout.
    assert_snapshot!(run(c, "+LCTL +LALT +LatA -LatA -LALT -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=4, latched=0, locked=0, group=0
        surface key pressed: 29
    +LALT  64 XK_Alt_L
        surface modifiers: depressed=12, latched=0, locked=0, group=0
        surface key pressed: 56
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -LALT  64 XK_Alt_L
        surface modifiers: depressed=4, latched=0, locked=0, group=0
        surface key released: 56
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 29
    ");

    // The same applies if the other key triggered a bind of its own.
    assert_snapshot!(run(c, "+LCTL +LALT +LatL -LatL -LALT -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=4, latched=0, locked=0, group=0
        surface key pressed: 29
    +LALT  64 XK_Alt_L
        surface modifiers: depressed=12, latched=0, locked=0, group=0
        surface key pressed: 56
    +AC09  46 XK_l
        niri test-action
    -AC09  46 XK_l
    -LALT  64 XK_Alt_L
        surface modifiers: depressed=4, latched=0, locked=0, group=0
        surface key released: 56
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 29
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
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +AD01  24 XK_q
        niri test-action
    -AD01  24 XK_q
        niri test-action
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    ");

    // The release action runs even if the modifiers are released first.
    assert_snapshot!(run(c, "+LWIN +LatQ -LWIN -LatQ"), @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +AD01  24 XK_q
        niri test-action
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    -AD01  24 XK_q
        niri test-action
    ");

    // And even if extra modifiers are held when the key is released.
    assert_snapshot!(run(c, "+LWIN +LatQ +LCTL +LFSH -LatQ -LFSH -LCTL -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +AD01  24 XK_q
        niri test-action
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +LFSH  50 XK_Shift_L
        surface modifiers: depressed=69, latched=0, locked=0, group=0
        surface key pressed: 42
    -AD01  24 XK_Q
        niri test-action
    -LFSH  50 XK_Shift_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key released: 42
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    ");

    // Push-to-talk: other input in between doesn't cancel the release action.
    assert_snapshot!(run(c, "+LWIN +LatQ +LatA -LatA -LatQ -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +AD01  24 XK_q
        niri test-action
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -AD01  24 XK_q
        niri test-action
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    ");

    // Pressing the key without the mod key held doesn't run either action.
    assert_snapshot!(run(c, "+LatQ +LWIN -LatQ -LWIN"), @"
    +AD01  24 XK_q
        surface key pressed: 16
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    -AD01  24 XK_q
        surface key released: 16
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
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

    // Both actions run, and Mod is still forwarded so that apps see the modifier.
    assert_snapshot!(run(c, "+LWIN -LWIN"), @"
    +LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    -LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    ");

    // Other input in between doesn't cancel the release action.
    assert_snapshot!(run(c, "+LWIN +LatA -LatA -LWIN"), @"
    +LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -LWIN 133 XK_Super_L
        niri test-action
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
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

    // A modifier key used as the trigger is still forwarded so that apps see the modifier, and both
    // actions run.
    assert_snapshot!(run(c, "+LWIN +LCTL -LCTL -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        niri test-action
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    -LCTL  37 XK_Control_L
        niri test-action
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    ");

    // Without the mod key held the bind doesn't match.
    assert_snapshot!(run(c, "+LCTL -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=4, latched=0, locked=0, group=0
        surface key pressed: 29
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 29
    ");

    // Alt works as a held modifier the same way.
    assert_snapshot!(run(c, "+LALT +LCTL -LCTL -LALT"), @"
    +LALT  64 XK_Alt_L
        surface modifiers: depressed=8, latched=0, locked=0, group=0
        surface key pressed: 56
    +LCTL  37 XK_Control_L
        niri test-action
        surface modifiers: depressed=12, latched=0, locked=0, group=0
        surface key pressed: 29
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=8, latched=0, locked=0, group=0
        surface key released: 29
    -LALT  64 XK_Alt_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 56
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
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    -LCTL  37 XK_Control_L
        niri test-action
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    ");

    // Pressing another key in between cancels it, just like for a bare modifier bind.
    assert_snapshot!(run(c, "+LWIN +LCTL +LFSH -LCTL -LFSH -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +LFSH  50 XK_Shift_L
        surface modifiers: depressed=69, latched=0, locked=0, group=0
        surface key pressed: 42
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=65, latched=0, locked=0, group=0
        surface key released: 29
    -LFSH  50 XK_Shift_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 42
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    ");
}

#[test]
fn bare_modifier_release_bind() {
    let c = "
    binds {
        Ctrl {
            release { close-window; }
        }
    }
    ";

    // Control_L is not the mod key, so `Ctrl` matches its keysym directly.
    assert_snapshot!(run(c, "+LCTL -LCTL"), @"
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=4, latched=0, locked=0, group=0
        surface key pressed: 29
    -LCTL  37 XK_Control_L
        niri test-action
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 29
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
        surface modifiers: depressed=4, latched=0, locked=0, group=0
        surface key pressed: 29
    +AD01  24 XK_q
        surface key pressed: 16
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 125
    -AD01  24 XK_q
        surface key released: 16
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=4, latched=0, locked=0, group=0
        surface key released: 125
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 29
    ");
}

#[test]
fn inhibiting_release_binds() {
    let config = "
    binds {
        Mod {
            release { toggle-overview; }
        }
        Mod+Q {
            release { close-window; }
        }
    }
    ";

    let (mut f, id, surface) = set_up(config);

    let inhibitor = f.client(id).state.inhibit_shortcuts(&surface);
    f.roundtrip(id);

    // While inhibiting, release binds don't run.
    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN +LatQ -LatQ -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +AD01  24 XK_q
        surface key pressed: 16
    -AD01  24 XK_q
        surface key released: 16
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    ");

    inhibitor.destroy();
    f.roundtrip(id);

    // Once the inhibitor is gone, they do.
    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN +LatQ -LatQ -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +AD01  24 XK_q
    -AD01  24 XK_q
        niri test-action
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    ");
}
