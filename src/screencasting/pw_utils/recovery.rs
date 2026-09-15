use super::*;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) enum DmaRecovery {
    #[default]
    Available,
    Pending,
    ShmOnly,
    Failed,
}

pub(super) fn schedule_shm_fallback(
    event_loop: &LoopHandle<'static, State>,
    inner: &mut CastInner,
    stream_id: CastStreamId,
) {
    if inner.dma_recovery != DmaRecovery::Available {
        return;
    }
    inner.dma_recovery = DmaRecovery::Pending;
    inner.is_active = false;
    // Rendering temporarily moves casts out of State. Renegotiate on the next dispatch.
    event_loop.insert_idle(move |state| {
        let Some(cast) = state
            .niri
            .casting
            .casts
            .iter_mut()
            .find(|c| c.stream_id == stream_id)
        else {
            return;
        };
        let mut inner = cast.inner.borrow_mut();
        if inner.dma_recovery != DmaRecovery::Pending {
            return;
        }
        let size = inner.state.expected_format_size();
        let refresh = inner.refresh;
        inner.state = CastState::ConfirmationPending {
            size,
            alpha: cast.offer_alpha,
            dma_negotiation: None,
        };
        cast.formats = FormatSet::default();
        drop(inner);
        make_params!(params, &cast.formats, size, refresh, cast.offer_alpha);
        if let Err(err) = cast.stream.update_params(&mut params) {
            warn!("error publishing SHM fallback: {err:?}");
            let session_id = cast.session_id;
            state.niri.stop_cast(session_id);
            return;
        }
        // A consumer that rejects SHM must not leave capture suspended indefinitely.
        if let Err(err) = state.niri.event_loop.insert_source(
            Timer::from_duration(Duration::from_secs(10)),
            move |_, _, state| {
                let session = state
                    .niri
                    .casting
                    .casts
                    .iter()
                    .find(|cast| {
                        cast.stream_id == stream_id
                            && cast.inner.borrow().dma_recovery == DmaRecovery::Pending
                    })
                    .map(|cast| cast.session_id);
                if let Some(session) = session {
                    warn!("timed out waiting for SHM fallback");
                    state.niri.stop_cast(session);
                }
                TimeoutAction::Drop
            },
        ) {
            warn!("error scheduling SHM fallback timeout: {err}");
            if let Some(session) = state
                .niri
                .casting
                .casts
                .iter()
                .find(|cast| cast.stream_id == stream_id)
                .map(|cast| cast.session_id)
            {
                state.niri.stop_cast(session);
            }
        }
    });
}
