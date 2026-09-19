use std::time::Duration;

/// A bounded capture cadence, independent of rendering and buffer delivery latency.
#[derive(Default)]
pub(super) struct FramePacing {
    pub last: Duration,
    next: Option<(Duration, Duration)>,
}

impl FramePacing {
    pub fn deadline(&self, interval: Duration) -> Duration {
        self.next
            .filter(|&(_, previous_interval)| previous_interval == interval)
            .map_or_else(|| self.last.saturating_add(interval), |(next, _)| next)
    }

    pub fn record(&mut self, time: Duration, interval: Duration) {
        let next = self.deadline(interval).saturating_add(interval);
        // Keep the cadence through small scheduling delays, but never accumulate a burst
        // of overdue frames after an idle period, a clock reset, or a rate change.
        let keep_cadence =
            self.next.is_some_and(|(_, old)| old == interval) && time >= self.last && next > time;
        self.next = Some((
            if keep_cadence {
                next
            } else {
                time.saturating_add(interval)
            },
            interval,
        ));
        self.last = time;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduling_jitter_does_not_accumulate() {
        let interval = Duration::from_nanos(16_666_667);
        let start = Duration::from_secs(1);
        let mut pacing = FramePacing::default();
        pacing.record(start, interval);
        for frame in 1..600 {
            let due = start + interval * frame;
            assert_eq!(pacing.deadline(interval), due);
            pacing.record(
                due + Duration::from_micros(frame as u64 % 5 * 100),
                interval,
            );
        }
    }

    #[test]
    fn idle_and_clock_reset_do_not_build_up_credit() {
        let interval = Duration::from_millis(20);
        let mut pacing = FramePacing::default();
        for time in [1000, 1020, 10000, 500, 520] {
            let time = Duration::from_millis(time);
            pacing.record(time, interval);
            assert_eq!(pacing.deadline(interval), time + interval);
        }
    }

    #[test]
    fn rate_changes_start_a_new_cadence() {
        let mut pacing = FramePacing::default();
        let start = Duration::from_secs(1);
        pacing.record(start, Duration::from_millis(10));
        let interval = Duration::from_millis(40);
        assert_eq!(pacing.deadline(interval), start + interval);
        pacing.record(start + interval, interval);
        assert_eq!(pacing.deadline(interval), start + interval * 2);
        pacing.record(start + interval * 2, Duration::ZERO);
        assert_eq!(pacing.deadline(Duration::ZERO), start + interval * 2);
    }

    #[test]
    fn faster_input_obeys_the_negotiated_cadence() {
        let interval = Duration::from_millis(40);
        let start = Duration::from_secs(1);
        let mut pacing = FramePacing::default();
        pacing.record(start, interval);
        let mut accepted = 0;
        for tick in 1..=1000 {
            let time = start + Duration::from_millis(tick);
            if time >= pacing.deadline(interval) {
                pacing.record(time, interval);
                accepted += 1;
            }
        }
        assert_eq!(accepted, 25);
    }
}
