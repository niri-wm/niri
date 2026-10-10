use niri_config::Config;
use smithay::utils::{Logical, Size};
use zbus::fdo;

use super::Fixture;
use crate::dbus::mutter_screen_cast::ScreenCastToNiri;

fn window_size(f: &mut Fixture, id: u64) -> fdo::Result<Option<Size<i32, Logical>>> {
    let (reply, rx) = async_channel::bounded(1);
    f.niri_state()
        .on_screen_cast_msg(ScreenCastToNiri::GetWindowSize { id, reply });
    rx.try_recv().unwrap()
}

#[test]
fn window_size_uses_logical_capture_bounds() {
    for scale in [1.0, 1.5, 2.0] {
        let config = format!(r#"output "headless-1" {{ scale {scale}; }}"#);
        let config = Config::parse_mem(&config).unwrap();
        let mut f = Fixture::with_config(config);
        f.add_output(1, (1920, 1080));
        assert_eq!(f.niri_output(1).current_scale().fractional_scale(), scale);

        let client = f.add_client();
        let window = f.client(client).create_window();
        let surface = window.surface.clone();
        window.commit();
        f.roundtrip(client);

        let window = f.client(client).window(&surface);
        window.attach_new_buffer();
        window.set_size(800, 600);
        // Window geometry excludes decorations, but capture includes the whole surface.
        window.xdg_surface.set_window_geometry(10, 20, 780, 570);
        window.ack_last_and_commit();
        f.double_roundtrip(client);

        let id = f.niri().layout.windows().next().unwrap().1.id().get();
        assert_eq!(window_size(&mut f, id).unwrap(), Some((800, 600).into()));

        let window = f.client(client).window(&surface);
        window.set_size(900, 700);
        window.commit();
        f.double_roundtrip(client);
        assert_eq!(window_size(&mut f, id).unwrap(), Some((900, 700).into()));

        let window = f.client(client).window(&surface);
        window.attach_null();
        window.commit();
        f.double_roundtrip(client);
        assert!(matches!(
            window_size(&mut f, id),
            Err(fdo::Error::Failed(_))
        ));
    }
}

#[test]
fn dynamic_target_has_no_initial_size() {
    let mut f = Fixture::new();
    let id = f.niri().casting.dynamic_cast_id_for_portal.get();
    assert_eq!(window_size(&mut f, id).unwrap(), None);
    assert!(matches!(window_size(&mut f, 0), Err(fdo::Error::Failed(_))));

    let (reply, rx) = async_channel::bounded(1);
    drop(rx);
    f.niri_state()
        .on_screen_cast_msg(ScreenCastToNiri::GetWindowSize { id, reply });
}
