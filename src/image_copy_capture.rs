//! Frame delivery for the ext-image-copy-capture-v1 protocol.
//!
//! The protocol objects themselves are handled by Smithay; this module keeps track of the
//! active capture sessions and renders the requested frames on output redraw, similarly to
//! wlr-screencopy and the PipeWire screencasts.

use std::mem;
use std::time::Duration;

use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::sync::SyncPoint;
use smithay::backend::renderer::{buffer_dimensions, buffer_type, BufferType};
use smithay::desktop::utils::bbox_from_surface_tree;
use smithay::output::{Output, OutputModeSource, WeakOutput};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{
    Buffer, IsAlive, Logical, Physical, Point, Rectangle, Scale, Size, Transform,
};
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::image_capture_source::ImageCaptureSource;
use smithay::wayland::image_copy_capture::{
    BufferConstraints, CaptureFailureReason, CursorSession, CursorSessionRef, DmabufConstraints,
    Frame, FrameRef, Session, SessionRef,
};

use crate::cursor::{RenderCursor, XCursor};
use crate::niri::{Niri, OutputRenderElements, PointerRenderElements, State};
use crate::niri_render_elements;
use crate::render_helpers::surface::push_elements_from_surface_tree;
use crate::render_helpers::{render_to_dmabuf, render_to_shm, RenderCtx, RenderTarget};
use crate::window::mapped::WindowCastRenderElements;

niri_render_elements! {
    CopyCaptureRenderElement<R> => {
        Output = OutputRenderElements<R>,
        Window = WindowCastRenderElements<R>,
        RelocatedPointer = RelocateRenderElement<PointerRenderElements<R>>,
    }
}

/// An active ext-image-copy-capture session together with niri-side state.
pub struct CopyCaptureSession {
    session: Session,
    damage_tracker: OutputDamageTracker,
    pending_frame: Option<Frame>,
}

impl CopyCaptureSession {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            damage_tracker: OutputDamageTracker::new((0, 0), 1., Transform::Normal),
            pending_frame: None,
        }
    }
}

/// An active ext-image-copy-capture cursor session together with niri-side state.
///
/// Only output sources support cursor capture; the frames hold the cursor image, while the
/// position and hotspot are delivered through session events.
pub struct CopyCaptureCursorSession {
    session: CursorSession,
    /// Tracks damage to the cursor image (not its movement).
    damage_tracker: OutputDamageTracker,
    pending_frame: Option<Frame>,
}

impl CopyCaptureCursorSession {
    pub fn new(session: CursorSession) -> Self {
        Self {
            session,
            damage_tracker: OutputDamageTracker::new((0, 0), 1., Transform::Normal),
            pending_frame: None,
        }
    }
}

/// What an image capture source points at.
pub enum CaptureSourceTarget {
    Output(Output),
    Toplevel(WlSurface),
}

/// Resolves a capture source into its target, if the target is still around.
pub fn source_target(niri: &Niri, source: &ImageCaptureSource) -> Option<CaptureSourceTarget> {
    if let Some(weak) = source.user_data().get::<WeakOutput>() {
        return weak
            .upgrade()
            .filter(|output| niri.output_state.contains_key(output))
            .map(CaptureSourceTarget::Output);
    }

    if let Some(surface) = source.user_data().get::<WlSurface>() {
        if surface.alive() {
            return Some(CaptureSourceTarget::Toplevel(surface.clone()));
        }
    }

    None
}

/// Buffer constraints for capturing the cursor of an output. Argb8888 since it has alpha.
fn cursor_capture_constraints(niri: &Niri, output: &Output) -> BufferConstraints {
    BufferConstraints {
        size: cursor_capture_size(niri, output),
        shm: vec![wl_shm::Format::Argb8888],
        dma: None,
    }
}

