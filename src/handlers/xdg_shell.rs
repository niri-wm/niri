use std::cell::Cell;

use calloop::Interest;
use niri_config::PresetSize;
use smithay::desktop::{
    find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output, utils, LayerSurface,
    PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, PopupUngrabStrategy, Window,
    WindowSurfaceType,
};
use smithay::input::pointer::Focus;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_positioner::ConstraintAdjustment;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::{self};
use smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration;
use smithay::reexports::wayland_server::protocol::wl_output;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{self, Resource, WEnum};
use smithay::utils::{Logical, Rectangle, Serial};
use smithay::wayland::compositor::{
    add_blocker, add_pre_commit_hook, with_states, BufferAssignment, CompositorHandler as _,
    HookId, SurfaceAttributes,
};
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::input_method::InputMethodSeat;
use smithay::wayland::selection::data_device::DnDGrab;
use smithay::wayland::shell::kde::decoration::{KdeDecorationHandler, KdeDecorationState};
use smithay::wayland::shell::wlr_layer::{self, Layer};
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::xdg_foreign::{XdgForeignHandler, XdgForeignState};
use smithay::{
    delegate_kde_decoration, delegate_xdg_decoration, delegate_xdg_foreign, delegate_xdg_shell,
};
use tracing::field::Empty;

