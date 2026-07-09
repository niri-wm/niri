use std::time::Duration;

use keyframe::functions::{EaseOutCubic, EaseOutQuad};
use keyframe::EasingFunction;

mod bezier;
use bezier::CubicBezier;

mod spring;
pub use spring::{Spring, SpringParams};

mod clock;
pub use clock::Clock;

#[derive(Debug, Clone)]
pub struct Animation {
    from: f64,
    to: f64,
    initial_velocity: f64,
    is_off: bool,
    duration: Duration,
    /// Time until the animation first reaches `to`.
    ///
    /// Best effort; not always exactly precise.
    clamped_duration: Duration,
    start_time: Duration,
    clock: Clock,
    kind: Kind,
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Easing {
        curve: Curve,
    },
    Spring(Spring),
    Deceleration {
        initial_velocity: f64,
        deceleration_rate: f64,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum Curve {
    Linear,
    EaseOutQuad,
    EaseOutCubic,
    EaseOutExpo,
    CubicBezier(CubicBezier),
}

impl Animation {
    pub fn new(
        clock: Clock,
        from: f64,
        to: f64,
        initial_velocity: f64,
        config: niri_config::Animation,
    ) -> Self {
        // Scale the velocity by rate to keep the touchpad gestures feeling right.
        let initial_velocity = initial_velocity / clock.rate().max(0.001);

        let mut rv = Self::ease(clock, from, to, initial_velocity, 0, Curve::EaseOutCubic);
        if config.off {
            rv.is_off = true;
            return rv;
        }

        rv.replace_config(config);
        rv
    }

    pub fn replace_config(&mut self, config: niri_config::Animation) {
        self.is_off = config.off;
        if config.off {
            self.duration = Duration::ZERO;
            self.clamped_duration = Duration::ZERO;
            return;
        }

        let start_time = self.start_time;

        match config.kind {
            niri_config::animations::Kind::Spring(p) => {
                let params = SpringParams::new(p.damping_ratio, f64::from(p.stiffness), p.epsilon);

                let spring = Spring {
                    from: self.from,
                    to: self.to,
                    initial_velocity: self.initial_velocity,
                    params,
                };
                *self = Self::spring(self.clock.clone(), spring);
            }
            niri_config::animations::Kind::Easing(p) => {
                *self = Self::ease(
                    self.clock.clone(),
                    self.from,
                    self.to,
                    self.initial_velocity,
                    u64::from(p.duration_ms),
                    Curve::from(p.curve),
                );
            }
        }

        self.start_time = start_time;
    }

    /// Restarts the animation using the previous config.
    pub fn restarted(&self, from: f64, to: f64, initial_velocity: f64) -> Self {
        if self.is_off {
            return self.clone();
        }

        // Scale the velocity by rate to keep the touchpad gestures feeling right.
        let initial_velocity = initial_velocity / self.clock.rate().max(0.001);

        match self.kind {
            Kind::Easing { curve } => Self::ease(
                self.clock.clone(),
                from,
                to,
                initial_velocity,
                self.duration.as_millis() as u64,
                curve,
            ),
            Kind::Spring(spring) => {
                let spring = Spring {
                    from,
                    to,
                    initial_velocity: self.initial_velocity,
                    params: spring.params,
                };
                Self::spring(self.clock.clone(), spring)
            }
            Kind::Deceleration {
                initial_velocity,
                deceleration_rate,
            } => {
                let threshold = 0.001; // FIXME
                Self::decelerate(
                    self.clock.clone(),
                    from,
                    initial_velocity,
                    deceleration_rate,
                    threshold,
                )
            }
        }
    }

    pub fn ease(
        clock: Clock,
        from: f64,
        to: f64,
        initial_velocity: f64,
        duration_ms: u64,
        curve: Curve,
    ) -> Self {
        let duration = Duration::from_millis(duration_ms);
        let kind = Kind::Easing { curve };

        Self {
            from,
            to,
            initial_velocity,
            is_off: false,
            duration,
            // Our current curves never overshoot.
            clamped_duration: duration,
            start_time: clock.now(),
            clock,
            kind,
        }
    }

    pub fn spring(clock: Clock, spring: Spring) -> Self {
        let _span = tracy_client::span!("Animation::spring");

        let duration = spring.duration();
        let clamped_duration = spring.clamped_duration().unwrap_or(duration);
        let kind = Kind::Spring(spring);

        Self {
            from: spring.from,
            to: spring.to,
            initial_velocity: spring.initial_velocity,
            is_off: false,
            duration,
            clamped_duration,
            start_time: clock.now(),
            clock,
            kind,
        }
    }

    pub fn decelerate(
        clock: Clock,
        from: f64,
        initial_velocity: f64,
        deceleration_rate: f64,
        threshold: f64,
    ) -> Self {
        let duration_s = if initial_velocity == 0. {
            0.
        } else {
            let coeff = 1000. * deceleration_rate.ln();
            (-coeff * threshold / initial_velocity.abs()).ln() / coeff
        };
        let duration = Duration::from_secs_f64(duration_s);

        let to = from - initial_velocity / (1000. * deceleration_rate.ln());

        let kind = Kind::Deceleration {
            initial_velocity,
            deceleration_rate,
        };

        Self {
            from,
            to,
            initial_velocity,
            is_off: false,
            duration,
            clamped_duration: duration,
            start_time: clock.now(),
            clock,
            kind,
        }
    }

    pub fn is_done(&self) -> bool {
        if self.clock.should_complete_instantly() {
            return true;
        }

        self.clock.now() >= self.start_time + self.duration
    }

    pub fn is_clamped_done(&self) -> bool {
        if self.clock.should_complete_instantly() {
            return true;
        }

        self.clock.now() >= self.start_time + self.clamped_duration
    }

    pub fn value_at(&self, at: Duration) -> f64 {
        if at <= self.start_time {
            // Return from when at == start_time so that when the animations are off, the behavior
            // within a single event loop cycle (i.e. no time had passed since the start of an
            // animation) matches the behavior when the animations are on.
            return self.from;
        } else if self.start_time + self.duration <= at {
            return self.to;
        }

        if self.clock.should_complete_instantly() {
            return self.to;
        }

        let passed = at.saturating_sub(self.start_time);

        match self.kind {
            Kind::Easing { curve } => {
                let passed = passed.as_secs_f64();
                let total = self.duration.as_secs_f64();
                let x = (passed / total).clamp(0., 1.);
                curve.y(x) * (self.to - self.from) + self.from
            }
            Kind::Spring(spring) => {
                let value = spring.value_at(passed);

                // Protect against numerical instability.
                let range = (self.to - self.from) * 10.;
                let a = self.from - range;
                let b = self.to + range;
                if self.from <= self.to {
                    value.clamp(a, b)
                } else {
                    value.clamp(b, a)
                }
            }
            Kind::Deceleration {
                initial_velocity,
                deceleration_rate,
            } => {
                let passed = passed.as_secs_f64();
                let coeff = 1000. * deceleration_rate.ln();
                self.from + (deceleration_rate.powf(1000. * passed) - 1.) / coeff * initial_velocity
            }
        }
    }

    pub fn value(&self) -> f64 {
        self.value_at(self.clock.now())
    }

    /// Returns a value that stops at the target value after first reaching it.
    ///
    /// Best effort; not always exactly precise.
    pub fn clamped_value(&self) -> f64 {
        if self.is_clamped_done() {
            return self.to;
        }

        self.value()
    }

    pub fn to(&self) -> f64 {
        self.to
    }

    pub fn from(&self) -> f64 {
        self.from
    }

    pub fn start_time(&self) -> Duration {
        self.start_time
    }

    pub fn end_time(&self) -> Duration {
        self.start_time + self.duration
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    pub fn offset(&mut self, offset: f64) {
        self.from += offset;
        self.to += offset;

        if let Kind::Spring(spring) = &mut self.kind {
            spring.from += offset;
            spring.to += offset;
        }
    }
}

impl Curve {
    pub fn y(self, x: f64) -> f64 {
        match self {
            Curve::Linear => x,
            Curve::EaseOutQuad => EaseOutQuad.y(x),
            Curve::EaseOutCubic => EaseOutCubic.y(x),
            Curve::EaseOutExpo => 1. - 2f64.powf(-10. * x),
            Curve::CubicBezier(b) => b.y(x),
        }
    }
}

impl From<niri_config::animations::Curve> for Curve {
    fn from(value: niri_config::animations::Curve) -> Self {
        match value {
            niri_config::animations::Curve::Linear => Curve::Linear,
            niri_config::animations::Curve::EaseOutQuad => Curve::EaseOutQuad,
            niri_config::animations::Curve::EaseOutCubic => Curve::EaseOutCubic,
            niri_config::animations::Curve::EaseOutExpo => Curve::EaseOutExpo,
            niri_config::animations::Curve::CubicBezier(x1, y1, x2, y2) => {
                Curve::CubicBezier(CubicBezier::new(x1, y1, x2, y2))
            }
        }
    }
}
