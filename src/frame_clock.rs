use std::num::NonZeroU64;
use std::time::Duration;

use crate::utils::get_monotonic_time;

#[derive(Debug)]
pub struct FrameClock {
    last_presentation_time: Option<Duration>,
    refresh_interval_ns: Option<NonZeroU64>,
    vrr: bool,
}

impl FrameClock {
    pub fn new(refresh_interval: Option<Duration>, vrr: bool) -> Self {
        let refresh_interval_ns = if let Some(interval) = &refresh_interval {
            assert_eq!(interval.as_secs(), 0);
            Some(NonZeroU64::new(interval.subsec_nanos().into()).unwrap())
        } else {
            None
        };

        Self {
            last_presentation_time: None,
            refresh_interval_ns,
            vrr,
        }
    }

    pub fn refresh_interval(&self) -> Option<Duration> {
        self.refresh_interval_ns
            .map(|r| Duration::from_nanos(r.get()))
    }

    pub fn set_vrr(&mut self, vrr: bool) {
        if self.vrr == vrr {
            return;
        }

        self.vrr = vrr;
        self.last_presentation_time = None;
    }

    pub fn vrr(&self) -> bool {
        self.vrr
    }

    pub fn presented(&mut self, presentation_time: Duration) {
        if presentation_time.is_zero() {
            // Not interested in these.
            return;
        }

        self.last_presentation_time = Some(presentation_time);
    }

    pub fn next_presentation_time(&self) -> Duration {
        let mut now = get_monotonic_time();

        let Some(refresh_interval_ns) = self.refresh_interval_ns else {
            return now;
        };
        let Some(last_presentation_time) = self.last_presentation_time else {
            return now;
        };

        let refresh_interval_ns = refresh_interval_ns.get();

        if now <= last_presentation_time {
            // Got an early VBlank.
            let orig_now = now;
            now += Duration::from_nanos(refresh_interval_ns);

            if now < last_presentation_time {
                // Not sure when this can happen.
                error!(
                    now = ?orig_now,
                    ?last_presentation_time,
                    "got a 2+ early VBlank, {:?} until presentation",
                    last_presentation_time - now,
                );
                now = last_presentation_time + Duration::from_nanos(refresh_interval_ns);
            }
        }

        let since_last = now - last_presentation_time;
        let since_last_ns =
            since_last.as_secs() * 1_000_000_000 + u64::from(since_last.subsec_nanos());
        let to_next_ns = (since_last_ns / refresh_interval_ns + 1) * refresh_interval_ns;

        // If VRR is enabled and more than one frame passed since last presentation, assume that we
        // can present immediately.
        if self.vrr && to_next_ns > refresh_interval_ns {
            now
        } else {
            last_presentation_time + Duration::from_nanos(to_next_ns)
        }
    }
}
