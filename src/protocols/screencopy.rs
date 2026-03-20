use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer, Fourcc};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::sync::SyncPoint;
use smithay::output::{Output, WeakOutput};
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Physical, Point, Rectangle, Size, Transform};
use smithay::wayland::{dmabuf, shm};
use wayland_backend::server::Credentials;
use zwlr_screencopy_frame_v1::{Flags, ZwlrScreencopyFrameV1};
use zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use crate::utils::{get_credentials_for_client, get_monotonic_time, CastSessionId, CastStreamId};

const VERSION: u32 = 3;

/// Inactivity timeout for considering a screencopy cast as stopped.
///
/// xdg-desktop-portal-wlr keeps the screencopy manager alive across casts, so there's no way to
/// tell that a screencast had stopped. So we use a timeout: if no new with_damage frames are
/// requested for this timeout, consider the screencast finished.
const CAST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ScreencopyQueue {
    /// Credentials of this wlr-screencopy client, if known.
    credentials: Option<Credentials>,
    damage_tracker: OutputDamageTracker,
    /// Frames waiting for the client to call copy or destroy.
    pending_frames: HashSet<ZwlrScreencopyFrameV1>,
    /// Queue of screencopies waiting for a corresponding output redraw with damage.
    screencopies: Vec<Screencopy>,
    /// Cast tracking, set when the first with_damage request arrives.
    cast: Option<ScreencopyCast>,
}

pub struct ScreencopyCast {
    pub session_id: CastSessionId,
    pub stream_id: CastStreamId,
    /// Output being captured.
    ///
    /// Generally equal to the front entry in the queue, and persisted here when the queue becomes
    /// empty.
    pub output: WeakOutput,
    /// Cached name of the output.
    pub output_name: String,
    /// Deadline after which this cast is considered stopped if no new frames arrive.
    pub deadline: Duration,
}

impl ScreencopyCast {
    fn new(output: &Output) -> Self {
        Self {
            session_id: CastSessionId::next(),
            stream_id: CastStreamId::next(),
            output: output.downgrade(),
            output_name: output.name(),
            deadline: get_monotonic_time() + CAST_TIMEOUT,
        }
    }

    fn update_deadline(&mut self) {
        self.deadline = get_monotonic_time() + CAST_TIMEOUT;
    }

    fn update_output(&mut self, output: &Output) {
        // Only allocate a new name when the output differs.
        let weak = output.downgrade();
        if self.output != weak {
            self.output = weak;
            self.output_name = output.name();
        }
    }
}

