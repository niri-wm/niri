use std::time::Duration;

use smithay::reexports::calloop::RegistrationToken;

const BUDGET_MIN: Duration = Duration::from_millis(1);
const BUDGET_START: Duration = Duration::from_millis(2);
const STEP_UP: Duration = Duration::from_micros(250);
const STEP_DOWN: Duration = Duration::from_micros(5);
const MIN_DEFER: Duration = Duration::from_micros(250);

/// After a few idle refreshes, GPUs often put the display and themselves to sleep, resulting in the
/// next frame taking longer than usual. Skip deferral in this case, there's likely nothing to batch
/// anyway.
const MAX_IDLE_REFRESHES: u32 = 8;
const MAX_IDLE_TIME: Duration = Duration::from_millis(35);

/// After a miss, decay to just above the budget that missed, and wait (exponential backoff) to
/// decay further. Otherwise, we'd end up constantly decaying the budget below what the hardware
/// needs, leading to missed vblanks.
const COOLDOWN_MIN: Duration = Duration::from_secs(1);
const COOLDOWN_MAX: Duration = Duration::from_secs(600);
const MISS_HEADROOM: Duration = Duration::from_micros(50);

#[derive(Debug, Clone, Copy)]
enum Deadline {
    /// Defer redraw until this timer fires.
    Pending(RegistrationToken),
    /// Timer fired, redraw immediately for this vblank.
    Reached(Duration),
}

#[derive(Debug)]
pub struct RenderDeadline {
    deadline: Option<Deadline>,
    /// Amount of time we expect redraw + flip to take
    budget: Duration,
    last_presentation_time: Option<Duration>,
    next_presentation_time: Option<Duration>,
    /// (budget, presentation_time)
    last_presentation_miss: Option<(Duration, Duration)>,
    cooldown: Duration,
}

impl Default for RenderDeadline {
    fn default() -> Self {
        Self {
            deadline: None,
            budget: BUDGET_START,
            last_presentation_time: None,
            next_presentation_time: None,
            last_presentation_miss: None,
            cooldown: COOLDOWN_MIN,
        }
    }
}

impl RenderDeadline {
    /// When to start rendering a frame for `target`, or `None` to render right away.
    pub fn deadline(&self, now: Duration, target: Duration, refresh: Duration) -> Option<Duration> {
        // Already deferred, or the deadline was reached and it's time to render.
        if self.deadline.is_some() || budget_too_large(self.budget, refresh) {
            return None;
        }
        if now.saturating_sub(self.last_presentation_time?)
            > (refresh * MAX_IDLE_REFRESHES).min(MAX_IDLE_TIME)
        {
            return None;
        }

        let deadline = target.checked_sub(self.budget)?;
        (deadline > now + MIN_DEFER).then_some(deadline)
    }

    pub fn set_pending(&mut self, token: RegistrationToken) {
        self.deadline = Some(Deadline::Pending(token));
    }

    pub fn is_pending(&self) -> bool {
        matches!(self.deadline, Some(Deadline::Pending(_)))
    }

    /// The deadline timer fired. `target` is the vblank it was for, or `None` if the redraw is no
    /// longer queued.
    pub fn timer_fired(&mut self, presentation_time: Option<Duration>) {
        self.deadline = presentation_time.map(Deadline::Reached);
    }

    /// Cancels a deferred redraw: the pending timer to remove, if any.
    pub fn take_timer(&mut self) -> Option<RegistrationToken> {
        match self.deadline.take() {
            Some(Deadline::Pending(token)) => Some(token),
            _ => None,
        }
    }

    /// A redraw is starting. If it's the one deferred to its deadline, the frame it queues is
    /// checked against the vblank the deadline was for.
    pub fn redraw_started(&mut self) {
        self.next_presentation_time = match self.deadline {
            Some(Deadline::Reached(target)) => {
                self.deadline = None;
                Some(target)
            }
            _ => None,
        };
    }

    pub fn presented(&mut self, presentation_time: Duration, refresh: Duration) {
        if presentation_time.is_zero() {
            return;
        }
        self.last_presentation_time = Some(presentation_time);

        // If we've recently missed the vblank, don't go below the missed budget + some headroom.
        let floor = match self.last_presentation_miss {
            Some((budget, time)) if presentation_time.saturating_sub(time) < self.cooldown => {
                budget + MISS_HEADROOM
            }
            _ => BUDGET_MIN,
        };
        let budget_decayed = self
            .budget
            .saturating_sub(STEP_DOWN)
            .max(floor)
            .min(self.budget);

        self.budget = match self.next_presentation_time {
            // Rendered with deferral but missed the vblank, raise the budget.
            Some(target) if presentation_time > target + refresh / 2 => {
                // Exponential backoff when repeated tries to lower the budget miss vblank.
                let miss_repeated = self
                    .last_presentation_miss
                    .is_some_and(|(budget, _)| self.budget.abs_diff(budget) <= MISS_HEADROOM);
                self.cooldown = if miss_repeated {
                    (self.cooldown * 2).min(COOLDOWN_MAX)
                } else {
                    COOLDOWN_MIN
                };

                self.last_presentation_miss = Some((self.budget, presentation_time));

                let budget = (self.budget + STEP_UP).min(refresh);
                debug!(
                    "render deadline: frame missed its vblank by {:?} with a budget of {:?}, \
                     budget now {budget:?}, holding for {:?}",
                    presentation_time - target,
                    self.budget,
                    self.cooldown,
                );
                if !budget_too_large(self.budget, refresh) && budget_too_large(budget, refresh) {
                    debug!(
                        "render deadline: budget grew to {budget:?}, over 3/4 of the {refresh:?} \
                         refresh, rendering immediately until it decays"
                    );
                }
                budget
            }

            // Rendered with deferral before the vblank.
            Some(_) => budget_decayed,

            // Rendered immediately because budget grew too large, decay it.
            None if budget_too_large(self.budget, refresh) => {
                if !budget_too_large(budget_decayed, refresh) {
                    debug!(
                        "render deadline: budget decayed to {budget_decayed:?}, \
                         rendering at deadlines again (refresh {refresh:?})"
                    );
                }
                budget_decayed
            }

            // Rendered immediately for some other reason, leave budget unchanged.
            None => self.budget,
        };
    }
}

