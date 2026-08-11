use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use niri_config::Config;
use niri_ipc::PositionChange;
use smithay::backend::input::{
    AbsolutePositionEvent, ButtonState, Device, DeviceCapability, Event, InputBackend, InputEvent,
    PointerButtonEvent, PointerMotionAbsoluteEvent, PointerMotionEvent, UnusedEvent,
};
use smithay::desktop::Window as DesktopWindow;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity,
};
use smithay::reexports::wayland_server::Resource as _;
use smithay::utils::Point;
use smithay::reexports::wayland_protocols::wp::pointer_constraints::zv1::client::zwp_pointer_constraints_v1::Lifetime as ClientConstraintLifetime;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::Proxy as _;

use super::*;
use crate::input::backend_ext::NiriInputDevice;
use crate::niri::PointerVisibility;
use crate::tests::client::{ClientEvent, ClientId, LayerConfigureProps, LayerMargin};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct TestDevice;

impl Device for TestDevice {
    fn id(&self) -> String {
        "test-pointer".into()
    }

    fn name(&self) -> String {
        "test pointer".into()
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        capability == DeviceCapability::Pointer
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<PathBuf> {
        None
    }
}

impl NiriInputDevice for TestDevice {
    fn output(&self, _state: &crate::niri::State) -> Option<smithay::output::Output> {
        None
    }
}

struct TestInputBackend;

struct TestButtonEvent(ButtonState);

impl Event<TestInputBackend> for TestButtonEvent {
    fn time(&self) -> u64 {
        0
    }

    fn device(&self) -> TestDevice {
        TestDevice
    }
}

impl PointerButtonEvent<TestInputBackend> for TestButtonEvent {
    fn button_code(&self) -> u32 {
        0x110
    }

    fn state(&self) -> ButtonState {
        self.0
    }
}

struct TestAbsoluteEvent {
    x: f64,
    y: f64,
}

impl Event<TestInputBackend> for TestAbsoluteEvent {
    fn time(&self) -> u64 {
        0
    }

    fn device(&self) -> TestDevice {
        TestDevice
    }
}

impl AbsolutePositionEvent<TestInputBackend> for TestAbsoluteEvent {
    fn x(&self) -> f64 {
        self.x
    }

    fn y(&self) -> f64 {
        self.y
    }

    fn x_transformed(&self, width: i32) -> f64 {
        self.x * f64::from(width)
    }

    fn y_transformed(&self, height: i32) -> f64 {
        self.y * f64::from(height)
    }
}

impl PointerMotionAbsoluteEvent<TestInputBackend> for TestAbsoluteEvent {}

struct TestRelativeEvent {
    dx: f64,
    dy: f64,
}

impl Event<TestInputBackend> for TestRelativeEvent {
    fn time(&self) -> u64 {
        0
    }

    fn device(&self) -> TestDevice {
        TestDevice
    }
}

impl PointerMotionEvent<TestInputBackend> for TestRelativeEvent {
    fn delta_x(&self) -> f64 {
        self.dx
    }

    fn delta_y(&self) -> f64 {
        self.dy
    }

    fn delta_x_unaccel(&self) -> f64 {
        self.dx
    }

    fn delta_y_unaccel(&self) -> f64 {
        self.dy
    }
}

impl InputBackend for TestInputBackend {
    type Device = TestDevice;
    type KeyboardKeyEvent = UnusedEvent;
    type PointerAxisEvent = UnusedEvent;
    type PointerButtonEvent = TestButtonEvent;
    type PointerMotionEvent = TestRelativeEvent;
    type PointerMotionAbsoluteEvent = TestAbsoluteEvent;
    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;
    type TouchDownEvent = UnusedEvent;
    type TouchUpEvent = UnusedEvent;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;
    type SwitchToggleEvent = UnusedEvent;
    type SpecialEvent = ();
}

fn fixture_with_window(config: Config) -> (Fixture, ClientId, WlSurface) {
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.set_maximized();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(1920, 1080);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    f.niri_state()
        .move_cursor(Point::from((960.0_f64, 540.0_f64)));
    f.double_roundtrip(id);
    f.niri_complete_animations();
    assert_eq!(f.client(id).pointer_focus(), Some(&surface));

    (f, id, surface)
}

fn fixture_with_floating_window() -> (Fixture, ClientId, WlSurface, DesktopWindow) {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^game$"
    open-floating true
    default-column-width { fixed 400; }
    default-window-height { fixed 300; }
}
"#,
    )
    .unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.set_title("game");
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(400, 300);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let protocol_id = surface.id().protocol_id();
    let window = f
        .niri()
        .layout
        .windows()
        .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
        .unwrap()
        .1
        .window
        .clone();
    f.niri().layout.move_floating_window(
        Some(&window),
        PositionChange::SetFixed(100.),
        PositionChange::SetFixed(100.),
        false,
    );
    f.niri_state()
        .move_cursor(Point::from((200.0_f64, 200.0_f64)));
    f.double_roundtrip(id);
    assert_eq!(f.client(id).pointer_focus(), Some(&surface));

    (f, id, surface, window)
}

