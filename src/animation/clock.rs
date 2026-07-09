use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crate::utils::get_monotonic_time;

/// Shareable lazy clock that can change rate.
///
/// The clock will fetch the time once and then retain it until explicitly cleared with
/// [`Clock::clear`].
#[derive(Debug, Default, Clone)]
pub struct Clock {
    inner: Rc<RefCell<AdjustableClock>>,
}

#[derive(Debug, Default)]
struct LazyClock {
    time: Option<Duration>,
}

/// Clock that can adjust its rate.
#[derive(Debug)]
struct AdjustableClock {
    inner: LazyClock,
    current_time: Duration,
    last_seen_time: Duration,
    rate: f64,
    complete_instantly: bool,
}

impl Clock {
    /// Creates a new clock with the given time.
    pub fn with_time(time: Duration) -> Self {
        let clock = AdjustableClock::new(LazyClock::with_time(time));
        Self {
            inner: Rc::new(RefCell::new(clock)),
        }
    }

    /// Returns the current time.
    pub fn now(&self) -> Duration {
        self.inner.borrow_mut().now()
    }

    /// Returns the underlying time not adjusted for rate change.
    pub fn now_unadjusted(&self) -> Duration {
        self.inner.borrow_mut().inner.now()
    }

    /// Sets the unadjusted clock time.
    pub fn set_unadjusted(&mut self, time: Duration) {
        self.inner.borrow_mut().inner.set(time);
    }

    /// Clears the stored time so it's re-fetched again next.
    pub fn clear(&mut self) {
        self.inner.borrow_mut().inner.clear();
    }

    /// Gets the clock rate.
    pub fn rate(&self) -> f64 {
        self.inner.borrow().rate()
    }

    /// Sets the clock rate.
    pub fn set_rate(&mut self, rate: f64) {
        self.inner.borrow_mut().set_rate(rate);
    }

    /// Returns whether animations should complete instantly.
    pub fn should_complete_instantly(&self) -> bool {
        self.inner.borrow().should_complete_instantly()
    }

    /// Sets whether animations should complete instantly.
    pub fn set_complete_instantly(&mut self, value: bool) {
        self.inner.borrow_mut().set_complete_instantly(value);
    }
}

impl PartialEq for Clock {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for Clock {}

impl LazyClock {
    pub fn with_time(time: Duration) -> Self {
        Self { time: Some(time) }
    }

    pub fn clear(&mut self) {
        self.time = None;
    }

    pub fn set(&mut self, time: Duration) {
        self.time = Some(time);
    }

    pub fn now(&mut self) -> Duration {
        *self.time.get_or_insert_with(get_monotonic_time)
    }
}

impl AdjustableClock {
    pub fn new(mut inner: LazyClock) -> Self {
        let time = inner.now();
        Self {
            inner,
            current_time: time,
            last_seen_time: time,
            rate: 1.,
            complete_instantly: false,
        }
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    pub fn set_rate(&mut self, rate: f64) {
        self.rate = rate.clamp(0., 1000.);
    }

    pub fn should_complete_instantly(&self) -> bool {
        self.complete_instantly
    }

    pub fn set_complete_instantly(&mut self, value: bool) {
        self.complete_instantly = value;
    }

    pub fn now(&mut self) -> Duration {
        let time = self.inner.now();

        if self.last_seen_time == time {
            return self.current_time;
        }

        if self.last_seen_time < time {
            let delta = time - self.last_seen_time;
            let delta = delta.mul_f64(self.rate);
            self.current_time = self.current_time.saturating_add(delta);
        } else {
            let delta = self.last_seen_time - time;
            let delta = delta.mul_f64(self.rate);
            self.current_time = self.current_time.saturating_sub(delta);
        }

        self.last_seen_time = time;
        self.current_time
    }
}

impl Default for AdjustableClock {
    fn default() -> Self {
        Self::new(LazyClock::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_clock() {
        let mut clock = Clock::with_time(Duration::ZERO);
        assert_eq!(clock.now(), Duration::ZERO);

        clock.set_unadjusted(Duration::from_millis(100));
        assert_eq!(clock.now(), Duration::from_millis(100));

        clock.set_unadjusted(Duration::from_millis(200));
        assert_eq!(clock.now(), Duration::from_millis(200));
    }

    #[test]
    fn rate_change() {
        let mut clock = Clock::with_time(Duration::ZERO);
        clock.set_rate(0.5);

        clock.set_unadjusted(Duration::from_millis(100));
        assert_eq!(clock.now_unadjusted(), Duration::from_millis(100));
        assert_eq!(clock.now(), Duration::from_millis(50));

        clock.set_unadjusted(Duration::from_millis(200));
        assert_eq!(clock.now_unadjusted(), Duration::from_millis(200));
        assert_eq!(clock.now(), Duration::from_millis(100));

        clock.set_unadjusted(Duration::from_millis(150));
        assert_eq!(clock.now_unadjusted(), Duration::from_millis(150));
        assert_eq!(clock.now(), Duration::from_millis(75));

        clock.set_rate(2.0);

        clock.set_unadjusted(Duration::from_millis(250));
        assert_eq!(clock.now_unadjusted(), Duration::from_millis(250));
        assert_eq!(clock.now(), Duration::from_millis(275));
    }
}