impl ScreencopyQueue {
    pub fn new(credentials: Option<Credentials>) -> Self {
        Self {
            damage_tracker: OutputDamageTracker::new((0, 0), 1.0, Transform::Normal),
            pending_frames: HashSet::new(),
            screencopies: Vec::new(),
            cast: None,
            credentials,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pending_frames.is_empty() && self.screencopies.is_empty()
    }

    /// Get the cast tracking info, if this queue is tracking a cast.
    pub fn cast(&self) -> Option<&ScreencopyCast> {
        self.cast.as_ref()
    }

    pub fn credentials(&self) -> Option<Credentials> {
        self.credentials
    }

    pub fn split(&mut self) -> (&mut OutputDamageTracker, Option<&Screencopy>) {
        let ScreencopyQueue {
            damage_tracker,
            screencopies,
            ..
        } = self;
        (damage_tracker, screencopies.first())
    }

    pub fn push(&mut self, screencopy: Screencopy) {
        // Screencopy without damage is rendered immediately without the queue.
        if !screencopy.with_damage() {
            error!("only screencopy with damage can be pushed in the queue");
        }

        if let Some(cast) = &mut self.cast {
            // Update cast output when pushing a new front screencopy.
            if self.screencopies.is_empty() {
                cast.update_output(screencopy.output());
            }
        } else {
            // First with_damage request, mark this as a screencast.
            let output = screencopy.output();
            self.cast = Some(ScreencopyCast::new(output));
        }

        self.screencopies.push(screencopy);
    }

    pub fn pop(&mut self) -> Screencopy {
        let rv = self.screencopies.remove(0);

        let cast = self.cast.as_mut().unwrap();
        if let Some(first) = self.screencopies.first() {
            // Update cast output (most of the time we expect this to be the same).
            cast.update_output(first.output());
        } else {
            // Queue became empty, update deadline for considering the cast stopped.
            cast.update_deadline();
        }

        rv
    }

    pub fn clear_expired_cast(&mut self) {
        if let Some(cast) = &self.cast {
            // Check deadline if there are no in-flight frames.
            if self.screencopies.is_empty() && cast.deadline <= get_monotonic_time() {
                self.cast = None;
            }
        }
    }

    fn remove_output(&mut self, output: &Output) {
        if self.screencopies.is_empty() {
            return;
        }

        self.screencopies
            .retain(|screencopy| screencopy.output() != output);

        if let Some(cast) = &mut self.cast {
            if self.screencopies.is_empty() {
                // Queue became empty, update deadline for considering the cast stopped.
                cast.update_deadline();
            }
        }
    }

    fn remove_frame(&mut self, frame: &ZwlrScreencopyFrameV1) {
        self.pending_frames.remove(frame);

        if self.screencopies.is_empty() {
            return;
        }

        self.screencopies
            .retain(|screencopy| screencopy.frame != *frame);

        if let Some(cast) = &mut self.cast {
            if self.screencopies.is_empty() {
                // Queue became empty, update deadline for considering the cast stopped.
                cast.update_deadline();
            }
        }
    }
}

#[derive(Default)]
pub struct ScreencopyManagerState {
    queues: HashMap<ZwlrScreencopyManagerV1, ScreencopyQueue>,
}

pub struct ScreencopyManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

impl ScreencopyManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrScreencopyManagerV1, ScreencopyManagerGlobalData>,
        D: Dispatch<ZwlrScreencopyManagerV1, ()>,
        D: Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameState>,
        D: ScreencopyHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = ScreencopyManagerGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, ZwlrScreencopyManagerV1, _>(VERSION, global_data);

        Self {
            queues: HashMap::new(),
        }
    }

    pub fn push(&mut self, manager: &ZwlrScreencopyManagerV1, screencopy: Screencopy) {
        let Some(queue) = self.queues.get_mut(manager) else {
            // Destroying the manager does not invalidate existing frames, so the queue should
            // keep existing.
            error!("screencopy queue must not be deleted as long as frames exist");
            return;
        };

        queue.push(screencopy);
    }

    pub fn damage_tracker(
        &mut self,
        manager: &ZwlrScreencopyManagerV1,
    ) -> Option<&mut OutputDamageTracker> {
        let queue = self.queues.get_mut(manager)?;
        Some(&mut queue.damage_tracker)
    }

    pub fn remove_output(&mut self, output: &Output) {
        for queue in self.queues.values_mut() {
            queue.remove_output(output);
        }

        self.cleanup_queues();
    }

    pub fn queues(&self) -> impl Iterator<Item = &ScreencopyQueue> {
        self.queues.values()
    }

    pub fn with_queues_mut(&mut self, mut f: impl FnMut(&mut ScreencopyQueue)) {
        for queue in self.queues.values_mut() {
            f(queue);
        }

        self.cleanup_queues();
    }

    fn cleanup_queues(&mut self) {
        self.queues
            .retain(|manager, queue| manager.is_alive() || !queue.is_empty());
    }

    pub fn clear_expired_casts(&mut self) {
        for queue in self.queues.values_mut() {
            queue.clear_expired_cast();
        }
    }
}

impl<D> GlobalDispatch<ZwlrScreencopyManagerV1, ScreencopyManagerGlobalData, D>
    for ScreencopyManagerState