/// Size the cursor renders at on this output.
fn cursor_capture_size(niri: &Niri, output: &Output) -> Size<i32, Buffer> {
    let int_scale = output.current_scale().integer_scale();
    let scale = Scale::from(output.current_scale().fractional_scale());

    let size: Size<i32, Physical> = match niri.cursor_manager.get_render_cursor(int_scale) {
        RenderCursor::Hidden => Size::from((0, 0)),
        RenderCursor::Surface { surface, .. } => {
            let bbox = bbox_from_surface_tree(&surface, (0, 0));
            bbox.to_f64().to_physical_precise_up(scale).size
        }
        RenderCursor::Named {
            scale: buffer_scale,
            cursor,
            ..
        } => {
            // All frames are the same size since CursorManager::load_xcursor() picks one size
            // and rejects frames which differ.
            let (_idx, frame) = cursor.frame(niri.start_time.elapsed().as_millis() as u32);
            // The image is loaded at the integer scale but drawn at the fractional one, so it
            // ends up smaller than its own buffer whenever the two differ.
            let logical = Size::<f64, Logical>::from((
                f64::from(frame.width) / f64::from(buffer_scale),
                f64::from(frame.height) / f64::from(buffer_scale),
            ));
            logical.to_physical_precise_ceil(scale)
        }
    };

    // Fall back to the nominal cursor size when the cursor is currently hidden or has no
    // buffer, so that the session always has valid constraints.
    if size.is_empty() {
        let fallback = i32::from(niri.config.borrow().cursor.xcursor_size) * int_scale;
        return Size::from((fallback, fallback));
    }

    Size::from((size.w, size.h))
}

/// Cursor hotspot in capture buffer coordinates.
fn cursor_capture_hotspot(niri: &Niri, output: &Output) -> Point<i32, Buffer> {
    let int_scale = output.current_scale().integer_scale();
    let scale = Scale::from(output.current_scale().fractional_scale());

    let hotspot: Point<i32, Physical> = match niri.cursor_manager.get_render_cursor(int_scale) {
        RenderCursor::Hidden => Point::from((0, 0)),
        RenderCursor::Surface { surface, hotspot } => {
            // The tree is shifted to put its bounding box at the origin in
            // render_cursor_for_capture(), so shift the hotspot too.
            let bbox = bbox_from_surface_tree(&surface, (0, 0));
            (hotspot - bbox.loc)
                .to_f64()
                .to_physical_precise_round(scale)
        }
        RenderCursor::Named {
            scale: buffer_scale,
            cursor,
            ..
        } => {
            let (_idx, frame) = cursor.frame(niri.start_time.elapsed().as_millis() as u32);
            // Same rescaling as in cursor_capture_size().
            XCursor::hotspot(frame)
                .to_logical(buffer_scale)
                .to_f64()
                .to_physical_precise_round(scale)
        }
    };

    Point::from((hotspot.x, hotspot.y))
}

impl State {
    /// Computes the current buffer constraints for a capture source.
    ///
    /// Returns `None` if the source is gone, which rejects the capture.
    pub fn image_capture_constraints(
        &mut self,
        source: &ImageCaptureSource,
    ) -> Option<BufferConstraints> {
        let size = self.image_capture_source_size(source)?;
        Some(self.build_image_capture_constraints(size))
    }

    /// Computes the current buffer size for a capture source.
    fn image_capture_source_size(&self, source: &ImageCaptureSource) -> Option<Size<i32, Buffer>> {
        let size = match source_target(&self.niri, source)? {
            // The buffer keeps the output's native (untransformed) orientation; the output
            // transform is delivered with each frame instead.
            CaptureSourceTarget::Output(output) => output.current_mode()?.size,
            CaptureSourceTarget::Toplevel(surface) => {
                let (mapped, output) = self.niri.layout.find_window_and_output(&surface)?;
                let scale = output
                    .map(|output| Scale::from(output.current_scale().fractional_scale()))
                    .unwrap_or(Scale::from(1.));
                mapped
                    .window
                    .bbox_with_popups()
                    .to_physical_precise_up(scale)
                    .size
            }
        };

        if size.is_empty() {
            return None;
        }

        Some(size.to_logical(1).to_buffer(1, Transform::Normal))
    }

