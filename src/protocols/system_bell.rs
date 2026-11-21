use std::sync::Arc;

use smithay::reexports::wayland_protocols::xdg::system_bell::v1::server::xdg_system_bell_v1::{
    self, XdgSystemBellV1,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::protocols::EmptyData;

pub struct SystemBellManagerState;

pub struct SystemBellGlobalData {
    filter: Arc<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait SystemBellHandler {
    fn ring(&mut self, surface: Option<WlSurface>);
}

impl SystemBellManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<XdgSystemBellV1, SystemBellGlobalData>,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = SystemBellGlobalData {
            filter: Arc::new(filter),
        };

        display.create_global::<D, XdgSystemBellV1, _>(1, global_data);

        Self
    }
}

impl<D> GlobalDispatch2<XdgSystemBellV1, D> for SystemBellGlobalData
where
    D: Dispatch<XdgSystemBellV1, EmptyData>,
{
    fn bind(
        &self,
        _: &mut D,
        _: &DisplayHandle,
        _: &Client,
        resource: New<XdgSystemBellV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, EmptyData);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl<D> Dispatch2<XdgSystemBellV1, D> for EmptyData
where
    D: SystemBellHandler,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _resource: &XdgSystemBellV1,
        request: <XdgSystemBellV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            xdg_system_bell_v1::Request::Ring { surface } => state.ring(surface),
            xdg_system_bell_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }
}
