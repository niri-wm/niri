//! ext-workspace protocol implementation.
//!
//! This is how we map the protocol concepts to the niri concepts:
//!
//! - Workspace groups are outputs.
//! - Workspace coordinates: X = 0, Y = workspace index. They need to be two-dimensional because 1D
//!   coordinates are defined to be a plain list without a geometric interpretation, while we do
//!   order workspaces in a vertical line.
//! - Workspace id: name for named workspaces, unset for unnamed. Because ids in this protocol are
//!   expected to be stable across sessions.
//! - Workspace name: name for named workspaces, index for unnamed.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::mem;

use arrayvec::ArrayVec;
use ext_workspace_group_handle_v1::ExtWorkspaceGroupHandleV1;
use ext_workspace_handle_v1::ExtWorkspaceHandleV1;
use ext_workspace_manager_v1::ExtWorkspaceManagerV1;
use smithay::output::{Output, WeakOutput};
use smithay::reexports::wayland_protocols::ext::workspace::v1::server::{
    ext_workspace_group_handle_v1, ext_workspace_handle_v1, ext_workspace_manager_v1,
};
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use wayland_backend::server::ClientId;

use crate::layout::monitor::Monitor;
use crate::layout::workspace::{Workspace, WorkspaceId};
use crate::niri::State;
use crate::window::Mapped;

const VERSION: u32 = 1;

pub trait ExtWorkspaceHandler {
    fn ext_workspace_manager_state(&mut self) -> &mut ExtWorkspaceManagerState;
    fn activate_workspace(&mut self, id: WorkspaceId);
    fn assign_workspace(&mut self, ws_id: WorkspaceId, output: Output);
}

enum Action {
    Assign(WorkspaceId, WeakOutput),
    Activate(WorkspaceId),
}

impl Action {
    fn order(&self) -> u8 {
        // First assign everything (move across outputs), then activate.
        match self {
            Action::Assign(_, _) => 0,
            Action::Activate(_) => 1,
        }
    }
}

pub struct ExtWorkspaceManagerState {
    display: DisplayHandle,
    instances: HashMap<ExtWorkspaceManagerV1, Vec<Action>>,
    workspace_groups: HashMap<Output, ExtWorkspaceGroupData>,
    workspaces: HashMap<WorkspaceId, ExtWorkspaceData>,
}

struct ExtWorkspaceGroupData {
    instances: Vec<ExtWorkspaceGroupHandleV1>,
}

struct ExtWorkspaceData {
    // id cannot change once set.
    id: Option<String>,
    name: String,
    coordinates: ArrayVec<u32, 2>,
    state: ext_workspace_handle_v1::State,
    instances: Vec<ExtWorkspaceHandleV1>,
    output: Option<Output>,
}

pub struct ExtWorkspaceGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub fn refresh(state: &mut State) {
    let _span = tracy_client::span!("ext_workspace::refresh");

    let protocol_state = &mut state.niri.ext_workspace_state;

    let mut changed = false;

    // Remove workspaces that no longer exist (sending workspace_leave to workspace groups).
    let mut seen_workspaces = HashMap::new();
    for (mon, _, ws) in state.niri.layout.workspaces() {
        let output = mon.map(|mon| mon.output());
        seen_workspaces.insert(ws.id(), output);
    }

    protocol_state.workspaces.retain(|id, workspace| {
        if seen_workspaces.contains_key(id) {
            return true;
        }

        remove_workspace_instances(&protocol_state.workspace_groups, workspace);
        changed = true;
        false
    });

    // Remove workspace groups for outputs that no longer exist.
    protocol_state.workspace_groups.retain(|output, data| {
        if state.niri.sorted_outputs.contains(output) {
            return true;
        }

        for group in &data.instances {
            // Send workspace_leave for all workspaces in this group with matching manager.
            let manager: &ExtWorkspaceManagerV1 = group.data().unwrap();
            for ws in protocol_state.workspaces.values() {
                if ws.output.as_ref() == Some(output) {
                    for workspace in &ws.instances {
                        if workspace.data() == Some(manager) {
                            group.workspace_leave(workspace);
                        }
                    }
                }
            }

            group.removed();
        }

        changed = true;
        false
    });

