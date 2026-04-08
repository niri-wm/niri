use std::f32::consts::{FRAC_PI_4, PI};
use std::time::Duration;

use niri::layout::focus_ring::FocusRing;
use niri::render_helpers::border::BorderRenderElement;
use niri_config::{Color, CornerRadius, GradientInterpolation};
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Rectangle, Size};

use super::{Args, TestCase};

pub struct GradientArea {
    progress: f32,
    border: FocusRing,
    prev_time: Duration,
}

impl GradientArea {
    pub fn new(_args: Args) -> Self {
        let border = FocusRing::new(niri_config::FocusRing {
            off: false,
            width: 1.,
            active_color: Color::from_rgba8_unpremul(255, 255, 255, 128),
            inactive_color: Color::default(),
            urgent_color: Color::default(),
            active_gradient: None,
            inactive_gradient: None,
            urgent_gradient: None,
        });

        Self {
            progress: 0.,
            border,
            prev_time: Duration::ZERO,
        }
    }
}

impl TestCase for GradientArea {
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

        self.progress += delta.as_secs_f32() * PI;

        if self.progress >= PI * 2. {
            self.progress -= PI * 2.
        }
    }

    fn render(
        &mut self,
        renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Vec<Box<dyn RenderElement<GlesRenderer>>> {
        let mut rv = Vec::new();

        let f = (self.progress.sin() + 1.) / 2.;

        let (a, b) = (size.w / 4, size.h / 4);
        let rect_size = Size::from((size.w - a * 2, size.h - b * 2));
        let area = Rectangle::new(Point::from((a, b)), rect_size).to_f64();

        let g_size = Size::from((
            (size.w as f32 / 8. + size.w as f32 / 8. * 7. * f).round() as i32,
            (size.h as f32 / 8. + size.h as f32 / 8. * 7. * f).round() as i32,
        ));
        let g_loc = Point::from(((size.w - g_size.w) / 2, (size.h - g_size.h) / 2)).to_f64();
        let g_size = g_size.to_f64();
        let mut g_area = Rectangle::new(g_loc, g_size);
        g_area.loc -= area.loc;

        self.border.update_render_elements(
            g_size,
            true,
            true,
            false,
            Rectangle::default(),
            CornerRadius::default(),
            1.,
            1.,
        );
        self.border
            .render(renderer, g_loc, &mut |elem| rv.push(Box::new(elem) as _));

        rv.extend(
            [BorderRenderElement::new(
                area.size,
                g_area,
                GradientInterpolation::default(),
                Color::new_unpremul(1., 0., 0., 1.),
                Color::new_unpremul(0., 1., 0., 1.),
                FRAC_PI_4,
                Rectangle::from_size(rect_size).to_f64(),
                0.,
                CornerRadius::default(),
                1.,
                1.,
            )
            .with_location(area.loc)]
            .into_iter()
            .map(|elem| Box::new(elem) as _),
        );

        rv
    }
}