    fn build_image_capture_constraints(&mut self, size: Size<i32, Buffer>) -> BufferConstraints {
        let dma = self.backend.primary_render_node().and_then(|node| {
            self.backend
                .with_primary_renderer(|renderer| {
                    // Keep the renderer's order, which is stable: clients rely on the list not
                    // changing between constraint updates to re-select the same format.
                    let mut formats: Vec<(Fourcc, Vec<Modifier>)> = Vec::new();
                    for format in renderer.egl_context().dmabuf_render_formats().iter() {
                        match formats.iter_mut().find(|(code, _)| *code == format.code) {
                            Some((_, modifiers)) => modifiers.push(format.modifier),
                            None => formats.push((format.code, vec![format.modifier])),
                        }
                    }
                    if formats.is_empty() {
                        return None;
                    }

                    // Put Xrgb8888 and Argb8888 first since some clients always take the first
                    // advertised format (e.g. wl-mirror, grim).
                    formats.sort_by_key(|(code, _)| match code {
                        Fourcc::Xrgb8888 => 0,
                        Fourcc::Argb8888 => 1,
                        _ => 2,
                    });

                    Some(DmabufConstraints { node, formats })
                })
                .flatten()
        });

        BufferConstraints {
            size,
            // render_to_shm() only supports Xrgb8888.
            shm: vec![wl_shm::Format::Xrgb8888],
            dma,
        }
    }

    /// Computes the cursor buffer constraints for a capture source.
    ///
    /// Returns `None` for toplevel sources: cursor capture is only supported for outputs.
    pub fn image_capture_cursor_constraints(
        &mut self,
        source: &ImageCaptureSource,
    ) -> Option<BufferConstraints> {
        match source_target(&self.niri, source)? {
            CaptureSourceTarget::Output(output) => {
                Some(cursor_capture_constraints(&self.niri, &output))
            }
            CaptureSourceTarget::Toplevel(_) => None,
        }
    }

    /// Adds a new capture session.
    pub fn new_image_copy_capture_session(&mut self, session: Session) {
        self.niri
            .copy_capture_sessions
            .push(CopyCaptureSession::new(session));
    }

    /// Adds a new cursor capture session.
    pub fn new_image_copy_capture_cursor_session(&mut self, session: CursorSession) {
        self.niri
            .copy_capture_cursor_sessions
            .push(CopyCaptureCursorSession::new(session));

        // Send the initial cursor position and hotspot.
        self.niri.refresh_image_copy_cursor_sessions();
    }

    /// Queues a capture frame for delivery on the next redraw with damage.
    pub fn image_copy_capture_frame_requested(&mut self, session: &SessionRef, frame: Frame) {
        let Some(entry) = self
            .niri
            .copy_capture_sessions
            .iter_mut()
            .find(|entry| entry.session == *session)
        else {
            frame.fail(CaptureFailureReason::Unknown);
            return;
        };

        if entry.pending_frame.is_some() {
            // Only one frame can be captured at a time.
            frame.fail(CaptureFailureReason::Unknown);
            return;
        }

        entry.pending_frame = Some(frame);

        // Queue a redraw of the source's output so the frame gets delivered even when nothing
        // else causes a redraw. If the source didn't change since the last frame, delivery will
        // wait until it does.
        let output = match source_target(&self.niri, &session.source()) {
            Some(CaptureSourceTarget::Output(output)) => Some(output),
            Some(CaptureSourceTarget::Toplevel(surface)) => self
                .niri
                .layout
                .find_window_and_output(&surface)
                .and_then(|(_, output)| output.cloned()),
            // The source is gone; the session will be stopped in the next refresh, failing the
            // frame.
            None => None,
        };

        if let Some(output) = output {
            if self.niri.output_state.contains_key(&output) {
                self.niri.queue_redraw(&output);
            }
        }
    }