fn add_overlay_layer(f: &mut Fixture, id: ClientId, left: i32, top: i32) -> WlSurface {
    let layer = f.client(id).create_layer(None, Layer::Overlay, "");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Top | Anchor::Left),
        size: Some((100, 100)),
        margin: Some(LayerMargin {
            top,
            left,
            ..Default::default()
        }),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    surface
}

fn add_floating_overlay_window(f: &mut Fixture, id: ClientId) -> (WlSurface, DesktopWindow) {
    let overlay = f.client(id).create_window();
    let surface = overlay.surface.clone();
    overlay.set_title("overlay");
    overlay.commit();
    f.roundtrip(id);

    let overlay = f.client(id).window(&surface);
    overlay.attach_new_buffer();
    overlay.set_size(200, 200);
    overlay.ack_last_and_commit();
    f.double_roundtrip(id);

    let window = {
        let protocol_id = surface.id().protocol_id();
        f.niri()
            .layout
            .windows()
            .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
            .unwrap()
            .1
            .window
            .clone()
    };
    f.niri().layout.move_floating_window(
        Some(&window),
        PositionChange::SetFixed(860.),
        PositionChange::SetFixed(440.),
        false,
    );

    (surface, window)
}

fn send_button(f: &mut Fixture, state: ButtonState) {
    f.niri_state()
        .process_input_event(InputEvent::<TestInputBackend>::PointerButton {
            event: TestButtonEvent(state),
        });
}

#[test]
fn cursor_position_hint_is_applied_only_after_unlock() {
    let (mut f, id, window_surface) = fixture_with_window(Config::default());
    let layer_surface = add_overlay_layer(&mut f, id, 0, 0);

    assert_eq!(f.client(id).pointer_focus(), Some(&window_surface));
    let (locked_pointer, lock_status) = f.client(id).lock_pointer(&window_surface);
    f.double_roundtrip(id);
    assert!(lock_status.locked.load(Ordering::Relaxed));

    locked_pointer.set_cursor_position_hint(50.0, 50.0);
    f.client(id).window(&window_surface).commit();
    f.double_roundtrip(id);

    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        Point::from((960.0_f64, 540.0_f64))
    );
    assert!(lock_status.locked.load(Ordering::Relaxed));
    assert!(!lock_status.unlocked.load(Ordering::Relaxed));
    assert_eq!(f.client(id).pointer_focus(), Some(&window_surface));

    locked_pointer.destroy();
    f.double_roundtrip(id);
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        Point::from((50.0_f64, 50.0_f64))
    );

    f.niri_state().refresh_pointer_contents();
    f.double_roundtrip(id);
    assert_eq!(f.client(id).pointer_focus(), Some(&layer_surface));
}

#[test]
fn new_surface_under_locked_pointer_does_not_end_lock() {
    let (mut f, id, window_surface) = fixture_with_window(Config::default());
    let (_locked_pointer, lock_status) = f.client(id).lock_pointer(&window_surface);
    f.double_roundtrip(id);
    assert!(lock_status.locked.load(Ordering::Relaxed));

    let layer_surface = add_overlay_layer(&mut f, id, 910, 490);
    assert_eq!(
        f.niri()
            .contents_under(Point::from((960.0_f64, 540.0_f64)))
            .surface
            .unwrap()
            .0
            .id()
            .protocol_id(),
        layer_surface.id().protocol_id()
    );

    assert!(!f.niri_state().update_pointer_contents());
    f.double_roundtrip(id);
    assert!(lock_status.locked.load(Ordering::Relaxed));
    assert!(!lock_status.unlocked.load(Ordering::Relaxed));
    assert_eq!(f.client(id).pointer_focus(), Some(&window_surface));
    assert!(f.niri().pointer_contents.window.is_some());
    assert!(f.niri().pointer_contents.layer.is_none());
}

