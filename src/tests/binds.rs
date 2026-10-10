use std::fmt::Write as _;

use insta::assert_snapshot;
use niri_config::{Action, BoundAction, Config};
use smithay::backend::input::{InputEvent, InputTime, KeyState, Keycode};
use smithay::input::keyboard::xkb::Keymap;
use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitor;
use wayland_client::protocol::wl_pointer;
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

/// Presses or releases the left mouse button via the window's virtual pointer.
fn mouse_button(f: &mut Fixture, id: ClientId, state: wl_pointer::ButtonState) {
    // BTN_LEFT
    const BUTTON: u32 = 0x110;

    let client = f.client(id);
    let manager = client.state.virtual_pointer_manager.as_ref().unwrap();
    let pointer = manager.create_virtual_pointer(None, &client.qh, ());
    pointer.button(0, BUTTON, state);
    pointer.frame();
    f.roundtrip(id);
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

/// Returns whether the window's keyboard shortcuts inhibitor is active.
fn inhibiting_shortcuts(f: &mut Fixture) -> bool {
    f.niri()
        .keyboard_shortcuts_inhibiting_surfaces
        .values()
        .any(KeyboardShortcutsInhibitor::is_active)
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

    // The same applies if the other key triggered a bind.
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

    // Release-only binds on regular keys are cancelled by other input the same way.
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

    // It runs even if the modifiers are released first...
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

    // ...but pressing another key in between cancels it, no matter the trigger.
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

    // Ctrl+Alt_L is a modifier-only bind too: its whole key consists of modifiers, so releasing the
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

    // Pressing another key in between cancels it, just like for a bare modifier bind, so using
    // Ctrl+Alt+U as an app shortcut does not additionally switch the layout.
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

    // The same applies if the other key triggered a bind of its own.
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

    // And even if extra modifiers are held when the key is released.
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

    // The window doesn't see the key press or release, but it still gets the modifier state: the
    // new window learns from the focus change that mod is held, and the intercepted release updates
    // it, so it doesn't get stuck with mod held.
    assert_snapshot!(run_f(&mut f, id, &surface2, "-LWIN +LatA -LatA"), @"
    -LWIN 133 XK_Super_L
        niri test-action
        surface enter: []
        surface modifiers: depressed=64, latched=0, locked=0, group=0
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

    // Alt works as a held modifier the same way: the Control_L press is intercepted, and only the
    // press action runs.
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

    // Pressing another key in between cancels it, just like for a bare modifier bind.
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
        surface key pressed: 29
        surface modifiers: depressed=4, latched=0, locked=0, group=0
    -LCTL  37 XK_Control_L
        niri test-action
        surface key released: 29
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn modifier_key_and_keysym_triggers() {
    // `Ctrl` is a modifier key, so it triggers on both Control_L and Control_R.
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

    // `Control_L` is a specific key instead, so it doesn't trigger on Control_R, which reaches the
    // window instead. The left and right keysyms are different keys, while the modifier and its
    // keysyms are the same key and would always conflict.
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

    inhibitor.destroy();
    f.roundtrip(id);

    // Once the inhibitor is gone, they do.
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

    // Toggling the inhibitor while a key is held doesn't change what the press decided: the press
    // picks whether the key is intercepted at all, and whether the release action is armed.

    let _inhibitor = f.client(id).state.inhibit_shortcuts(&surface);
    f.roundtrip(id);

    // Press while inhibiting: the whole key is forwarded, and no release action is armed.
    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN +LatQ"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
        surface key pressed: 16
    ");

    // Deactivating it (like with Mod+Escape) with the keys still held doesn't arm the release
    // action retroactively: the release is still forwarded, and still runs no action.
    f.niri_state()
        .do_action(Action::ToggleKeyboardShortcutsInhibit, false);
    assert!(!inhibiting_shortcuts(&mut f));
    f.roundtrip(id);
    assert_snapshot!(run_f(&mut f, id, &surface, "-LatQ -LWIN"), @"
    -AD01  24 XK_q
        surface key released: 16
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // Press while not inhibiting: the trigger key is intercepted, and its release action is armed.
    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN +LatQ"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
    ");

    // Activating it with the keys still held doesn't disarm the armed release action either: the
    // release action runs, since its press decided, and the release stays intercepted, since its
    // press was too.
    f.niri_state()
        .do_action(Action::ToggleKeyboardShortcutsInhibit, false);
    assert!(inhibiting_shortcuts(&mut f));
    f.roundtrip(id);
    assert_snapshot!(run_f(&mut f, id, &surface, "-LatQ -LWIN"), @"
    -AD01  24 XK_q
        niri test-action
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // A modifier press is forwarded either way, so the inhibitor only decides whether it arms the
    // release action.
    f.niri_state()
        .do_action(Action::ToggleKeyboardShortcutsInhibit, false);
    assert!(!inhibiting_shortcuts(&mut f));
    f.roundtrip(id);
    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    ");

    // ...and the armed release action runs even if the inhibitor came back by release time.
    f.niri_state()
        .do_action(Action::ToggleKeyboardShortcutsInhibit, false);
    assert!(inhibiting_shortcuts(&mut f));
    f.roundtrip(id);
    assert_snapshot!(run_f(&mut f, id, &surface, "-LWIN"), @"
    -LWIN 133 XK_Super_L
        niri test-action
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn release_action_not_run_while_screenshot_ui_open() {
    let c = "
    binds {
        Mod+Q {
            press { close-window; }
            release { center-column; }
        }
    }
    ";

    let (mut f, id, surface) = set_up(c);
    f.niri_state().backend.headless().add_renderer().unwrap();

    // The press action runs and records the release action.
    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN +LatQ"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
        niri test-action
    ");

    // The screenshot UI opens in between, like it would from the bind's own press action. It takes
    // the keyboard focus away from the window.
    f.niri_state().open_screenshot_ui(false, None);
    assert!(f.niri().screenshot_ui.is_open());
    f.roundtrip(id);
    let _ = f.client(id).state.recent_keyboard_events(&surface);

    // The release action isn't allowed while the screenshot UI is open, so it doesn't run. The
    // releases are still intercepted, and the window keeps not seeing anything, since the UI has
    // the keyboard focus.
    assert_snapshot!(run_f(&mut f, id, &surface, "-LatQ -LWIN"), @"
    -AD01  24 XK_q
    -LWIN 133 XK_Super_L
    ");
}

#[test]
fn cooldown_skips_whole_bind() {
    // A bind whose press is rate-limited by its cooldown is skipped entirely: no action runs, and
    // the release action isn't recorded either, so the release runs no action. Mouse binds decide
    // this when the button is pressed, same as keyboard binds.
    let c = "
    binds {
        Mod+MouseLeft cooldown-ms=1000 {
            press { close-window; }
            release { center-column; }
        }
    }
    ";

    let (mut f, id, surface) = set_up(c);
    run_f(&mut f, id, &surface, "+LWIN");

    // The first click runs the press action and records the release action.
    mouse_button(&mut f, id, wl_pointer::ButtonState::Pressed);
    assert_eq!(f.niri().test_action_count, 1);

    // So its release runs the release action.
    mouse_button(&mut f, id, wl_pointer::ButtonState::Released);
    assert_eq!(f.niri().test_action_count, 2);

    // While the bind is on cooldown, the second click runs no action at all: the press action is
    // rate-limited, and the release action isn't recorded, so the release runs no action either.
    mouse_button(&mut f, id, wl_pointer::ButtonState::Pressed);
    mouse_button(&mut f, id, wl_pointer::ButtonState::Released);
    assert_eq!(f.niri().test_action_count, 2);
}

#[test]
fn cooldown_only_skips_the_action() {
    let c = "
    binds {
        Mod+Control_L cooldown-ms=1000 { close-window; }
    }
    ";

    // The first press runs the action, and both the press and the release are intercepted: the
    // window doesn't see the key, only the modifier state.
    //
    // The second press is on cooldown, so it runs no action, but it is intercepted the same way.
    // The cooldown only skips the action; it doesn't change what the window sees.
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
fn cooldown_applies_to_press_action_only() {
    let c = "
    binds {
        Mod+Q cooldown-ms=1000 {
            press { close-window; }
            release { center-column; }
        }
    }
    ";

    let (mut f, id, surface) = set_up(c);

    // The bind's cooldown applies to its press action only, so the release action still runs when
    // the key is released.
    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN +LatQ -LatQ"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    +AD01  24 XK_q
        niri test-action
    -AD01  24 XK_q
        niri test-action
    ");

    // While the bind is still on cooldown (it can't expire during the test), no action runs at all:
    // the press action is rate-limited, and the release action only runs if its press action ran.
    // The key is still hidden from the window.
    assert_snapshot!(run_f(&mut f, id, &surface, "+LatQ -LatQ -LWIN"), @"
    +AD01  24 XK_q
    -AD01  24 XK_q
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}

#[test]
fn cooldown_applies_to_release_action_of_release_only_bind() {
    let c = "
    binds {
        Mod cooldown-ms=1000 {
            release { toggle-overview; }
        }
    }
    ";

    let (mut f, id, surface) = set_up(c);

    // The release action is what starts this bind, so the cooldown rate-limits it. It still runs
    // for the first release.
    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        niri test-action
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");

    // While the bind is on cooldown, releasing the key runs no action.
    assert_snapshot!(run_f(&mut f, id, &surface, "+LWIN -LWIN"), @"
    +LWIN 133 XK_Super_L
        surface key pressed: 125
        surface modifiers: depressed=64, latched=0, locked=0, group=0
    -LWIN 133 XK_Super_L
        surface key released: 125
        surface modifiers: depressed=0, latched=0, locked=0, group=0
    ");
}
