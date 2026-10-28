use std::os::fd::AsFd as _;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use niri_config::Config;
use smithay::backend::input::InputEvent;
use smithay::input::pointer::CursorImageStatus;
use smithay::reexports::rustix::fs::{ftruncate, memfd_create, MemfdFlags};
use smithay::reexports::wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
};
use smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_cursor_session_v1::ExtImageCopyCaptureCursorSessionV1,
    ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1,
    ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1,
    ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
};
use smithay::utils::Point;
use wayland_client::protocol::{
    wl_pointer::WlPointer,
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
};

use super::client::State as ClientState;
use super::test_input_backend::{TestInputBackend, TestPointerAxisEvent};
use super::Fixture;
use crate::cursor::RenderCursor;
use crate::niri::{PointerVisibility, RedrawState};

wayland_client::delegate_noop!(ClientState: ignore ExtImageCaptureSourceV1);
wayland_client::delegate_noop!(ClientState: ignore ExtOutputImageCaptureSourceManagerV1);
wayland_client::delegate_noop!(ClientState: ignore ExtImageCopyCaptureManagerV1);
wayland_client::delegate_noop!(ClientState: ignore ExtImageCopyCaptureCursorSessionV1);
wayland_client::delegate_noop!(ClientState: ignore ExtImageCopyCaptureSessionV1);
wayland_client::delegate_noop!(ClientState: ignore ExtImageCopyCaptureFrameV1);
wayland_client::delegate_noop!(ClientState: ignore WlPointer);
wayland_client::delegate_noop!(ClientState: ignore WlShm);
wayland_client::delegate_noop!(ClientState: ignore WlShmPool);

struct AnimatedTheme(PathBuf);

impl AnimatedTheme {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "niri-animation-output-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let cursor = path.join("cursors_scalable/default");
        std::fs::create_dir_all(&cursor).unwrap();
        std::fs::write(
            cursor.join("metadata.json"),
            r#"[{"filename":"red.svg","nominal_size":24,"hotspot_x":4,"hotspot_y":4,"delay":20},{"filename":"blue.svg","nominal_size":24,"hotspot_x":4,"hotspot_y":4,"delay":20}]"#,
        )
        .unwrap();
        for color in ["red", "blue"] {
            std::fs::write(
                cursor.join(format!("{color}.svg")),
                format!("<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><path fill='{color}' d='M0 0H8V8H0Z'/></svg>"),
            )
            .unwrap();
        }
        Self(path)
    }

    fn fixture(&self) -> Fixture {
        let mut config = Config::default();
        config.cursor.theme = self.0.to_str().unwrap().to_owned();
        config.cursor.size = 24;
        let mut f = Fixture::with_config(config);
        f.add_output(1, (100, 100));
        f.add_output(2, (100, 100));
        // Exercise real worker preparation and completion, not an injected animation predicate.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            f.state.server.dispatch();
            if matches!(f.niri().cursor_manager.get_render_cursor(1.0), RenderCursor::Named { cursor } if cursor.is_animated_cursor())
            {
                break;
            }
            assert!(Instant::now() < deadline, "animated SVG did not load");
            std::thread::sleep(Duration::from_millis(1));
        }
        f.niri_complete_animations();
        f
    }
}

