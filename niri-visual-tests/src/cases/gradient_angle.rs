use std::f32::consts::{FRAC_PI_2, PI};
use std::time::Duration;

use niri::render_helpers::border::BorderRenderElement;
use niri_config::{Color, CornerRadius, GradientInterpolation};
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Rectangle, Size};

use super::{Args, TestCase};

pub struct GradientAngle {
    angle: f32,
    prev_time: Duration,
}

impl GradientAngle {
    pub fn new(_args: Args) -> Self {
        Self {
            angle: 0.,
            prev_time: Duration::ZERO,
        }
    }
}

impl TestCase for GradientAngle {
    fn are_animations_ongoing(&self) -> bool {
        true
    }

    fn advance_animations(&mut self, current_time: Duration) {
        let delta = if self.prev_time.is_zero() {
            Duration::ZERO
        } else {
            current_time.saturating_sub(self.prev_time)
        };
        self.prev_time = current_time;

        self.angle += delta.as_secs_f32() * PI;

        if self.angle >= PI * 2. {
            self.angle -= PI * 2.
        }
    }

    fn render(
        &mut self,
        _renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Vec<Box<dyn RenderElement<GlesRenderer>>> {
        let (a, b) = (size.w / 4, size.h / 4);
        let size = (size.w - a * 2, size.h - b * 2);
        let area = Rectangle::new(Point::from((a, b)), Size::from(size)).to_f64();

        [BorderRenderElement::new(
            area.size,
            Rectangle::from_size(area.size),
            GradientInterpolation::default(),
            Color::new_unpremul(1., 0., 0., 1.),
            Color::new_unpremul(0., 1., 0., 1.),
            self.angle - FRAC_PI_2,
            Rectangle::from_size(area.size),
            0.,
            CornerRadius::default(),
            1.,
            1.,
        )
        .with_location(area.loc)]
        .into_iter()
        .map(|elem| Box::new(elem) as _)
        .collect()
    }
}
