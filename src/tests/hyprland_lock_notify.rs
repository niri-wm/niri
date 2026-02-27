use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use wayland_client::protocol::{wl_callback, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

use crate::tests::fixture::Fixture;

mod hyprland_lock_notify {
    pub mod v1 {
        pub use self::generated::client;
        mod generated {
            pub mod client {
                #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
                #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
                #![allow(missing_docs, clippy::all)]

                use wayland_client;
                use wayland_client::protocol::*;

                pub mod __interfaces {
                    use wayland_client;
                    use wayland_client::protocol::__interfaces::*;
                    wayland_scanner::generate_interfaces!("resources/hyprland-lock-notify-v1.xml");
                }
                use self::__interfaces::*;

                wayland_scanner::generate_client_code!("resources/hyprland-lock-notify-v1.xml");
            }
        }
    }
}

use hyprland_lock_notify::v1::client::hyprland_lock_notification_v1::{
    self, HyprlandLockNotificationV1,
};
use hyprland_lock_notify::v1::client::hyprland_lock_notifier_v1::HyprlandLockNotifierV1;

struct TestState {
    locked: bool,
    unlocked: bool,
}

struct SyncData {
    done: AtomicBool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for TestState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_callback::WlCallback, Arc<SyncData>> for TestState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_callback::WlCallback,
        event: wl_callback::Event,
        data: &Arc<SyncData>,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            wl_callback::Event::Done { .. } => data.done.store(true, Ordering::Relaxed),
            _ => unreachable!(),
        }
    }
}

impl Dispatch<HyprlandLockNotifierV1, ()> for TestState {
    fn event(
        _state: &mut Self,
        _proxy: &HyprlandLockNotifierV1,
        _event: <HyprlandLockNotifierV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<HyprlandLockNotificationV1, ()> for TestState {
    fn event(
        state: &mut Self,
        _proxy: &HyprlandLockNotificationV1,
        event: <HyprlandLockNotificationV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            hyprland_lock_notification_v1::Event::Locked => state.locked = true,
            hyprland_lock_notification_v1::Event::Unlocked => state.unlocked = true,
        }
    }
}

fn roundtrip(
    fixture: &mut Fixture,
    conn: &Connection,
    qh: &QueueHandle<TestState>,
    state: &mut TestState,
    event_queue: &mut wayland_client::EventQueue<TestState>,
) {
    let done = Arc::new(SyncData {
        done: AtomicBool::new(false),
    });
    conn.display().sync(qh, done.clone());
    conn.flush().unwrap();

    while !done.done.load(Ordering::Relaxed) {
        fixture.dispatch();

        if let Some(guard) = event_queue.prepare_read() {
            guard.read().unwrap();
        }
        event_queue.dispatch_pending(state).unwrap();
    }
}

#[test]
fn test_lock_notify() {
    let mut fixture = Fixture::new();
    let client_id = fixture.add_client();
    let client = fixture.client(client_id);
    let conn = client.connection.clone();

    let mut state = TestState {
        locked: false,
        unlocked: false,
    };
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let display = conn.display();
    let registry = display.get_registry(&qh, ());

    let global = client
        .state
        .globals
        .iter()
        .find(|g| g.interface == HyprlandLockNotifierV1::interface().name)
        .expect("hyprland_lock_notifier_v1 global not found");

    let notifier =
        registry.bind::<HyprlandLockNotifierV1, _, _>(global.name, global.version, &qh, ());
    let _notification = notifier.get_lock_notification(&qh, ());

    // Initial roundtrip to establish objects
    roundtrip(&mut fixture, &conn, &qh, &mut state, &mut event_queue);

    // Trigger locked
    fixture.niri().hyprland_lock_notify_state.send_locked();

    roundtrip(&mut fixture, &conn, &qh, &mut state, &mut event_queue);
    assert!(state.locked, "Did not receive locked event");
    state.locked = false;

    fixture.niri().unlock();

    roundtrip(&mut fixture, &conn, &qh, &mut state, &mut event_queue);
    assert!(state.unlocked, "Did not receive unlocked event");
}