/// Rendering takes >3/4 of a frame, meaning the user's PC can't keep up.
fn budget_too_large(budget: Duration, refresh: Duration) -> bool {
    budget * 4 >= refresh * 3
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFRESH_240: Duration = Duration::from_nanos(4_166_667);
    const REFRESH_60: Duration = Duration::from_nanos(16_666_667);

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn with_budget(budget: Duration) -> RenderDeadline {
        RenderDeadline {
            budget,
            ..Default::default()
        }
    }

    fn immediately_presented_at(rd: &mut RenderDeadline, time: Duration, refresh: Duration) {
        rd.redraw_started();
        rd.presented(time, refresh);
    }

    fn deferred_presented_at(rd: &mut RenderDeadline, target: Duration, time: Duration) {
        rd.timer_fired(Some(target));
        rd.redraw_started();
        rd.presented(time, REFRESH_240);
    }

    fn simulate_renders(
        rd: &mut RenderDeadline,
        refresh: Duration,
        need: Duration,
        secs: u64,
    ) -> u32 {
        let mut now = ms(100);
        immediately_presented_at(rd, now, refresh);
        let end = now + Duration::from_secs(secs);
        let mut misses = 0;
        while now < end {
            let target = now + refresh;
            let deferred = rd.deadline(now, target, refresh).is_some();
            if deferred {
                rd.timer_fired(Some(target));
            }
            rd.redraw_started();
            now = if deferred && rd.budget < need {
                misses += 1;
                target + refresh
            } else {
                target
            };
            rd.presented(now, refresh);
        }
        misses
    }

    #[test]
    fn deadline() {
        let budget = ms(2);
        let mut rd = with_budget(budget);
        assert_eq!(rd.deadline(ms(100), ms(104), REFRESH_240), None);

        let last = ms(100);
        immediately_presented_at(&mut rd, last, REFRESH_240);
        assert_eq!(rd.deadline(ms(101), ms(104), REFRESH_240), Some(ms(102)));
        let now = Duration::from_micros(101_900);
        assert_eq!(rd.deadline(now, ms(104), REFRESH_240), None);

        // Idle for 7 refreshes, deferred
        let target = last + REFRESH_240 * 8;
        let deadline = Some(target - budget);
        assert_eq!(
            rd.deadline(last + REFRESH_240 * 7, target, REFRESH_240),
            deadline
        );
        // Idle for 9 refreshes, rendered immediately
        let target = last + REFRESH_240 * 10;
        assert_eq!(
            rd.deadline(last + REFRESH_240 * 9, target, REFRESH_240),
            None
        );

        // At 60hz, the idle limit is 35 ms (~2 frames)
        immediately_presented_at(&mut rd, last, REFRESH_60);
        let target = last + ms(50);
        assert_eq!(
            rd.deadline(last + ms(34), target, REFRESH_60),
            Some(target - budget)
        );
        assert_eq!(rd.deadline(last + ms(36), target, REFRESH_60), None);

        // Deadline reached, redraw hasn't started yet
        immediately_presented_at(&mut rd, last, REFRESH_240);
        rd.timer_fired(Some(ms(104)));
        assert_eq!(rd.deadline(ms(101), ms(104), REFRESH_240), None);

        // At 1000hz, minimum budget too high, always immediate
        let mut rd = with_budget(BUDGET_MIN);
        immediately_presented_at(&mut rd, ms(100), ms(1));
        assert_eq!(rd.deadline(ms(100), ms(102), ms(1)), None);
    }

    #[test]
    fn adaptive_budget() {
        let start = ms(2);
        let mut rd = with_budget(start);
        immediately_presented_at(&mut rd, ms(100), REFRESH_240);

        // Deferred frame but landed on next vblank, raises budget
        deferred_presented_at(&mut rd, ms(104), ms(108));
        assert_eq!(rd.budget, start + STEP_UP);

        // Deferred frame, decays budget
        deferred_presented_at(&mut rd, ms(112), ms(112));
        assert_eq!(rd.budget, start + STEP_UP - STEP_DOWN);

        // Immediate frame, keeps budget
        immediately_presented_at(&mut rd, ms(116), REFRESH_240);
        assert_eq!(rd.budget, start + STEP_UP - STEP_DOWN);

        // During cooldown, hits decay to previously missed budget
        for _ in 0..1000 {
            deferred_presented_at(&mut rd, ms(120), ms(120));
        }
        assert_eq!(rd.budget, start + MISS_HEADROOM);

        // After cooldown, down to minimum
        let after = ms(108) + COOLDOWN_MIN;
        for _ in 0..1000 {
            deferred_presented_at(&mut rd, after, after);
        }
        assert_eq!(rd.budget, BUDGET_MIN);
    }

    #[test]
    fn exponential_backoff() {
        let mut rd = RenderDeadline::default();
        assert_eq!(
            simulate_renders(&mut rd, REFRESH_240, Duration::from_micros(1500), 600),
            10
        );
    }

    #[test]
    fn slow_rendering_falls_back() {
        let mut rd = RenderDeadline::default();
        assert_eq!(simulate_renders(&mut rd, REFRESH_60, ms(14), 600), 52);
    }
}