    /// Queues a cursor capture frame for delivery on the next redraw with a cursor image
    /// change.
    pub fn image_copy_capture_cursor_frame_requested(
        &mut self,
        session: &CursorSessionRef,
        frame: Frame,
    ) {
        let Some(entry) = self
            .niri
            .copy_capture_cursor_sessions
            .iter_mut()
            .find(|entry| entry.session == *session)
        else {
            frame.fail(CaptureFailureReason::Unknown);
            return;
        };

        if entry.pending_frame.is_some() {
            // Only one frame can be captured at a time.
            frame.fail(CaptureFailureReason::Unknown);
            return;
        }

        entry.pending_frame = Some(frame);

        if let Some(CaptureSourceTarget::Output(output)) =
            source_target(&self.niri, &session.source())
        {
            self.niri.queue_redraw(&output);
        }
    }

    /// Drops the queued frame that the client aborted.
    pub fn image_copy_capture_frame_aborted(&mut self, frame: &FrameRef) {
        for entry in &mut self.niri.copy_capture_sessions {
            if entry.pending_frame.as_deref() == Some(frame) {
                entry.pending_frame = None;
            }
        }
        for entry in &mut self.niri.copy_capture_cursor_sessions {
            if entry.pending_frame.as_deref() == Some(frame) {
                entry.pending_frame = None;
            }
        }
    }

    /// Removes the session that the client destroyed.
    pub fn image_copy_capture_session_destroyed(&mut self, session: &SessionRef) {
        self.niri
            .copy_capture_sessions
            .retain(|entry| entry.session != *session);
    }

    /// Removes the cursor session that the client destroyed.
    pub fn image_copy_capture_cursor_session_destroyed(&mut self, session: &CursorSessionRef) {
        self.niri
            .copy_capture_cursor_sessions
            .retain(|entry| entry.session != *session);
    }

    /// Updates capture session constraints and stops sessions whose source is gone.
    pub fn refresh_image_copy_capture(&mut self) {
        let _span = tracy_client::span!("State::refresh_image_copy_capture");

        let mut sessions = mem::take(&mut self.niri.copy_capture_sessions);
        sessions.retain_mut(|entry| {
            if !entry.session.alive() {
                return false;
            }

            let source = entry.session.source();
            let Some(size) = self.image_capture_source_size(&source) else {
                // The source is gone; dropping the session stops it and fails all pending
                // frames.
                return false;
            };

            if entry.session.current_constraints().map(|c| c.size) != Some(size) {
                let constraints = self.build_image_capture_constraints(size);
                entry.session.update_constraints(constraints);

                // The pending frame's buffer no longer matches; fail it so the client can
                // reallocate.
                if let Some(frame) = entry.pending_frame.take() {
                    frame.fail(CaptureFailureReason::BufferConstraints);
                }
            }

            true
        });
        self.niri.copy_capture_sessions = sessions;

        // Cursor session constraints and positions are refreshed in
        // refresh_image_copy_cursor_sessions(); here only sessions with a dead source are
        // stopped.
        let mut cursor_sessions = mem::take(&mut self.niri.copy_capture_cursor_sessions);
        cursor_sessions.retain(|entry| {
            entry.session.alive() && source_target(&self.niri, &entry.session.source()).is_some()
        });
        self.niri.copy_capture_cursor_sessions = cursor_sessions;

        // Drop Smithay's own references to dead sessions.
        self.niri.image_copy_capture_state.cleanup();
    }
}

