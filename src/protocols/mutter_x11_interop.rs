use mutter_x11_interop::MutterX11Interop;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::{Dispatch2, GlobalData, GlobalDispatch2};

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

impl<D> GlobalDispatch2<MutterX11Interop, D> for MutterX11InteropManagerGlobalData
where
    D: Dispatch<MutterX11Interop, GlobalData>,
    D: MutterX11InteropHandler,
{
    fn bind(
        &self,
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<MutterX11Interop>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, GlobalData);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl<D> Dispatch2<MutterX11Interop, D> for GlobalData
where
    D: MutterX11InteropHandler,
{
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        _resource: &MutterX11Interop,
        request: <MutterX11Interop as Resource>::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            mutter_x11_interop::Request::Destroy => (),
            mutter_x11_interop::Request::SetX11Parent { .. } => (),
        }
    }
}
