use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::mem;
use std::time::Duration;

use anyhow::Context as _;
use calloop::LoopHandle;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::Window;
use smithay::output::Output;
use smithay::reexports::gbm::Modifier;
use smithay::utils::{Physical, Point, Scale, Size};
use zbus::object_server::SignalEmitter;

use crate::dbus::mutter_screen_cast::{self, CursorMode, ScreenCastToNiri, StreamTargetId};
use crate::niri::{CastTarget, Niri, OutputRenderElements, PointerRenderElements, State};
use crate::niri_render_elements;
use crate::render_helpers::RenderTarget;
use crate::utils::{get_monotonic_time, CastSessionId, CastStreamId};
use crate::window::mapped::{MappedId, WindowCastRenderElements};

mod pw_utils;
use pw_utils::{Cast, CastSizeChange, CursorData, PipeWire, PwToNiri};

pub struct Screencasting {
    pub casts: Vec<Cast>,

    /// Dynamic-target casts waiting for their first target to start.
    pub pending_dynamic_casts: Vec<PendingCast>,

    pub pw_to_niri: calloop::channel::Sender<PwToNiri>,

    /// Screencast output for each mapped window.
    pub mapped_cast_output: HashMap<Window, Output>,

    /// Window ID for the "dynamic cast" special window for the xdp-gnome picker.
    pub dynamic_cast_id_for_portal: MappedId,

    // Drop PipeWire last, and specifically after casts, to prevent a double-free (yay).
    pub pipewire: Option<PipeWire>,
}

/// A screencast request that hasn't been started yet.
pub struct PendingCast {
    pub session_id: CastSessionId,
    pub stream_id: CastStreamId,
    pub cursor_mode: CursorMode,
    pub signal_ctx: SignalEmitter<'static>,
}

impl Screencasting {
    pub fn new(event_loop: &LoopHandle<'static, State>) -> Self {
        let pw_to_niri = {
            let (pw_to_niri, from_pipewire) = calloop::channel::channel();
            event_loop
                .insert_source(from_pipewire, move |event, _, state| match event {
                    calloop::channel::Event::Msg(msg) => state.on_pw_msg(msg),
                    calloop::channel::Event::Closed => (),
                })
                .unwrap();
            pw_to_niri
        };

        Self {
            casts: vec![],
            pending_dynamic_casts: vec![],
            pw_to_niri,
            mapped_cast_output: HashMap::new(),
            dynamic_cast_id_for_portal: MappedId::next(),
            pipewire: None,
        }
    }
}

impl State {
    fn prepare_pw_cast(&mut self) -> anyhow::Result<(GbmDevice<DrmDeviceFd>, FormatSet)> {
        let gbm = self
            .backend
            .gbm_device()
            .context("no GBM device available")?;

        // Ensure PipeWire is initialized.
        if self.niri.casting.pipewire.is_none() {
            let pw = PipeWire::new(
                self.niri.event_loop.clone(),
                self.niri.casting.pw_to_niri.clone(),
            )
            .context("error initializing PipeWire")?;
            self.niri.casting.pipewire = Some(pw);
        }

        let mut render_formats = self
            .backend
            .with_primary_renderer(|renderer| {
                renderer.egl_context().dmabuf_render_formats().clone()
            })
            .unwrap_or_default();

        {
            let config = self.niri.config.borrow();
            if config.debug.force_pipewire_invalid_modifier {
                render_formats = render_formats
                    .into_iter()
                    .filter(|f| f.modifier == Modifier::Invalid)
                    .collect();
            }
        }

        Ok((gbm, render_formats))
    }

    pub fn on_pw_msg(&mut self, msg: PwToNiri) {
        match msg {
            PwToNiri::StopCast { session_id } => self.niri.stop_cast(session_id),
            PwToNiri::Redraw { stream_id } => self.redraw_cast(stream_id),
            PwToNiri::FatalError => {
                warn!("stopping PipeWire due to fatal error");
                let casting = &mut self.niri.casting;
                if let Some(pw) = casting.pipewire.take() {
                    let mut ids = HashSet::new();
                    for cast in &casting.pending_dynamic_casts {
                        ids.insert(cast.session_id);
                    }
                    for cast in &casting.casts {
                        ids.insert(cast.session_id);
                    }
                    for id in ids {
                        self.niri.stop_cast(id);
                    }
                    self.niri.event_loop.remove(pw.token);
                }
            }
        }
    }