use crate::input::move_grab::MoveGrab;
use crate::input::resize_grab::ResizeGrab;
use crate::input::touch_resize_grab::TouchResizeGrab;
use crate::input::{PointerOrTouchStartData, DOUBLE_CLICK_TIME};
use crate::layout::ActivateWindow;
use crate::niri::{CastTarget, PopupGrabState, State};
use crate::utils::transaction::Transaction;
use crate::utils::{
    get_monotonic_time, output_matches_name, send_scale_transform, update_tiled_state, ResizeEdge,
};
use crate::window::{InitialConfigureState, ResolvedWindowRules, Unmapped, WindowRef};

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.niri.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface().clone();
        let unmapped = Unmapped::new(Window::new_wayland_window(surface));
        let existing = self.niri.unmapped_windows.insert(wl_surface, unmapped);
        assert!(existing.is_none());
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let popup = PopupKind::Xdg(surface);
        self.unconstrain_popup(&popup);

        if let Err(err) = self.niri.popups.track_popup(popup) {
            warn!("error tracking popup: {err:?}");
        }
    }

    fn move_request(&mut self, surface: ToplevelSurface, _seat: WlSeat, serial: Serial) {
        let wl_surface = surface.wl_surface();

        let mut grab_start_data = None;

        // See if this comes from a pointer grab.
        let pointer = self.niri.seat.get_pointer().unwrap();
        pointer.with_grab(|grab_serial, grab| {
            if grab_serial == serial {
                let start_data = grab.start_data();
                if let Some((focus, _)) = &start_data.focus {
                    if focus.id().same_client_as(&wl_surface.id()) {
                        // Deny move requests from DnD grabs to work around
                        // https://gitlab.gnome.org/GNOME/gtk/-/issues/7113
                        let is_dnd_grab = grab.as_any().is::<DnDGrab<Self>>();

                        if !is_dnd_grab {
                            grab_start_data =
                                Some(PointerOrTouchStartData::Pointer(start_data.clone()));
                        }
                    }
                }
            }
        });

        // See if this comes from a touch grab.
        if let Some(touch) = self.niri.seat.get_touch() {
            touch.with_grab(|grab_serial, grab| {
                if grab_serial == serial {
                    let start_data = grab.start_data();
                    if let Some((focus, _)) = &start_data.focus {
                        if focus.id().same_client_as(&wl_surface.id()) {
                            // Deny move requests from DnD grabs to work around
                            // https://gitlab.gnome.org/GNOME/gtk/-/issues/7113
                            let is_dnd_grab = grab.as_any().is::<DnDGrab<Self>>();

                            if !is_dnd_grab {
                                grab_start_data =
                                    Some(PointerOrTouchStartData::Touch(start_data.clone()));
                            }
                        }
                    }
                }
            });
        }

        let Some(start_data) = grab_start_data else {
            return;
        };

        let Some((mapped, output)) = self.niri.layout.find_window_and_output(wl_surface) else {
            return;
        };

        let Some(output) = output else {
            return;
        };

        let window = mapped.window.clone();
        let output = output.clone();

        match &start_data {
            PointerOrTouchStartData::Pointer(_) => {
                if let Some(grab) = MoveGrab::new(self, start_data, window.clone(), true) {
                    pointer.set_grab(self, grab, serial, Focus::Clear);
                }
            }
            PointerOrTouchStartData::Touch(_) => {
                let touch = self.niri.seat.get_touch().unwrap();
                if let Some(grab) = MoveGrab::new(self, start_data, window.clone(), true) {
                    touch.set_grab(self, grab, serial);
                }
            }
        }

        self.niri.queue_redraw(&output);
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let wl_surface = surface.wl_surface();

        let mut grab_start_data = None;

        // See if this comes from a pointer grab.
        let pointer = self.niri.seat.get_pointer().unwrap();
        if pointer.has_grab(serial) {
            if let Some(start_data) = pointer.grab_start_data() {
                if let Some((focus, _)) = &start_data.focus {
                    if focus.id().same_client_as(&wl_surface.id()) {
                        grab_start_data = Some(PointerOrTouchStartData::Pointer(start_data));
                    }
                }
            }
        }

        // See if this comes from a touch grab.
        if let Some(touch) = self.niri.seat.get_touch() {
            if touch.has_grab(serial) {
                if let Some(start_data) = touch.grab_start_data() {
                    if let Some((focus, _)) = &start_data.focus {
                        if focus.id().same_client_as(&wl_surface.id()) {
                            grab_start_data = Some(PointerOrTouchStartData::Touch(start_data));
                        }
                    }
                }
            }
        }

        let Some(start_data) = grab_start_data else {
            return;
        };

        let Some((mapped, _)) = self.niri.layout.find_window_and_output(wl_surface) else {
            return;
        };

        let edges = ResizeEdge::from(edges);
        let window = mapped.window.clone();

        // See if we got a double resize-click gesture.
        let time = get_monotonic_time();
        let last_cell = mapped.last_interactive_resize_start();
        let mut last = last_cell.get();
        last_cell.set(Some((time, edges)));

        // Floating windows don't have either of the double-resize-click gestures, so just allow it
        // to resize.
        if mapped.is_floating() {
            last = None;
            last_cell.set(None);
        }

        if let Some((last_time, last_edges)) = last {
            if time.saturating_sub(last_time) <= DOUBLE_CLICK_TIME {
                // Allow quick resize after a triple click.
                last_cell.set(None);

                let intersection = edges.intersection(last_edges);
                if intersection.intersects(ResizeEdge::LEFT_RIGHT) {
                    // FIXME: don't activate once we can pass specific windows to actions.
                    self.niri.layout.activate_window(&window);
                    self.niri.layer_shell_on_demand_focus = None;
                    self.niri.layout.toggle_full_width();
                }
                if intersection.intersects(ResizeEdge::TOP_BOTTOM) {
                    self.niri.layer_shell_on_demand_focus = None;
                    self.niri.layout.reset_window_height(Some(&window));
                }
                // FIXME: granular.
                self.niri.queue_redraw_all();
                return;
            }
        }

        if !self
            .niri
            .layout
            .interactive_resize_begin(window.clone(), edges)
        {
            return;
        }

        match start_data {
            PointerOrTouchStartData::Pointer(start_data) => {
                let grab = ResizeGrab::new(start_data, window);
                pointer.set_grab(self, grab, serial, Focus::Clear);
            }
            PointerOrTouchStartData::Touch(start_data) => {
                let touch = self.niri.seat.get_touch().unwrap();
                let grab = TouchResizeGrab::new(start_data, window);
                touch.set_grab(self, grab, serial);
            }
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&PopupKind::Xdg(surface.clone()));
        surface.send_repositioned(token);
    }

    fn grab(&mut self, surface: PopupSurface, _seat: WlSeat, serial: Serial) {
        // HACK: ignore grabs (pretend they work without actually grabbing) if the input method has
        // a grab. It will likely need refactors in Smithay to support properly since grabs just
        // replace each other.
        // FIXME: do this properly.
        if self.niri.seat.input_method().keyboard_grabbed() {
            trace!("ignoring popup grab because IME has keyboard grabbed");
            return;
        }

        let popup = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&popup) else {
            trace!("ignoring popup grab because no root surface");
            return;
        };

        // We need to hand out the grab in a way consistent with what update_keyboard_focus()
        // thinks the current focus is, otherwise it will desync and cause weird issues with
        // keyboard focus being at the wrong place.
        if self.niri.exit_confirm_dialog.is_open() {
            trace!("ignoring popup grab because the exit confirm dialog is open");
            let _ = PopupManager::dismiss_popup(&root, &popup);
            return;
        } else if self.niri.is_locked() {
            if Some(&root) != self.niri.lock_surface_focus().as_ref() {
                trace!("ignoring popup grab because the session is locked");
                let _ = PopupManager::dismiss_popup(&root, &popup);
                return;
            }
        } else if self.niri.screenshot_ui.is_open() {
            trace!("ignoring popup grab because the screenshot UI is open");
            let _ = PopupManager::dismiss_popup(&root, &popup);
            return;
        } else if let Some(output) = self.niri.layout.active_output() {
            let layers = layer_map_for_output(output);

            // FIXME: somewhere here we probably need to check is_overview_open to match the logic
            // in update_keyboard_focus().

            if let Some(layer) = layers.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL) {
                // This is a grab for a layer surface.

                if let Some(mapped) = self.niri.mapped_layer_surfaces.get(layer) {
                    if mapped.place_within_backdrop() {
                        trace!("ignoring popup grab for a layer surface within overview backdrop");
                        let _ = PopupManager::dismiss_popup(&root, &popup);
                        return;
                    }
                }
            } else {
                // This is a grab for a regular window; check that there's no layer surface with a
                // higher input priority.

                if layers.layers_on(Layer::Overlay).any(|l| {
                    (l.cached_state().keyboard_interactivity
                        == wlr_layer::KeyboardInteractivity::Exclusive
                        || Some(l) == self.niri.layer_shell_on_demand_focus.as_ref())
                        && self.niri.mapped_layer_surfaces.contains_key(l)
                }) {
                    trace!("ignoring toplevel popup grab because the overlay layer has focus");
                    let _ = PopupManager::dismiss_popup(&root, &popup);
                    return;
                }

                let mon = self.niri.layout.monitor_for_output(output).unwrap();
                if !mon.render_above_top_layer()
                    && layers.layers_on(Layer::Top).any(|l| {
                        (l.cached_state().keyboard_interactivity
                            == wlr_layer::KeyboardInteractivity::Exclusive
                            || Some(l) == self.niri.layer_shell_on_demand_focus.as_ref())
                            && self.niri.mapped_layer_surfaces.contains_key(l)
                    })
                {
                    trace!("ignoring toplevel popup grab because the top layer has focus");
                    let _ = PopupManager::dismiss_popup(&root, &popup);
                    return;
                }

                let layout_focus = self.niri.layout.focus();
                if Some(&root) != layout_focus.map(|win| win.toplevel().wl_surface()) {
                    trace!("ignoring toplevel popup grab because another window has focus");
                    let _ = PopupManager::dismiss_popup(&root, &popup);
                    return;
                }
            }
        } else {
            trace!("ignoring popup grab because no output is active");
            let _ = PopupManager::dismiss_popup(&root, &popup);
            return;
        }

        let seat = &self.niri.seat;
        let mut grab = match self
            .niri
            .popups
            .grab_popup(root.clone(), popup, seat, serial)
        {
            Ok(grab) => grab,
            Err(err) => {
                trace!("ignoring popup grab: {err:?}");
                return;
            }
        };

        let keyboard = seat.get_keyboard().unwrap();
        let pointer = seat.get_pointer().unwrap();

        let can_receive_keyboard_focus = self
            .niri
            .layout
            .active_output()
            .and_then(|output| {
                layer_map_for_output(output)
                    .layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)
                    .map(|layer_surface| layer_surface.can_receive_keyboard_focus())
            })
            .unwrap_or(true);

        let keyboard_grab_mismatches = keyboard.is_grabbed()
            && !(keyboard.has_grab(serial)
                || grab
                    .previous_serial()
                    .map_or(true, |s| keyboard.has_grab(s)));
        let pointer_grab_mismatches = pointer.is_grabbed()
            && !(pointer.has_grab(serial)
                || grab.previous_serial().map_or(true, |s| pointer.has_grab(s)));
        if (can_receive_keyboard_focus && keyboard_grab_mismatches) || pointer_grab_mismatches {
            trace!("ignoring popup grab because of current grab mismatch");
            grab.ungrab(PopupUngrabStrategy::All);
            return;
        }

        trace!("new grab for root {:?}", root);
        if can_receive_keyboard_focus {
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        self.niri.popup_grab = Some(PopupGrabState {
            root,
            grab,
            has_keyboard_grab: can_receive_keyboard_focus,
        });
    }

    fn maximize_request(&mut self, toplevel: ToplevelSurface) {
        if let Some((mapped, _)) = self
            .niri
            .layout
            .find_window_and_output_mut(toplevel.wl_surface())
        {
            // A configure is required in response to this event regardless if there are pending
            // changes.
            mapped.set_needs_configure();

            let window = mapped.window.clone();
            self.niri.layout.set_maximized(&window, true);
        } else if let Some(unmapped) = self.niri.unmapped_windows.get_mut(toplevel.wl_surface()) {
            match &mut unmapped.state {
                InitialConfigureState::NotConfigured {
                    wants_maximized, ..
                } => {
                    *wants_maximized = true;

                    // The required configure will be the initial configure.
                }
                InitialConfigureState::Configured {
                    rules,
                    output,
                    is_pending_maximized,
                    ..
                } => {
                    // Figure out the monitor following a similar logic to initial configure.
                    // FIXME: deduplicate.
                    let mon = output
                        .as_ref()
                        .and_then(|o| self.niri.layout.monitor_for_output(o))
                        .map(|mon| (mon, false))
                        // If not, check if we have a parent with a monitor.
                        .or_else(|| {
                            toplevel
                                .parent()
                                .and_then(|parent| self.niri.layout.find_window_and_output(&parent))
                                .and_then(|(_win, output)| output)
                                .and_then(|o| self.niri.layout.monitor_for_output(o))
                                .map(|mon| (mon, true))
                        })
                        // If not, fall back to the active monitor.
                        .or_else(|| {
                            self.niri
                                .layout
                                .active_monitor_ref()
                                .map(|mon| (mon, false))
                        });

                    *output = mon
                        .filter(|(_, parent)| !parent)
                        .map(|(mon, _)| mon.output().clone());
                    let mon = mon.map(|(mon, _)| mon);

                    let ws = mon
                        .map(|mon| mon.active_workspace_ref())
                        .or_else(|| self.niri.layout.active_workspace());

                    if let Some(ws) = ws {
                        // If the window is pending fullscreen, then this will do nothing. But
                        // that's expected: the window remains fullscreen, and we simply remember
                        // that it is now pending maximized.
                        *is_pending_maximized = true;
                        toplevel.with_pending_state(|state| {
                            if !state.states.contains(xdg_toplevel::State::Fullscreen) {
                                state.states.set(xdg_toplevel::State::Maximized);
                            }
                        });
                        ws.configure_new_window(&unmapped.window, None, None, false, rules);
                    }

                    // We already sent the initial configure, so we need to reconfigure.
                    toplevel.send_configure();
                }
            }
        } else {
            error!("couldn't find the toplevel in maximize_request()");
            toplevel.send_configure();
        }
    }

    fn unmaximize_request(&mut self, toplevel: ToplevelSurface) {
        if let Some((mapped, _)) = self
            .niri
            .layout
            .find_window_and_output_mut(toplevel.wl_surface())
        {
            // A configure is required in response to this event regardless if there are pending
            // changes.
            mapped.set_needs_configure();

            let window = mapped.window.clone();
            self.niri.layout.set_maximized(&window, false);
        } else if let Some(unmapped) = self.niri.unmapped_windows.get_mut(toplevel.wl_surface()) {
            match &mut unmapped.state {
                InitialConfigureState::NotConfigured {
                    wants_maximized, ..
                } => {
                    *wants_maximized = false;

                    // The required configure will be the initial configure.
                }
                InitialConfigureState::Configured {
                    rules,
                    width,
                    height,
                    floating_width,
                    floating_height,
                    is_full_width,
                    output,
                    workspace_name,
                    is_pending_maximized,
                } => {
                    // Figure out the monitor following a similar logic to initial configure.
                    // FIXME: deduplicate.
                    let mon = workspace_name
                        .as_deref()
                        .and_then(|name| self.niri.layout.monitor_for_workspace(name))
                        .map(|mon| (mon, false));

                    let mon = mon.or_else(|| {
                        output
                            .as_ref()
                            .and_then(|o| self.niri.layout.monitor_for_output(o))
                            .map(|mon| (mon, false))
                            // If not, check if we have a parent with a monitor.
                            .or_else(|| {
                                toplevel
                                    .parent()
                                    .and_then(|parent| {
                                        self.niri.layout.find_window_and_output(&parent)
                                    })
                                    .and_then(|(_win, output)| output)
                                    .and_then(|o| self.niri.layout.monitor_for_output(o))
                                    .map(|mon| (mon, true))
                            })
                            // If not, fall back to the active monitor.
                            .or_else(|| {
                                self.niri
                                    .layout
                                    .active_monitor_ref()
                                    .map(|mon| (mon, false))
                            })
                    });

                    *output = mon
                        .filter(|(_, parent)| !parent)
                        .map(|(mon, _)| mon.output().clone());
                    let mon = mon.map(|(mon, _)| mon);

                    let ws = workspace_name
                        .as_deref()
                        .and_then(|name| mon.map(|mon| mon.find_named_workspace(name)))
                        .unwrap_or_else(|| {
                            mon.map(|mon| mon.active_workspace_ref())
                                .or_else(|| self.niri.layout.active_workspace())
                        });

                    if let Some(ws) = ws {
                        // If the window is pending fullscreen, then this will do nothing since
                        // then the Maximized state is already unset. But that's expected: the
                        // window remains fullscreen, and we simply remember that it is no
                        // longer pending maximized.
                        *is_pending_maximized = false;
                        toplevel.with_pending_state(|state| {
                            state.states.unset(xdg_toplevel::State::Maximized);
                        });

                        let is_floating = rules.compute_open_floating(&toplevel);
                        let configure_width = if is_floating {
                            *floating_width
                        } else if *is_full_width {
                            Some(PresetSize::Proportion(1.))
                        } else {
                            *width
                        };
                        let configure_height = if is_floating {
                            *floating_height
                        } else {
                            *height
                        };
                        ws.configure_new_window(
                            &unmapped.window,
                            configure_width,
                            configure_height,
                            is_floating,
                            rules,
                        );
                    }

                    // We already sent the initial configure, so we need to reconfigure.
                    toplevel.send_configure();
                }
            }
        } else {
            error!("couldn't find the toplevel in unmaximize_request()");
            toplevel.send_configure();
        }
    }

    fn fullscreen_request(
        &mut self,
        toplevel: ToplevelSurface,
        wl_output: Option<wl_output::WlOutput>,
    ) {
        let requested_output = wl_output.as_ref().and_then(Output::from_resource);

        if let Some((mapped, current_output)) = self
            .niri
            .layout
            .find_window_and_output_mut(toplevel.wl_surface())
        {
            // A configure is required in response to this event regardless if there are pending
            // changes.
            mapped.set_needs_configure();

            let window = mapped.window.clone();

            if let Some(requested_output) = requested_output {
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
        } else if let Some(unmapped) = self.niri.unmapped_windows.get_mut(toplevel.wl_surface()) {
            match &mut unmapped.state {
                InitialConfigureState::NotConfigured {
                    wants_fullscreen, ..
                } => {
                    *wants_fullscreen = Some(requested_output);

                    // The required configure will be the initial configure.
                }
                InitialConfigureState::Configured { rules, output, .. } => {
                    // Figure out the monitor following a similar logic to initial configure.
                    // FIXME: deduplicate.
                    let mon = requested_output
                        .as_ref()
                        // If none requested, try currently configured output.
                        .or(output.as_ref())
                        .and_then(|o| self.niri.layout.monitor_for_output(o))
                        .map(|mon| (mon, false))
                        // If not, check if we have a parent with a monitor.
                        .or_else(|| {
                            toplevel
                                .parent()
                                .and_then(|parent| self.niri.layout.find_window_and_output(&parent))
                                .and_then(|(_win, output)| output)
                                .and_then(|o| self.niri.layout.monitor_for_output(o))
                                .map(|mon| (mon, true))
                        })
                        // If not, fall back to the active monitor.
                        .or_else(|| {
                            self.niri
                                .layout
                                .active_monitor_ref()
                                .map(|mon| (mon, false))
                        });

                    *output = mon
                        .filter(|(_, parent)| !parent)
                        .map(|(mon, _)| mon.output().clone());
                    let mon = mon.map(|(mon, _)| mon);

                    let ws = mon
                        .map(|mon| mon.active_workspace_ref())
                        .or_else(|| self.niri.layout.active_workspace());

                    if let Some(ws) = ws {
                        toplevel.with_pending_state(|state| {
                            state.states.set(xdg_toplevel::State::Fullscreen);
                            state.states.unset(xdg_toplevel::State::Maximized);
                        });
                        ws.configure_new_window(&unmapped.window, None, None, false, rules);
                    }

                    // We already sent the initial configure, so we need to reconfigure.
                    toplevel.send_configure();
                }
            }
        } else {
            error!("couldn't find the toplevel in fullscreen_request()");
            toplevel.send_configure();
        }
    }

    fn unfullscreen_request(&mut self, toplevel: ToplevelSurface) {
        if let Some((mapped, _)) = self
            .niri
            .layout
            .find_window_and_output_mut(toplevel.wl_surface())
        {
            // A configure is required in response to this event regardless if there are pending
            // changes.
            mapped.set_needs_configure();

            let window = mapped.window.clone();
            self.niri.layout.set_fullscreen(&window, false);
        } else if let Some(unmapped) = self.niri.unmapped_windows.get_mut(toplevel.wl_surface()) {
            match &mut unmapped.state {
                InitialConfigureState::NotConfigured {
                    wants_fullscreen, ..
                } => {
                    *wants_fullscreen = None;

                    // The required configure will be the initial configure.
                }
                InitialConfigureState::Configured {
                    rules,
                    width,
                    height,
                    floating_width,
                    floating_height,
                    is_full_width,
                    output,
                    workspace_name,
                    is_pending_maximized,
                } => {
                    // Figure out the monitor following a similar logic to initial configure.
                    // FIXME: deduplicate.
                    let mon = workspace_name
                        .as_deref()
                        .and_then(|name| self.niri.layout.monitor_for_workspace(name))
                        .map(|mon| (mon, false));

                    let mon = mon.or_else(|| {
                        output
                            .as_ref()
                            .and_then(|o| self.niri.layout.monitor_for_output(o))
                            .map(|mon| (mon, false))
                            // If not, check if we have a parent with a monitor.
                            .or_else(|| {
                                toplevel
                                    .parent()
                                    .and_then(|parent| {
                                        self.niri.layout.find_window_and_output(&parent)
                                    })
                                    .and_then(|(_win, output)| output)
                                    .and_then(|o| self.niri.layout.monitor_for_output(o))
                                    .map(|mon| (mon, true))
                            })
                            // If not, fall back to the active monitor.
                            .or_else(|| {
                                self.niri
                                    .layout
                                    .active_monitor_ref()
                                    .map(|mon| (mon, false))
                            })
                    });

                    *output = mon
                        .filter(|(_, parent)| !parent)
                        .map(|(mon, _)| mon.output().clone());
                    let mon = mon.map(|(mon, _)| mon);

                    let ws = workspace_name
                        .as_deref()
                        .and_then(|name| mon.map(|mon| mon.find_named_workspace(name)))
                        .unwrap_or_else(|| {
                            mon.map(|mon| mon.active_workspace_ref())
                                .or_else(|| self.niri.layout.active_workspace())
                        });

                    if let Some(ws) = ws {
                        toplevel.with_pending_state(|state| {
                            state.states.unset(xdg_toplevel::State::Fullscreen);

                            if *is_pending_maximized {
                                state.states.set(xdg_toplevel::State::Maximized);
                            }
                        });

                        let is_floating = rules.compute_open_floating(&toplevel);
                        let configure_width = if is_floating {
                            *floating_width
                        } else if *is_full_width {
                            Some(PresetSize::Proportion(1.))
                        } else {
                            *width
                        };
                        let configure_height = if is_floating {
                            *floating_height
                        } else {
                            *height
                        };
                        ws.configure_new_window(
                            &unmapped.window,
                            configure_width,
                            configure_height,
                            is_floating,
                            rules,
                        );
                    }

                    // We already sent the initial configure, so we need to reconfigure.
                    toplevel.send_configure();
                }
            }
        } else {
            error!("couldn't find the toplevel in unfullscreen_request()");
            toplevel.send_configure();
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if self
            .niri
            .unmapped_windows
            .remove(surface.wl_surface())
            .is_some()
        {
            // An unmapped toplevel got destroyed.
            return;
        }

        let win_out = self
            .niri
            .layout
            .find_window_and_output(surface.wl_surface());

        let Some((mapped, output)) = win_out else {
            // I have no idea how this can happen, but I saw it happen once, in a weird interaction
            // involving laptop going to sleep and resuming.
            error!("toplevel missing from both unmapped_windows and layout");
            return;
        };
        let window = mapped.window.clone();
        let output = output.cloned();

        let id = mapped.id();
        self.niri
            .stop_casts_for_target(CastTarget::Window { id: id.get() });

        self.backend.with_primary_renderer(|renderer| {
            self.niri.layout.store_unmap_snapshot(renderer, &window);
        });

        let transaction = Transaction::new();
        let blocker = transaction.blocker();
        self.backend.with_primary_renderer(|renderer| {
            self.niri
                .layout
                .start_close_animation_for_window(renderer, &window, blocker);
        });

        let active_window = self.niri.layout.focus().map(|m| &m.window);
        let was_active = active_window == Some(&window);

        self.niri.window_mru_ui.remove_window(id);
        self.niri.layout.remove_window(&window, transaction.clone());
        self.add_default_dmabuf_pre_commit_hook(surface.wl_surface());

        // If this is the only instance, then this transaction will complete immediately, so no
        // need to set the timer.
        if !transaction.is_last() {
            transaction.register_deadline_timer(&self.niri.event_loop);
        }

        if was_active {
            self.maybe_warp_cursor_to_focus();
        }

        if let Some(output) = output {
            self.niri.queue_redraw(&output);
            self.niri.queue_redraw_mru_output();
        }
    }

    fn popup_destroyed(&mut self, surface: PopupSurface) {
        if let Some(output) = self.output_for_popup(&PopupKind::Xdg(surface)) {
            self.niri.queue_redraw(&output.clone());
        }
    }

    fn app_id_changed(&mut self, toplevel: ToplevelSurface) {
        self.update_window_rules(&toplevel);
    }

    fn title_changed(&mut self, toplevel: ToplevelSurface) {
        self.update_window_rules(&toplevel);
    }

    fn parent_changed(&mut self, toplevel: ToplevelSurface) {
        let Some(parent) = toplevel.parent() else {
            return;
        };

        if let Some((mapped, output)) = self.niri.layout.find_window_and_output_mut(&parent) {
            let output = output.cloned();
            let window = mapped.window.clone();
            if self.niri.layout.descendants_added(&window) {
                if let Some(output) = output {
                    self.niri.queue_redraw(&output);
                }
            }
        }
    }
}

delegate_xdg_shell!(State);

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        // If we want CSD, we hide this global altogether.
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(zxdg_toplevel_decoration_v1::Mode::ServerSide);
        });
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: zxdg_toplevel_decoration_v1::Mode) {
        // Set whatever the client wants, rather than our preferred mode. This especially matters
        // for SDL2 which has a bug where forcing a different (client-side) decoration mode during
        // their window creation sequence would leave the window permanently hidden.
        //
        // https://github.com/libsdl-org/SDL/issues/8173
        //
        // The bug has been fixed, but there's a ton of apps which will use the buggy version for a
        // long while...
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });

        // A configure is required in response to this event. However, if an initial configure
        // wasn't sent, then we will send this as part of the initial configure later.
        if toplevel.is_initial_configure_sent() {
            // If this is a mapped window, flag it as needs configure to avoid duplicate configures.
            let surface = toplevel.wl_surface();
            if let Some((mapped, _)) = self.niri.layout.find_window_and_output_mut(surface) {
                mapped.set_needs_configure();
            } else {
                toplevel.send_configure();
            }
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        // If we want CSD, we hide this global altogether.
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(zxdg_toplevel_decoration_v1::Mode::ServerSide);
        });

        // A configure is required in response to this event. However, if an initial configure
        // wasn't sent, then we will send this as part of the initial configure later.
        if toplevel.is_initial_configure_sent() {
            // If this is a mapped window, flag it as needs configure to avoid duplicate configures.
            let surface = toplevel.wl_surface();
            if let Some((mapped, _)) = self.niri.layout.find_window_and_output_mut(surface) {
                mapped.set_needs_configure();
            } else {
                toplevel.send_configure();
            }
        }
    }
}
delegate_xdg_decoration!(State);

