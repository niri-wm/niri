use std::time::{Duration, Instant};

use niri_config::Config;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::ImportMem;

use super::server::Server;

fn texture_exists(server: &mut Server, id: u32) -> bool {
    server
        .state
        .backend
        .with_primary_renderer(|renderer| {
            renderer
                .with_context(|gl| unsafe { gl.IsTexture(id) != 0 })
                .unwrap()
        })
        .unwrap()
}

fn wait_for_cleanup(server: &mut Server, id: u32) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while texture_exists(server, id) && Instant::now() < deadline {
        server
            .event_loop
            .dispatch(Duration::from_millis(100), &mut server.state)
            .unwrap();
        server.state.refresh_and_flush_clients();
    }
    assert!(
        !texture_exists(server, id),
        "texture was not reclaimed without a frame"
    );
}

fn check_cleanup(monitors_off: bool) {
    let mut server = Server::new(Config::default());
    server.state.backend.headless().add_renderer().unwrap();
    if monitors_off {
        let state = &mut server.state;
        state
            .backend
            .headless()
            .add_output(&mut state.niri, 1, (1280, 720));
        state.niri.deactivate_monitors(&mut state.backend);
    }

    let (live, dead_id) = server
        .state
        .backend
        .with_primary_renderer(|renderer| {
            let live = renderer
                .import_memory(&[255; 4], Fourcc::Abgr8888, (1, 1).into(), false)
                .unwrap();
            let dead = renderer
                .import_memory(&[255; 4], Fourcc::Abgr8888, (1, 1).into(), false)
                .unwrap();
            let dead_id = dead.tex_id();
            drop(dead);
            (live, dead_id)
        })
        .unwrap();

    // Drop queues GL destruction; it cannot delete the object immediately.
    assert!(texture_exists(&mut server, dead_id));
    wait_for_cleanup(&mut server, dead_id);
    let live_id = live.tex_id();
    assert!(texture_exists(&mut server, live_id));

    // Maintenance must keep running, not just clean up once after power-off.
    drop(live);
    assert!(texture_exists(&mut server, live_id));
    wait_for_cleanup(&mut server, live_id);
}

#[test]
fn egl_cleanup_without_outputs() {
    check_cleanup(false);
}

#[test]
fn egl_cleanup_with_monitors_off() {
    check_cleanup(true);
}