    fn redraw_cast(&mut self, stream_id: CastStreamId) {
        let _span = tracy_client::span!("State::redraw_cast");

        let casts = &mut self.niri.casting.casts;
        let Some(idx) = casts.iter().position(|cast| cast.stream_id == stream_id) else {
            warn!("cast to redraw is missing");
            return;
        };
        let cast = &mut casts[idx];

        let id = match &cast.target {
            CastTarget::Nothing => {
                self.backend.with_primary_renderer(|renderer| {
                    if cast.dequeue_buffer_and_clear(renderer) {
                        cast.last_frame_time = get_monotonic_time();
                    }
                });
                return;
            }
            CastTarget::Output { output, .. } => {
                if let Some(output) = output.upgrade() {
                    self.niri.queue_redraw(&output);
                }
                return;
            }
            CastTarget::Window { id } => *id,
        };

        // Lack of partial borrowing strikes again...
        let mut casts = mem::take(&mut self.niri.casting.casts);
        let cast = &mut casts[idx];
        let mut stop = false;
        // Use a loop {} so we can break instead of early-return.
        #[allow(clippy::never_loop)]
        loop {
            let mut windows = self.niri.layout.windows();
            let Some((_, mapped)) = windows.find(|(_, mapped)| mapped.id().get() == id) else {
                break;
            };

            // Use the cached output since it will be present even if the output was
            // currently disconnected.
            let Some(output) = self.niri.casting.mapped_cast_output.get(&mapped.window) else {
                break;
            };

            let scale = Scale::from(output.current_scale().fractional_scale());
            let bbox = mapped
                .window
                .bbox_with_popups()
                .to_physical_precise_up(scale);

            match cast.ensure_size(bbox.size) {
                Ok(CastSizeChange::Ready) => (),
                Ok(CastSizeChange::Pending) => break,
                Err(err) => {
                    warn!("error updating stream size, stopping screencast: {err:?}");
                    stop = true;
                    break;
                }
            }

            self.backend.with_primary_renderer(|renderer| {
                let mut elements = Vec::new();
                mapped.render_for_screen_cast(renderer, scale, &mut |elem| {
                    elements.push(CastRenderElement::from(elem))
                });

                let mut pointer_elements = Vec::new();
                let mut pointer_location = Point::default();

                if self.niri.pointer_visibility.is_visible() {
                    if let Some((pointer_pos, win_pos)) =
                        self.niri.pointer_pos_for_window_cast(mapped)
                    {
                        // Pointer location must be relative to the screencast buffer.
                        // - win_pos is the position of the main window surface in output-local
                        //   coordinates
                        // - bbox.loc moves us relative to the screencast buffer
                        let buf_pos = win_pos + bbox.loc.to_f64().to_logical(scale);
                        let output_pos =
                            self.niri.global_space.output_geometry(output).unwrap().loc;
                        pointer_location = pointer_pos - output_pos.to_f64() - buf_pos;

                        let pos = buf_pos.to_physical_precise_round(scale).upscale(-1);
                        self.niri.render_pointer(renderer, output, &mut |elem| {
                            let elem =
                                RelocateRenderElement::from_element(elem, pos, Relocate::Relative);
                            pointer_elements.push(CastRenderElement::from(elem));
                        });
                    }
                }
                let cursor_data = CursorData::compute(&pointer_elements, pointer_location, scale);

                if cast.dequeue_buffer_and_render(
                    renderer,
                    &elements,
                    &cursor_data,
                    bbox.size,
                    scale,
                ) {
                    cast.last_frame_time = get_monotonic_time();
                }
            });

            break;
        }
        let session_id = cast.session_id;
        self.niri.casting.casts = casts;

        if stop {
            self.niri.stop_cast(session_id);
        }
    }

