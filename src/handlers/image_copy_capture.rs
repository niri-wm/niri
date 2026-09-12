use std::time::Duration;

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer as _, Fourcc, Modifier};
use smithay::backend::drm::DrmNode;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::sync::SyncPoint;
use smithay::output::{Output, WeakOutput};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_pointer::WlPointer;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::utils::{
    Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform,
};
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::image_capture_source::{
    ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler,
    OutputCaptureSourceState,
};
use smithay::wayland::image_copy_capture::{
    BufferConstraints, CaptureFailureReason, CursorSession, CursorSessionRef, DmabufConstraints,
    Frame, FrameRef, ImageCopyCaptureHandler, ImageCopyCaptureState, Session, SessionRef,
};
use smithay::wayland::shm;

use crate::cursor::{RenderCursor, XCursor};
use crate::niri::{Niri, State};
use crate::utils::{CastSessionId, CastStreamId};

/// Output capture session.
pub struct ImageCopySession {
    pub session: Session,
    pub damage_tracker: OutputDamageTracker,
    /// Frame waiting for output damage.
    pub pending_frame: Option<Frame>,
    /// Cast session id, shared with the cursor session of the same source.
    pub session_id: CastSessionId,
    /// Cast stream id, unique to this session.
    pub stream_id: CastStreamId,
}

/// Cursor capture session of an output.
pub struct ImageCopyCursorSession {
    pub session: CursorSession,
    /// Damage to the cursor image (i.e., not movement).
    pub damage_tracker: OutputDamageTracker,
    /// Frame waiting for cursor image change.
    pub pending_frame: Option<Frame>,
    /// Cast session id, shared with the output session of the same source.
    pub session_id: CastSessionId,
    /// Cast stream id, unique to this session.
    pub stream_id: CastStreamId,
}

/// Cast session id of an image capture source.
///
/// Stored on the source so that the output session and the corresponding cursor
/// session are reported as two streams of a single cast session.
fn source_session_id(source: &ImageCaptureSource) -> CastSessionId {
    source.user_data().insert_if_missing(CastSessionId::next);
    *source.user_data().get::<CastSessionId>().unwrap()
}

/// Output captured by a session, or `None` if it's gone (sessions can outlive
/// their output).
pub fn source_output(source: &ImageCaptureSource) -> Option<Output> {
    source.user_data().get::<WeakOutput>()?.upgrade()
}

/// Buffer constraints for capturing an output.
///
/// `render_node` is the primary renderer's DRM render node (see
/// `Backend::primary_render_node()`), and only shm will be supported without
/// one. It is required since querying it from the EGL context fails for the TTY
/// backend: since Mesa 23.3 and until at least 26.1.8, `_eglGetGbmDisplay()`
/// clears the display's EGLDevice once a second EGLDisplay is created for the
/// same GBM device, which the TTY backend does during initialization. See
/// https://gitlab.freedesktop.org/mesa/mesa/-/merge_requests/44351.
pub fn output_capture_constraints(
    renderer: &GlesRenderer,
    render_node: Option<DrmNode>,
    output: &Output,
) -> Option<BufferConstraints> {
    let mode = output.current_mode()?;
    let size = Size::<i32, BufferCoords>::from((mode.size.w, mode.size.h));

    let dma = (|| {
        let node = render_node?;
        let egl = renderer.egl_context();

        // Offer all formats the renderer can draw into to avoid unnecessary
        // conversions, preserving the original order (many clients depend on
        // the order being stable to select the same format when renegotiating).
        let mut formats: Vec<(Fourcc, Vec<Modifier>)> = Vec::new();
        for format in egl.dmabuf_render_formats().iter() {
            match formats.iter_mut().find(|(code, _)| *code == format.code) {
                Some((_, modifiers)) => modifiers.push(format.modifier),
                None => formats.push((format.code, vec![format.modifier])),
            }
        }
        if formats.is_empty() {
            return None;
        }

        // Put Xrgb8888 and Argb8888 first since some clients always take the
        // first advertised format (e.g. wl-mirror, grim).
        formats.sort_by_key(|(code, _)| match code {
            Fourcc::Xrgb8888 => 0,
            Fourcc::Argb8888 => 1,
            _ => 2,
        });

        Some(DmabufConstraints { node, formats })
    })();

    Some(BufferConstraints {
        size,
        shm: vec![wl_shm::Format::Xrgb8888],
        dma,
    })
}