    // Update existing workspaces and create new ones.
    for (mon, ws_idx, ws) in state.niri.layout.workspaces() {
        changed |= refresh_workspace(protocol_state, mon, ws_idx, ws);
    }

    // Update workspace groups and create new ones, sending workspace_enter events as needed.
    for output in &state.niri.sorted_outputs {
        changed |= refresh_workspace_group(protocol_state, output);
    }

    if changed {
        for manager in protocol_state.instances.keys() {
            manager.done();
        }
    }
}

pub fn on_output_bound(state: &mut State, output: &Output, wl_output: &WlOutput) {
    let Some(client) = wl_output.client() else {
        return;
    };

    let mut sent = false;

    let protocol_state = &mut state.niri.ext_workspace_state;
    if let Some(data) = protocol_state.workspace_groups.get_mut(output) {
        for group in &mut data.instances {
            if group.client().as_ref() != Some(&client) {
                continue;
            }

            group.output_enter(wl_output);
            sent = true;
        }
    }

    if !sent {
        return;
    }

    for manager in protocol_state.instances.keys() {
        if manager.client().as_ref() == Some(&client) {
            manager.done();
        }
    }
}

fn refresh_workspace_group(protocol_state: &mut ExtWorkspaceManagerState, output: &Output) -> bool {
    if protocol_state.workspace_groups.contains_key(output) {
        // Existing workspace group. Nothing can actually change since our workspace groups are tied
        // to an output.
        return false;
    }

    // New workspace group, start tracking it.
    let mut data = ExtWorkspaceGroupData {
        instances: Vec::new(),
    };

    // Create workspace group handle for each manager instance.
    for manager in protocol_state.instances.keys() {
        if let Some(client) = manager.client() {
            data.add_instance::<State>(&protocol_state.display, &client, manager, output);
        }
    }

    // Send workspace_enter for all existing workspaces on this output.
    for group in &data.instances {
        let manager: &ExtWorkspaceManagerV1 = group.data().unwrap();
        for (_, ws) in protocol_state.workspaces.iter() {
            for workspace in &ws.instances {
                if workspace.data() == Some(manager) {
                    group.workspace_enter(workspace);
                }
            }
        }
    }

    protocol_state.workspace_groups.insert(output.clone(), data);
    true
}

fn send_workspace_enter_leave(
    workspace_groups: &HashMap<Output, ExtWorkspaceGroupData>,
    data: &ExtWorkspaceData,
    enter: bool,
) {
    if let Some(output) = &data.output {
        if let Some(group_data) = workspace_groups.get(output) {
            for group in &group_data.instances {
                let manager: &ExtWorkspaceManagerV1 = group.data().unwrap();
                for workspace in &data.instances {
                    if workspace.data() == Some(manager) {
                        if enter {
                            group.workspace_enter(workspace);
                        } else {
                            group.workspace_leave(workspace);
                        }
                    }
                }
            }
        }
    }
}

fn remove_workspace_instances(
    workspace_groups: &HashMap<Output, ExtWorkspaceGroupData>,
    data: &ExtWorkspaceData,
) {
    send_workspace_enter_leave(workspace_groups, data, false);

    for workspace in &data.instances {
        workspace.removed();
    }
}

fn build_name(ws: &Workspace<Mapped>, ws_idx: usize) -> String {
    ws.name().cloned().unwrap_or_else(|| {
        // Add 1 since this is a human-readable name, and our action indexing is 1-based.
        (ws_idx + 1).to_string()
    })
}