#[test]
fn absolute_motion_does_not_end_active_lock() {
    let (mut f, id, window_surface) = fixture_with_window(Config::default());
    let layer_surface = add_overlay_layer(&mut f, id, 0, 0);
    let (_locked_pointer, lock_status) = f.client(id).lock_pointer(&window_surface);
    f.double_roundtrip(id);
    assert!(lock_status.locked.load(Ordering::Relaxed));

    f.niri_state()
        .process_input_event(InputEvent::<TestInputBackend>::PointerMotionAbsolute {
            event: TestAbsoluteEvent { x: 0.025, y: 0.045 },
        });
    f.double_roundtrip(id);

    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        Point::from((960.0_f64, 540.0_f64))
    );
    assert!(lock_status.locked.load(Ordering::Relaxed));
    assert!(!lock_status.unlocked.load(Ordering::Relaxed));
    assert_eq!(f.client(id).pointer_focus(), Some(&window_surface));
    assert_ne!(f.client(id).pointer_focus(), Some(&layer_surface));
}

#[test]
fn clicking_window_under_locked_pointer_keeps_keyboard_focus() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, window_surface) = fixture_with_window(config);
    let (_locked_pointer, lock_status) = f.client(id).lock_pointer(&window_surface);
    f.double_roundtrip(id);
    assert!(lock_status.locked.load(Ordering::Relaxed));

    let overlay = f.client(id).create_window();
    let overlay_surface = overlay.surface.clone();
    overlay.set_title("overlay");
    overlay.commit();
    f.roundtrip(id);

    let overlay = f.client(id).window(&overlay_surface);
    overlay.attach_new_buffer();
    overlay.set_size(200, 200);
    overlay.ack_last_and_commit();
    f.double_roundtrip(id);

    let overlay_window = {
        let protocol_id = overlay_surface.id().protocol_id();
        f.niri()
            .layout
            .windows()
            .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
            .unwrap()
            .1
            .window
            .clone()
    };
    f.niri().layout.move_floating_window(
        Some(&overlay_window),
        PositionChange::SetFixed(860.),
        PositionChange::SetFixed(440.),
        false,
    );
    assert_eq!(
        f.niri()
            .contents_under(Point::from((960.0_f64, 540.0_f64)))
            .surface
            .unwrap()
            .0
            .id()
            .protocol_id(),
        overlay_surface.id().protocol_id()
    );
    assert_eq!(
        f.niri()
            .layout
            .focus()
            .unwrap()
            .toplevel()
            .wl_surface()
            .id()
            .protocol_id(),
        window_surface.id().protocol_id()
    );

    send_button(&mut f, ButtonState::Pressed);
    send_button(&mut f, ButtonState::Released);
    f.double_roundtrip(id);

    assert_eq!(
        f.niri()
            .layout
            .focus()
            .unwrap()
            .toplevel()
            .wl_surface()
            .id()
            .protocol_id(),
        window_surface.id().protocol_id()
    );
    assert!(lock_status.locked.load(Ordering::Relaxed));
    assert!(!lock_status.unlocked.load(Ordering::Relaxed));
    assert_eq!(f.client(id).pointer_focus(), Some(&window_surface));
}

#[test]
fn keyboard_focus_leave_deactivates_persistent_lock() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, window_surface) = fixture_with_window(config);

    let game_window = {
        let protocol_id = window_surface.id().protocol_id();
        f.niri()
            .layout
            .windows()
            .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
            .unwrap()
            .1
            .window
            .clone()
    };

    let (_locked_pointer, lock_status) = f.client(id).lock_pointer(&window_surface);
    f.double_roundtrip(id);
    assert!(lock_status.locked.load(Ordering::Relaxed));

    let overlay = f.client(id).create_window();
    let overlay_surface = overlay.surface.clone();
    overlay.set_title("overlay");
    overlay.commit();
    f.roundtrip(id);

    let overlay = f.client(id).window(&overlay_surface);
    overlay.attach_new_buffer();
    overlay.set_size(200, 200);
    overlay.ack_last_and_commit();
    f.double_roundtrip(id);

    let overlay_window = {
        let protocol_id = overlay_surface.id().protocol_id();
        f.niri()
            .layout
            .windows()
            .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
            .unwrap()
            .1
            .window
            .clone()
    };

    // This models Alt-Tab or a compositor focus keybind, not a pointer click.
    f.client(id).take_events();
    f.niri().layout.activate_window(&overlay_window);
    f.double_roundtrip(id);

    assert!(!lock_status.locked.load(Ordering::Relaxed));
    assert!(lock_status.unlocked.load(Ordering::Relaxed));
    assert_eq!(f.client(id).keyboard_focus(), Some(&overlay_surface));
    assert_eq!(
        f.client(id).take_events(),
        vec![
            ClientEvent::PointerUnlocked,
            ClientEvent::KeyboardLeave(window_surface.id().protocol_id()),
            ClientEvent::KeyboardEnter(overlay_surface.id().protocol_id()),
        ]
    );

    f.niri().layout.activate_window(&game_window);
    f.double_roundtrip(id);

    assert!(lock_status.locked.load(Ordering::Relaxed));
    assert_eq!(f.client(id).keyboard_focus(), Some(&window_surface));
    assert_eq!(
        f.client(id).take_events(),
        vec![
            ClientEvent::KeyboardLeave(overlay_surface.id().protocol_id()),
            ClientEvent::KeyboardEnter(window_surface.id().protocol_id()),
            ClientEvent::PointerLocked,
        ]
    );
}

