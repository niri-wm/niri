use std::fmt::Write as _;

use insta::assert_snapshot;
use niri_ipc::SizeChange;
use wayland_client::protocol::wl_surface::WlSurface;

use super::client::ClientId;
use super::*;
use crate::layout::LayoutElement;
use crate::niri::Niri;

fn format_window_sizes(niri: &Niri) -> String {
    let mut buf = String::new();
    for (_out, mapped) in niri.layout.windows() {
        let size = mapped.size();
        writeln!(&mut buf, "{} × {}", size.w, size.h).unwrap();
    }
    buf
}

fn create_window(f: &mut Fixture, id: ClientId, w: u16, h: u16) -> WlSurface {
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(w, h);
    window.ack_last_and_commit();
    f.roundtrip(id);

    surface
}

#[test]
fn column_resize_waits_for_both_windows() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let surface1 = create_window(&mut f, id, 100, 100);
    let surface2 = create_window(&mut f, id, 200, 200);
    f.double_roundtrip(id);

    let _ = f.client(id).window(&surface1).recent_configures();
    let _ = f.client(id).window(&surface2).recent_configures();

    // Consume into one column.
    f.niri().layout.consume_or_expel_window_left(None);
    f.double_roundtrip(id);

    // Commit for the column consume.
    let window = f.client(id).window(&surface1);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 936 × 516, bounds: 1888 × 1048, states: []"
    );
    window.ack_last_and_commit();

    let window = f.client(id).window(&surface2);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 936 × 516, bounds: 1888 × 1048, states: [Activated]"
    );
    window.ack_last_and_commit();

    f.double_roundtrip(id);

    // This should say 100 × 100 and 200 × 200.
    assert_snapshot!(format_window_sizes(f.niri()), @r"
    100 × 100
    200 × 200
    ");

    // Issue a resize.
    f.niri()
        .layout
        .set_column_width(SizeChange::AdjustFixed(10));
    f.double_roundtrip(id);

    // Commit window 1 in response to resize.
    let window = f.client(id).window(&surface1);
    window.set_size(300, 300);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    // This should still say 100 × 100 as we're waiting in a transaction for the second window.
    assert_snapshot!(format_window_sizes(f.niri()), @r"
    100 × 100
    200 × 200
    ");

    // Commit window 2 in response to resize.
    let window = f.client(id).window(&surface2);
    window.set_size(400, 400);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    // This should say 300 × 300 and 400 × 400 as the transaction completed.
    assert_snapshot!(format_window_sizes(f.niri()), @r"
    300 × 300
    400 × 400
    ");
}