impl Drop for AnimatedTheme {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn redraw(f: &mut Fixture) {
    let state = f.niri_state();
    state.niri.queue_redraw_all();
    state.niri.redraw_queued_outputs(&mut state.backend);
}

fn animation_flags(f: &mut Fixture) -> [bool; 2] {
    let outputs = [f.niri_output(1), f.niri_output(2)];
    outputs.map(|output| f.niri().output_state[&output].unfinished_animations_remain)
}

fn tablet_at(f: &mut Fixture, output: u8, offset: (f64, f64)) {
    let output = f.niri_output(output);
    let origin = f.niri().global_space.output_geometry(&output).unwrap().loc;
    f.niri().tablet_cursor_location = Some(origin.to_f64() + Point::from(offset));
}

#[test]
fn hidden_animated_svg_does_not_keep_outputs_redrawing() {
    let theme = AnimatedTheme::new();
    let mut f = theme.fixture();
    tablet_at(&mut f, 1, (50., 50.));
    for visibility in [PointerVisibility::Hidden, PointerVisibility::Disabled] {
        f.niri().pointer_visibility = visibility;
        redraw(&mut f);
        assert_eq!(animation_flags(&mut f), [false, false]);
        assert!(matches!(
            f.niri().cursor_manager.cursor_image(),
            CursorImageStatus::Named(_)
        ));
        assert!(matches!(
            f.niri().cursor_manager.get_render_cursor(1.0),
            RenderCursor::Named { cursor } if cursor.is_animated_cursor()
        ));
    }
}

#[test]
fn wheel_reveal_restarts_idle_cursor_animation() {
    let theme = AnimatedTheme::new();
    let mut f = theme.fixture();
    f.niri_state().move_cursor(Point::from((50., 50.)));
    for (visibility, tablet) in [
        (PointerVisibility::Hidden, false),
        (PointerVisibility::Disabled, false),
        (PointerVisibility::Visible, true),
    ] {
        f.niri().pointer_visibility = visibility;
        if tablet {
            tablet_at(&mut f, 2, (50., 50.));
        }
        redraw(&mut f);
        let output = f.niri_output(1);
        assert!(matches!(
            f.niri().output_state[&output].redraw_state,
            RedrawState::Idle
        ));
        let state = f.niri_state();
        state.process_input_event(InputEvent::<TestInputBackend>::PointerAxis {
            event: TestPointerAxisEvent,
        });
        // No artificial queue_redraw_all(): the input transition must wake the output.
        state.niri.redraw_queued_outputs(&mut state.backend);
        assert_eq!(animation_flags(&mut f), [true, false]);
    }
}

#[test]
fn animated_svg_only_keeps_overlapping_outputs_redrawing() {
    let theme = AnimatedTheme::new();
    let mut f = theme.fixture();
    f.niri().pointer_visibility = PointerVisibility::Visible;
    tablet_at(&mut f, 1, (50., 50.));
    redraw(&mut f);
    assert_eq!(animation_flags(&mut f), [true, false]);

    tablet_at(&mut f, 2, (50., 50.));
    redraw(&mut f);
    assert_eq!(animation_flags(&mut f), [false, true]);

    // Both outputs abut horizontally. The hotspot lies outside output 1, but its image
    // still overlaps it. This also exercises the tablet location rather than the mouse.
    let first = f.niri_output(1);
    let second = f.niri_output(2);
    let first_geo = f.niri().global_space.output_geometry(&first).unwrap();
    let second_geo = f.niri().global_space.output_geometry(&second).unwrap();
    assert_eq!(first_geo.loc.x + first_geo.size.w, second_geo.loc.x);
    tablet_at(&mut f, 2, (2., 50.));
    redraw(&mut f);
    assert_eq!(animation_flags(&mut f), [true, true]);

    // No display consumer when the entire image is outside all outputs.
    tablet_at(&mut f, 1, (-20., -20.));
    redraw(&mut f);
    assert_eq!(animation_flags(&mut f), [false, false]);
}

#[test]
fn hidden_cursor_capture_only_animates_while_a_frame_is_pending() {
    let theme = AnimatedTheme::new();
    let mut f = theme.fixture();
    let id = f.add_client();
    f.double_roundtrip(id);
    let (cursor_session, capture_session, shm) = {
        let client = f.client(id);
        let registry = client.display.get_registry(&client.qh, ());
        let global = |interface: &str| {
            client
                .state
                .globals
                .iter()
                .find(|g| g.interface == interface)
                .unwrap()
                .name
        };
        let source_manager: ExtOutputImageCaptureSourceManagerV1 = registry.bind(
            global("ext_output_image_capture_source_manager_v1"),
            1,
            &client.qh,
            (),
        );
        let capture_manager: ExtImageCopyCaptureManagerV1 = registry.bind(
            global("ext_image_copy_capture_manager_v1"),
            1,
            &client.qh,
            (),
        );
        let shm: WlShm = registry.bind(global("wl_shm"), 1, &client.qh, ());
        let output = client
            .state
            .outputs
            .iter()
            .find(|(_, name)| *name == "headless-1")
            .unwrap()
            .0;
        let source = source_manager.create_source(output, &client.qh, ());
        let seat = client.state.seats.keys().next().unwrap();
        let pointer = seat.get_pointer(&client.qh, ());
        let cursor_session =
            capture_manager.create_pointer_cursor_session(&source, &pointer, &client.qh, ());
        let capture_session = cursor_session.get_capture_session(&client.qh, ());
        (cursor_session, capture_session, shm)
    };
    f.roundtrip(id);
    assert_eq!(f.niri().image_copy_cursor_sessions.len(), 1);
    f.niri().pointer_visibility = PointerVisibility::Hidden;
    tablet_at(&mut f, 2, (50., 50.));
    redraw(&mut f);
    assert_eq!(animation_flags(&mut f), [false, false]);

    let output = f.niri_output(1);
    let size =
        crate::handlers::image_copy_capture::cursor_capture_constraints(f.niri(), &output).size;
    let fd = memfd_create("niri-test-cursor-capture", MemfdFlags::CLOEXEC).unwrap();
    let len = size.w * size.h * 4;
    ftruncate(&fd, len as u64).unwrap();
    let frame = {
        let client = f.client(id);
        let pool = shm.create_pool(fd.as_fd(), len, &client.qh, ());
        let buffer = pool.create_buffer(
            0,
            size.w,
            size.h,
            size.w * 4,
            wl_shm::Format::Argb8888,
            &client.qh,
            (),
        );
        let frame = capture_session.create_frame(&client.qh, ());
        frame.attach_buffer(&buffer);
        frame.damage_buffer(0, 0, size.w, size.h);
        frame.capture();
        frame
    };
    f.roundtrip(id);
    assert!(f.niri().image_copy_cursor_sessions[0]
        .pending_frame
        .is_some());
    // Explicit image demand is scoped to its source, even when hidden and on another output.
    redraw(&mut f);
    assert_eq!(animation_flags(&mut f), [true, false]);

    frame.destroy();
    f.roundtrip(id);
    assert!(f.niri().image_copy_cursor_sessions[0]
        .pending_frame
        .is_none());
    redraw(&mut f);
    assert_eq!(animation_flags(&mut f), [false, false]);
    capture_session.destroy();
    cursor_session.destroy();
    f.roundtrip(id);
}

#[test]
fn egl_hidden_cursor_remains_toggleable_in_screenshots() {
    let theme = AnimatedTheme::new();
    let mut f = theme.fixture();
    f.niri_state().backend.headless().add_renderer().unwrap();
    tablet_at(&mut f, 1, (50., 50.));
    let output = f.niri_output(1);
    let state = f.niri_state();
    state.niri.pointer_visibility = PointerVisibility::Hidden;
    state
        .backend
        .with_primary_renderer(|renderer| {
            let screenshots = state.niri.capture_screenshots(renderer).collect();
            assert!(state.niri.screenshot_ui.open(
                renderer,
                screenshots,
                output.clone(),
                false,
                None
            ));
            let (_, without_pointer) = state.niri.screenshot_ui.capture(renderer).unwrap();
            state.niri.screenshot_ui.toggle_pointer();
            let (_, with_pointer) = state.niri.screenshot_ui.capture(renderer).unwrap();
            assert_ne!(
                with_pointer, without_pointer,
                "hidden image must remain toggleable in screenshots"
            );
            state.niri.screenshot_ui.close();

            state.niri.pointer_visibility = PointerVisibility::Disabled;
            let screenshots = state.niri.capture_screenshots(renderer).collect();
            assert!(state
                .niri
                .screenshot_ui
                .open(renderer, screenshots, output, false, None));
            let (_, without_pointer) = state.niri.screenshot_ui.capture(renderer).unwrap();
            state.niri.screenshot_ui.toggle_pointer();
            let (_, with_pointer) = state.niri.screenshot_ui.capture(renderer).unwrap();
            assert_eq!(
                with_pointer, without_pointer,
                "disabled pointer must stay absent from screenshots"
            );
        })
        .unwrap();
}
