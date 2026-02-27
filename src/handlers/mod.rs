mod compositor;
mod layer_shell;
mod xdg_shell;

use std::fs::File;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::drm::DrmNode;
use smithay::backend::input::{InputEvent, TabletToolDescriptor};
use smithay::desktop::{PopupKind, PopupManager};
use smithay::input::dnd::{self, DnDGrab, DndGrabHandler, DndTarget};
use smithay::input::pointer::{CursorIcon, CursorImageStatus, Focus, PointerHandle};
use smithay::input::{keyboard, Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::rustix::fs::{fcntl_setfl, OFlags};
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::{Logical, Point, Rectangle, Serial};
use smithay::wayland::compositor::{get_parent, with_states};
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};
use smithay::wayland::drm_lease::{
    DrmLease, DrmLeaseBuilder, DrmLeaseHandler, DrmLeaseRequest, DrmLeaseState, LeaseRejected,
};
use smithay::wayland::fractional_scale::FractionalScaleHandler;
use smithay::wayland::idle_inhibit::IdleInhibitHandler;
use smithay::wayland::idle_notify::{IdleNotifierHandler, IdleNotifierState};
use smithay::wayland::input_method::{InputMethodHandler, PopupSurface};
use smithay::wayland::keyboard_shortcuts_inhibit::{
    KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::pointer_constraints::{with_pointer_constraint, PointerConstraintsHandler};
use smithay::wayland::security_context::{
    SecurityContext, SecurityContextHandler, SecurityContextListenerSource,
};
use smithay::wayland::selection::data_device::{
    set_data_device_focus, DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler,
};
use smithay::wayland::selection::ext_data_control::{
    DataControlHandler as ExtDataControlHandler, DataControlState as ExtDataControlState,
};
use smithay::wayland::selection::primary_selection::{
    set_primary_focus, PrimarySelectionHandler, PrimarySelectionState,
};
use smithay::wayland::selection::wlr_data_control::{
    DataControlHandler as WlrDataControlHandler, DataControlState as WlrDataControlState,
};
use smithay::wayland::selection::{SelectionHandler, SelectionTarget};
use smithay::wayland::session_lock::{
    LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
};
use smithay::wayland::tablet_manager::TabletSeatHandler;
use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};
use smithay::{
    delegate_cursor_shape, delegate_data_control, delegate_data_device, delegate_dmabuf,
    delegate_drm_lease, delegate_ext_data_control, delegate_fractional_scale,
    delegate_idle_inhibit, delegate_idle_notify, delegate_input_method_manager,
    delegate_keyboard_shortcuts_inhibit, delegate_output, delegate_pointer_constraints,
    delegate_pointer_gestures, delegate_presentation, delegate_primary_selection,
    delegate_relative_pointer, delegate_seat, delegate_security_context, delegate_session_lock,
    delegate_single_pixel_buffer, delegate_tablet_manager, delegate_text_input_manager,
    delegate_viewporter, delegate_xdg_activation,
};

pub use crate::handlers::xdg_shell::KdeDecorationsModeState;
use crate::layout::workspace::WorkspaceId;
use crate::layout::ActivateWindow;
use crate::niri::{DndIcon, NewClient, State};
use crate::protocols::ext_workspace::{self, ExtWorkspaceHandler, ExtWorkspaceManagerState};
use crate::protocols::foreign_toplevel::{
    self, ForeignToplevelHandler, ForeignToplevelManagerState,
};
use crate::protocols::gamma_control::{GammaControlHandler, GammaControlManagerState};
use crate::protocols::mutter_x11_interop::MutterX11InteropHandler;
use crate::protocols::output_management::{OutputManagementHandler, OutputManagementManagerState};
use crate::protocols::screencopy::{Screencopy, ScreencopyHandler, ScreencopyManagerState};
use crate::protocols::virtual_pointer::{
    VirtualPointerAxisEvent, VirtualPointerButtonEvent, VirtualPointerHandler,
    VirtualPointerInputBackend, VirtualPointerManagerState, VirtualPointerMotionAbsoluteEvent,
    VirtualPointerMotionEvent,
};
use crate::utils::{output_size, send_scale_transform};
use crate::{
    delegate_ext_workspace, delegate_foreign_toplevel, delegate_gamma_control,
    delegate_mutter_x11_interop, delegate_output_management, delegate_screencopy,
    delegate_virtual_pointer,
};