/// Whether KDE server decorations are in use.
#[derive(Default, Clone)]
pub struct KdeDecorationsModeState {
    server: Cell<bool>,
}

impl KdeDecorationsModeState {
    pub fn is_server(&self) -> bool {
        self.server.get()
    }
}

impl KdeDecorationHandler for State {
    fn kde_decoration_state(&self) -> &KdeDecorationState {
        &self.niri.kde_decoration_state
    }

    fn request_mode(
        &mut self,
        surface: &WlSurface,
        decoration: &org_kde_kwin_server_decoration::OrgKdeKwinServerDecoration,
        mode: wayland_server::WEnum<org_kde_kwin_server_decoration::Mode>,
    ) {
        let WEnum::Value(mode) = mode else {
            return;
        };

        decoration.mode(mode);

        with_states(surface, |states| {
            let state = states
                .data_map
                .get_or_insert(KdeDecorationsModeState::default);
            state
                .server
                .set(mode == org_kde_kwin_server_decoration::Mode::Server);
        });
    }
}
delegate_kde_decoration!(State);

impl XdgForeignHandler for State {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.niri.xdg_foreign_state
    }
}
delegate_xdg_foreign!(State);

impl State {
    pub fn send_initial_configure(&mut self, toplevel: &ToplevelSurface) {
        let _span = tracy_client::span!("State::send_initial_configure");

        let Some(unmapped) = self.niri.unmapped_windows.get_mut(toplevel.wl_surface()) else {
            error!("window must be present in unmapped_windows in send_initial_configure()");
            return;
        };

        let config = self.niri.config.borrow();
        let rules = ResolvedWindowRules::compute(
            &config.window_rules,
            WindowRef::Unmapped(unmapped),
            self.niri.is_at_startup,
        );

        let Unmapped { window, state, .. } = unmapped;

        let InitialConfigureState::NotConfigured {
            wants_fullscreen,
            wants_maximized,
        } = state
        else {
            error!("window must not be already configured in send_initial_configure()");
            return;
        };

        // Pick the target monitor. First, check if we had a workspace set in the window rules.
        let mon = rules
            .open_on_workspace
            .as_deref()
            .and_then(|name| self.niri.layout.monitor_for_workspace(name));

        // If not, check if we had an output set in the window rules.
        let mon = mon.or_else(|| {
            rules
                .open_on_output
                .as_deref()
                .and_then(|name| {
                    self.niri
                        .global_space
                        .outputs()
                        .find(|output| output_matches_name(output, name))
                })
                .and_then(|o| self.niri.layout.monitor_for_output(o))
        });

        // If not, check if the window requested one for fullscreen.
        let mon = mon.or_else(|| {
            wants_fullscreen
                .as_ref()
                .and_then(|x| x.as_ref())
                // The monitor might not exist if the output was disconnected.
                .and_then(|o| self.niri.layout.monitor_for_output(o))
        });

        // If not, check if this is a dialog with a parent, to place it next to the parent.
        let mon = mon.map(|mon| (mon, false)).or_else(|| {
            toplevel
                .parent()
                .and_then(|parent| self.niri.layout.find_window_and_output(&parent))
                .and_then(|(_win, output)| output)
                .and_then(|o| self.niri.layout.monitor_for_output(o))
                .map(|mon| (mon, true))
        });

        // If not, use the active monitor.
        let mon = mon.or_else(|| {
            self.niri
                .layout
                .active_monitor_ref()
                .map(|mon| (mon, false))
        });

        // If we're following the parent, don't set the target output, so that when the window is
        // mapped, it fetches the possibly changed parent's output again, and shows up there.
        let output = mon
            .filter(|(_, parent)| !parent)
            .map(|(mon, _)| mon.output().clone());
        let mon = mon.map(|(mon, _)| mon);

        let mut width = None;
        let mut floating_width = None;
        let mut height = None;
        let mut floating_height = None;
        let is_full_width = rules.open_maximized.unwrap_or(false);
        let is_floating = rules.compute_open_floating(toplevel);

        // Tell the surface the preferred size and bounds for its likely output.
        let ws = rules
            .open_on_workspace
            .as_deref()
            .and_then(|name| mon.map(|mon| mon.find_named_workspace(name)))
            .unwrap_or_else(|| {
                mon.map(|mon| mon.active_workspace_ref())
                    .or_else(|| self.niri.layout.active_workspace())
            });

        let mut is_pending_maximized = false;
        if let Some(ws) = ws {
            // Set a fullscreen and maximized state based on window request and window rule.
            is_pending_maximized = (*wants_maximized && rules.open_maximized_to_edges.is_none())
                || rules.open_maximized_to_edges == Some(true);

            if (wants_fullscreen.is_some() && rules.open_fullscreen.is_none())
                || rules.open_fullscreen == Some(true)
            {
                toplevel.with_pending_state(|state| {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                });
            } else if is_pending_maximized {
                toplevel.with_pending_state(|state| {
                    state.states.set(xdg_toplevel::State::Maximized);
                });
            }

            width = ws.resolve_default_width(rules.default_width, false);
            floating_width = ws.resolve_default_width(rules.default_width, true);
            height = ws.resolve_default_height(rules.default_height, false);
            floating_height = ws.resolve_default_height(rules.default_height, true);

            let configure_width = if is_floating {
                floating_width
            } else if is_full_width {
                Some(PresetSize::Proportion(1.))
            } else {
                width
            };
            let configure_height = if is_floating { floating_height } else { height };
            ws.configure_new_window(
                window,
                configure_width,
                configure_height,
                is_floating,
                &rules,
            );
        }

        // Set the tiled state for the initial configure.
        update_tiled_state(toplevel, config.prefer_no_csd, rules.tiled_state);

        // Set the configured settings.
        *state = InitialConfigureState::Configured {
            rules,
            width,
            height,
            floating_width,
            floating_height,
            is_full_width,
            output,
            workspace_name: ws.and_then(|w| w.name().cloned()),
            is_pending_maximized,
        };

        trace!(surface = %toplevel.wl_surface().id(), "sending initial configure");
        toplevel.send_configure();
    }