#[test]
fn reactivation_cancels_pending_cursor_hint_warp() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, window_surface) = fixture_with_window(config);

    let game_window = {
        let protocol_id = window_surface.id().protocol_id();
        f.niri()
            .layout
            .windows()
            .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
            .unwrap()
            .1
            .window
            .clone()
    };

    let (locked_pointer, lock_status) = f.client(id).lock_pointer(&window_surface);
    f.double_roundtrip(id);
    assert_eq!(lock_status.locked_count.load(Ordering::Relaxed), 1);

    locked_pointer.set_cursor_position_hint(50.0, 50.0);
    f.client(id).window(&window_surface).commit();
    f.double_roundtrip(id);

    let overlay = f.client(id).create_window();
    let overlay_surface = overlay.surface.clone();
    overlay.set_title("overlay");
    overlay.commit();
    f.roundtrip(id);

    let overlay = f.client(id).window(&overlay_surface);
    overlay.attach_new_buffer();
    overlay.set_size(200, 200);
    overlay.ack_last_and_commit();
    f.double_roundtrip(id);

    let overlay_window = {
        let protocol_id = overlay_surface.id().protocol_id();
        f.niri()
            .layout
            .windows()
            .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
            .unwrap()
            .1
            .window
            .clone()
    };

    // Exercise two keyboard-focus transitions in one compositor dispatch. The hint warp queued
    // by the first transition must not run after the persistent constraint has reactivated.
    f.niri().layout.activate_window(&overlay_window);
    f.niri_state().update_keyboard_focus();
    f.niri().layout.activate_window(&game_window);
    f.niri_state().update_keyboard_focus();
    f.double_roundtrip(id);

    assert_eq!(lock_status.unlocked_count.load(Ordering::Relaxed), 1);
    assert_eq!(lock_status.locked_count.load(Ordering::Relaxed), 2);
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        Point::from((960.0_f64, 540.0_f64))
    );
}

#[test]
fn inactive_constraint_does_not_consume_another_constraints_hint() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, game_surface) = fixture_with_window(config);
    let (game_lock, game_status) = f.client(id).lock_pointer(&game_surface);
    f.double_roundtrip(id);

    let overlay = f.client(id).create_window();
    let overlay_surface = overlay.surface.clone();
    overlay.set_title("overlay");
    overlay.commit();
    f.roundtrip(id);

    let overlay = f.client(id).window(&overlay_surface);
    overlay.attach_new_buffer();
    overlay.set_size(200, 200);
    overlay.ack_last_and_commit();
    f.double_roundtrip(id);

    let overlay_window = {
        let protocol_id = overlay_surface.id().protocol_id();
        f.niri()
            .layout
            .windows()
            .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
            .unwrap()
            .1
            .window
            .clone()
    };
    f.niri().layout.move_floating_window(
        Some(&overlay_window),
        PositionChange::SetFixed(860.),
        PositionChange::SetFixed(440.),
        false,
    );

    // Keep the post-unlock pointer under the overlay, then explicitly focus it. This deactivates
    // A while retaining its persistent protocol object.
    assert_eq!(
        f.niri()
            .contents_under(Point::from((960.0_f64, 540.0_f64)))
            .surface
            .unwrap()
            .0
            .id()
            .protocol_id(),
        overlay_surface.id().protocol_id()
    );
    game_lock.set_cursor_position_hint(960.0, 540.0);
    f.client(id).window(&game_surface).commit();
    f.niri().layout.activate_window(&overlay_window);
    f.double_roundtrip(id);
    f.dispatch();
    f.double_roundtrip(id);
    assert!(!game_status.locked.load(Ordering::Relaxed));
    assert_eq!(
        f.niri()
            .seat
            .get_pointer()
            .unwrap()
            .current_focus()
            .unwrap()
            .id()
            .protocol_id(),
        overlay_surface.id().protocol_id()
    );
    assert_eq!(f.client(id).pointer_focus(), Some(&overlay_surface));

    let (overlay_lock, overlay_status) = f.client(id).lock_pointer(&overlay_surface);
    f.double_roundtrip(id);
    assert!(overlay_status.locked.load(Ordering::Relaxed));

    overlay_lock.set_cursor_position_hint(25.0, 25.0);
    f.client(id).window(&overlay_surface).commit();
    f.double_roundtrip(id);

    // Destroying inactive A must not consume or apply active B's committed hint.
    game_lock.destroy();
    f.double_roundtrip(id);
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        Point::from((960.0_f64, 540.0_f64))
    );
    assert!(overlay_status.locked.load(Ordering::Relaxed));
}