pub const XDG_ACTIVATION_TOKEN_TIMEOUT: Duration = Duration::from_secs(10);

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.niri.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, mut image: CursorImageStatus) {
        // FIXME: this hack should be removable once the screenshot UI is tracked with a
        // PointerFocus properly.
        if self.niri.screenshot_ui.is_open() {
            image = CursorImageStatus::Named(CursorIcon::Crosshair);
        }
        self.niri.cursor_manager.set_cursor_image(image);
        // FIXME: more granular
        self.niri.queue_redraw_all();
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.niri.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
    }

    fn led_state_changed(&mut self, _seat: &Seat<Self>, led_state: keyboard::LedState) {
        let keyboards = self
            .niri
            .devices
            .keys()
            .filter(|device| device.has_capability(input::DeviceCapability::Keyboard))
            .cloned();

        for mut keyboard in keyboards {
            keyboard.led_update(led_state.into());
        }
    }
}
delegate_seat!(State);
delegate_cursor_shape!(State);
delegate_pointer_gestures!(State);
delegate_relative_pointer!(State);
delegate_text_input_manager!(State);

impl TabletSeatHandler for State {
    fn tablet_tool_image(&mut self, _tool: &TabletToolDescriptor, image: CursorImageStatus) {
        // FIXME: tablet tools should have their own cursors.
        self.niri.cursor_manager.set_cursor_image(image);
        // FIXME: granular.
        self.niri.queue_redraw_all();
    }
}
delegate_tablet_manager!(State);

impl PointerConstraintsHandler for State {
    fn new_constraint(&mut self, _surface: &WlSurface, _pointer: &PointerHandle<Self>) {
        // Pointer constraints track pointer focus internally, so make sure it's up to date before
        // activating a new one.
        self.refresh_pointer_contents();

        self.niri.maybe_activate_pointer_constraint();
    }

    fn cursor_position_hint(
        &mut self,
        surface: &WlSurface,
        pointer: &PointerHandle<Self>,
        location: Point<f64, Logical>,
    ) {
        let is_constraint_active = with_pointer_constraint(surface, pointer, |constraint| {
            constraint.is_some_and(|c| c.is_active())
        });

        if !is_constraint_active {
            return;
        }

        // Note: this is surface under pointer, not pointer focus. So if you start, say, a
        // middle-drag in Blender, then touchpad-swipe the window away, the surface under pointer
        // will change, even though the real pointer focus remains on the Blender surface due to
        // the click grab.
        //
        // Ideally we would just use the constraint surface, but we need its origin. So this is
        // more of a hack because pointer contents has the surface origin available.
        //
        // FIXME: use the constraint surface somehow, don't use pointer contents.
        let Some((ref surface_under_pointer, origin)) = self.niri.pointer_contents.surface else {
            return;
        };

        if surface_under_pointer != surface {
            return;
        }

        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }

        let target = self
            .niri
            .output_for_root(&root)
            .and_then(|output| self.niri.global_space.output_geometry(output))
            .map_or(origin + location, |mut output_geometry| {
                // i32 sizes are exclusive, but f64 sizes are inclusive.
                output_geometry.size -= (1, 1).into();
                (origin + location).constrain(output_geometry.to_f64())
            });
        pointer.set_location(target);

        // Redraw to update the cursor position if it's visible.
        if self.niri.pointer_visibility.is_visible() {
            // FIXME: redraw only outputs overlapping the cursor.
            self.niri.queue_redraw_all();
        }
    }
}
delegate_pointer_constraints!(State);

impl InputMethodHandler for State {
    fn new_popup(&mut self, surface: PopupSurface) {
        let popup = PopupKind::InputMethod(surface);
        if let Some(output) = self.output_for_popup(&popup) {
            let scale = output.current_scale();
            let transform = output.current_transform();
            let wl_surface = popup.wl_surface();
            with_states(wl_surface, |data| {
                send_scale_transform(wl_surface, data, scale, transform);
            });
        }

        self.unconstrain_popup(&popup);

        if let Err(err) = self.niri.popups.track_popup(popup) {
            warn!("error tracking ime popup {err:?}");
        }
    }

