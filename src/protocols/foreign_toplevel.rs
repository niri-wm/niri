use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arrayvec::ArrayVec;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::server::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1}, ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1}, zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::shell::xdg::{
    ToplevelState, ToplevelStateSet, XdgToplevelSurfaceRoleAttributes
};

use crate::niri::State;
use crate::window::mapped::MappedId;
use crate::utils::with_toplevel_role_and_current;

const EXT_LIST_VERSION: u32 = 1;
const WLR_MANAGEMENT_VERSION: u32 = 3;

pub struct ForeignToplevelManagerState {
    display: DisplayHandle,
    ext_list_instances: HashSet<ExtForeignToplevelListV1>,
    wlr_management_instances: HashSet<ZwlrForeignToplevelManagerV1>,
    toplevels: HashMap<WlSurface, ToplevelData>,
}

pub trait ForeignToplevelHandler {
    fn foreign_toplevel_manager_state(&mut self) -> &mut ForeignToplevelManagerState;
    fn activate(&mut self, wl_surface: WlSurface);
    fn close(&mut self, wl_surface: WlSurface);
    fn set_fullscreen(&mut self, wl_surface: WlSurface, wl_output: Option<WlOutput>);
    fn unset_fullscreen(&mut self, wl_surface: WlSurface);
    fn set_maximized(&mut self, wl_surface: WlSurface);
    fn unset_maximized(&mut self, wl_surface: WlSurface);
}

struct ToplevelData {
    identifier: MappedId,
    title: Option<String>,
    app_id: Option<String>,
    states: ArrayVec<u32, 3>,
    output: Option<Output>,

    ext_list_instances: HashSet<ExtForeignToplevelHandleV1>,
    wlr_management_instances: HashMap<ZwlrForeignToplevelHandleV1, Vec<WlOutput>>,
    // FIXME: parent.
}

#[derive(Clone)]
pub struct ForeignToplevelGlobalData {
    filter: Arc<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

impl ForeignToplevelManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignToplevelGlobalData>,
        D: GlobalDispatch<ExtForeignToplevelListV1, ForeignToplevelGlobalData>,
        D: Dispatch<ZwlrForeignToplevelManagerV1, ()>,
        D: Dispatch<ExtForeignToplevelListV1, ()>,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = ForeignToplevelGlobalData {
            filter: Arc::new(filter),
        };
        display
            .create_global::<D, ExtForeignToplevelListV1, _>(EXT_LIST_VERSION, global_data.clone());
        display.create_global::<D, ZwlrForeignToplevelManagerV1, _>(
            WLR_MANAGEMENT_VERSION,
            global_data,
        );
        Self {
            display: display.clone(),
            ext_list_instances: HashSet::new(),
            wlr_management_instances: HashSet::new(),
            toplevels: HashMap::new(),
        }
    }
}

pub fn refresh(state: &mut State) {
    let _span = tracy_client::span!("foreign_toplevel::refresh");

    let protocol_state = &mut state.niri.foreign_toplevel_state;

    // Handle closed windows.
    protocol_state.toplevels.retain(|surface, data| {
        if state.niri.layout.find_window_and_output(surface).is_some() {
            return true;
        }

        for instance in data.ext_list_instances.iter() {
            instance.closed();
        }

        for instance in data.wlr_management_instances.keys() {
            instance.closed();
        }

        false
    });

    // Handle new and existing windows.
    //
    // Save the focused window for last, this way when the focus changes, we will first deactivate
    // the previous window and only then activate the newly focused window.
    let mut focused = None;
    state.niri.layout.with_windows(|mapped, output, _, _| {
        let toplevel = mapped.toplevel();
        let wl_surface = toplevel.wl_surface();
        with_toplevel_role_and_current(toplevel, |role, cur| {
            let Some(cur) = cur else {
                error!("mapped must have had initial commit");
                return;
            };

            if state.niri.keyboard_focus.surface() == Some(wl_surface) {
                focused = Some((mapped.id(), mapped.window.clone(), output.cloned()));
            } else {
                refresh_toplevel(
                    protocol_state,
                    wl_surface,
                    mapped.id(),
                    role,
                    cur,
                    output,
                    false,
                );
            }
        });
    });

    // Finally, refresh the focused window.
    if let Some((identifier, window, output)) = focused {
        let toplevel = window.toplevel().expect("no X11 support");
        let wl_surface = toplevel.wl_surface();
        with_toplevel_role_and_current(toplevel, |role, cur| {
            let Some(cur) = cur else {
                error!("mapped must have had initial commit");
                return;
            };

            refresh_toplevel(
                protocol_state,
                wl_surface,
                identifier,
                role,
                cur,
                output.as_ref(),
                true,
            );
        });
    }
}