#[test]
fn oneshot_constraint_does_not_reactivate_after_keyboard_focus_returns() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, game_surface) = fixture_with_window(config);
    let game_window = {
        let protocol_id = game_surface.id().protocol_id();
        f.niri()
            .layout
            .windows()
            .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
            .unwrap()
            .1
            .window
            .clone()
    };
    let (_lock, status) = f
        .client(id)
        .lock_pointer_with_lifetime(&game_surface, ClientConstraintLifetime::Oneshot);
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 1);

    let (_overlay_surface, overlay_window) = add_floating_overlay_window(&mut f, id);
    f.niri().layout.activate_window(&overlay_window);
    f.double_roundtrip(id);
    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 1);

    f.niri().layout.activate_window(&game_window);
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 1);
    assert!(!status.locked.load(Ordering::Relaxed));
    assert!(f.niri().pointer_constraint_placements.is_empty());
}

#[test]
fn destroying_inactive_persistent_constraint_does_not_warp_or_leave_tracking() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, game_surface) = fixture_with_window(config);
    let (lock, status) = f.client(id).lock_pointer(&game_surface);
    f.double_roundtrip(id);
    lock.set_cursor_position_hint(50.0, 50.0);
    f.client(id).window(&game_surface).commit();
    f.double_roundtrip(id);

    let (_overlay_surface, overlay_window) = add_floating_overlay_window(&mut f, id);
    f.niri().layout.activate_window(&overlay_window);
    f.double_roundtrip(id);
    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 1);
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        Point::from((50.0_f64, 50.0_f64))
    );

    f.niri_state()
        .move_cursor(Point::from((960.0_f64, 540.0_f64)));
    f.double_roundtrip(id);
    lock.destroy();
    f.double_roundtrip(id);

    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        Point::from((960.0_f64, 540.0_f64))
    );
    assert!(f.niri().suspended_pointer_constraints.is_empty());
    assert!(f.niri().pointer_constraint_placements.is_empty());
}

#[test]
fn programmatic_cursor_warp_wins_over_unlock_hint() {
    let (mut f, id, game_surface) = fixture_with_window(Config::default());
    let overlay_surface = add_overlay_layer(&mut f, id, 0, 0);
    let (lock, status) = f.client(id).lock_pointer(&game_surface);
    f.double_roundtrip(id);

    lock.set_cursor_position_hint(20.0, 20.0);
    f.client(id).window(&game_surface).commit();
    f.double_roundtrip(id);

    // A compositor-initiated warp is an explicit focus transition. It must deactivate the lock
    // before pointer.motion() and cancel the resulting deferred hint, otherwise the hint races the
    // intentional destination and can move the cursor a second time.
    f.niri_state()
        .move_cursor(Point::from((50.0_f64, 50.0_f64)));
    f.double_roundtrip(id);

    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 1);
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        Point::from((50.0_f64, 50.0_f64))
    );
    assert_eq!(f.client(id).pointer_focus(), Some(&overlay_surface));
}

#[test]
fn multiple_persistent_constraints_resume_independently() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, game_surface) = fixture_with_window(config);
    let game_window = {
        let protocol_id = game_surface.id().protocol_id();
        f.niri()
            .layout
            .windows()
            .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
            .unwrap()
            .1
            .window
            .clone()
    };
    let (game_lock, game_status) = f.client(id).lock_pointer(&game_surface);
    f.double_roundtrip(id);
    game_lock.set_cursor_position_hint(960.0, 540.0);
    f.client(id).window(&game_surface).commit();
    f.double_roundtrip(id);

    let (overlay_surface, overlay_window) = add_floating_overlay_window(&mut f, id);
    let (_overlay_lock, overlay_status) = f.client(id).lock_pointer(&overlay_surface);
    f.double_roundtrip(id);
    assert_eq!(overlay_status.locked_count.load(Ordering::Relaxed), 0);

    f.niri().layout.activate_window(&overlay_window);
    f.double_roundtrip(id);
    assert_eq!(game_status.unlocked_count.load(Ordering::Relaxed), 1);
    assert_eq!(overlay_status.locked_count.load(Ordering::Relaxed), 1);

    f.niri().layout.activate_window(&game_window);
    f.double_roundtrip(id);
    assert_eq!(overlay_status.unlocked_count.load(Ordering::Relaxed), 1);
    assert_eq!(game_status.locked_count.load(Ordering::Relaxed), 2);
}

