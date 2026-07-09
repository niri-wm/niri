use mutter_x11_interop::MutterX11Interop;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use super::raw::mutter_x11_interop::v1::server::mutter_x11_interop;

const VERSION: u32 = 1;

pub struct MutterX11InteropManagerState {}

pub struct MutterX11InteropManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait MutterX11InteropHandler {}

impl MutterX11InteropManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<MutterX11Interop, MutterX11InteropManagerGlobalData>,
        D: Dispatch<MutterX11Interop, ()>,
        D: MutterX11InteropHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = MutterX11InteropManagerGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, MutterX11Interop, _>(VERSION, global_data);

        Self {}
    }
}

impl<D> GlobalDispatch<MutterX11Interop, MutterX11InteropManagerGlobalData, D>
    for MutterX11InteropManagerState
where
    D: GlobalDispatch<MutterX11Interop, MutterX11InteropManagerGlobalData>,
    D: Dispatch<MutterX11Interop, ()>,
    D: MutterX11InteropHandler,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<MutterX11Interop>,
        _manager_state: &MutterX11InteropManagerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ());
    }

    fn can_view(client: Client, global_data: &MutterX11InteropManagerGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<MutterX11Interop, (), D> for MutterX11InteropManagerState
where
    D: Dispatch<MutterX11Interop, ()>,
    D: MutterX11InteropHandler,
    D: 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &MutterX11Interop,
        request: <MutterX11Interop as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            mutter_x11_interop::Request::Destroy => (),
            mutter_x11_interop::Request::SetX11Parent { .. } => (),
        }
    }
}

#[macro_export]
macro_rules! delegate_mutter_x11_interop {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::mutter_x11_interop::v1::server::mutter_x11_interop::MutterX11Interop: $crate::protocols::mutter_x11_interop::MutterX11InteropManagerGlobalData
        ] => $crate::protocols::mutter_x11_interop::MutterX11InteropManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::mutter_x11_interop::v1::server::mutter_x11_interop::MutterX11Interop: ()
        ] => $crate::protocols::mutter_x11_interop::MutterX11InteropManagerState);
    };
}