    pub fn queue_initial_configure(&self, toplevel: ToplevelSurface) {
        // Send the initial configure in an idle, in case the client sent some more info after the
        // initial commit.
        self.niri.event_loop.insert_idle(move |state| {
            if !toplevel.alive() {
                return;
            }

            if let Some(unmapped) = state.niri.unmapped_windows.get(toplevel.wl_surface()) {
                if unmapped.needs_initial_configure() {
                    state.send_initial_configure(&toplevel);
                }
            }
        });
    }

    /// Should be called on `WlSurface::commit`
    pub fn popups_handle_commit(&mut self, surface: &WlSurface) {
        self.niri.popups.commit(surface);

        if let Some(popup) = self.niri.popups.find_popup(surface) {
            match popup {
                PopupKind::Xdg(ref popup) => {
                    if !popup.is_initial_configure_sent() {
                        if let Some(output) = self.output_for_popup(&PopupKind::Xdg(popup.clone()))
                        {
                            let scale = output.current_scale();
                            let transform = output.current_transform();
                            with_states(surface, |data| {
                                send_scale_transform(surface, data, scale, transform);
                            });
                        }
                        popup.send_configure().expect("initial configure failed");
                    }
                }
                // Input method popup can arbitrary change its geometry, so we need to unconstrain
                // it on commit.
                PopupKind::InputMethod(_) => {
                    self.unconstrain_popup(&popup);
                }
            }
        }
    }