where
    D: GlobalDispatch<ZwlrScreencopyManagerV1, ScreencopyManagerGlobalData>,
    D: Dispatch<ZwlrScreencopyManagerV1, ()>,
    D: Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameState>,
    D: ScreencopyHandler,
    D: 'static,
{
    fn bind(
        state: &mut D,
        dh: &DisplayHandle,
        client: &Client,
        manager: New<ZwlrScreencopyManagerV1>,
        _manager_state: &ScreencopyManagerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(manager, ());

        let state = state.screencopy_state();
        let credentials = get_credentials_for_client(dh, client);
        let queue = ScreencopyQueue::new(credentials);
        state.queues.insert(manager.clone(), queue);
    }

    fn can_view(client: Client, global_data: &ScreencopyManagerGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ZwlrScreencopyManagerV1, (), D> for ScreencopyManagerState
where
    D: GlobalDispatch<ZwlrScreencopyManagerV1, ScreencopyManagerGlobalData>,
    D: Dispatch<ZwlrScreencopyManagerV1, ()>,
    D: Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameState>,
    D: ScreencopyHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let (frame, overlay_cursor, buffer_size, region_loc, output) = match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => {
                let Some(output) = Output::from_resource(&output) else {
                    trace!("screencopy client requested non-existent output");
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };

                let buffer_size = output.current_mode().unwrap().size;
                let region_loc = Point::from((0, 0));

                (frame, overlay_cursor, buffer_size, region_loc, output)
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                x,
                y,
                width,
                height,
                output,
            } => {
                if width <= 0 || height <= 0 {
                    trace!("screencopy client requested invalid sized region");
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                }

                let Some(output) = Output::from_resource(&output) else {
                    trace!("screencopy client requested non-existent output");
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };

                let output_transform = output.current_transform();
                let output_physical_size =
                    output_transform.transform_size(output.current_mode().unwrap().size);
                let output_rect = Rectangle::from_size(output_physical_size);

                let rect = Rectangle::new(Point::from((x, y)), Size::from((width, height)));

                let output_scale = output.current_scale().fractional_scale();
                let physical_rect = rect.to_physical_precise_round(output_scale);

                // Clamp captured region to the output.
                let Some(clamped_rect) = physical_rect.intersection(output_rect) else {
                    trace!("screencopy client requested region outside of output");
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };

                let untransformed_rect = output_transform
                    .invert()
                    .transform_rect_in(clamped_rect, &output_physical_size);

                (
                    frame,
                    overlay_cursor,
                    untransformed_rect.size,
                    clamped_rect.loc,
                    output,
                )
            }
            zwlr_screencopy_manager_v1::Request::Destroy => return,
            _ => unreachable!(),
        };

        // Create the frame.
        let overlay_cursor = overlay_cursor != 0;
        let info = ScreencopyFrameInfo {
            output,
            overlay_cursor,
            buffer_size,
            region_loc,
        };
        let frame = data_init.init(
            frame,
            ScreencopyFrameState::Pending {
                manager: manager.clone(),
                info,
                copied: Arc::new(AtomicBool::new(false)),
            },
        );

        // Send desired SHM buffer parameters.
        frame.buffer(
            Format::Xrgb8888,
            buffer_size.w as u32,
            buffer_size.h as u32,
            buffer_size.w as u32 * 4,
        );

        if frame.version() >= 3 {
            // Send desired DMA buffer parameters.
            frame.linux_dmabuf(
                Fourcc::Xrgb8888 as u32,
                buffer_size.w as u32,
                buffer_size.h as u32,
            );

            // Notify client that all supported buffers were enumerated.
            frame.buffer_done();
        }

        let state = state.screencopy_state();
        let queue = state.queues.get_mut(manager).unwrap();
        queue.pending_frames.insert(frame);
    }

    fn destroyed(
        state: &mut D,
        _client: wayland_backend::server::ClientId,
        manager: &ZwlrScreencopyManagerV1,
        _data: &(),
    ) {
        let state = state.screencopy_state();

        let Some(queue) = state.queues.get_mut(manager) else {
            // This happened once. I'm really not sure how exactly though.
            //
            // I've dug into wayland-server and wayland-backend, and apparently there are a bunch
            // of places where calling destroyed() is delayed (even on a +1 ms timer). Then, it's
            // quite possible for some code to run cleanup_queues() *before* this destroyed()
            // handler, and delete the queue because the manager is no longer .is_alive() by then.
            // Then, queue will be None here.
            //
            // My attempts to reproduce this in a test have failed though. Perhaps it requires a
            // tricky timing condition where the client disconnects at some precise spot inside our
            // State::refresh_and_flush_clients() call.
            return;
        };

        // Clean up the queue if this was the last object.
        if queue.is_empty() {
            state.queues.remove(manager);
        }
    }
}

