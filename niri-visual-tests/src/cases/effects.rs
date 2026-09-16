use niri::render_helpers::background_effect::RenderParams;
use niri::render_helpers::framebuffer_effect::FramebufferEffect;
use niri::render_helpers::shadow::ShadowRenderElement;
use niri::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use niri_config::{Color, CornerRadius};
use smithay::backend::renderer::element::{Kind, RenderElement};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};

use super::{Args, TestCase};

#[derive(Debug, Clone, Copy)]
enum Mode {
    Refraction,
    Feather,
    Shadow,
}

#[derive(Debug)]
pub struct Effects {
    mode: Mode,
    effect: FramebufferEffect,
}

impl Effects {
    pub fn refraction(_args: Args) -> Self {
        Self::new(Mode::Refraction)
    }

    pub fn feather(_args: Args) -> Self {
        Self::new(Mode::Feather)
    }

    pub fn shadow(_args: Args) -> Self {
        Self::new(Mode::Shadow)
    }

    fn new(mode: Mode) -> Self {
        Self {
            mode,
            effect: FramebufferEffect::new(),
        }
    }
}

impl TestCase for Effects {
    fn render(
        &mut self,
        _renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Vec<Box<dyn RenderElement<GlesRenderer>>> {
        let output_size = Size::<f64, Logical>::from((size.w as f64, size.h as f64));
        let mut elements: Vec<Box<dyn RenderElement<GlesRenderer>>> = Vec::new();

        match self.mode {
            Mode::Refraction | Mode::Feather => {
                let effect_size = Size::from((output_size.w * 0.72, output_size.h * 0.66));
                let effect_loc = Point::from((
                    (output_size.w - effect_size.w) * 0.5,
                    (output_size.h - effect_size.h) * 0.5,
                ));
                let effect_geometry = Rectangle::new(effect_loc, effect_size);
                let corner_radius = CornerRadius::from(28.);

                let (refraction, refraction_bevel, feather, dim) = match self.mode {
                    Mode::Refraction => (0.8, 28., 0., 0.),
                    Mode::Feather => (0., 0., 24., 0.15),
                    Mode::Shadow => unreachable!(),
                };
                let params = RenderParams {
                    geometry: effect_geometry,
                    clip: Some((effect_geometry, corner_radius)),
                    subregion: None,
                    scale: 1.,
                };
                let effect = self.effect.render(
                    None,
                    params,
                    None,
                    0.,
                    1.,
                    refraction,
                    refraction_bevel,
                    1.3,
                    1.1,
                    feather,
                    dim,
                );

                // SmithayView draws elements in reverse order. Put the effect
                // first so it captures the colored background elements below.
                elements.push(Box::new(effect));

                let accent_size = Size::from((output_size.w * 0.42, output_size.h * 0.38));
                let accent_loc = Point::from((
                    (output_size.w - accent_size.w) * 0.5,
                    (output_size.h - accent_size.h) * 0.5,
                ));
                let accent = SolidColorBuffer::new(accent_size, [0.95, 0.36, 0.12, 1.]);
                elements.push(Box::new(SolidColorRenderElement::from_buffer(
                    &accent,
                    accent_loc,
                    1.,
                    Kind::Unspecified,
                )));

                let background = SolidColorBuffer::new(output_size, [0.06, 0.12, 0.22, 1.]);
                elements.push(Box::new(SolidColorRenderElement::from_buffer(
                    &background,
                    Point::from((0., 0.)),
                    1.,
                    Kind::Unspecified,
                )));
            }
            Mode::Shadow => {
                let shadow_size = Size::from((output_size.w * 0.58, output_size.h * 0.52));
                let shadow_loc = Point::from((
                    (output_size.w - shadow_size.w) * 0.5,
                    (output_size.h - shadow_size.h) * 0.5,
                ));
                let shadow_geometry = Rectangle::new(shadow_loc, shadow_size);
                let feather_size = Size::from((shadow_size.w * 0.68, shadow_size.h * 0.68));
                let feather_loc = Point::from((
                    (output_size.w - feather_size.w) * 0.5,
                    (output_size.h - feather_size.h) * 0.5,
                ));
                let feather_geometry = Rectangle::new(feather_loc, feather_size);
                let feather_radius = CornerRadius::from(20.);

                let shadow = ShadowRenderElement::new_with_feather(
                    output_size,
                    shadow_geometry,
                    Color::from_rgba8_unpremul(0, 0, 0, 190),
                    12.,
                    feather_radius,
                    1.,
                    feather_geometry,
                    feather_radius,
                    1.,
                    feather_geometry,
                    feather_radius,
                    24.,
                );

                // Draw the client surface over the shadow, leaving enough
                // alpha to make the inward feather visible.
                let client = SolidColorBuffer::new(feather_size, [0.15, 0.55, 0.9, 0.72]);
                elements.push(Box::new(SolidColorRenderElement::from_buffer(
                    &client,
                    feather_loc,
                    1.,
                    Kind::Unspecified,
                )));
                elements.push(Box::new(shadow));

                let background = SolidColorBuffer::new(output_size, [0.06, 0.12, 0.22, 1.]);
                elements.push(Box::new(SolidColorRenderElement::from_buffer(
                    &background,
                    Point::from((0., 0.)),
                    1.,
                    Kind::Unspecified,
                )));
            }
        }

        elements
    }
}