    pub fn output_for_popup(&self, popup: &PopupKind) -> Option<&Output> {
        let root = find_popup_root_surface(popup).ok()?;
        self.niri.output_for_root(&root)
    }

    pub fn unconstrain_popup(&self, popup: &PopupKind) {
        let _span = tracy_client::span!("Niri::unconstrain_popup");

        // Popups with a NULL parent will get repositioned in their respective protocol handlers
        // (i.e. layer-shell).
        let Ok(root) = find_popup_root_surface(popup) else {
            return;
        };

        // Figure out if the root is a window or a layer surface.
        if let Some((mapped, _)) = self.niri.layout.find_window_and_output(&root) {
            self.unconstrain_window_popup(popup, &mapped.window);
        } else if let Some((layer_surface, output)) = self.niri.layout.outputs().find_map(|o| {
            let map = layer_map_for_output(o);
            let layer_surface = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)?;
            Some((layer_surface.clone(), o))
        }) {
            self.unconstrain_layer_shell_popup(popup, &layer_surface, output);
        }
    }

    fn unconstrain_window_popup(&self, popup: &PopupKind, window: &Window) {
        // The target geometry for the positioner should be relative to its parent's geometry, so
        // we will compute that here.
        let mut target = self.niri.layout.popup_target_rect(window);
        target.loc -= get_popup_toplevel_coords(popup).to_f64();

        self.position_popup_within_rect(popup, target);
    }

    pub fn unconstrain_layer_shell_popup(
        &self,
        popup: &PopupKind,
        layer_surface: &LayerSurface,
        output: &Output,
    ) {
        let output_geo = self.niri.global_space.output_geometry(output).unwrap();
        let map = layer_map_for_output(output);
        let Some(layer_geo) = map.layer_geometry(layer_surface) else {
            return;
        };

        // The target geometry for the positioner should be relative to its parent's geometry, so
        // we will compute that here.
        let mut target = Rectangle::from_size(output_geo.size);

        // Background and bottom layer popups render below the top and the overlay layer, so let's
        // put them into the non-exclusive zone.
        //
        // FIXME: ideally this should use the "top and overlay layer" non-exclusive zone, but
        // Smithay only computes the "all layers" non-exclusive zone atm.
        //
        // FIXME: related to the above, top layer popups should use the "overlay layer"
        // non-exclusive zone.
        if matches!(layer_surface.layer(), Layer::Background | Layer::Bottom) {
            target = map.non_exclusive_zone();
        }

        target.loc -= layer_geo.loc;
        target.loc -= get_popup_toplevel_coords(popup);

        self.position_popup_within_rect(popup, target.to_f64());
    }

    fn position_popup_within_rect(&self, popup: &PopupKind, target: Rectangle<f64, Logical>) {
        match popup {
            PopupKind::Xdg(popup) => {
                popup.with_pending_state(|state| {
                    state.geometry = unconstrain_with_padding(state.positioner, target);
                });
            }
            PopupKind::InputMethod(popup) => {
                let text_input_rectangle = popup.text_input_rectangle();
                let mut bbox =
                    utils::bbox_from_surface_tree(popup.wl_surface(), text_input_rectangle.loc)
                        .to_f64();

                // Position bbox horizontally first.
                let overflow_x = (bbox.loc.x + bbox.size.w) - (target.loc.x + target.size.w);
                if overflow_x > 0. {
                    bbox.loc.x -= overflow_x;
                }

                // Ensure that the popup starts within the window.
                bbox.loc.x = f64::max(bbox.loc.x, target.loc.x);

                // Try to position IME popup below the text input rectangle.
                let mut below = bbox;
                below.loc.y += f64::from(text_input_rectangle.size.h);

                let mut above = bbox;
                above.loc.y -= bbox.size.h;

                if target.loc.y + target.size.h >= below.loc.y + below.size.h {
                    popup.set_location(below.loc.to_i32_round());
                } else {
                    popup.set_location(above.loc.to_i32_round());
                }
            }
        }
    }

    pub fn update_reactive_popups(&self, window: &Window) {
        let _span = tracy_client::span!("Niri::update_reactive_popups");

        for (popup, _) in PopupManager::popups_for_surface(
            window.toplevel().expect("no x11 support").wl_surface(),
        ) {
            match &popup {
                xdg_popup @ PopupKind::Xdg(popup) => {
                    if popup.with_pending_state(|state| state.positioner.reactive) {
                        self.unconstrain_window_popup(xdg_popup, window);
                        if let Err(err) = popup.send_pending_configure() {
                            warn!("error re-configuring reactive popup: {err:?}");
                        }
                    }
                }
                PopupKind::InputMethod(_) => (),
            }
        }
    }

    pub fn update_window_rules(&mut self, toplevel: &ToplevelSurface) {
        let config = self.niri.config.borrow();
        let window_rules = &config.window_rules;

        if let Some(unmapped) = self.niri.unmapped_windows.get_mut(toplevel.wl_surface()) {
            let new_rules = ResolvedWindowRules::compute(
                window_rules,
                WindowRef::Unmapped(unmapped),
                self.niri.is_at_startup,
            );
            if let InitialConfigureState::Configured { rules, .. } = &mut unmapped.state {
                *rules = new_rules;
            }
        } else if let Some((mapped, output)) = self
            .niri
            .layout
            .find_window_and_output_mut(toplevel.wl_surface())
        {
            if mapped.recompute_window_rules(window_rules, self.niri.is_at_startup) {
                drop(config);
                let output = output.cloned();
                let window = mapped.window.clone();
                self.niri.layout.update_window(&window, None);

                if let Some(output) = output {
                    self.niri.queue_redraw(&output);
                }
            }
        }
    }
}

