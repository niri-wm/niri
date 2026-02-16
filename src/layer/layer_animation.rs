use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Context as _;
use glam::{Mat3, Vec2};
use smithay::backend::renderer::element::utils::{
    Relocate, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::{Element as _, Kind, RenderElement, RenderElementStates};
use smithay::backend::renderer::gles::{GlesRenderer, Uniform};
use smithay::backend::renderer::{Renderer, Texture};
use smithay::utils::{Logical, Point, Rectangle, Scale, Size};

use crate::animation::Animation;
use crate::niri_render_elements;
use crate::render_helpers::offscreen::{OffscreenBuffer, OffscreenData, OffscreenRenderElement};
use crate::render_helpers::shader_element::ShaderRenderElement;
use crate::render_helpers::shaders::{mat3_uniform, ProgramType, Shaders};

#[derive(Debug)]
pub struct LayerAnimation {
    anim: Animation,
    random_seed: f32,
    buffer: OffscreenBuffer,
    is_open: bool,
    snapshot: RefCell<Option<OffscreenRenderElement>>,
}

niri_render_elements! {
    LayerAnimationRenderElement => {
        Offscreen = RelocateRenderElement<RescaleRenderElement<OffscreenRenderElement>>,
        Shader = ShaderRenderElement,
    }
}

impl LayerAnimation {
    pub fn new_open(anim: Animation) -> Self {
        Self {
            anim,
            random_seed: fastrand::f32(),
            buffer: OffscreenBuffer::default(),
            is_open: true,
            snapshot: RefCell::new(None),
        }
    }

    pub fn new_close(anim: Animation) -> Self {
        Self {
            anim,
            random_seed: fastrand::f32(),
            buffer: OffscreenBuffer::default(),
            is_open: false,
            snapshot: RefCell::new(None),
        }
    }

    pub fn is_done(&self) -> bool {
        self.anim.is_done()
    }

    pub fn clamped_value(&self) -> f64 {
        self.anim.clamped_value()
    }

    pub fn render(
        &self,
        renderer: &mut GlesRenderer,
        elements: &[impl RenderElement<GlesRenderer>],
        geo_size: Size<f64, Logical>,
        location: Point<f64, Logical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> anyhow::Result<(LayerAnimationRenderElement, OffscreenData)> {
        let progress = self.anim.value();
        let clamped_progress = self.anim.clamped_value().clamp(0., 1.);
        let program_type = if self.is_open {
            ProgramType::LayerOpen
        } else {
            ProgramType::LayerClose
        };

        let (elem, sync_point, data) = if !self.is_open {
            let mut snapshot = self.snapshot.borrow_mut();
            if let Some(ref elem) = *snapshot {
                if *elem.renderer_context_id() != renderer.context_id() {
                    trace!("invalidating close snapshot due to renderer context change");
                    *snapshot = None;
                }
            }
            if snapshot.is_none() {
                let (elem, sync_point, data) = self
                    .buffer
                    .render(renderer, scale, elements)
                    .context("error rendering layer to offscreen buffer")?;
                *snapshot = Some(elem.clone());
                return self.render_with_element(
                    renderer,
                    elem,
                    sync_point,
                    data,
                    geo_size,
                    location,
                    scale,
                    alpha,
                    progress,
                    clamped_progress,
                    program_type,
                );
            }
            let s = snapshot.clone().unwrap();
            let s_id = s.id().clone();
            (
                s,
                Default::default(),
                OffscreenData {
                    id: s_id,
                    states: RenderElementStates::default(),
                },
            )
        } else {
            self.buffer
                .render(renderer, scale, elements)
                .context("error rendering layer to offscreen buffer")?
        };

        self.render_with_element(
            renderer,
            elem,
            sync_point,
            data,
            geo_size,
            location,
            scale,
            alpha,
            progress,
            clamped_progress,
            program_type,
        )
    }

    fn render_with_element(
        &self,
        renderer: &mut GlesRenderer,
        elem: OffscreenRenderElement,
        _sync_point: smithay::backend::renderer::sync::SyncPoint,
        mut data: OffscreenData,
        geo_size: Size<f64, Logical>,
        location: Point<f64, Logical>,
        scale: Scale<f64>,
        alpha: f32,
        progress: f64,
        clamped_progress: f64,
        program_type: ProgramType,
    ) -> anyhow::Result<(LayerAnimationRenderElement, OffscreenData)> {
        if Shaders::get(renderer).program(program_type).is_some() {
            let offset = elem.offset();
            let texture = elem.texture();
            let texture_size = elem.logical_size();

            let mut area = Rectangle::new(location + offset, texture_size);

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
            let geo_size_vec = Vec2::new(geo_size.w as f32, geo_size.h as f32);

            let input_to_geo = Mat3::from_scale(area_size / geo_size_vec)
                * Mat3::from_translation((area_loc - geo_loc) / area_size);

            let tex_scale = Vec2::new(scale.x as f32, scale.y as f32);
            let tex_loc = Vec2::new(offset.x as f32, offset.y as f32);
            let tex_size = Vec2::new(texture.width() as f32, texture.height() as f32) / tex_scale;

            let geo_to_tex = Mat3::from_translation(-tex_loc / tex_size)
                * Mat3::from_scale(geo_size_vec / tex_size);

            let effective_alpha = if self.is_open {
                clamped_progress as f32 * alpha
            } else {
                (1. - clamped_progress as f32) * alpha
            };

            let elem = ShaderRenderElement::new(
                program_type,
                area.size,
                None,
                scale.x as f32,
                effective_alpha,
                Rc::new([
                    mat3_uniform("niri_input_to_geo", input_to_geo),
                    Uniform::new("niri_geo_size", geo_size_vec.to_array()),
                    mat3_uniform("niri_geo_to_tex", geo_to_tex),
                    Uniform::new("niri_progress", progress as f32),
                    Uniform::new("niri_clamped_progress", clamped_progress as f32),
                    Uniform::new("niri_random_seed", self.random_seed),
                ]),
                HashMap::from([(String::from("niri_tex"), texture.clone())]),
                Kind::Unspecified,
            )
            .with_location(area.loc);

            data.id = elem.id().clone();

            return Ok((elem.into(), data));
        }

        let effective_alpha = if self.is_open {
            clamped_progress as f32 * alpha
        } else {
            (1. - clamped_progress as f32) * alpha
        };

        if self.is_open {
            let elem = elem.with_alpha(effective_alpha);

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

            Ok((LayerAnimationRenderElement::Offscreen(elem), data))
        } else {
            let elem = elem.with_alpha(effective_alpha);
            let scaled = RescaleRenderElement::from_element(elem, Point::from((0i32, 0i32)), 1.0);

            let elem = RelocateRenderElement::from_element(
                scaled,
                location.to_physical_precise_round(scale),
                Relocate::Relative,
            );

            Ok((LayerAnimationRenderElement::Offscreen(elem), data))
        }
    }
}