#[test]
fn persistent_constraint_reactivation_respects_updated_region() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, game_surface) = fixture_with_window(config);
    let game_window = {
        let protocol_id = game_surface.id().protocol_id();
        f.niri()
            .layout
            .windows()
            .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
            .unwrap()
            .1
            .window
            .clone()
    };
    f.niri_state()
        .move_cursor(Point::from((50.0_f64, 50.0_f64)));
    f.double_roundtrip(id);

    let initial_region = f.client(id).create_region(0, 0, 100, 100);
    let (lock, status) = f.client(id).lock_pointer_with_region(
        &game_surface,
        Some(&initial_region),
        ClientConstraintLifetime::Persistent,
    );
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 1);

    lock.set_cursor_position_hint(960.0, 540.0);
    f.client(id).window(&game_surface).commit();
    let (_overlay_surface, overlay_window) = add_floating_overlay_window(&mut f, id);
    f.niri().layout.activate_window(&overlay_window);
    f.double_roundtrip(id);
    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 1);

    let updated_region = f.client(id).create_region(1000, 1000, 100, 100);
    lock.set_region(Some(&updated_region));
    f.client(id).window(&game_surface).commit();
    f.double_roundtrip(id);

    f.niri().layout.activate_window(&game_window);
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 1);
}

#[test]
fn persistent_constraint_reactivation_respects_updated_surface_input_region() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, game_surface) = fixture_with_window(config);
    let game_window = {
        let protocol_id = game_surface.id().protocol_id();
        f.niri()
            .layout
            .windows()
            .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
            .unwrap()
            .1
            .window
            .clone()
    };
    let (_lock, status) = f.client(id).lock_pointer(&game_surface);
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 1);

    let (_overlay_surface, overlay_window) = add_floating_overlay_window(&mut f, id);
    f.niri().layout.activate_window(&overlay_window);
    f.double_roundtrip(id);
    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 1);

    // The saved position is the center of the game surface. Exclude it from the surface input
    // region while the persistent constraint is inactive.
    let input_region = f.client(id).create_region(0, 0, 100, 100);
    game_surface.set_input_region(Some(&input_region));
    game_surface.commit();
    f.double_roundtrip(id);

    f.niri().layout.activate_window(&game_window);
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 1);
}

#[test]
fn layer_surface_cursor_position_hint_is_applied_after_unlock() {
    let mut f = Fixture::with_config(Config::default());
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let layer = f.client(id).create_layer(None, Layer::Overlay, "");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Top | Anchor::Left),
        size: Some((100, 100)),
        margin: Some(LayerMargin {
            top: 100,
            left: 100,
            ..Default::default()
        }),
        kb_interactivity: Some(KeyboardInteractivity::Exclusive),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(100, 100);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    f.niri_state()
        .move_cursor(Point::from((150.0_f64, 150.0_f64)));
    f.double_roundtrip(id);
    assert_eq!(f.client(id).pointer_focus(), Some(&surface));

    let (lock, status) = f.client(id).lock_pointer(&surface);
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 1);

    lock.set_cursor_position_hint(20.0, 20.0);
    f.client(id).layer(&surface).commit();
    f.double_roundtrip(id);
    lock.destroy();
    f.double_roundtrip(id);

    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        Point::from((120.0_f64, 120.0_f64))
    );
}

#[test]
fn destroyed_surface_drops_suspended_constraint_tracking() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, game_surface) = fixture_with_window(config);
    let (_lock, status) = f.client(id).lock_pointer(&game_surface);
    f.double_roundtrip(id);

    let (_overlay_surface, overlay_window) = add_floating_overlay_window(&mut f, id);
    f.niri().layout.activate_window(&overlay_window);
    f.double_roundtrip(id);
    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 1);
    assert_eq!(f.niri().suspended_pointer_constraints.len(), 1);

    f.client(id).state.destroy_window(&game_surface);
    f.double_roundtrip(id);

    assert!(f.niri().suspended_pointer_constraints.is_empty());
    assert!(f.niri().pointer_constraint_placements.is_empty());
}