impl Niri {
    /// Renders and delivers queued capture frames for sources on this output.
    ///
    /// Called after a redraw, so the render elements are up to date. Frames are only delivered
    /// when their source accumulated damage since the last delivered frame; a fresh session's
    /// damage tracker reports full damage, so the first frame is delivered right away.
    pub fn render_for_image_copy_capture(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        target_presentation_time: Duration,
    ) {
        let _span = tracy_client::span!("Niri::render_for_image_copy_capture");

        let mut sessions = mem::take(&mut self.copy_capture_sessions);
        for entry in &mut sessions {
            if entry.pending_frame.is_none() {
                continue;
            }

            match source_target(self, &entry.session.source()) {
                Some(CaptureSourceTarget::Output(o)) if o == *output => {
                    self.render_output_capture_session(
                        entry,
                        renderer,
                        output,
                        target_presentation_time,
                    );
                }
                Some(CaptureSourceTarget::Toplevel(surface)) => {
                    self.render_toplevel_capture_session(
                        entry,
                        renderer,
                        output,
                        &surface,
                        target_presentation_time,
                    );
                }
                _ => (),
            }
        }
        self.copy_capture_sessions = sessions;
    }

    fn render_output_capture_session(
        &self,
        entry: &mut CopyCaptureSession,
        renderer: &mut GlesRenderer,
        output: &Output,
        presentation_time: Duration,
    ) {
        let Some(mode) = output.current_mode() else {
            return;
        };
        let size = mode.size;
        let transform = output.current_transform();
        let scale = Scale::from(output.current_scale().fractional_scale());

        ensure_damage_tracker(&mut entry.damage_tracker, size, scale, transform);

        let mut elements: Vec<CopyCaptureRenderElement<GlesRenderer>> = Vec::new();
        let ctx = RenderCtx {
            renderer: &mut *renderer,
            target: RenderTarget::ScreenCapture,
            xray: None,
        };
        self.render(ctx, output, entry.session.draw_cursor(), &mut |elem| {
            elements.push(CopyCaptureRenderElement::from(elem));
        });

        deliver_frame(
            entry,
            renderer,
            &elements,
            size,
            transform,
            presentation_time,
            &self.event_loop,
        );
    }

    fn render_toplevel_capture_session(
        &self,
        entry: &mut CopyCaptureSession,
        renderer: &mut GlesRenderer,
        output: &Output,
        surface: &WlSurface,
        presentation_time: Duration,
    ) {
        let Some((mapped, mapped_output)) = self.layout.find_window_and_output(surface) else {
            return;
        };
        if mapped_output != Some(output) {
            return;
        }

        let scale = Scale::from(output.current_scale().fractional_scale());
        let bbox = mapped
            .window
            .bbox_with_popups()
            .to_physical_precise_up(scale);
        let size = bbox.size;

        ensure_damage_tracker(&mut entry.damage_tracker, size, scale, Transform::Normal);

        let mut elements: Vec<CopyCaptureRenderElement<GlesRenderer>> = Vec::new();

        if entry.session.draw_cursor() {
            if let Some((_, win_pos)) = self.pointer_pos_for_window_cast(mapped) {
                // See render_windows_for_screen_cast() for the coordinate space logic.
                let buf_pos = win_pos + bbox.loc.to_f64().to_logical(scale);
                let pos = buf_pos.to_physical_precise_round(scale).upscale(-1);
                self.render_pointer(renderer, output, &mut |elem| {
                    let elem = RelocateRenderElement::from_element(elem, pos, Relocate::Relative);
                    elements.push(CopyCaptureRenderElement::from(elem));
                });
            }
        }

        mapped.render_for_screen_cast(renderer, scale, &mut |elem| {
            elements.push(CopyCaptureRenderElement::from(elem));
        });

        deliver_frame(
            entry,
            renderer,
            &elements,
            size,
            Transform::Normal,
            presentation_time,
            &self.event_loop,
        );
    }

