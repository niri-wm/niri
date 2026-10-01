use std::io;

use calloop::{EventSource, Interest, LoopHandle};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::wayland::compositor::{add_blocker, with_states, Blocker, CompositorHandler as _};
use smithay::wayland::drm_syncobj::DrmSyncobjCachedState;

use crate::niri::State;

pub(super) fn add_dmabuf_readiness_blocker(
    event_loop: &LoopHandle<'static, State>,
    surface: &WlSurface,
    dmabuf: Option<Dmabuf>,
) {
    add_dmabuf_readiness_blocker_with_ready_callback(event_loop, surface, dmabuf, |_| {});
}

pub(super) fn add_dmabuf_readiness_blocker_with_ready_callback<F>(
    event_loop: &LoopHandle<'static, State>,
    surface: &WlSurface,
    dmabuf: Option<Dmabuf>,
    on_blocker_ready: F,
) where
    F: FnMut(&mut State) + 'static,
{
    let Some(client) = surface.client() else {
        return;
    };
    let Some(dmabuf) = dmabuf else {
        return;
    };

    let acquire_point = with_states(surface, |states| {
        states
            .cached_state
            .get::<DrmSyncobjCachedState>()
            .pending()
            .acquire_point
            .clone()
    });

    if let Some(acquire_point) = acquire_point {
        if let Ok((blocker, source)) = acquire_point.generate_blocker() {
            if add_blocker_with_ready_callback(
                event_loop,
                surface,
                client,
                blocker,
                source,
                on_blocker_ready,
            ) {
                trace!("added DRM syncobj blocker");
            }
            return;
        }
    }

    let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) else {
        return;
    };

    if add_blocker_with_ready_callback(
        event_loop,
        surface,
        client,
        blocker,
        source,
        on_blocker_ready,
    ) {
        trace!("added dmabuf blocker");
    }
}

fn add_blocker_with_ready_callback<S, F>(
    event_loop: &LoopHandle<'static, State>,
    surface: &WlSurface,
    client: Client,
    blocker: impl Blocker + Send + 'static,
    source: S,
    mut on_blocker_ready: F,
) -> bool
where
    S: EventSource<Event = (), Ret = Result<(), io::Error>> + 'static,
    F: FnMut(&mut State) + 'static,
{
    let Ok(_) = event_loop.insert_source(source, move |_, _, state| {
        on_blocker_ready(state);

        let display_handle = state.niri.display_handle.clone();
        state
            .client_compositor_state(&client)
            .blocker_cleared(state, &display_handle);

        Ok(())
    }) else {
        return false;
    };

    add_blocker(surface, blocker);
    true
}