fn unconstrain_with_padding(
    positioner: PositionerState,
    target: Rectangle<f64, Logical>,
) -> Rectangle<i32, Logical> {
    // Try unconstraining with a small padding first which looks nicer, then if it doesn't fit try
    // unconstraining without padding.
    const PADDING: f64 = 8.;

    let mut padded = target;
    if PADDING * 2. < padded.size.w {
        padded.loc.x += PADDING;
        padded.size.w -= PADDING * 2.;
    }
    if PADDING * 2. < padded.size.h {
        padded.loc.y += PADDING;
        padded.size.h -= PADDING * 2.;
    }

    // No padding, so just unconstrain with the original target.
    if padded == target {
        return positioner.get_unconstrained_geometry(target.to_i32_round());
    }

    // Do not try to resize to fit the padded target rectangle.
    let mut no_resize = positioner;
    no_resize
        .constraint_adjustment
        .remove(ConstraintAdjustment::ResizeX);
    no_resize
        .constraint_adjustment
        .remove(ConstraintAdjustment::ResizeY);

    let geo = no_resize.get_unconstrained_geometry(padded.to_i32_round());
    if padded.contains_rect(geo.to_f64()) {
        return geo;
    }

    // Could not unconstrain into the padded target, so resort to the regular one.
    positioner.get_unconstrained_geometry(target.to_i32_round())
}