fn refresh_workspace(
    protocol_state: &mut ExtWorkspaceManagerState,
    mon: Option<&Monitor<Mapped>>,
    ws_idx: usize,
    ws: &Workspace<Mapped>,
) -> bool {
    let mut state = ext_workspace_handle_v1::State::empty();
    if mon.is_some_and(|mon| mon.active_workspace_idx() == ws_idx) {
        state |= ext_workspace_handle_v1::State::Active;
    }
    if ws.is_urgent() {
        state |= ext_workspace_handle_v1::State::Urgent;
    }

    let output = mon.map(|mon| mon.output());

    match protocol_state.workspaces.entry(ws.id()) {
        Entry::Occupied(entry) => {
            // Existing workspace, check if anything changed.
            let data = entry.into_mut();

            let mut id_set = false;
            let mut recreate = false;
            let id = ws.name();
            if data.id.as_ref() != id {
                if data.id.is_some() {
                    recreate = true;
                } else {
                    id_set = true;
                }
                data.id = id.cloned();
            }

            let mut coordinates_changed = false;
            if data.coordinates[1] != ws_idx as u32 {
                data.coordinates[1] = ws_idx as u32;
                coordinates_changed = true;
            }

            let mut state_changed = false;
            if data.state != state {
                data.state = state;
                state_changed = true;
            }

            // Recreate means name got changed or unset (meaning data.name is back to ws_idx).
            let check = recreate
                || if data.id.is_some() {
                    // True means workspace got named, going from ws_idx to name.
                    id_set
                } else {
                    // The workspace is unnamed, check if ws_idx changed.
                    coordinates_changed
                };
            let mut name_changed = false;
            if check {
                let new_name = build_name(ws, ws_idx);
                // This will likely be true, except if the workspace got named its index.
                if data.name != new_name {
                    data.name = new_name;
                    name_changed = true;
                }
            }

            let mut output_changed = false;
            if data.output.as_ref() != output {
                send_workspace_enter_leave(&protocol_state.workspace_groups, data, false);
                data.output = output.cloned();
                output_changed = true;
            }

            if recreate {
                remove_workspace_instances(&protocol_state.workspace_groups, data);
                data.instances.clear();

                for manager in protocol_state.instances.keys() {
                    if let Some(client) = manager.client() {
                        data.add_instance::<State>(&protocol_state.display, &client, manager);
                    }
                }

                send_workspace_enter_leave(&protocol_state.workspace_groups, data, true);
                return true;
            }

            if output_changed {
                // Send workspace_enter to the new output's group. If the group doesn't exist yet
                // (new groups are created after refreshing workspaces), then workspace_enter() will
                // be sent when the group is created.
                send_workspace_enter_leave(&protocol_state.workspace_groups, data, true);
            }

            let something_changed = id_set || name_changed || coordinates_changed || state_changed;
            if something_changed {
                for instance in &data.instances {
                    if id_set {
                        instance.id(data.id.clone().unwrap());
                    }
                    if name_changed {
                        instance.name(data.name.clone());
                    }
                    if coordinates_changed {
                        instance.coordinates(
                            data.coordinates
                                .iter()
                                .flat_map(|x| x.to_ne_bytes())
                                .collect(),
                        );
                    }
                    if state_changed {
                        instance.state(data.state);
                    }
                }
            }

            output_changed || something_changed
        }
        Entry::Vacant(entry) => {
            // New workspace, start tracking it.
            let mut data = ExtWorkspaceData {
                id: ws.name().cloned(),
                name: build_name(ws, ws_idx),
                coordinates: ArrayVec::from([0, ws_idx as u32]),
                state,
                instances: Vec::new(),
                output: output.cloned(),
            };

            for manager in protocol_state.instances.keys() {
                if let Some(client) = manager.client() {
                    data.add_instance::<State>(&protocol_state.display, &client, manager);
                }
            }

            send_workspace_enter_leave(&protocol_state.workspace_groups, &data, true);
            entry.insert(data);
            true
        }
    }
}

impl ExtWorkspaceGroupData {
    fn add_instance<D>(
        &mut self,
        handle: &DisplayHandle,
        client: &Client,
        manager: &ExtWorkspaceManagerV1,
        output: &Output,
    ) -> &ExtWorkspaceGroupHandleV1
    where
        D: Dispatch<ExtWorkspaceGroupHandleV1, ExtWorkspaceManagerV1>,
        D: 'static,
    {
        let group = client
            .create_resource::<ExtWorkspaceGroupHandleV1, _, D>(
                handle,
                manager.version(),
                manager.clone(),
            )
            .unwrap();
        manager.workspace_group(&group);

        group.capabilities(ext_workspace_group_handle_v1::GroupCapabilities::empty());

        for wl_output in output.client_outputs(client) {
            group.output_enter(&wl_output);
        }

        self.instances.push(group);
        self.instances.last().unwrap()
    }
}