/// Handler trait for wlr-screencopy.
pub trait ScreencopyHandler {
    /// Handle new screencopy request.
    ///
    /// The handler must synchronously either ready/fail the screencopy, or submit it to the
    /// manager queue.
    fn frame(&mut self, manager: &ZwlrScreencopyManagerV1, screencopy: Screencopy);

    fn screencopy_state(&mut self) -> &mut ScreencopyManagerState;
}

#[allow(missing_docs)]
#[macro_export]
macro_rules! delegate_screencopy {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1: $crate::protocols::screencopy::ScreencopyManagerGlobalData
        ] => $crate::protocols::screencopy::ScreencopyManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1: ()
        ] => $crate::protocols::screencopy::ScreencopyManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1: $crate::protocols::screencopy::ScreencopyFrameState
        ] => $crate::protocols::screencopy::ScreencopyManagerState);
    };
}

#[derive(Clone)]
pub struct ScreencopyFrameInfo {
    output: Output,
    buffer_size: Size<i32, Physical>,
    region_loc: Point<i32, Physical>,
    overlay_cursor: bool,
}

pub enum ScreencopyFrameState {
    Failed,
    Pending {
        manager: ZwlrScreencopyManagerV1,
        info: ScreencopyFrameInfo,
        copied: Arc<AtomicBool>,
    },
}

impl<D> Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameState, D> for ScreencopyManagerState
where
    D: Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameState>,
    D: ScreencopyHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &ScreencopyFrameState,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if matches!(request, zwlr_screencopy_frame_v1::Request::Destroy) {
            return;
        }

        let ScreencopyFrameState::Pending {
            manager,
            info,
            copied,
        } = data
        else {
            return;
        };

        if copied.load(Ordering::SeqCst) {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "copy was already requested",
            );
            return;
        }

        let (buffer, with_damage) = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => (buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => (buffer, true),
            _ => unreachable!(),
        };

        let size = info.buffer_size;

        let buffer = if let Ok(dmabuf) = dmabuf::get_dmabuf(&buffer) {
            if dmabuf.format().code == Fourcc::Xrgb8888
                && dmabuf.width() == size.w as u32
                && dmabuf.height() == size.h as u32
            {
                ScreencopyBuffer::Dmabuf(dmabuf.clone())
            } else {
                frame.post_error(
                    zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                    "invalid dmabuf parameters",
                );
                return;
            }
        } else if shm::with_buffer_contents(&buffer, |_, shm_len, buffer_data| {
            buffer_data.format == Format::Xrgb8888
                && buffer_data.width == size.w
                && buffer_data.height == size.h
                && buffer_data.stride == size.w * 4
                && shm_len == buffer_data.stride as usize * buffer_data.height as usize
        })
        .unwrap_or(false)
        {
            ScreencopyBuffer::Shm(buffer)
        } else {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "invalid buffer",
            );
            return;
        };

        copied.store(true, Ordering::SeqCst);

        state.frame(
            manager,
            Screencopy {
                buffer,
                frame: frame.clone(),
                info: info.clone(),
                with_damage,
                submitted: false,
            },
        );

        // By this point the frame should've been either copied or failed or pushed to the queue,
        // so remove it from pending frames.
        let state = state.screencopy_state();
        let queue = state.queues.get_mut(manager).unwrap();
        queue.pending_frames.remove(frame);
        if queue.is_empty() && !manager.is_alive() {
            state.queues.remove(manager);
        }
    }

    fn destroyed(
        state: &mut D,
        _client: wayland_backend::server::ClientId,
        frame: &ZwlrScreencopyFrameV1,
        data: &ScreencopyFrameState,
    ) {
        let ScreencopyFrameState::Pending { manager, .. } = data else {
            return;
        };

        let state = state.screencopy_state();
        let Some(queue) = state.queues.get_mut(manager) else {
            // I think this can happen when we post_error() on a pending frame? Either way better
            // safe than sorry.
            return;
        };

        queue.remove_frame(frame);

        // Clean up the queue if this was the last object.
        if queue.is_empty() && !manager.is_alive() {
            state.queues.remove(manager);
        }
    }
}

