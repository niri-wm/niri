use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Context as _;
use glam::{Mat3, Vec2};
use smithay::backend::renderer::element::utils::{
    Relocate, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::{Element as _, Kind, RenderElement};
use smithay::backend::renderer::gles::{GlesRenderer, Uniform};
use smithay::backend::renderer::Texture;
use smithay::utils::{Logical, Point, Rectangle, Scale, Size};

use crate::animation::Animation;
use crate::niri_render_elements;
use crate::render_helpers::offscreen::{OffscreenBuffer, OffscreenData, OffscreenRenderElement};
use crate::render_helpers::shader_element::ShaderRenderElement;
use crate::render_helpers::shaders::{mat3_uniform, ProgramType, Shaders};

#[derive(Debug)]
pub struct OpenAnimation {
    anim: Animation,
    random_seed: f32,
    buffer: OffscreenBuffer,
}

niri_render_elements! {
    OpeningWindowRenderElement => {
        Offscreen = RelocateRenderElement<RescaleRenderElement<OffscreenRenderElement>>,
        Shader = ShaderRenderElement,
    }
}

impl OpenAnimation {
    pub fn new(anim: Animation) -> Self {
        Self {
            anim,
            random_seed: fastrand::f32(),
            buffer: OffscreenBuffer::default(),
        }
    }

    pub fn is_done(&self) -> bool {
        self.anim.is_done()
    }

    // We can't depend on view_rect here, because the result of window opening can be snapshot and
    // then rendered elsewhere.
    pub fn render(
        &self,
        renderer: &mut GlesRenderer,
        elements: &[impl RenderElement<GlesRenderer>],
        geo_size: Size<f64, Logical>,
        location: Point<f64, Logical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> anyhow::Result<(OpeningWindowRenderElement, OffscreenData)> {
        let progress = self.anim.value();
        let clamped_progress = self.anim.clamped_value().clamp(0., 1.);

        let (elem, _sync_point, mut data) = self
            .buffer
            .render(renderer, scale, elements)
            .context("error rendering to offscreen buffer")?;

        if Shaders::get(renderer).program(ProgramType::Open).is_some() {
            // OffscreenBuffer renders with Transform::Normal and the scale that we passed, so we
            // can assume that below.
            let offset = elem.offset();
            let texture = elem.texture();
            let texture_size = elem.logical_size();

            let mut area = Rectangle::new(location + offset, texture_size);

            // Expand the area a bit to allow for more varied effects.
            let mut target_size = area.size.upscale(1.5);
            target_size.w = f64::max(area.size.w + 1000., target_size.w);
            target_size.h = f64::max(area.size.h + 1000., target_size.h);
            let diff = (target_size.to_point() - area.size.to_point()).downscale(2.);
            let diff = diff.to_physical_precise_round(scale).to_logical(scale);
            area.loc -= diff;
            area.size += diff.upscale(2.).to_size();

            let area_loc = Vec2::new(area.loc.x as f32, area.loc.y as f32);
            let area_size = Vec2::new(area.size.w as f32, area.size.h as f32);

            let geo_loc = Vec2::new(location.x as f32, location.y as f32);
            let geo_size = Vec2::new(geo_size.w as f32, geo_size.h as f32);

            let input_to_geo = Mat3::from_scale(area_size / geo_size)
                * Mat3::from_translation((area_loc - geo_loc) / area_size);

            let tex_scale = Vec2::new(scale.x as f32, scale.y as f32);
            let tex_loc = Vec2::new(offset.x as f32, offset.y as f32);
            let tex_size = Vec2::new(texture.width() as f32, texture.height() as f32) / tex_scale;

            let geo_to_tex =
                Mat3::from_translation(-tex_loc / tex_size) * Mat3::from_scale(geo_size / tex_size);

            let elem = ShaderRenderElement::new(
                ProgramType::Open,
                area.size,
                None,
                scale.x as f32,
                alpha,
                Rc::new([
                    mat3_uniform("niri_input_to_geo", input_to_geo),
                    Uniform::new("niri_geo_size", geo_size.to_array()),
                    mat3_uniform("niri_geo_to_tex", geo_to_tex),
                    Uniform::new("niri_progress", progress as f32),
                    Uniform::new("niri_clamped_progress", clamped_progress as f32),
                    Uniform::new("niri_random_seed", self.random_seed),
                ]),
                HashMap::from([(String::from("niri_tex"), texture.clone())]),
                Kind::Unspecified,
            )
            .with_location(area.loc);

            // We're drawing the shader, not the offscreen itself.
            data.id = elem.id().clone();

            return Ok((elem.into(), data));
        }

        let elem = elem.with_alpha(clamped_progress as f32 * alpha);

        let center = geo_size.to_point().downscale(2.);
        let elem = RescaleRenderElement::from_element(
            elem,
            center.to_physical_precise_round(scale),
            (progress / 2. + 0.5).max(0.),
        );

        let elem = RelocateRenderElement::from_element(
            elem,
            location.to_physical_precise_round(scale),
            Relocate::Relative,
        );

        Ok((elem.into(), data))
    }
}