impl ExtWorkspaceData {
    fn add_instance<D>(
        &mut self,
        handle: &DisplayHandle,
        client: &Client,
        manager: &ExtWorkspaceManagerV1,
    ) -> &ExtWorkspaceHandleV1
    where
        D: Dispatch<ExtWorkspaceHandleV1, ExtWorkspaceManagerV1>,
        D: 'static,
    {
        let workspace = client
            .create_resource::<ExtWorkspaceHandleV1, _, D>(
                handle,
                manager.version(),
                manager.clone(),
            )
            .unwrap();
        manager.workspace(&workspace);

        if let Some(id) = self.id.clone() {
            workspace.id(id);
        }

        workspace.name(self.name.clone());
        workspace.coordinates(
            self.coordinates
                .iter()
                .flat_map(|x| x.to_ne_bytes())
                .collect(),
        );
        workspace.state(self.state);
        workspace.capabilities(
            ext_workspace_handle_v1::WorkspaceCapabilities::Activate
                | ext_workspace_handle_v1::WorkspaceCapabilities::Assign,
        );

        self.instances.push(workspace);
        self.instances.last().unwrap()
    }
}

impl ExtWorkspaceManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ExtWorkspaceManagerV1, ExtWorkspaceGlobalData>,
        D: Dispatch<ExtWorkspaceManagerV1, ()>,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = ExtWorkspaceGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, ExtWorkspaceManagerV1, _>(VERSION, global_data);
        Self {
            display: display.clone(),
            instances: HashMap::new(),
            workspace_groups: HashMap::new(),
            workspaces: HashMap::new(),
        }
    }
}

impl<D> GlobalDispatch<ExtWorkspaceManagerV1, ExtWorkspaceGlobalData, D>
    for ExtWorkspaceManagerState