/// Screencopy buffer.
#[derive(Clone)]
pub enum ScreencopyBuffer {
    Dmabuf(Dmabuf),
    Shm(WlBuffer),
}

/// Screencopy frame.
pub struct Screencopy {
    info: ScreencopyFrameInfo,
    frame: ZwlrScreencopyFrameV1,
    buffer: ScreencopyBuffer,
    with_damage: bool,
    submitted: bool,
}

impl Drop for Screencopy {
    fn drop(&mut self) {
        if !self.submitted {
            self.frame.failed();
        }
    }
}

impl Screencopy {
    /// Get the target buffer to copy to.
    pub fn buffer(&self) -> &ScreencopyBuffer {
        &self.buffer
    }

    pub fn region_loc(&self) -> Point<i32, Physical> {
        self.info.region_loc
    }

    pub fn buffer_size(&self) -> Size<i32, Physical> {
        self.info.buffer_size
    }

    pub fn output(&self) -> &Output {
        &self.info.output
    }

    pub fn overlay_cursor(&self) -> bool {
        self.info.overlay_cursor
    }

    pub fn with_damage(&self) -> bool {
        self.with_damage
    }

    pub fn damage(&self, damages: impl Iterator<Item = Rectangle<i32, smithay::utils::Buffer>>) {
        for Rectangle { loc, size } in damages {
            self.frame
                .damage(loc.x as u32, loc.y as u32, size.w as u32, size.h as u32);
        }
    }

    /// Submit the copied content.
    fn submit(mut self, y_invert: bool, timestamp: Duration) {
        // Notify client that buffer is ordinary.
        self.frame.flags(if y_invert {
            Flags::YInvert
        } else {
            Flags::empty()
        });

        // Notify client about successful copy.
        let tv_sec_hi = (timestamp.as_secs() >> 32) as u32;
        let tv_sec_lo = (timestamp.as_secs() & 0xFFFFFFFF) as u32;
        let tv_nsec = timestamp.subsec_nanos();
        self.frame.ready(tv_sec_hi, tv_sec_lo, tv_nsec);

        // Mark frame as submitted to ensure destructor isn't run.
        self.submitted = true;
    }

    pub fn submit_after_sync<T>(
        self,
        y_invert: bool,
        sync_point: Option<SyncPoint>,
        event_loop: &LoopHandle<'_, T>,
    ) {
        let timestamp = get_monotonic_time();
        match sync_point.and_then(|s| s.export()) {
            None => self.submit(y_invert, timestamp),
            Some(sync_fd) => {
                let source = Generic::new(sync_fd, Interest::READ, Mode::OneShot);
                let mut screencopy = Some(self);
                event_loop
                    .insert_source(source, move |_, _, _| {
                        screencopy.take().unwrap().submit(y_invert, timestamp);
                        Ok(PostAction::Remove)
                    })
                    .unwrap();
            }
        }
    }
}
