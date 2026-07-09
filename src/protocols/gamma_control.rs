use std::collections::HashMap;
use std::fs::File;
use std::io::Read;

use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use wayland_protocols_wlr::gamma_control::v1::server::{
    zwlr_gamma_control_manager_v1, zwlr_gamma_control_v1,
};
use zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1;
use zwlr_gamma_control_v1::ZwlrGammaControlV1;

const VERSION: u32 = 1;

pub struct GammaControlManagerState {
    // Active gamma controls only. Failed ones are removed.
    gamma_controls: HashMap<Output, ZwlrGammaControlV1>,
}

pub struct GammaControlManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait GammaControlHandler {
    fn gamma_control_manager_state(&mut self) -> &mut GammaControlManagerState;
    fn get_gamma_size(&mut self, output: &Output) -> Option<u32>;
    fn set_gamma(&mut self, output: &Output, ramp: Option<Vec<u16>>) -> Option<()>;
}

pub struct GammaControlState {
    gamma_size: u32,
}

impl GammaControlManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrGammaControlManagerV1, GammaControlManagerGlobalData>,
        D: Dispatch<ZwlrGammaControlManagerV1, ()>,
        D: Dispatch<ZwlrGammaControlV1, GammaControlState>,
        D: GammaControlHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = GammaControlManagerGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, ZwlrGammaControlManagerV1, _>(VERSION, global_data);

        Self {
            gamma_controls: HashMap::new(),
        }
    }

    pub fn output_removed(&mut self, output: &Output) {
        if let Some(gamma_control) = self.gamma_controls.remove(output) {
            gamma_control.failed();
        }
    }
}

impl<D> GlobalDispatch<ZwlrGammaControlManagerV1, GammaControlManagerGlobalData, D>
    for GammaControlManagerState
where
    D: GlobalDispatch<ZwlrGammaControlManagerV1, GammaControlManagerGlobalData>,
    D: Dispatch<ZwlrGammaControlManagerV1, ()>,
    D: Dispatch<ZwlrGammaControlV1, GammaControlState>,
    D: GammaControlHandler,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<ZwlrGammaControlManagerV1>,
        _manager_state: &GammaControlManagerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ());
    }

    fn can_view(client: Client, global_data: &GammaControlManagerGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ZwlrGammaControlManagerV1, (), D> for GammaControlManagerState
where
    D: Dispatch<ZwlrGammaControlManagerV1, ()>,
    D: Dispatch<ZwlrGammaControlV1, GammaControlState>,
    D: GammaControlHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &ZwlrGammaControlManagerV1,
        request: <ZwlrGammaControlManagerV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_gamma_control_manager_v1::Request::GetGammaControl { id, output } => {
                if let Some(output) = Output::from_resource(&output) {
                    // We borrow state in the middle.
                    #[allow(clippy::map_entry)]
                    if !state
                        .gamma_control_manager_state()
                        .gamma_controls
                        .contains_key(&output)
                    {
                        if let Some(gamma_size) = state.get_gamma_size(&output) {
                            let zwlr_gamma_control =
                                data_init.init(id, GammaControlState { gamma_size });
                            zwlr_gamma_control.gamma_size(gamma_size);
                            state
                                .gamma_control_manager_state()
                                .gamma_controls
                                .insert(output, zwlr_gamma_control);
                            return;
                        }
                    }
                }

                data_init
                    .init(id, GammaControlState { gamma_size: 0 })
                    .failed();
            }
            zwlr_gamma_control_manager_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }
}

impl<D> Dispatch<ZwlrGammaControlV1, GammaControlState, D> for GammaControlManagerState
where
    D: Dispatch<ZwlrGammaControlV1, GammaControlState>,
    D: GammaControlHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ZwlrGammaControlV1,
        request: <ZwlrGammaControlV1 as Resource>::Request,
        data: &GammaControlState,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_gamma_control_v1::Request::SetGamma { fd } => {
                let gamma_controls = &mut state.gamma_control_manager_state().gamma_controls;
                let Some((output, _)) = gamma_controls.iter().find(|(_, x)| *x == resource) else {
                    return;
                };
                let output = output.clone();

                trace!("setting gamma for output {}", output.name());

                // Start with a u16 slice so it's aligned correctly.
                let mut gamma = vec![0u16; data.gamma_size as usize * 3];
                let buf = bytemuck::cast_slice_mut(&mut gamma);
                let mut file = File::from(fd);
                {
                    let _span = tracy_client::span!("read gamma from fd");

                    if let Err(err) = file.read_exact(buf) {
                        warn!("failed to read gamma data: {err:?}");
                        resource.failed();
                        gamma_controls.remove(&output);
                        let _ = state.set_gamma(&output, None);
                        return;
                    }

                    // Verify that there's no more data.
                    {
                        match file.read(&mut [0]) {
                            Ok(0) => (),
                            Ok(_) => {
                                warn!("gamma data is too large");
                                resource.failed();
                                gamma_controls.remove(&output);
                                let _ = state.set_gamma(&output, None);
                                return;
                            }
                            Err(err) => {
                                warn!("error reading gamma data: {err:?}");
                                resource.failed();
                                gamma_controls.remove(&output);
                                let _ = state.set_gamma(&output, None);
                                return;
                            }
                        }
                    }
                }

                if state.set_gamma(&output, Some(gamma)).is_none() {
                    resource.failed();
                    let gamma_controls = &mut state.gamma_control_manager_state().gamma_controls;
                    gamma_controls.remove(&output);
                    let _ = state.set_gamma(&output, None);
                }
            }
            zwlr_gamma_control_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ZwlrGammaControlV1,
        _data: &GammaControlState,
    ) {
        let gamma_controls = &mut state.gamma_control_manager_state().gamma_controls;
        let Some((output, _)) = gamma_controls.iter().find(|(_, x)| *x == resource) else {
            return;
        };
        let output = output.clone();
        gamma_controls.remove(&output);

        let _ = state.set_gamma(&output, None);
    }
}

#[macro_export]
macro_rules! delegate_gamma_control {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::gamma_control::v1::server::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1: $crate::protocols::gamma_control::GammaControlManagerGlobalData
        ] => $crate::protocols::gamma_control::GammaControlManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::gamma_control::v1::server::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1: ()
        ] => $crate::protocols::gamma_control::GammaControlManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::gamma_control::v1::server::zwlr_gamma_control_v1::ZwlrGammaControlV1:  $crate::protocols::gamma_control::GammaControlState
        ] => $crate::protocols::gamma_control::GammaControlManagerState);
    };
}