    fn popup_repositioned(&mut self, surface: PopupSurface) {
        let popup = PopupKind::InputMethod(surface);
        self.unconstrain_popup(&popup);
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.niri
            .layout
            .find_window_and_output(parent)
            .map(|(mapped, _)| mapped.window.geometry())
            .unwrap_or_default()
    }
}

impl KeyboardShortcutsInhibitHandler for State {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.niri.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        // FIXME: show a confirmation dialog with a "remember for this application" kind of toggle.
        inhibitor.activate();
        self.niri
            .keyboard_shortcuts_inhibiting_surfaces
            .insert(inhibitor.wl_surface().clone(), inhibitor);
    }

    fn inhibitor_destroyed(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        self.niri
            .keyboard_shortcuts_inhibiting_surfaces
            .remove(&inhibitor.wl_surface().clone());
    }
}

delegate_input_method_manager!(State);
delegate_keyboard_shortcuts_inhibit!(State);

impl SelectionHandler for State {
    type SelectionUserData = Arc<[u8]>;

    fn send_selection(
        &mut self,
        _ty: SelectionTarget,
        _mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        user_data: &Self::SelectionUserData,
    ) {
        let _span = tracy_client::span!("send_selection");

        let buf = user_data.clone();
        thread::spawn(move || {
            // Clear O_NONBLOCK, otherwise File::write_all() will stop halfway.
            if let Err(err) = fcntl_setfl(&fd, OFlags::empty()) {
                warn!("error clearing flags on selection target fd: {err:?}");
            }
            if let Err(err) = File::from(fd).write_all(&buf) {
                warn!("error writing selection: {err:?}");
            }
        });
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.niri.data_device_state
    }
}

impl WaylandDndGrabHandler for State {
    fn dnd_requested<S: dnd::Source>(
        &mut self,
        source: S,
        icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: dnd::GrabType,
    ) {
        self.niri.dnd_icon = icon.map(|surface| DndIcon {
            surface,
            offset: Point::new(0, 0),
        });

        match type_ {
            dnd::GrabType::Pointer => {
                let pointer = seat.get_pointer().unwrap();
                let start_data = pointer.grab_start_data().unwrap();
                let grab =
                    DnDGrab::new_pointer(&self.niri.display_handle, start_data, source, seat);
                pointer.set_grab(self, grab, serial, Focus::Keep);
            }
            dnd::GrabType::Touch => {
                let touch = seat.get_touch().unwrap();
                let start_data = touch.grab_start_data().unwrap();
                let grab = DnDGrab::new_touch(&self.niri.display_handle, start_data, source, seat);
                touch.set_grab(self, grab, serial);
            }
        }

        // FIXME: more granular
        self.niri.queue_redraw_all();
    }
}

impl DndGrabHandler for State {
    fn dropped(
        &mut self,
        target: Option<DndTarget<'_, Self>>,
        validated: bool,
        _seat: Seat<Self>,
        location: Point<f64, Logical>,
    ) {
        let target: Option<&WlSurface> = target.map(DndTarget::into_inner);
        trace!("dnd dropped, target: {target:?}, validated: {validated}");

        // End DnD before activating a specific window below so that it takes precedence.
        self.niri.layout.dnd_end();

        // Activate the target output, since that's how Firefox drag-tab-into-new-window works for
        // example. On successful drop, additionally activate the target window.
        let mut activate_output = true;
        if let Some(target) = validated.then_some(target).flatten() {
            let root = self.niri.find_root_shell_surface(target);
            if let Some((mapped, _)) = self.niri.layout.find_window_and_output(&root) {
                let window = mapped.window.clone();
                self.niri.layout.activate_window(&window);
                self.niri.layer_shell_on_demand_focus = None;
                activate_output = false;
            }
        }

        if activate_output {
            // Find the output from drop coordinates.
            if let Some((output, _)) = self.niri.output_under(location) {
                let output = output.clone();
                self.niri.layout.focus_output(&output);
            }
        }

        self.niri.dnd_icon = None;
        // FIXME: more granular
        self.niri.queue_redraw_all();
    }
}

delegate_data_device!(State);

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.niri.primary_selection_state
    }
}
delegate_primary_selection!(State);

impl WlrDataControlHandler for State {
    fn data_control_state(&mut self) -> &mut WlrDataControlState {
        &mut self.niri.wlr_data_control_state
    }
}

delegate_data_control!(State);