#[test]
fn destroying_just_deactivated_constraint_keeps_one_pending_hint() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, game_surface) = fixture_with_window(config);
    let (lock, _status) = f.client(id).lock_pointer(&game_surface);
    f.double_roundtrip(id);
    lock.set_cursor_position_hint(50.0, 50.0);
    f.client(id).window(&game_surface).commit();
    f.double_roundtrip(id);

    let (_overlay_surface, overlay_window) = add_floating_overlay_window(&mut f, id);
    f.niri().layout.activate_window(&overlay_window);
    f.niri_state().update_keyboard_focus();
    assert_eq!(f.niri().pending_pointer_constraint_warps.len(), 1);

    // The terminal removal is processed before the compositor's idle callback. It must clean
    // tracking without replacing the already queued deactivation hint with an empty update.
    lock.destroy();
    f.double_roundtrip(id);
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        Point::from((50.0_f64, 50.0_f64))
    );
    assert!(f.niri().pending_pointer_constraint_warps.is_empty());
    assert!(f.niri().suspended_pointer_constraints.is_empty());
    assert!(f.niri().pointer_constraint_placements.is_empty());
}

#[test]
fn persistent_constraint_uses_hint_committed_while_inactive_on_next_unlock() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, game_surface) = fixture_with_window(config);
    let game_window = {
        let protocol_id = game_surface.id().protocol_id();
        f.niri()
            .layout
            .windows()
            .find(|(_, mapped)| mapped.toplevel().wl_surface().id().protocol_id() == protocol_id)
            .unwrap()
            .1
            .window
            .clone()
    };
    let (lock, status) = f.client(id).lock_pointer(&game_surface);
    f.double_roundtrip(id);
    lock.set_cursor_position_hint(960.0, 540.0);
    f.client(id).window(&game_surface).commit();
    f.double_roundtrip(id);

    let (_overlay_surface, overlay_window) = add_floating_overlay_window(&mut f, id);
    f.niri().layout.activate_window(&overlay_window);
    f.double_roundtrip(id);
    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 1);

    lock.set_cursor_position_hint(100.0, 100.0);
    f.client(id).window(&game_surface).commit();
    f.double_roundtrip(id);

    f.niri().layout.activate_window(&game_window);
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 2);

    f.niri().layout.activate_window(&overlay_window);
    f.double_roundtrip(id);
    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 2);
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        Point::from((100.0_f64, 100.0_f64))
    );
}

#[test]
fn moved_locked_surface_uses_current_geometry_for_hint_and_reactivation() {
    let (mut f, id, game_surface, game_window) = fixture_with_floating_window();
    let (lock, status) = f.client(id).lock_pointer(&game_surface);
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 1);

    lock.set_cursor_position_hint(20.0, 20.0);
    f.client(id).window(&game_surface).commit();
    f.double_roundtrip(id);

    f.niri().layout.move_floating_window(
        Some(&game_window),
        PositionChange::SetFixed(600.),
        PositionChange::SetFixed(400.),
        false,
    );

    let point_in_moved_window = Point::from((700.0_f64, 500.0_f64));
    let (surface, current_origin) = f
        .niri()
        .contents_under(point_in_moved_window)
        .surface
        .unwrap();
    assert_eq!(surface.id().protocol_id(), game_surface.id().protocol_id());
    let expected_hint_target = current_origin + Point::from((20.0_f64, 20.0_f64));

    let (_overlay_surface, overlay_window) = add_floating_overlay_window(&mut f, id);
    f.niri().layout.activate_window(&overlay_window);
    f.double_roundtrip(id);
    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 1);
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        expected_hint_target
    );

    f.niri().layout.activate_window(&game_window);
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 2);
    assert_eq!(f.client(id).pointer_focus(), Some(&game_surface));
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        expected_hint_target
    );
}

#[test]
fn bobbing_locked_surface_uses_rendered_geometry_for_hint() {
    let config = Config::parse_mem(
        r#"
window-rule {
    baba-is-float true
}
"#,
    )
    .unwrap();
    let (mut f, id, game_surface) = fixture_with_window(config);

    // Freeze the adjustable clock at a deterministic point where the bob offset is non-zero.
    let now = f.niri().clock.now();
    f.niri().clock.set_unadjusted(now);
    let _ = f.niri().clock.now();
    f.niri().clock.set_unadjusted(Duration::ZERO);
    f.niri().clock.set_rate(1.);
    let _ = f.niri().clock.now();
    f.niri().clock.set_rate(0.);

    let (_, surface_origin) = f
        .niri()
        .contents_under(Point::from((960.0_f64, 540.0_f64)))
        .surface
        .unwrap();
    assert_ne!(surface_origin.y, 0.);

    let (lock, _status) = f.client(id).lock_pointer(&game_surface);
    f.double_roundtrip(id);
    lock.set_cursor_position_hint(50.0, 50.0);
    f.client(id).window(&game_surface).commit();
    f.double_roundtrip(id);
    lock.destroy();
    f.double_roundtrip(id);

    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        surface_origin + Point::from((50.0_f64, 50.0_f64))
    );
}

