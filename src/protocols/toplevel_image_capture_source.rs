use smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::server::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use smithay::reexports::wayland_protocols::ext::image_capture_source::v1::server::ext_foreign_toplevel_image_capture_source_manager_v1::{
    self, ExtForeignToplevelImageCaptureSourceManagerV1,
};
use smithay::reexports::wayland_protocols::ext::image_capture_source::v1::server::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::image_capture_source::{
    ImageCaptureSource, ImageCaptureSourceData, ImageCaptureSourceHandler,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::protocols::EmptyData;

const VERSION: u32 = 1;

/// State for the toplevel image capture source manager.
///
/// This binds the [`ExtForeignToplevelImageCaptureSourceManagerV1`] global, allowing clients to
/// create capture sources from foreign toplevels. It is a niri-side replacement for Smithay's
/// `ToplevelCaptureSourceState`, which requires Smithay's own ext-foreign-toplevel-list
/// implementation, while niri has its own in [`crate::protocols::foreign_toplevel`].
pub struct ToplevelImageCaptureManagerState {
    global: GlobalId,
}

pub struct ToplevelImageCaptureGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait ToplevelImageCaptureHandler: ImageCaptureSourceHandler {
    /// Looks up the toplevel surface for a foreign toplevel handle.
    fn lookup_toplevel_surface(&mut self, handle: &ExtForeignToplevelHandleV1)
        -> Option<WlSurface>;
}

impl ToplevelImageCaptureManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<
            ExtForeignToplevelImageCaptureSourceManagerV1,
            ToplevelImageCaptureGlobalData,
        >,
        D: ToplevelImageCaptureHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = ToplevelImageCaptureGlobalData {
            filter: Box::new(filter),
        };
        let global = display.create_global::<D, ExtForeignToplevelImageCaptureSourceManagerV1, _>(
            VERSION,
            global_data,
        );

        Self { global }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

impl<D> GlobalDispatch2<ExtForeignToplevelImageCaptureSourceManagerV1, D>
    for ToplevelImageCaptureGlobalData
where
    D: Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, EmptyData>,
    D: ToplevelImageCaptureHandler,
    D: 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<ExtForeignToplevelImageCaptureSourceManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, EmptyData);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl<D> Dispatch2<ExtForeignToplevelImageCaptureSourceManagerV1, D> for EmptyData
where
    D: Dispatch<ExtImageCaptureSourceV1, ImageCaptureSourceData>,
    D: ToplevelImageCaptureHandler,
    D: 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _resource: &ExtForeignToplevelImageCaptureSourceManagerV1,
        request: <ExtForeignToplevelImageCaptureSourceManagerV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_foreign_toplevel_image_capture_source_manager_v1::Request::CreateSource {
                source,
                toplevel_handle,
            } => {
                let capture_source = ImageCaptureSource::new();

                // Look up the toplevel surface and remember it on the source. If the toplevel is
                // already gone, the source is left without a surface, and capture attempts will
                // be rejected.
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
            ext_foreign_toplevel_image_capture_source_manager_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }
}
