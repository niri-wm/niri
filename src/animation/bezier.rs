use keyframe::EasingFunction;

#[derive(Debug, Clone, Copy)]
pub struct CubicBezier {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

impl CubicBezier {
    pub fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        Self { x1, y1, x2, y2 }
    }

    // Based on libadwaita (LGPL-2.1-or-later):
    // https://gitlab.gnome.org/GNOME/libadwaita/-/blob/1.7.6/src/adw-easing.c?ref_type=tags#L469-531

    fn x_for_t(&self, t: f64) -> f64 {
        let omt = 1. - t;
        3. * omt * omt * t * self.x1 + 3. * omt * t * t * self.x2 + t * t * t
    }

    fn y_for_t(&self, t: f64) -> f64 {
        let omt = 1. - t;
        3. * omt * omt * t * self.y1 + 3. * omt * t * t * self.y2 + t * t * t
    }

    fn t_for_x(&self, x: f64) -> f64 {
        let mut min_t = 0.;
        let mut max_t = 1.;

        for _ in 0..=30 {
            let guess_t = (min_t + max_t) / 2.;
            let guess_x = self.x_for_t(guess_t);

            if x < guess_x {
                max_t = guess_t;
            } else {
                min_t = guess_t;
            }
        }

        (min_t + max_t) / 2.
    }
}

impl EasingFunction for CubicBezier {
    fn y(&self, x: f64) -> f64 {
        if x <= f64::EPSILON {
            return 0.;
        }

        if 1. - f64::EPSILON <= x {
            return 1.;
        }

        self.y_for_t(self.t_for_x(x))
    }
}