pub fn on_output_bound(state: &mut State, output: &Output, wl_output: &WlOutput) {
    let _span = tracy_client::span!("foreign_toplevel::on_output_bound");

    let Some(client) = wl_output.client() else {
        return;
    };

    let protocol_state = &mut state.niri.foreign_toplevel_state;
    for data in protocol_state.toplevels.values_mut() {
        if data.output.as_ref() != Some(output) {
            continue;
        }

        for (instance, outputs) in &mut data.wlr_management_instances {
            if instance.client().as_ref() != Some(&client) {
                continue;
            }

            instance.output_enter(wl_output);
            instance.done();
            outputs.push(wl_output.clone());
        }
    }
}

fn refresh_toplevel(
    protocol_state: &mut ForeignToplevelManagerState,
    wl_surface: &WlSurface,
    identifier: MappedId,
    role: &XdgToplevelSurfaceRoleAttributes,
    current: &ToplevelState,
    output: Option<&Output>,
    has_focus: bool,
) {
    let states = to_state_vec(&current.states, has_focus);

    match protocol_state.toplevels.entry(wl_surface.clone()) {
        Entry::Occupied(entry) => {
            // Existing window, check if anything changed.
            let data = entry.into_mut();

            let mut new_title = None;
            if data.title != role.title {
                data.title.clone_from(&role.title);
                new_title = role.title.as_deref();

                if new_title.is_none() {
                    error!("toplevel title changed to None");
                }
            }

            let mut new_app_id = None;
            if data.app_id != role.app_id {
                data.app_id.clone_from(&role.app_id);
                new_app_id = role.app_id.as_deref();

                if new_app_id.is_none() {
                    error!("toplevel app_id changed to None");
                }
            }

            let mut states_changed = false;
            if data.states != states {
                data.states = states;
                states_changed = true;
            }

            let mut output_changed = false;
            if data.output.as_ref() != output {
                data.output = output.cloned();
                output_changed = true;
            }

            let something_changed_for_ext = new_title.is_some() || new_app_id.is_some();
            let something_changed_for_wlr =
                new_title.is_some() || new_app_id.is_some() || states_changed || output_changed;

            if something_changed_for_ext {
                for instance in &data.ext_list_instances {
                    if let Some(new_title) = new_title {
                        instance.title(new_title.to_owned());
                    }
                    if let Some(new_app_id) = new_app_id {
                        instance.app_id(new_app_id.to_owned());
                    }
                    instance.done();
                }
            }

            if something_changed_for_wlr {
                for (instance, outputs) in &mut data.wlr_management_instances {
                    if let Some(new_title) = new_title {
                        instance.title(new_title.to_owned());
                    }
                    if let Some(new_app_id) = new_app_id {
                        instance.app_id(new_app_id.to_owned());
                    }
                    if states_changed {
                        instance.state(data.states.iter().flat_map(|x| x.to_ne_bytes()).collect());
                    }
                    if output_changed {
                        for wl_output in outputs.drain(..) {
                            instance.output_leave(&wl_output);
                        }
                        if let Some(output) = &data.output {
                            if let Some(client) = instance.client() {
                                for wl_output in output.client_outputs(&client) {
                                    instance.output_enter(&wl_output);
                                    outputs.push(wl_output);
                                }
                            }
                        }
                    }
                    instance.done();
                }
            }

            for outputs in data.wlr_management_instances.values_mut() {
                // Clean up dead wl_outputs.
                outputs.retain(|x| x.is_alive());
            }
        }
        Entry::Vacant(entry) => {
            // New window, start tracking it.
            let mut data = ToplevelData {
                identifier,
                title: role.title.clone(),
                app_id: role.app_id.clone(),
                states,
                output: output.cloned(),
                ext_list_instances: HashSet::new(),
                wlr_management_instances: HashMap::new(),
            };

            for manager in &protocol_state.ext_list_instances {
                if let Some(client) = manager.client() {
                    data.add_ext_instance::<State>(&protocol_state.display, &client, manager);
                }
            }

            for manager in &protocol_state.wlr_management_instances {
                if let Some(client) = manager.client() {
                    data.add_wlr_instance::<State>(&protocol_state.display, &client, manager);
                }
            }

            entry.insert(data);
        }
    }
}