    /// Sends the cursor position, hotspot and size to cursor sessions.
    ///
    /// Runs on every refresh cycle, independently of redraws, since the position changes
    /// without causing cursor image damage.
    pub fn refresh_image_copy_cursor_sessions(&mut self) {
        if self.copy_capture_cursor_sessions.is_empty() {
            return;
        }

        let _span = tracy_client::span!("Niri::refresh_image_copy_cursor_sessions");

        let pointer_pos = self
            .tablet_cursor_location
            .unwrap_or_else(|| self.seat.get_pointer().unwrap().current_location());

        let mut sessions = mem::take(&mut self.copy_capture_cursor_sessions);
        for entry in &mut sessions {
            let target = source_target(self, &entry.session.source());
            let Some(CaptureSourceTarget::Output(output)) = target else {
                entry.session.set_cursor_pos(None);
                continue;
            };
            let Some(geo) = self.global_space.output_geometry(&output) else {
                entry.session.set_cursor_pos(None);
                continue;
            };
            let Some(mode) = output.current_mode() else {
                entry.session.set_cursor_pos(None);
                continue;
            };

            let scale = Scale::from(output.current_scale().fractional_scale());

            // Update the constraints if the cursor image size changed.
            let constraints = cursor_capture_constraints(self, &output);
            let cursor_size = constraints.size;
            let size_changed = entry
                .session
                .current_constraints()
                .is_none_or(|c| c.size != constraints.size);
            if size_changed {
                // The pending frame's buffer no longer matches; fail it before sending the new
                // constraints so the client doesn't miss the new `done`.
                if let Some(frame) = entry.pending_frame.take() {
                    frame.fail(CaptureFailureReason::BufferConstraints);
                }
                entry.session.update_constraints(constraints);
            }

            let hotspot = cursor_capture_hotspot(self, &output);
            entry.session.set_cursor_hotspot(hotspot);

            // Unlike frame damage, the position is in transformed buffer coordinates, i.e. the
            // displayed orientation, so the output transform is not undone here. This matches
            // wlroots.
            let pos: Point<i32, Physical> =
                (pointer_pos - geo.loc.to_f64()).to_physical_precise_round(scale);

            // The cursor counts as entered while any part of its image intersects the output,
            // not just the hotspot, so the position may be negative or past the edge. The
            // protocol specifies this interpretation even though it differs from
            // wl_pointer.enter; this is also how wlroots implements it.
            //
            // A hidden cursor (e.g. before the first pointer motion of a session) has no
            // image, so it counts as left.
            let int_scale = output.current_scale().integer_scale();
            let cursor_hidden = matches!(
                self.cursor_manager.get_render_cursor(int_scale),
                RenderCursor::Hidden
            );
            let hotspot = Point::<i32, Physical>::from((hotspot.x, hotspot.y));
            let image = Rectangle::new(pos - hotspot, Size::from((cursor_size.w, cursor_size.h)));
            let output_rect =
                Rectangle::from_size(output.current_transform().transform_size(mode.size));
            if self.pointer_visibility.is_visible() && !cursor_hidden && image.overlaps(output_rect)
            {
                entry
                    .session
                    .set_cursor_pos(Some(Point::from((pos.x, pos.y))));
            } else {
                entry.session.set_cursor_pos(None);
            }
        }
        self.copy_capture_cursor_sessions = sessions;
    }

