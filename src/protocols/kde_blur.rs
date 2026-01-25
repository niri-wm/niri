use std::sync::Mutex;

use smithay::reexports::wayland_server;
use smithay::wayland::compositor::{
    get_region_attributes, with_states, Cacheable, RegionAttributes,
};
use wayland_protocols_plasma::blur::server::org_kde_kwin_blur::{self, OrgKdeKwinBlur};
use wayland_protocols_plasma::blur::server::org_kde_kwin_blur_manager::{
    self, OrgKdeKwinBlurManager,
};
use wayland_server::backend::GlobalId;
use wayland_server::protocol::wl_surface::WlSurface;
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
};

pub trait KdeBlurHandler:
    GlobalDispatch<OrgKdeKwinBlurManager, ()>
    + Dispatch<OrgKdeKwinBlurManager, ()>
    + Dispatch<OrgKdeKwinBlur, KdeBlurSurfaceUserData>
    + 'static
{
    /// Called when a blur region becomes pending on a surface, awaiting a commit.
    fn set_blur_region(&mut self, wl_surface: WlSurface) {
        let _ = wl_surface;
    }

    /// Called when a blur region unset becomes pending on a surface, awaiting a commit.
    fn unset_blur_region(&mut self, wl_surface: WlSurface) {
        let _ = wl_surface;
    }
}

#[derive(Debug, Clone, Default)]
pub struct KdeBlurSurfaceCachedState {
    /// Region of the surface that will have its background blurred.
    ///
    /// `None` means no blurring.
    pub blur_region: Option<KdeBlurRegion>,
}

#[derive(Debug, Clone, Default)]
pub enum KdeBlurRegion {
    #[default]
    WholeSurface,
    Region(RegionAttributes),
}

impl Cacheable for KdeBlurSurfaceCachedState {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        self.clone()
    }

    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        *into = self;
    }
}

#[derive(Debug)]
pub struct KdeBlurSurfaceUserData {
    surface: Weak<WlSurface>,
    pending_region: Mutex<KdeBlurRegion>,
}

impl KdeBlurSurfaceUserData {
    fn new(surface: WlSurface) -> Self {
        Self {
            surface: surface.downgrade(),
            pending_region: Mutex::new(KdeBlurRegion::WholeSurface),
        }
    }

    fn wl_surface(&self) -> Option<WlSurface> {
        self.surface.upgrade().ok()
    }
}

#[derive(Debug)]
pub struct KdeBlurState {
    global: GlobalId,
}

impl KdeBlurState {
    pub fn new<D: KdeBlurHandler>(display: &DisplayHandle) -> KdeBlurState {
        let global = display.create_global::<D, OrgKdeKwinBlurManager, _>(1, ());
        KdeBlurState { global }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

impl<D: KdeBlurHandler> GlobalDispatch<OrgKdeKwinBlurManager, (), D> for KdeBlurState {
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<OrgKdeKwinBlurManager>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        let _manager = data_init.init(resource, ());
    }
}

impl<D: KdeBlurHandler> Dispatch<OrgKdeKwinBlurManager, (), D> for KdeBlurState {
    fn request(
        state: &mut D,
        _client: &Client,
        _manager: &OrgKdeKwinBlurManager,
        request: org_kde_kwin_blur_manager::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            org_kde_kwin_blur_manager::Request::Create { id, surface } => {
                data_init.init(id, KdeBlurSurfaceUserData::new(surface));
            }
            org_kde_kwin_blur_manager::Request::Unset { surface } => {
                with_states(&surface, |states| {
                    let mut cached = states.cached_state.get::<KdeBlurSurfaceCachedState>();
                    let pending = cached.pending();
                    pending.blur_region = None;
                });
                state.unset_blur_region(surface);
            }
            _ => {}
        }
    }
}

impl<D: KdeBlurHandler> Dispatch<OrgKdeKwinBlur, KdeBlurSurfaceUserData, D> for KdeBlurState {
    fn request(
        state: &mut D,
        _client: &Client,
        _obj: &OrgKdeKwinBlur,
        request: org_kde_kwin_blur::Request,
        data: &KdeBlurSurfaceUserData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            org_kde_kwin_blur::Request::SetRegion { region } => {
                let region = region.as_ref().map(get_region_attributes);

                // In the KDE blur protocol, an empty region means whole surface.
                let region = match region {
                    Some(region) if !region.rects.is_empty() => KdeBlurRegion::Region(region),
                    _ => KdeBlurRegion::WholeSurface,
                };

                *data.pending_region.lock().unwrap() = region;
            }
            org_kde_kwin_blur::Request::Commit => {
                let Some(surface) = data.wl_surface() else {
                    return;
                };

                with_states(&surface, |states| {
                    let mut cached = states.cached_state.get::<KdeBlurSurfaceCachedState>();
                    let pending = cached.pending();
                    let region = data.pending_region.lock().unwrap().clone();
                    pending.blur_region = Some(region);
                });
                state.set_blur_region(surface);
            }
            org_kde_kwin_blur::Request::Release => {
                // No-op.
            }
            _ => {}
        }
    }

    fn destroyed(
        _state: &mut D,
        _client_id: wayland_server::backend::ClientId,
        _object: &OrgKdeKwinBlur,
        _data: &KdeBlurSurfaceUserData,
    ) {
        // No-op: cleanup is handled by double-buffering and surface destruction
    }
}

#[macro_export]
macro_rules! delegate_kde_blur {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        const _: () = {
            use smithay::reexports::wayland_server;
            use wayland_protocols_plasma::blur::server::{
                org_kde_kwin_blur_manager::OrgKdeKwinBlurManager,
                org_kde_kwin_blur::OrgKdeKwinBlur,
            };
            use wayland_server::{delegate_dispatch, delegate_global_dispatch};
            use $crate::protocols::kde_blur::{KdeBlurState, KdeBlurSurfaceUserData};

            delegate_global_dispatch!(
                $(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)?
                $ty: [OrgKdeKwinBlurManager: ()] => KdeBlurState
            );

            delegate_dispatch!(
                $(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)?
                $ty: [OrgKdeKwinBlurManager: ()] => KdeBlurState
            );

            delegate_dispatch!(
                $(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)?
                $ty: [OrgKdeKwinBlur: KdeBlurSurfaceUserData] => KdeBlurState
            );
        };
    };
}