    pub fn set_dynamic_cast_target(&mut self, target: CastTarget) {
        let _span = tracy_client::span!("State::set_dynamic_cast_target");

        let mut refresh = None;
        match &target {
            // Leave refresh as is when clearing. Chances are, the next refresh will match it,
            // then we'll avoid reconfiguring.
            CastTarget::Nothing => (),
            CastTarget::Output { output, .. } => {
                if let Some(output) = output.upgrade() {
                    refresh = Some(output.current_mode().unwrap().refresh as u32);
                }
            }
            CastTarget::Window { id } => {
                let mut windows = self.niri.layout.windows();
                if let Some((_, mapped)) = windows.find(|(_, mapped)| mapped.id().get() == *id) {
                    if let Some(output) = self.niri.casting.mapped_cast_output.get(&mapped.window) {
                        refresh = Some(output.current_mode().unwrap().refresh as u32);
                    }
                }
            }
        }

        let mut to_redraw = Vec::new();
        let mut to_stop = Vec::new();
        for cast in &mut self.niri.casting.casts {
            if !cast.dynamic_target {
                continue;
            }

            if let Some(refresh) = refresh {
                if let Err(err) = cast.set_refresh(refresh) {
                    warn!("error changing cast FPS: {err:?}");
                    to_stop.push(cast.session_id);
                    continue;
                }
            }

            cast.target = target.clone();
            to_redraw.push(cast.stream_id);
        }

        for id in to_redraw {
            self.redraw_cast(id);
        }

        // Start any pending dynamic casts if we have a real target.
        if !matches!(target, CastTarget::Nothing) {
            self.start_pending_dynamic_casts(&target);
        }
    }

    fn start_pending_dynamic_casts(&mut self, target: &CastTarget) {
        let pending = &self.niri.casting.pending_dynamic_casts;
        if pending.is_empty() {
            return;
        }
        debug!("starting {} pending dynamic cast(s)", pending.len());

        let _span = tracy_client::span!("State::start_pending_dynamic_casts");

        // We don't stop dynamic casts on missing output/window.
        let (size, refresh) = match target {
            CastTarget::Nothing => panic!("dynamic cast starting target must not be Nothing"),
            CastTarget::Output { output, .. } => {
                let Some(output) = output.upgrade() else {
                    return;
                };
                cast_params_for_output(&output)
            }
            CastTarget::Window { id } => {
                let Some((size, refresh)) = self.niri.cast_params_for_window(*id) else {
                    return;
                };
                (size, refresh)
            }
        };

        let (gbm, render_formats) = match self.prepare_pw_cast() {
            Ok(x) => x,
            Err(err) => {
                warn!("error starting pending screencasts: {err:?}");
                let mut ids = HashSet::new();
                for pending in self.niri.casting.pending_dynamic_casts.drain(..) {
                    ids.insert(pending.session_id);
                }
                for id in ids {
                    self.niri.stop_cast(id);
                }
                return;
            }
        };
        let pw = self.niri.casting.pipewire.as_ref().unwrap();

        // Alpha is always true since the dynamic target can change between window & output.
        let alpha = true;

        // Start each pending cast.
        let mut to_stop = HashSet::new();
        for pending in self.niri.casting.pending_dynamic_casts.drain(..) {
            let res = pw.start_cast(
                gbm.clone(),
                render_formats.clone(),
                pending.session_id,
                pending.stream_id,
                target.clone(),
                size,
                refresh,
                alpha,
                pending.cursor_mode,
                pending.signal_ctx,
            );
            match res {
                Ok(mut cast) => {
                    cast.dynamic_target = true;
                    self.niri.casting.casts.push(cast);
                }
                Err(err) => {
                    warn!("error starting pending screencast: {err:?}");
                    to_stop.insert(pending.session_id);
                }
            }
        }

        for session_id in to_stop {
            self.niri.stop_cast(session_id);
        }
    }

