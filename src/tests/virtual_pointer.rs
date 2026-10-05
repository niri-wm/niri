use wayland_client::protocol::wl_pointer;

use super::*;

#[test]
fn axis_discrete_overflow() {
    let mut f = Fixture::new();
    let id = f.add_client();

    let client = f.client(id);
    let manager = client.state.virtual_pointer_manager.as_ref().unwrap();
    let pointer = manager.create_virtual_pointer(None, &client.qh, ());
    pointer.axis_discrete(0, wl_pointer::Axis::VerticalScroll, 0., i32::MAX);
    f.roundtrip(id);
}