impl ExtDataControlHandler for State {
    fn data_control_state(&mut self) -> &mut ExtDataControlState {
        &mut self.niri.ext_data_control_state
    }
}

delegate_ext_data_control!(State);

impl OutputHandler for State {
    fn output_bound(&mut self, output: Output, wl_output: WlOutput) {
        foreign_toplevel::on_output_bound(self, &output, &wl_output);
        ext_workspace::on_output_bound(self, &output, &wl_output);
    }
}
delegate_output!(State);

delegate_presentation!(State);

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.niri.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        if self.backend.import_dmabuf(&dmabuf) {
            let _ = notifier.successful::<State>();
        } else {
            notifier.failed();
        }
    }
}
delegate_dmabuf!(State);

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.niri.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        self.niri.lock(confirmation);
    }

    fn unlock(&mut self) {
        self.niri.unlock();
        self.niri.activate_monitors(&mut self.backend);
        self.niri.notify_activity();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else {
            warn!("no Output matching WlOutput");
            return;
        };

        configure_lock_surface(&surface, &output);
        self.niri.new_lock_surface(surface, &output);
    }
}
delegate_session_lock!(State);

pub fn configure_lock_surface(surface: &LockSurface, output: &Output) {
    surface.with_pending_state(|states| {
        let size = output_size(output);
        states.size = Some(size.to_i32_round());
    });
    let scale = output.current_scale();
    let transform = output.current_transform();
    let wl_surface = surface.wl_surface();
    with_states(wl_surface, |data| {
        send_scale_transform(wl_surface, data, scale, transform);
    });
    surface.send_configure();
}

impl SecurityContextHandler for State {
    fn context_created(&mut self, source: SecurityContextListenerSource, context: SecurityContext) {
        self.niri
            .event_loop
            .insert_source(source, move |client, _, state| {
                trace!("inserting a new restricted client, context={context:?}");
                state.niri.insert_client(NewClient {
                    client,
                    restricted: true,
                    credentials_unknown: false,
                });
            })
            .unwrap();
    }
}
delegate_security_context!(State);

impl IdleNotifierHandler for State {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.niri.idle_notifier_state
    }
}
delegate_idle_notify!(State);

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        self.niri.idle_inhibiting_surfaces.insert(surface);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.niri.idle_inhibiting_surfaces.remove(&surface);
    }
}
delegate_idle_inhibit!(State);

impl ForeignToplevelHandler for State {
    fn foreign_toplevel_manager_state(&mut self) -> &mut ForeignToplevelManagerState {
        &mut self.niri.foreign_toplevel_state
    }

    fn activate(&mut self, wl_surface: WlSurface) {
        if let Some((mapped, _)) = self.niri.layout.find_window_and_output(&wl_surface) {
            let window = mapped.window.clone();
            self.niri.layout.activate_window(&window);
            self.niri.layer_shell_on_demand_focus = None;
            self.niri.queue_redraw_all();
        }
    }

    fn close(&mut self, wl_surface: WlSurface) {
        if let Some((mapped, _)) = self.niri.layout.find_window_and_output(&wl_surface) {
            mapped.toplevel().send_close();
        }
    }

    fn set_fullscreen(&mut self, wl_surface: WlSurface, wl_output: Option<WlOutput>) {
        if let Some((mapped, current_output)) = self.niri.layout.find_window_and_output(&wl_surface)
        {
            let window = mapped.window.clone();

            if let Some(requested_output) = wl_output.as_ref().and_then(Output::from_resource) {
                if Some(&requested_output) != current_output {
                    self.niri.layout.move_to_output(
                        Some(&window),
                        &requested_output,
                        None,
                        ActivateWindow::Smart,
                    );
                }
            }

            self.niri.layout.set_fullscreen(&window, true);
        }
    }

    fn unset_fullscreen(&mut self, wl_surface: WlSurface) {
        if let Some((mapped, _)) = self.niri.layout.find_window_and_output(&wl_surface) {
            let window = mapped.window.clone();
            self.niri.layout.set_fullscreen(&window, false);
        }
    }

    fn set_maximized(&mut self, wl_surface: WlSurface) {
        if let Some((mapped, _)) = self.niri.layout.find_window_and_output(&wl_surface) {
            let window = mapped.window.clone();
            self.niri.layout.set_maximized(&window, true);
        }
    }