where
    D: GlobalDispatch<ExtWorkspaceManagerV1, ExtWorkspaceGlobalData>,
    D: Dispatch<ExtWorkspaceManagerV1, ()>,
    D: Dispatch<ExtWorkspaceHandleV1, ExtWorkspaceManagerV1>,
    D: ExtWorkspaceHandler,
{
    fn bind(
        state: &mut D,
        handle: &DisplayHandle,
        client: &Client,
        resource: New<ExtWorkspaceManagerV1>,
        _global_data: &ExtWorkspaceGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ());

        let state = state.ext_workspace_manager_state();

        // Send existing workspaces to the new client.
        let mut new_workspaces: HashMap<_, Vec<_>> = HashMap::new();
        for data in state.workspaces.values_mut() {
            let output = data.output.clone();
            let workspace = data.add_instance::<State>(handle, client, &manager);

            if let Some(output) = output {
                new_workspaces.entry(output).or_default().push(workspace);
            }
        }

        // Create workspace groups for all outputs.
        for (output, group_data) in &mut state.workspace_groups {
            let group = group_data.add_instance::<State>(handle, client, &manager, output);

            for workspace in new_workspaces.get(output).into_iter().flatten() {
                group.workspace_enter(workspace);
            }
        }

        manager.done();
        state.instances.insert(manager, Vec::new());
    }

    fn can_view(client: Client, global_data: &ExtWorkspaceGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ExtWorkspaceManagerV1, (), D> for ExtWorkspaceManagerState
where
    D: Dispatch<ExtWorkspaceManagerV1, ()>,
    D: ExtWorkspaceHandler,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ExtWorkspaceManagerV1,
        request: <ExtWorkspaceManagerV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_workspace_manager_v1::Request::Commit => {
                let protocol_state = state.ext_workspace_manager_state();
                let actions = protocol_state.instances.get_mut(resource).unwrap();
                let mut actions = mem::take(actions);

                actions.sort_by_key(Action::order);

                for action in actions {
                    match action {
                        Action::Assign(ws_id, output) => {
                            if let Some(output) = output.upgrade() {
                                state.assign_workspace(ws_id, output);
                            }
                        }
                        Action::Activate(id) => state.activate_workspace(id),
                    }
                }
            }
            ext_workspace_manager_v1::Request::Stop => {
                resource.finished();

                let state = state.ext_workspace_manager_state();
                state.instances.retain(|x, _| x != resource);

                for data in state.workspace_groups.values_mut() {
                    data.instances
                        .retain(|instance| instance.data() != Some(resource));
                }

                for data in state.workspaces.values_mut() {
                    data.instances
                        .retain(|instance| instance.data() != Some(resource));
                }
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(state: &mut D, _client: ClientId, resource: &ExtWorkspaceManagerV1, _data: &()) {
        let state = state.ext_workspace_manager_state();
        state.instances.retain(|x, _| x != resource);
    }
}

impl<D> Dispatch<ExtWorkspaceHandleV1, ExtWorkspaceManagerV1, D> for ExtWorkspaceManagerState
where
    D: Dispatch<ExtWorkspaceHandleV1, ExtWorkspaceManagerV1>,
    D: ExtWorkspaceHandler,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ExtWorkspaceHandleV1,
        request: <ExtWorkspaceHandleV1 as Resource>::Request,
        data: &ExtWorkspaceManagerV1,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let protocol_state = state.ext_workspace_manager_state();

        let Some((workspace, _)) = protocol_state
            .workspaces
            .iter()
            .find(|(_, data)| data.instances.contains(resource))
        else {
            return;
        };
        let workspace = *workspace;

        match request {
            ext_workspace_handle_v1::Request::Activate => {
                let actions = protocol_state.instances.get_mut(data).unwrap();
                actions.push(Action::Activate(workspace));
            }
            ext_workspace_handle_v1::Request::Deactivate => (),
            ext_workspace_handle_v1::Request::Assign { workspace_group } => {
                if let Some(output) = protocol_state
                    .workspace_groups
                    .iter()
                    .find(|(_, data)| data.instances.contains(&workspace_group))
                    .map(|(output, _)| output.clone())
                {
                    let actions = protocol_state.instances.get_mut(data).unwrap();
                    actions.push(Action::Assign(workspace, output.downgrade()));
                }
            }
            ext_workspace_handle_v1::Request::Remove => (),
            ext_workspace_handle_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ExtWorkspaceHandleV1,
        _data: &ExtWorkspaceManagerV1,
    ) {
        let state = state.ext_workspace_manager_state();
        for data in state.workspaces.values_mut() {
            data.instances.retain(|instance| instance != resource);
        }
    }
}

impl<D> Dispatch<ExtWorkspaceGroupHandleV1, ExtWorkspaceManagerV1, D> for ExtWorkspaceManagerState
where
    D: Dispatch<ExtWorkspaceGroupHandleV1, ExtWorkspaceManagerV1>,
    D: ExtWorkspaceHandler,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &ExtWorkspaceGroupHandleV1,
        request: <ExtWorkspaceGroupHandleV1 as Resource>::Request,
        _data: &ExtWorkspaceManagerV1,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_workspace_group_handle_v1::Request::CreateWorkspace { .. } => (),
            ext_workspace_group_handle_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ExtWorkspaceGroupHandleV1,
        _data: &ExtWorkspaceManagerV1,
    ) {
        let state = state.ext_workspace_manager_state();
        for data in state.workspace_groups.values_mut() {
            data.instances.retain(|instance| instance != resource);
        }
    }
}

#[macro_export]
macro_rules! delegate_ext_workspace {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_manager_v1::ExtWorkspaceManagerV1: $crate::protocols::ext_workspace::ExtWorkspaceGlobalData
        ] => $crate::protocols::ext_workspace::ExtWorkspaceManagerState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_manager_v1::ExtWorkspaceManagerV1: ()
        ] => $crate::protocols::ext_workspace::ExtWorkspaceManagerState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_handle_v1::ExtWorkspaceHandleV1: smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_manager_v1::ExtWorkspaceManagerV1
        ] => $crate::protocols::ext_workspace::ExtWorkspaceManagerState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_group_handle_v1::ExtWorkspaceGroupHandleV1: smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_manager_v1::ExtWorkspaceManagerV1
        ] => $crate::protocols::ext_workspace::ExtWorkspaceManagerState);
    };
}