    /// Renders and delivers queued cursor capture frames for sources on this output.
    ///
    /// A frame is only delivered when the cursor image itself changed; movement is delivered
    /// through session events in refresh_image_copy_cursor_sessions().
    pub fn render_for_image_copy_cursor_capture(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        target_presentation_time: Duration,
    ) {
        if self.copy_capture_cursor_sessions.is_empty() {
            return;
        }

        let _span = tracy_client::span!("Niri::render_for_image_copy_cursor_capture");

        let scale = Scale::from(output.current_scale().fractional_scale());

        // The cursor render is the same for all sessions on the output.
        let mut cached_elements = None;

        let mut sessions = mem::take(&mut self.copy_capture_cursor_sessions);
        for entry in &mut sessions {
            if entry.pending_frame.is_none() {
                continue;
            }

            match source_target(self, &entry.session.source()) {
                Some(CaptureSourceTarget::Output(o)) if o == *output => (),
                _ => continue,
            }

            // The constraints are kept up to date with the cursor image size in
            // refresh_image_copy_cursor_sessions(), which also fails pending frames on change.
            let Some(constraints) = entry.session.current_constraints() else {
                let frame = entry.pending_frame.take().unwrap();
                frame.fail(CaptureFailureReason::BufferConstraints);
                continue;
            };
            let size = Size::<i32, Physical>::from((constraints.size.w, constraints.size.h));

            ensure_damage_tracker(&mut entry.damage_tracker, size, scale, Transform::Normal);

            let elements = cached_elements
                .get_or_insert_with(|| self.render_cursor_for_capture(renderer, output));

            let (damage, states) = match entry.damage_tracker.damage_output(1, elements) {
                Ok(x) => x,
                Err(err) => {
                    warn!("error computing damage for cursor capture: {err:?}");
                    continue;
                }
            };
            if damage.is_none() {
                // The cursor image didn't change; wait.
                continue;
            }

            let frame = entry.pending_frame.take().unwrap();
            let buffer = frame.buffer();

            if buffer_dimensions(&buffer) != Some(constraints.size) {
                frame.fail(CaptureFailureReason::BufferConstraints);
                // Recreate the damage tracker to report full damage next time.
                entry.damage_tracker = OutputDamageTracker::new((0, 0), 1., Transform::Normal);
                continue;
            }

            let res = render_to_shm(
                renderer,
                &mut entry.damage_tracker,
                &buffer,
                wl_shm::Format::Argb8888,
                elements,
                states,
            );
            match res {
                Ok(()) => {
                    let full_damage = vec![Rectangle::from_size(constraints.size)];
                    frame.success(Transform::Normal, full_damage, target_presentation_time);
                }
                Err(err) => {
                    warn!("error rendering for cursor capture: {err:?}");
                    frame.fail(CaptureFailureReason::Unknown);
                    // Recreate the damage tracker to report full damage next time.
                    entry.damage_tracker = OutputDamageTracker::new((0, 0), 1., Transform::Normal);
                }
            }
        }
        self.copy_capture_cursor_sessions = sessions;
    }

    /// Renders the cursor image at the origin for cursor capture sessions.
    fn render_cursor_for_capture(
        &self,
        renderer: &mut GlesRenderer,
        output: &Output,
    ) -> Vec<PointerRenderElements<GlesRenderer>> {
        let int_scale = output.current_scale().integer_scale();
        let output_scale = Scale::from(output.current_scale().fractional_scale());

        let mut elements = Vec::new();
        match self.cursor_manager.get_render_cursor(int_scale) {
            RenderCursor::Hidden => (),
            RenderCursor::Surface { surface, .. } => {
                // Subsurfaces can extend above or to the left of the root surface, so shift the
                // tree to put its bounding box at the origin. The hotspot is shifted to match
                // in cursor_capture_hotspot().
                let bbox = bbox_from_surface_tree(&surface, (0, 0));
                let loc = Point::<i32, Logical>::from((-bbox.loc.x, -bbox.loc.y))
                    .to_f64()
                    .to_physical_precise_round(output_scale);
                push_elements_from_surface_tree(
                    renderer,
                    &surface,
                    loc,
                    output_scale,
                    1.,
                    Kind::Cursor,
                    &mut |elem| elements.push(elem.into()),
                );
            }
            RenderCursor::Named {
                icon,
                scale,
                cursor,
            } => {
                let (idx, _frame) = cursor.frame(self.start_time.elapsed().as_millis() as u32);
                let texture = self.cursor_texture_cache.get(icon, scale, &cursor, idx);
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    Point::<f64, _>::from((0., 0.)),
                    &texture,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                ) {
                    Ok(element) => elements.push(element.into()),
                    Err(err) => {
                        warn!("error importing a cursor texture: {err:?}");
                    }
                }
            }
        }

        elements
    }
}