    fn unset_maximized(&mut self, wl_surface: WlSurface) {
        if let Some((mapped, _)) = self.niri.layout.find_window_and_output(&wl_surface) {
            let window = mapped.window.clone();
            self.niri.layout.set_maximized(&window, false);
        }
    }
}
delegate_foreign_toplevel!(State);

impl ExtWorkspaceHandler for State {
    fn ext_workspace_manager_state(&mut self) -> &mut ExtWorkspaceManagerState {
        &mut self.niri.ext_workspace_state
    }

    fn activate_workspace(&mut self, id: WorkspaceId) {
        let reference = niri_config::WorkspaceReference::Id(id.get());
        if let Some((mut output, index)) = self.niri.find_output_and_workspace_index(reference) {
            if let Some(active) = self.niri.layout.active_output() {
                if output.as_ref() == Some(active) {
                    output = None;
                }
            }

            if let Some(output) = output {
                self.niri.layout.focus_output(&output);
            }
            self.niri.layout.switch_workspace(index);
            // No mouse warp: assuming the layer-shell bar workspaces use-case.

            // FIXME: granular
            self.niri.queue_redraw_all();
        }
    }

    fn assign_workspace(&mut self, ws_id: WorkspaceId, output: Output) {
        let reference = niri_config::WorkspaceReference::Id(ws_id.get());
        if let Some((old_output, old_idx)) = self.niri.find_output_and_workspace_index(reference) {
            self.niri
                .layout
                .move_workspace_to_output_by_id(old_idx, old_output, &output);
        }
    }
}
delegate_ext_workspace!(State);

impl ScreencopyHandler for State {
    fn frame(&mut self, manager: &ZwlrScreencopyManagerV1, screencopy: Screencopy) {
        // If with_damage then push it onto the queue for redraw of the output,
        // otherwise render it immediately.
        if screencopy.with_damage() {
            self.niri.screencopy_state.push(manager, screencopy);
        } else {
            self.backend.with_primary_renderer(|renderer| {
                if let Err(err) = self
                    .niri
                    .render_for_screencopy_without_damage(renderer, manager, screencopy)
                {
                    warn!("error rendering for screencopy: {err:?}");
                }
            });
        }
    }

    fn screencopy_state(&mut self) -> &mut ScreencopyManagerState {
        &mut self.niri.screencopy_state
    }
}
delegate_screencopy!(State);

impl VirtualPointerHandler for State {
    fn virtual_pointer_manager_state(&mut self) -> &mut VirtualPointerManagerState {
        &mut self.niri.virtual_pointer_state
    }

    fn on_virtual_pointer_motion(&mut self, event: VirtualPointerMotionEvent) {
        self.process_input_event(InputEvent::<VirtualPointerInputBackend>::PointerMotion { event });
    }

    fn on_virtual_pointer_motion_absolute(&mut self, event: VirtualPointerMotionAbsoluteEvent) {
        self.process_input_event(
            InputEvent::<VirtualPointerInputBackend>::PointerMotionAbsolute { event },
        );
    }

    fn on_virtual_pointer_button(&mut self, event: VirtualPointerButtonEvent) {
        self.process_input_event(InputEvent::<VirtualPointerInputBackend>::PointerButton { event });
    }

    fn on_virtual_pointer_axis(&mut self, event: VirtualPointerAxisEvent) {
        self.process_input_event(InputEvent::<VirtualPointerInputBackend>::PointerAxis { event });
    }
}
delegate_virtual_pointer!(State);

impl DrmLeaseHandler for State {
    fn drm_lease_state(&mut self, node: DrmNode) -> &mut DrmLeaseState {
        self.backend
            .tty()
            .get_device_from_node(node)
            .unwrap()
            .drm_lease_state
            .as_mut()
            .unwrap()
    }

    fn lease_request(
        &mut self,
        node: DrmNode,
        request: DrmLeaseRequest,
    ) -> Result<DrmLeaseBuilder, LeaseRejected> {
        debug!(
            "Received lease request for {} connectors",
            request.connectors.len()
        );
        self.backend
            .tty()
            .get_device_from_node(node)
            .unwrap()
            .lease_request(request)
    }

    fn new_active_lease(&mut self, node: DrmNode, lease: DrmLease) {
        debug!("Lease success");
        self.backend
            .tty()
            .get_device_from_node(node)
            .unwrap()
            .new_lease(lease);
    }