    pub fn on_screen_cast_msg(&mut self, msg: ScreenCastToNiri) {
        match msg {
            ScreenCastToNiri::StartCast {
                session_id,
                stream_id,
                target,
                cursor_mode,
                signal_ctx,
            } => {
                let _span = tracy_client::span!("StartCast");
                let _span = debug_span!("StartCast", %session_id, %stream_id).entered();

                let (target, size, refresh, alpha) = match target {
                    StreamTargetId::Output { name } => {
                        let global_space = &self.niri.global_space;
                        let output = global_space.outputs().find(|out| out.name() == name);
                        let Some(output) = output else {
                            warn!("error starting screencast: requested output is missing");
                            self.niri.stop_cast(session_id);
                            return;
                        };

                        let (size, refresh) = cast_params_for_output(output);
                        (CastTarget::output(output), size, refresh, false)
                    }
                    StreamTargetId::Window { id }
                        if id == self.niri.casting.dynamic_cast_id_for_portal.get() =>
                    {
                        debug!("delaying dynamic cast until target is set");
                        self.niri.casting.pending_dynamic_casts.push(PendingCast {
                            session_id,
                            stream_id,
                            cursor_mode,
                            signal_ctx,
                        });
                        return;
                    }
                    StreamTargetId::Window { id } => {
                        let Some((size, refresh)) = self.niri.cast_params_for_window(id) else {
                            warn!("error starting screencast: requested window is missing");
                            self.niri.stop_cast(session_id);
                            return;
                        };
                        (CastTarget::Window { id }, size, refresh, true)
                    }
                };

                let (gbm, render_formats) = match self.prepare_pw_cast() {
                    Ok(x) => x,
                    Err(err) => {
                        warn!("error starting screencast: {err:?}");
                        self.niri.stop_cast(session_id);
                        return;
                    }
                };
                let pw = self.niri.casting.pipewire.as_ref().unwrap();

                let res = pw.start_cast(
                    gbm,
                    render_formats,
                    session_id,
                    stream_id,
                    target,
                    size,
                    refresh,
                    alpha,
                    cursor_mode,
                    signal_ctx,
                );
                match res {
                    Ok(cast) => {
                        self.niri.casting.casts.push(cast);
                    }
                    Err(err) => {
                        warn!("error starting screencast: {err:?}");
                        self.niri.stop_cast(session_id);
                    }
                }
            }
            ScreenCastToNiri::StopCast { session_id } => self.niri.stop_cast(session_id),
        }
    }
}

impl Niri {
    pub fn refresh_mapped_cast_window_rules(&mut self) {
        // O(N^2) but should be fine since there aren't many casts usually.
        self.layout.with_windows_mut(|mapped, _| {
            let id = mapped.id().get();
            // Find regardless of cast.is_active.
            let value = self
                .casting
                .casts
                .iter()
                .any(|cast| cast.target == (CastTarget::Window { id }));
            mapped.set_is_window_cast_target(value);
        });
    }

    pub fn refresh_mapped_cast_outputs(&mut self) {
        let mut seen = HashSet::new();
        let mut output_changed = vec![];

        self.layout.with_windows(|mapped, output, _, _| {
            seen.insert(mapped.window.clone());

            let Some(output) = output else {
                return;
            };

            match self.casting.mapped_cast_output.entry(mapped.window.clone()) {
                Entry::Occupied(mut entry) => {
                    if entry.get() != output {
                        entry.insert(output.clone());
                        output_changed.push((mapped.id(), output.clone()));
                    }
                }
                Entry::Vacant(entry) => {
                    entry.insert(output.clone());
                }
            }
        });

        self.casting
            .mapped_cast_output
            .retain(|win, _| seen.contains(win));

        let mut to_stop = vec![];
        for (id, out) in output_changed {
            let refresh = out.current_mode().unwrap().refresh as u32;
            let target = CastTarget::Window { id: id.get() };
            for cast in self
                .casting
                .casts
                .iter_mut()
                .filter(|cast| cast.target == target)
            {
                if let Err(err) = cast.set_refresh(refresh) {
                    warn!("error changing cast FPS: {err:?}");
                    to_stop.push(cast.session_id);
                };
            }
        }

        for session_id in to_stop {
            self.stop_cast(session_id);
        }
    }