impl ToplevelData {
    fn add_ext_instance<D>(
        &mut self,
        handle: &DisplayHandle,
        client: &Client,
        manager: &ExtForeignToplevelListV1,
    ) where
        D: Dispatch<ExtForeignToplevelHandleV1, ()>,
        D: 'static,
    {
        let toplevel = client
            .create_resource::<ExtForeignToplevelHandleV1, _, D>(handle, manager.version(), ())
            .unwrap();
        manager.toplevel(&toplevel);

        toplevel.identifier(self.identifier.to_protocol_identifier());

        if let Some(title) = &self.title {
            toplevel.title(title.clone());
        }
        if let Some(app_id) = &self.app_id {
            toplevel.app_id(app_id.clone());
        }

        toplevel.done();

        self.ext_list_instances.insert(toplevel);
    }

    fn add_wlr_instance<D>(
        &mut self,
        handle: &DisplayHandle,
        client: &Client,
        manager: &ZwlrForeignToplevelManagerV1,
    ) where
        D: Dispatch<ZwlrForeignToplevelHandleV1, ()>,
        D: 'static,
    {
        let toplevel = client
            .create_resource::<ZwlrForeignToplevelHandleV1, _, D>(handle, manager.version(), ())
            .unwrap();
        manager.toplevel(&toplevel);

        if let Some(title) = &self.title {
            toplevel.title(title.clone());
        }
        if let Some(app_id) = &self.app_id {
            toplevel.app_id(app_id.clone());
        }

        toplevel.state(self.states.iter().flat_map(|x| x.to_ne_bytes()).collect());

        let mut outputs = Vec::new();
        if let Some(output) = &self.output {
            for wl_output in output.client_outputs(client) {
                toplevel.output_enter(&wl_output);
                outputs.push(wl_output);
            }
        }

        toplevel.done();

        self.wlr_management_instances.insert(toplevel, outputs);
    }
}

impl<D> GlobalDispatch<ExtForeignToplevelListV1, ForeignToplevelGlobalData, D>
    for ForeignToplevelManagerState
where
    D: GlobalDispatch<ExtForeignToplevelListV1, ForeignToplevelGlobalData>,
    D: Dispatch<ExtForeignToplevelListV1, ()>,
    D: Dispatch<ExtForeignToplevelHandleV1, ()>,
    D: ForeignToplevelHandler,
{
    fn bind(
        state: &mut D,
        handle: &DisplayHandle,
        client: &Client,
        resource: New<ExtForeignToplevelListV1>,
        _global_data: &ForeignToplevelGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ());

        let state = state.foreign_toplevel_manager_state();

        for data in state.toplevels.values_mut() {
            data.add_ext_instance::<D>(handle, client, &manager);
        }

        state.ext_list_instances.insert(manager);
    }

    fn can_view(client: Client, global_data: &ForeignToplevelGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ExtForeignToplevelListV1, (), D> for ForeignToplevelManagerState
where
    D: Dispatch<ExtForeignToplevelListV1, ()>,
    D: ForeignToplevelHandler,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ExtForeignToplevelListV1,
        request: <ExtForeignToplevelListV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_foreign_toplevel_list_v1::Request::Stop => {
                resource.finished();

                // remove the instance here so we won't send any more events.
                let state = state.foreign_toplevel_manager_state();
                state.ext_list_instances.remove(resource);
            }
            ext_foreign_toplevel_list_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ExtForeignToplevelListV1,
        _data: &(),
    ) {
        // also remove the instance here, in case `stop` was never sent, e.g. sudden disconnect.
        let state = state.foreign_toplevel_manager_state();
        state.ext_list_instances.remove(resource);
    }
}

impl<D> Dispatch<ExtForeignToplevelHandleV1, (), D> for ForeignToplevelManagerState
where
    D: Dispatch<ExtForeignToplevelHandleV1, ()>,
    D: ForeignToplevelHandler,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &ExtForeignToplevelHandleV1,
        request: <ExtForeignToplevelHandleV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_foreign_toplevel_handle_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ExtForeignToplevelHandleV1,
        _data: &(),
    ) {
        let state = state.foreign_toplevel_manager_state();
        for data in state.toplevels.values_mut() {
            data.ext_list_instances.remove(resource);
        }
    }
}

impl<D> GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignToplevelGlobalData, D>
    for ForeignToplevelManagerState