fn ensure_damage_tracker(
    damage_tracker: &mut OutputDamageTracker,
    size: Size<i32, Physical>,
    scale: Scale<f64>,
    transform: Transform,
) {
    let OutputModeSource::Static {
        size: last_size,
        scale: last_scale,
        transform: last_transform,
    } = damage_tracker.mode().clone()
    else {
        unreachable!("damage tracker must have static mode");
    };

    if size != last_size || scale != last_scale || transform != last_transform {
        *damage_tracker = OutputDamageTracker::new(size, scale, transform);
    }
}

fn deliver_frame(
    entry: &mut CopyCaptureSession,
    renderer: &mut GlesRenderer,
    elements: &[CopyCaptureRenderElement<GlesRenderer>],
    size: Size<i32, Physical>,
    transform: Transform,
    presentation_time: Duration,
    event_loop: &LoopHandle<'static, State>,
) {
    let (damage, states) = match entry.damage_tracker.damage_output(1, elements) {
        Ok(x) => x,
        Err(err) => {
            warn!("error computing damage for image copy capture: {err:?}");
            return;
        }
    };

    // No damage since the last delivered frame; wait for the next redraw.
    let Some(damage) = damage else {
        return;
    };

    // Convert the damage from output physical coordinates back to buffer coordinates.
    let physical_size = transform.transform_size(size);
    let buffer_damage: Vec<Rectangle<i32, Buffer>> = damage
        .iter()
        .map(|dmg| {
            dmg.to_logical(1)
                .to_buffer(1, transform.invert(), &physical_size.to_logical(1))
        })
        .collect();

    let frame = entry.pending_frame.take().unwrap();
    let buffer = frame.buffer();

    // The size might have changed since the buffer passed Smithay's constraint check.
    if buffer_dimensions(&buffer) != Some(size.to_logical(1).to_buffer(1, Transform::Normal)) {
        frame.fail(CaptureFailureReason::BufferConstraints);
        // Recreate the damage tracker to report full damage next time.
        entry.damage_tracker = OutputDamageTracker::new((0, 0), 1., Transform::Normal);
        return;
    }

    let res = match buffer_type(&buffer) {
        Some(BufferType::Shm) => render_to_shm(
            renderer,
            &mut entry.damage_tracker,
            &buffer,
            wl_shm::Format::Xrgb8888,
            elements,
            states,
        )
        .map(|()| None),
        Some(BufferType::Dma) => match get_dmabuf(&buffer) {
            Ok(dmabuf) => render_to_dmabuf(
                renderer,
                &mut entry.damage_tracker,
                dmabuf.clone(),
                elements,
                states,
            )
            .map(Some),
            Err(err) => Err(anyhow::anyhow!("error getting dmabuf from buffer: {err:?}")),
        },
        _ => Err(anyhow::anyhow!("unsupported buffer type")),
    };

    match res {
        Ok(sync) => success_after_sync(
            frame,
            transform,
            buffer_damage,
            presentation_time,
            sync,
            event_loop,
        ),
        Err(err) => {
            warn!("error rendering for image copy capture: {err:?}");
            // Recreate the damage tracker to report full damage next time.
            entry.damage_tracker = OutputDamageTracker::new((0, 0), 1., Transform::Normal);
            frame.fail(CaptureFailureReason::Unknown);
        }
    }
}

/// Signals a successful capture, delaying it until the GPU is done rendering if necessary.
fn success_after_sync(
    frame: Frame,
    transform: Transform,
    damage: Vec<Rectangle<i32, Buffer>>,
    presentation_time: Duration,
    sync: Option<SyncPoint>,
    event_loop: &LoopHandle<'static, State>,
) {
    match sync.and_then(|sync| sync.export()) {
        None => frame.success(transform, damage, presentation_time),
        Some(sync_fd) => {
            let source = Generic::new(sync_fd, Interest::READ, Mode::OneShot);
            let mut frame = Some(frame);
            let mut damage = Some(damage);
            event_loop
                .insert_source(source, move |_, _, _| {
                    frame.take().unwrap().success(
                        transform,
                        damage.take().unwrap(),
                        presentation_time,
                    );
                    Ok(PostAction::Remove)
                })
                .unwrap();
        }
    }
}