    pub fn render_for_screen_cast(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        target_presentation_time: Duration,
    ) {
        let _span = tracy_client::span!("Niri::render_for_screen_cast");

        let weak = output.downgrade();
        let size = output.current_mode().unwrap().size;
        let transform = output.current_transform();
        let size = transform.transform_size(size);

        let scale = Scale::from(output.current_scale().fractional_scale());

        let mut elements = Vec::new();
        let mut pointer = Vec::new();
        let mut cursor_data = None;

        let mut casts_to_stop = vec![];

        let mut casts = mem::take(&mut self.casting.casts);
        for cast in &mut casts {
            if !cast.is_active() {
                continue;
            }

            if !cast.target.matches_output(&weak) {
                continue;
            }

            match cast.ensure_size(size) {
                Ok(CastSizeChange::Ready) => (),
                Ok(CastSizeChange::Pending) => continue,
                Err(err) => {
                    warn!("error updating stream size, stopping screencast: {err:?}");
                    casts_to_stop.push(cast.session_id);
                }
            }

            if cast.check_time_and_schedule(output, target_presentation_time) {
                continue;
            }

            if cursor_data.is_none() {
                self.render_inner(
                    renderer,
                    output,
                    false,
                    RenderTarget::Screencast,
                    &mut |elem| elements.push(elem.into()),
                );

                let mut pointer_pos = Point::default();
                if self.pointer_visibility.is_visible() {
                    let output_geo = self.global_space.output_geometry(output).unwrap().to_f64();
                    let pointer_loc = self
                        .tablet_cursor_location
                        .unwrap_or_else(|| self.seat.get_pointer().unwrap().current_location());
                    // Only render when the pointer is within the output. Otherwise, it will
                    // happily appear anywhere outside the output video source in OBS.
                    if output_geo.contains(pointer_loc) {
                        pointer_pos = pointer_loc - output_geo.loc;
                        self.render_pointer(renderer, output, &mut |elem| {
                            pointer.push(elem.into())
                        });
                    }
                }

                cursor_data = Some(CursorData::compute(&pointer, pointer_pos, scale));
            }
            let cursor_data = cursor_data.as_ref().unwrap();

            if cast.dequeue_buffer_and_render(renderer, &elements, cursor_data, size, scale) {
                cast.last_frame_time = target_presentation_time;
            }
        }
        self.casting.casts = casts;

        for id in casts_to_stop {
            self.stop_cast(id);
        }
    }

    pub fn render_windows_for_screen_cast(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        target_presentation_time: Duration,
    ) {
        let _span = tracy_client::span!("Niri::render_windows_for_screen_cast");

        let scale = Scale::from(output.current_scale().fractional_scale());

        let mut casts_to_stop = vec![];

        let mut casts = mem::take(&mut self.casting.casts);
        for cast in &mut casts {
            if !cast.is_active() {
                continue;
            }

            let CastTarget::Window { id } = cast.target else {
                continue;
            };

            let mut windows = self.layout.windows_for_output(output);
            let Some(mapped) = windows.find(|win| win.id().get() == id) else {
                continue;
            };

            let bbox = mapped
                .window
                .bbox_with_popups()
                .to_physical_precise_up(scale);

            match cast.ensure_size(bbox.size) {
                Ok(CastSizeChange::Ready) => (),
                Ok(CastSizeChange::Pending) => continue,
                Err(err) => {
                    warn!("error updating stream size, stopping screencast: {err:?}");
                    casts_to_stop.push(cast.session_id);
                }
            }

            if cast.check_time_and_schedule(output, target_presentation_time) {
                continue;
            }

            let mut elements = Vec::new();
            mapped.render_for_screen_cast(renderer, scale, &mut |elem| {
                elements.push(CastRenderElement::from(elem))
            });

            let mut pointer_elements = Vec::new();
            let mut pointer_location = Point::default();

            if self.pointer_visibility.is_visible() {
                if let Some((pointer_pos, win_pos)) = self.pointer_pos_for_window_cast(mapped) {
                    // Pointer location must be relative to the screencast buffer.
                    // - win_pos is the position of the main window surface in output-local
                    //   coordinates
                    // - bbox.loc moves us relative to the screencast buffer
                    let buf_pos = win_pos + bbox.loc.to_f64().to_logical(scale);
                    let output_pos = self.global_space.output_geometry(output).unwrap().loc;
                    pointer_location = pointer_pos - output_pos.to_f64() - buf_pos;

                    let pos = buf_pos.to_physical_precise_round(scale).upscale(-1);
                    self.render_pointer(renderer, output, &mut |elem| {
                        let elem =
                            RelocateRenderElement::from_element(elem, pos, Relocate::Relative);
                        pointer_elements.push(CastRenderElement::from(elem));
                    });
                }
            }
            let cursor_data = CursorData::compute(&pointer_elements, pointer_location, scale);

            if cast.dequeue_buffer_and_render(renderer, &elements, &cursor_data, bbox.size, scale) {
                cast.last_frame_time = target_presentation_time;
            }
        }
        self.casting.casts = casts;

        for id in casts_to_stop {
            self.stop_cast(id);
        }
    }