    fn lease_destroyed(&mut self, node: DrmNode, lease_id: u32) {
        debug!("Destroyed lease");
        self.backend
            .tty()
            .get_device_from_node(node)
            .unwrap()
            .remove_lease(lease_id);
    }
}
delegate_drm_lease!(State);

delegate_viewporter!(State);

impl GammaControlHandler for State {
    fn gamma_control_manager_state(&mut self) -> &mut GammaControlManagerState {
        &mut self.niri.gamma_control_manager_state
    }

    fn get_gamma_size(&mut self, output: &Output) -> Option<u32> {
        match self.backend.tty().get_gamma_size(output) {
            Ok(0) => None, // Setting gamma is not supported.
            Ok(size) => Some(size),
            Err(err) => {
                warn!(
                    "error getting gamma size for output {}: {err:?}",
                    output.name()
                );
                None
            }
        }
    }

    fn set_gamma(&mut self, output: &Output, ramp: Option<Vec<u16>>) -> Option<()> {
        match self.backend.tty().set_gamma(output, ramp) {
            Ok(()) => Some(()),
            Err(err) => {
                warn!("error setting gamma for output {}: {err:?}", output.name());
                None
            }
        }
    }
}
delegate_gamma_control!(State);

struct UrgentOnlyMarker;

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.niri.activation_state
    }

    fn token_created(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        // Tokens without a serial are urgency-only. This is not specified, but it seems to be the
        // common client behavior.
        //
        // See also: https://gitlab.freedesktop.org/wayland/wayland-protocols/-/issues/150
        let Some((serial, seat)) = data.serial else {
            data.user_data.insert_if_missing(|| UrgentOnlyMarker);
            return true;
        };
        let Some(seat) = Seat::<State>::from_resource(&seat) else {
            return false;
        };

        // Widely-used clients such as Discord and Telegram make new tokens (with invalid serials)
        // upon clicking on their tray icon or on their notification. This debug flag makes that
        // work.
        //
        // Clicking on a notification sends clients a perfectly valid activation token from the
        // notification daemon, but alas they ignore it. Maybe in the future the clients are fixed,
        // and we can remove this debug flag.
        let config = self.niri.config.borrow();
        if config.debug.honor_xdg_activation_with_invalid_serial {
            return true;
        }

        // Check the serial against both a keyboard and a pointer, since layer-shell surfaces
        // with no keyboard interactivity won't have any keyboard focus.
        let kb_last_enter = seat.get_keyboard().unwrap().last_enter();
        if kb_last_enter.is_some_and(|last_enter| serial.is_no_older_than(&last_enter)) {
            return true;
        }

        let pointer_last_enter = seat.get_pointer().unwrap().last_enter();
        if pointer_last_enter.is_some_and(|last_enter| serial.is_no_older_than(&last_enter)) {
            return true;
        }

        false
    }

    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        if token_data.timestamp.elapsed() < XDG_ACTIVATION_TOKEN_TIMEOUT {
            if let Some((mapped, _)) = self.niri.layout.find_window_and_output_mut(&surface) {
                let window = mapped.window.clone();
                if token_data.user_data.get::<UrgentOnlyMarker>().is_some() {
                    mapped.set_urgent(true);
                    self.niri.queue_redraw_all();
                } else {
                    self.niri.layout.activate_window(&window);
                    self.niri.layer_shell_on_demand_focus = None;
                    self.niri.queue_redraw_all();
                }
            } else if let Some(unmapped) = self.niri.unmapped_windows.get_mut(&surface) {
                unmapped.activation_token_data = Some(token_data);
            }
        }

        self.niri.activation_state.remove_token(&token);
    }
}
delegate_xdg_activation!(State);

impl FractionalScaleHandler for State {}
delegate_fractional_scale!(State);

impl OutputManagementHandler for State {
    fn output_management_state(&mut self) -> &mut OutputManagementManagerState {
        &mut self.niri.output_management_state
    }

    fn apply_output_config(&mut self, config: niri_config::Outputs) {
        self.niri.config.borrow_mut().outputs = config;
        self.reload_output_config();
    }
}
delegate_output_management!(State);

impl MutterX11InteropHandler for State {}
delegate_mutter_x11_interop!(State);

delegate_single_pixel_buffer!(State);