pub fn add_mapped_toplevel_pre_commit_hook(toplevel: &ToplevelSurface) -> HookId {
    add_pre_commit_hook::<State, _>(toplevel.wl_surface(), move |state, _dh, surface| {
        let _span = tracy_client::span!("mapped toplevel pre-commit");
        let span =
            trace_span!("toplevel pre-commit", surface = %surface.id(), serial = Empty).entered();

        let Some((mapped, _)) = state.niri.layout.find_window_and_output_mut(surface) else {
            error!("pre-commit hook for mapped surfaces must be removed upon unmapping");
            return;
        };

        let (got_unmapped, dmabuf, commit_serial) = with_states(surface, |states| {
            let (got_unmapped, dmabuf) = {
                let mut guard = states.cached_state.get::<SurfaceAttributes>();
                match guard.pending().buffer.as_ref() {
                    Some(BufferAssignment::NewBuffer(buffer)) => {
                        let dmabuf = get_dmabuf(buffer).cloned().ok();
                        (false, dmabuf)
                    }
                    Some(BufferAssignment::Removed) => (true, None),
                    None => (false, None),
                }
            };

            let role = states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap();
            let serial = role.last_acked.as_ref().map(|c| c.serial);

            (got_unmapped, dmabuf, serial)
        });

        let mut transaction_for_dmabuf = None;
        let mut animate = false;
        if let Some(serial) = commit_serial {
            if !span.is_disabled() {
                span.record("serial", format!("{serial:?}"));
            }

            trace!("taking pending transaction");
            if let Some(transaction) = mapped.take_pending_transaction(serial) {
                // Transaction can be already completed if it ran past the deadline.
                let disable = state.niri.config.borrow().debug.disable_transactions;
                if !transaction.is_completed() && !disable {
                    // Register the deadline even if this is the last pending, since dmabuf
                    // rendering can still run over the deadline.
                    transaction.register_deadline_timer(&state.niri.event_loop);

                    let is_last = transaction.is_last();

                    // If this is the last transaction, we don't need to add a separate
                    // notification, because the transaction will complete in our dmabuf blocker
                    // callback, which already calls blocker_cleared(), or by the end of this
                    // function, in which case there would be no blocker in the first place.
                    if !is_last {
                        // Waiting for some other surface; register a notification and add a
                        // transaction blocker.
                        if let Some(client) = surface.client() {
                            transaction.add_notification(
                                state.niri.blocker_cleared_tx.clone(),
                                client.clone(),
                            );
                            add_blocker(surface, transaction.blocker());
                        }
                    }

                    // Delay dropping (and completing) the transaction until the dmabuf is ready.
                    // If there's no dmabuf, this will be dropped by the end of this pre-commit
                    // hook.
                    transaction_for_dmabuf = Some(transaction);
                }
            }

            animate = mapped.should_animate_commit(serial);
        } else if !got_unmapped {
            error!("commit on a mapped surface without a configured serial");
        };

        if let Some((blocker, source)) =
            dmabuf.and_then(|dmabuf| dmabuf.generate_blocker(Interest::READ).ok())
        {
            if let Some(client) = surface.client() {
                let res = state
                    .niri
                    .event_loop
                    .insert_source(source, move |_, _, state| {
                        // This surface is now ready for the transaction.
                        drop(transaction_for_dmabuf.take());

                        let display_handle = state.niri.display_handle.clone();
                        state
                            .client_compositor_state(&client)
                            .blocker_cleared(state, &display_handle);

                        Ok(())
                    });
                if res.is_ok() {
                    add_blocker(surface, blocker);
                    trace!("added dmabuf blocker");
                }
            }
        }

        let window = mapped.window.clone();
        if got_unmapped {
            state.backend.with_primary_renderer(|renderer| {
                state.niri.layout.store_unmap_snapshot(renderer, &window);
            });
        } else {
            if animate {
                state.backend.with_primary_renderer(|renderer| {
                    mapped.store_animation_snapshot(renderer);
                });
            }

            // The toplevel remains mapped; clear any stored unmap snapshot.
            state.niri.layout.clear_unmap_snapshot(&window);
        }
    })
}
