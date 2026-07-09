use niri::render_helpers::border::BorderRenderElement;
use niri_config::{
    Color, CornerRadius, GradientColorSpace, GradientInterpolation, HueInterpolation,
};
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Rectangle, Size};

use super::{Args, TestCase};

pub struct GradientOklchDecreasing {
    gradient_format: GradientInterpolation,
}

impl GradientOklchDecreasing {
    pub fn new(_args: Args) -> Self {
        Self {
            gradient_format: GradientInterpolation {
                color_space: GradientColorSpace::Oklch,
                hue_interpolation: HueInterpolation::Decreasing,
            },
        }
    }
}

impl TestCase for GradientOklchDecreasing {
    fn render(
        &mut self,
        _renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Vec<Box<dyn RenderElement<GlesRenderer>>> {
        let (a, b) = (size.w / 6, size.h / 3);
        let size = (size.w - a * 2, size.h - b * 2);
        let area = Rectangle::new(Point::from((a, b)), Size::from(size)).to_f64();

        [BorderRenderElement::new(
            area.size,
            Rectangle::from_size(area.size),
            self.gradient_format,
            Color::new_unpremul(1., 0., 0., 1.),
            Color::new_unpremul(0., 1., 0., 1.),
            0.,
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