/// Buffer constraints for capturing the cursor of an output. Argb8888 since it has alpha.
pub fn cursor_capture_constraints(niri: &Niri, output: &Output) -> BufferConstraints {
    BufferConstraints {
        size: cursor_capture_size(niri, output),
        shm: vec![wl_shm::Format::Argb8888],
        dma: None,
    }
}

/// Size the cursor renders at on this output.
fn cursor_capture_size(niri: &Niri, output: &Output) -> Size<i32, BufferCoords> {
    let int_scale = output.current_scale().integer_scale();
    let scale = Scale::from(output.current_scale().fractional_scale());

    let size: Size<i32, Physical> = match niri.cursor_manager.get_render_cursor(int_scale) {
        RenderCursor::Hidden => Size::from((0, 0)),
        RenderCursor::Surface { surface, .. } => {
            let bbox = smithay::desktop::utils::bbox_from_surface_tree(&surface, (0, 0));
            bbox.to_f64().to_physical_precise_up(scale).size
        }
        RenderCursor::Named {
            scale: buffer_scale,
            cursor,
            ..
        } => {
            // All frames are the same size since CursorManager::load_xcursor()
            // picks one size and rejects frames which differ.
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
pub fn cursor_capture_hotspot(niri: &Niri, output: &Output) -> Point<i32, BufferCoords> {
    let int_scale = output.current_scale().integer_scale();
    let scale = Scale::from(output.current_scale().fractional_scale());

    let hotspot: Point<i32, Physical> = match niri.cursor_manager.get_render_cursor(int_scale) {
        RenderCursor::Hidden => Point::from((0, 0)),
        RenderCursor::Surface { surface, hotspot } => {
            // The tree is shifted to put its bounding box at the origin in
            // render_cursor_for_capture(), so shift the hotspot too.
            let bbox = smithay::desktop::utils::bbox_from_surface_tree(&surface, (0, 0));
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

pub enum CaptureBuffer {
    Dma(Dmabuf),
    Shm,
}

/// Checks a frame's buffer for compatibility with the render helpers.
///
/// Smithay only validates buffers against the protocol constraints, which allow
/// a client to attach a larger buffer than asked for, but the render helpers
/// need an exact match, and checking them here allows mismatches to be reported
/// to the client.
pub fn capture_buffer(
    buffer: &WlBuffer,
    size: Size<i32, BufferCoords>,
    format: wl_shm::Format,
) -> Option<CaptureBuffer> {
    if let Ok(dmabuf) = get_dmabuf(buffer) {
        let size_matches = dmabuf.width() == size.w as u32 && dmabuf.height() == size.h as u32;
        return size_matches.then(|| CaptureBuffer::Dma(dmabuf.clone()));
    }

    // Stride and pool placement are defined by the client, but the format and
    // dimensions need to match.
    let matches = shm::with_buffer_contents(buffer, |_ptr, _len, data| {
        data.format == format && data.width == size.w && data.height == size.h
    })
    .unwrap_or(false);
    matches.then_some(CaptureBuffer::Shm)
}

/// Finishes a frame, waiting for `sync_point` first if the render is still in flight.
///
/// Like `Screencopy::submit_after_sync()`.
pub fn frame_success_after_sync<T>(
    frame: Frame,
    transform: Transform,
    damage: Vec<Rectangle<i32, BufferCoords>>,
    presented: Duration,
    sync_point: Option<SyncPoint>,
    event_loop: &LoopHandle<'static, T>,
) {
    let Some(sync_fd) = sync_point.and_then(|sync| sync.export()) else {
        frame.success(transform, damage, presented);
        return;
    };

    let mut pending = Some((frame, damage));
    let source = Generic::new(sync_fd, Interest::READ, Mode::OneShot);
    let res = event_loop.insert_source(source, move |_, _, _| {
        let (frame, damage) = pending.take().unwrap();
        frame.success(transform, damage, presented);
        Ok(PostAction::Remove)
    });
    if let Err(err) = res {
        // The client will retry (smithay sends an Unknown error when Drop is
        // called on the Frame).
        warn!("error waiting for image copy capture sync point: {err}");
    }
}

impl ImageCaptureSourceHandler for State {}

impl OutputCaptureSourceHandler for State {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.niri.output_capture_source_state
    }

    fn output_source_created(&mut self, source: ImageCaptureSource, output: &Output) {
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ImageCopyCaptureHandler for State {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.niri.image_copy_capture_state
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        let output = source_output(source)?;
        if !self.niri.output_state.contains_key(&output) {
            return None;
        }

        let render_node = self.backend.primary_render_node();
        self.backend
            .with_primary_renderer(|renderer| {
                output_capture_constraints(renderer, render_node, &output)
            })
            .flatten()
    }

    fn cursor_capture_constraints(
        &mut self,
        source: &ImageCaptureSource,
        _pointer: &WlPointer,
    ) -> Option<BufferConstraints> {
        let output = source_output(source)?;
        if !self.niri.output_state.contains_key(&output) {
            return None;
        }

        Some(cursor_capture_constraints(&self.niri, &output))
    }

    fn new_session(&mut self, session: Session) {
        // Will be updated with the output properties before capture.
        let damage_tracker = OutputDamageTracker::new((0, 0), 1.0, Transform::Normal);
        let session_id = source_session_id(&session.source());
        self.niri.image_copy_sessions.push(ImageCopySession {
            session,
            damage_tracker,
            pending_frame: None,
            session_id,
            stream_id: CastStreamId::next(),
        });
    }

    fn new_cursor_session(&mut self, session: CursorSession) {
        // Will be updated with the output properties before capture.
        let damage_tracker = OutputDamageTracker::new((0, 0), 1.0, Transform::Normal);
        let session_id = source_session_id(&session.source());
        self.niri
            .image_copy_cursor_sessions
            .push(ImageCopyCursorSession {
                session,
                damage_tracker,
                pending_frame: None,
                session_id,
                stream_id: CastStreamId::next(),
            });
        // Send the initial cursor position and hotspot.
        self.niri.refresh_image_copy_cursor_sessions();
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        let Some(s) = self
            .niri
            .image_copy_sessions
            .iter_mut()
            .find(|s| s.session == *session)
        else {
            frame.fail(CaptureFailureReason::Unknown);
            return;
        };

        // A session may only have one frame object in flight at a time, a
        // second one is a duplicate_frame protocol error. Smithay doesn't check
        // itself it (create_frame pushes onto active_frames unconditionally)
        // and doesn't expose the session object for us to post_error() on, so
        // fail the frame rather than leak it.
        if s.pending_frame.is_some() {
            warn!("client created a second frame while one was still in flight"); // client bug
            frame.fail(CaptureFailureReason::Unknown);
            return;
        }
        s.pending_frame = Some(frame);

        // The frame is captured on the next redraw with damage.
        if let Some(output) = source_output(&session.source()) {
            // The output may be gone already (the global lingers).
            if self.niri.output_exists(&output) {
                self.niri.queue_redraw(&output);
            }
        }
    }

    fn cursor_frame(&mut self, session: &CursorSessionRef, frame: Frame) {
        let Some(s) = self
            .niri
            .image_copy_cursor_sessions
            .iter_mut()
            .find(|s| s.session == *session)
        else {
            frame.fail(CaptureFailureReason::Unknown);
            return;
        };

        // Same as above.
        if s.pending_frame.is_some() {
            warn!("client created a second cursor frame while one was still in flight");
            frame.fail(CaptureFailureReason::Unknown);
            return;
        }
        s.pending_frame = Some(frame);

        // The frame is captured on the next redraw when the cursor image changes.
        if let Some(output) = source_output(&session.source()) {
            // The output may be gone already (the global lingers).
            if self.niri.output_exists(&output) {
                self.niri.queue_redraw(&output);
            }
        }
    }

    fn frame_aborted(&mut self, frame: FrameRef) {
        for s in &mut self.niri.image_copy_sessions {
            if s.pending_frame.as_deref() == Some(&frame) {
                s.pending_frame = None;
            }
        }
        for s in &mut self.niri.image_copy_cursor_sessions {
            if s.pending_frame.as_deref() == Some(&frame) {
                s.pending_frame = None;
            }
        }
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        let sessions = &mut self.niri.image_copy_sessions;
        if let Some(idx) = sessions.iter().position(|s| s.session == session) {
            let s = sessions.remove(idx);
            if let Some(frame) = s.pending_frame {
                // Like wlroots.
                frame.fail(CaptureFailureReason::Stopped);
            }
        }
        self.niri.image_copy_capture_state.cleanup();
    }

    fn cursor_session_destroyed(&mut self, session: CursorSessionRef) {
        // Called for both the cursor session and its capture session, so this
        // may run twice (the session won't be found the second time).
        let sessions = &mut self.niri.image_copy_cursor_sessions;
        if let Some(idx) = sessions.iter().position(|s| s.session == session) {
            let s = sessions.remove(idx);
            if let Some(frame) = s.pending_frame {
                // Like wlroots.
                frame.fail(CaptureFailureReason::Stopped);
            }
        }
        self.niri.image_copy_capture_state.cleanup();
    }
}