where
    D: GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignToplevelGlobalData>,
    D: Dispatch<ZwlrForeignToplevelManagerV1, ()>,
    D: Dispatch<ZwlrForeignToplevelHandleV1, ()>,
    D: ForeignToplevelHandler,
{
    fn bind(
        state: &mut D,
        handle: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _global_data: &ForeignToplevelGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ());

        let state = state.foreign_toplevel_manager_state();

        for data in state.toplevels.values_mut() {
            data.add_wlr_instance::<D>(handle, client, &manager);
        }

        state.wlr_management_instances.insert(manager);
    }

    fn can_view(client: Client, global_data: &ForeignToplevelGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ZwlrForeignToplevelManagerV1, (), D> for ForeignToplevelManagerState
where
    D: Dispatch<ZwlrForeignToplevelManagerV1, ()>,
    D: ForeignToplevelHandler,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ZwlrForeignToplevelManagerV1,
        request: <ZwlrForeignToplevelManagerV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_foreign_toplevel_manager_v1::Request::Stop => {
                resource.finished();

                // remove the instance here so we won't send any more events.
                let state = state.foreign_toplevel_manager_state();
                state.wlr_management_instances.remove(resource);
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ZwlrForeignToplevelManagerV1,
        _data: &(),
    ) {
        // also remove the instance here, in case `stop` was never sent, e.g. sudden disconnect.
        let state = state.foreign_toplevel_manager_state();
        state.wlr_management_instances.remove(resource);
    }
}

impl<D> Dispatch<ZwlrForeignToplevelHandleV1, (), D> for ForeignToplevelManagerState
where
    D: Dispatch<ZwlrForeignToplevelHandleV1, ()>,
    D: ForeignToplevelHandler,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ZwlrForeignToplevelHandleV1,
        request: <ZwlrForeignToplevelHandleV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let protocol_state = state.foreign_toplevel_manager_state();

        let Some((surface, _)) = protocol_state
            .toplevels
            .iter()
            .find(|(_, data)| data.wlr_management_instances.contains_key(resource))
        else {
            return;
        };
        let surface = surface.clone();

        match request {
            zwlr_foreign_toplevel_handle_v1::Request::SetMaximized => state.set_maximized(surface),
            zwlr_foreign_toplevel_handle_v1::Request::UnsetMaximized => {
                state.unset_maximized(surface)
            }
            zwlr_foreign_toplevel_handle_v1::Request::SetMinimized => (),
            zwlr_foreign_toplevel_handle_v1::Request::UnsetMinimized => (),
            zwlr_foreign_toplevel_handle_v1::Request::Activate { .. } => {
                state.activate(surface);
            }
            zwlr_foreign_toplevel_handle_v1::Request::Close => {
                state.close(surface);
            }
            zwlr_foreign_toplevel_handle_v1::Request::SetRectangle { .. } => (),
            zwlr_foreign_toplevel_handle_v1::Request::Destroy => (),
            zwlr_foreign_toplevel_handle_v1::Request::SetFullscreen { output } => {
                state.set_fullscreen(surface, output);
            }
            zwlr_foreign_toplevel_handle_v1::Request::UnsetFullscreen => {
                state.unset_fullscreen(surface);
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ZwlrForeignToplevelHandleV1,
        _data: &(),
    ) {
        let state = state.foreign_toplevel_manager_state();
        for data in state.toplevels.values_mut() {
            data.wlr_management_instances.remove(resource);
        }
    }
}

fn to_state_vec(states: &ToplevelStateSet, has_focus: bool) -> ArrayVec<u32, 3> {
    let mut rv = ArrayVec::new();
    if states.contains(xdg_toplevel::State::Maximized) {
        rv.push(zwlr_foreign_toplevel_handle_v1::State::Maximized as u32);
    }
    if states.contains(xdg_toplevel::State::Fullscreen) {
        rv.push(zwlr_foreign_toplevel_handle_v1::State::Fullscreen as u32);
    }

    // HACK: wlr-foreign-toplevel-management states:
    //
    // These have the same meaning as the states with the same names defined in xdg-toplevel
    //
    // However, clients such as sfwbar and fcitx seem to treat the activated state as keyboard
    // focus, i.e. they don't expect multiple windows to have it set at once. Even Waybar which
    // handles multiple activated windows correctly uses it in its design in such a way that
    // keyboard focus would make more sense. Let's do what the clients expect.
    if has_focus {
        rv.push(zwlr_foreign_toplevel_handle_v1::State::Activated as u32);
    }

    rv
}

#[macro_export]
macro_rules! delegate_foreign_toplevel {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::server::ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1: $crate::protocols::foreign_toplevel::ForeignToplevelGlobalData
        ] => $crate::protocols::foreign_toplevel::ForeignToplevelManagerState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::server::ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1: ()
        ] => $crate::protocols::foreign_toplevel::ForeignToplevelManagerState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::server::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1: ()
        ] => $crate::protocols::foreign_toplevel::ForeignToplevelManagerState);

        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1: $crate::protocols::foreign_toplevel::ForeignToplevelGlobalData
        ] => $crate::protocols::foreign_toplevel::ForeignToplevelManagerState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1: ()
        ] => $crate::protocols::foreign_toplevel::ForeignToplevelManagerState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1: ()
        ] => $crate::protocols::foreign_toplevel::ForeignToplevelManagerState);
    };
}
