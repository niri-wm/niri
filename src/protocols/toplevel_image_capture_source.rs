use std::collections::HashSet;

use smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::server::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use smithay::reexports::wayland_protocols::ext::image_capture_source::v1::server::{
    ext_foreign_toplevel_image_capture_source_manager_v1::{
        self, ExtForeignToplevelImageCaptureSourceManagerV1,
    },
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
    backend::GlobalId,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::image_capture_source::{ImageCaptureSource, ImageCaptureSourceData, ImageCaptureSourceHandler};

/// Data for the toplevel image capture source manager global.
pub struct ToplevelImageCaptureGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// State for the toplevel image capture source manager.
///
/// This binds the [`ExtForeignToplevelImageCaptureSourceManagerV1`] global,
/// allowing clients to create capture sources from foreign toplevels.
pub struct ToplevelImageCaptureManagerState {
    global: GlobalId,
    instances: HashSet<ExtForeignToplevelImageCaptureSourceManagerV1>,
}

impl ToplevelImageCaptureManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ToplevelImageCaptureGlobalData>,
        D: Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()>,
        D: Dispatch<ExtImageCaptureSourceV1, ImageCaptureSourceData>,
        D: ImageCaptureSourceHandler,
        D: ToplevelImageCaptureHandler,
        D: 'static,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, ExtForeignToplevelImageCaptureSourceManagerV1, _>(
            1,
            ToplevelImageCaptureGlobalData {
                filter: Box::new(filter),
            },
        );

        Self {
            global,
            instances: HashSet::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

impl<D> GlobalDispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ToplevelImageCaptureGlobalData, D>
    for ToplevelImageCaptureManagerState
where
    D: GlobalDispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ToplevelImageCaptureGlobalData>,
    D: Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()>,
    D: Dispatch<ExtImageCaptureSourceV1, ImageCaptureSourceData>,
    D: ImageCaptureSourceHandler,
    D: ToplevelImageCaptureHandler,
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ExtForeignToplevelImageCaptureSourceManagerV1>,
        _global_data: &ToplevelImageCaptureGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &ToplevelImageCaptureGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, (), D>
    for ToplevelImageCaptureManagerState
where
    D: GlobalDispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ToplevelImageCaptureGlobalData>,
    D: Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()>,
    D: Dispatch<ExtImageCaptureSourceV1, ImageCaptureSourceData>,
    D: ImageCaptureSourceHandler,
    D: ToplevelImageCaptureHandler,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &ExtForeignToplevelImageCaptureSourceManagerV1,
        request: ext_foreign_toplevel_image_capture_source_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_foreign_toplevel_image_capture_source_manager_v1::Request::CreateSource {
                source,
                toplevel_handle,
            } => {
                let capture_source = ImageCaptureSource::new();

                // Look up the WlSurface for this toplevel handle and store it in user_data.
                if let Some(wl_surface) = state.lookup_toplevel_surface(&toplevel_handle) {
                    capture_source.user_data().insert_if_missing(|| wl_surface);
                }

                let source_resource = data_init.init(
                    source,
                    ImageCaptureSourceData {
                        source: capture_source.clone(),
                    },
                );

                capture_source.add_instance(&source_resource);
            }
            ext_foreign_toplevel_image_capture_source_manager_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        _resource: &ExtForeignToplevelImageCaptureSourceManagerV1,
        _data: &(),
    ) {
        let manager_state = state.toplevel_image_capture_manager_state();
        manager_state.instances.clear();
    }
}

/// Trait for looking up toplevel surfaces from foreign toplevel handles
/// and accessing the manager state.
pub trait ToplevelImageCaptureHandler {
    fn toplevel_image_capture_manager_state(&mut self) -> &mut ToplevelImageCaptureManagerState;

    fn lookup_toplevel_surface(
        &mut self,
        handle: &ExtForeignToplevelHandleV1,
    ) -> Option<WlSurface>;
}

#[macro_export]
macro_rules! delegate_toplevel_image_capture_source {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::image_capture_source::v1::server::ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1: $crate::protocols::toplevel_image_capture_source::ToplevelImageCaptureGlobalData
        ] => $crate::protocols::toplevel_image_capture_source::ToplevelImageCaptureManagerState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::image_capture_source::v1::server::ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1: ()
        ] => $crate::protocols::toplevel_image_capture_source::ToplevelImageCaptureManagerState);
    };
}