#[test]
fn offscreen_workspace_constraint_uses_last_valid_placement() {
    let mut config = Config::default();
    config.animations.off = true;
    let (mut f, id, game_surface) = fixture_with_window(config);
    let (lock, status) = f.client(id).lock_pointer(&game_surface);
    f.double_roundtrip(id);

    lock.set_cursor_position_hint(40.0, 40.0);
    f.client(id).window(&game_surface).commit();
    f.double_roundtrip(id);

    // With animations disabled the old workspace is immediately culled, so current layout
    // geometry can no longer locate its window by the time keyboard focus leaves it.
    f.niri().layout.switch_workspace_down();
    f.double_roundtrip(id);
    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 1);
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        Point::from((40.0_f64, 40.0_f64))
    );

    f.niri().layout.switch_workspace_up();
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 2);
    assert_eq!(f.client(id).pointer_focus(), Some(&game_surface));
}

#[test]
fn overview_close_reactivation_preserves_surface_local_position() {
    let (mut f, id, game_surface) = fixture_with_window(Config::default());
    let region = f.client(id).create_region(950, 530, 20, 20);
    let (_lock, status) = f.client(id).lock_pointer_with_region(
        &game_surface,
        Some(&region),
        ClientConstraintLifetime::Persistent,
    );
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 1);

    assert!(f.niri().layout.open_overview());
    f.niri_state().update_keyboard_focus();
    f.double_roundtrip(id);
    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 1);
    f.niri_complete_animations();

    assert!(f.niri().layout.close_overview());
    f.niri_state().update_keyboard_focus();
    f.double_roundtrip(id);
    // Pointer focus cannot express the inverse transform of a scaled overview surface, so keep
    // the constraint suspended until the close animation reaches its normal scale.
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 1);

    // hide-when-typing may disable ordinary pointer hit-testing while the overview keybind runs.
    // Suspended constraints still need a lifecycle retry once stable geometry returns.
    f.niri().pointer_visibility = PointerVisibility::Disabled;
    f.niri_complete_animations();
    f.double_roundtrip(id);
    assert_eq!(status.locked_count.load(Ordering::Relaxed), 2);

    f.niri_state()
        .process_input_event(InputEvent::<TestInputBackend>::PointerMotion {
            event: TestRelativeEvent { dx: 1., dy: 1. },
        });
    f.double_roundtrip(id);

    assert_eq!(status.locked_count.load(Ordering::Relaxed), 2);
    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 1);
    assert_eq!(f.client(id).pointer_focus(), Some(&game_surface));
}

#[test]
fn same_surface_compositor_warp_cancels_pending_hint() {
    let config = Config::parse_mem(
        r#"
window-rule {
    match title="^overlay$"
    open-floating true
    open-focused false
    default-column-width { fixed 200; }
    default-window-height { fixed 200; }
}
"#,
    )
    .unwrap();
    let (mut f, id, game_surface) = fixture_with_window(config);
    let (lock, status) = f.client(id).lock_pointer(&game_surface);
    f.double_roundtrip(id);
    lock.set_cursor_position_hint(20.0, 20.0);
    f.client(id).window(&game_surface).commit();
    f.double_roundtrip(id);

    let (_overlay_surface, overlay_window) = add_floating_overlay_window(&mut f, id);
    f.niri().layout.activate_window(&overlay_window);
    f.niri_state().update_keyboard_focus();
    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 0);
    assert_eq!(f.niri().pending_pointer_constraint_warps.len(), 1);

    // This point is still on the fullscreen game surface, so pointer focus does not change. The
    // compositor warp is nevertheless newer than the queued client hint and must win.
    let compositor_target = Point::from((500.0_f64, 500.0_f64));
    f.niri_state().move_cursor(compositor_target);
    assert!(f.niri().pending_pointer_constraint_warps.is_empty());
    f.double_roundtrip(id);

    assert_eq!(status.unlocked_count.load(Ordering::Relaxed), 1);
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        compositor_target
    );
}