    pub fn stop_cast(&mut self, session_id: CastSessionId) {
        let _span = tracy_client::span!("Niri::stop_cast");
        let _span = debug_span!("stop_cast", %session_id).entered();

        self.casting
            .pending_dynamic_casts
            .retain(|p| p.session_id != session_id);

        for i in (0..self.casting.casts.len()).rev() {
            let cast = &self.casting.casts[i];
            if cast.session_id != session_id {
                continue;
            }

            let cast = self.casting.casts.swap_remove(i);
            if let Err(err) = cast.stream.disconnect() {
                warn!("error disconnecting stream: {err:?}");
            }
        }

        let dbus = &self.dbus.as_ref().unwrap();
        let server = dbus.conn_screen_cast.as_ref().unwrap().object_server();
        let path = format!("/org/gnome/Mutter/ScreenCast/Session/u{}", session_id.get());
        if let Ok(iface) = server.interface::<_, mutter_screen_cast::Session>(path) {
            let _span = tracy_client::span!("invoking Session::stop");

            async_io::block_on(async move {
                iface
                    .get()
                    .stop(server.inner(), iface.signal_emitter().clone())
                    .await
            });
        }
    }

    pub fn stop_casts_for_target(&mut self, target: CastTarget) {
        let _span = tracy_client::span!("Niri::stop_casts_for_target");

        // This is O(N^2) but it shouldn't be a problem I think.
        let mut saw_dynamic = false;
        let mut ids = Vec::new();
        for cast in &self.casting.casts {
            if cast.target != target {
                continue;
            }

            if cast.dynamic_target {
                saw_dynamic = true;
                continue;
            }

            ids.push(cast.session_id);
        }

        for id in ids {
            self.stop_cast(id);
        }

        // We don't stop dynamic casts, instead we switch them to Nothing.
        if saw_dynamic {
            self.event_loop
                .insert_idle(|state| state.set_dynamic_cast_target(CastTarget::Nothing));
        }
    }

    fn cast_params_for_window(&self, window_id: u64) -> Option<(Size<i32, Physical>, u32)> {
        let (_, mapped) = self
            .layout
            .windows()
            .find(|(_, m)| m.id().get() == window_id)?;
        let output = self.casting.mapped_cast_output.get(&mapped.window)?;
        let scale = Scale::from(output.current_scale().fractional_scale());
        let bbox = mapped
            .window
            .bbox_with_popups()
            .to_physical_precise_up(scale);
        let refresh = output.current_mode().unwrap().refresh as u32;
        Some((bbox.size, refresh))
    }
}

fn cast_params_for_output(output: &Output) -> (Size<i32, Physical>, u32) {
    let mode = output.current_mode().unwrap();
    let transform = output.current_transform();
    let size = transform.transform_size(mode.size);
    let refresh = mode.refresh as u32;
    (size, refresh)
}

niri_render_elements! {
    CastRenderElement<R> => {
        Output = OutputRenderElements<R>,
        Window = WindowCastRenderElements<R>,
        Pointer = PointerRenderElements<R>,
        RelocatedPointer = RelocateRenderElement<PointerRenderElements<R>>,
    }
}
