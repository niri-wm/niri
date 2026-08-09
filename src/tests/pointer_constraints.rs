use std::path::PathBuf;
use std::sync::atomic::Ordering;

use niri_config::Config;
use niri_ipc::PositionChange;
use smithay::backend::input::{
    AbsolutePositionEvent, ButtonState, Device, DeviceCapability, Event, InputBackend, InputEvent,
    PointerButtonEvent, PointerMotionAbsoluteEvent, UnusedEvent,
};
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;
use smithay::reexports::wayland_server::Resource as _;
use smithay::utils::Point;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::Proxy as _;

use super::*;
use crate::input::backend_ext::NiriInputDevice;
use crate::tests::client::{ClientId, LayerConfigureProps, LayerMargin};

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

impl InputBackend for TestInputBackend {
    type Device = TestDevice;
    type KeyboardKeyEvent = UnusedEvent;
    type PointerAxisEvent = UnusedEvent;
    type PointerButtonEvent = TestButtonEvent;
    type PointerMotionEvent = UnusedEvent;
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
