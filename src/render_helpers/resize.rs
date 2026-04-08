use std::collections::HashMap;
use std::rc::Rc;

use glam::{Mat3, Vec2};
use niri_config::CornerRadius;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture, Uniform};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::Texture as _;
use smithay::gpu_span_location;
use smithay::utils::{Buffer, Logical, Physical, Rectangle, Scale, Size, Transform};

use super::renderer::{AsGlesFrame, NiriRenderer};
use super::shader_element::ShaderRenderElement;
use super::shaders::{mat3_uniform, ProgramType, Shaders};
use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};

#[derive(Debug)]
pub struct ResizeRenderElement(ShaderRenderElement);

impl ResizeRenderElement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        area: Rectangle<f64, Logical>,
        scale: Scale<f64>,
        texture_prev: (GlesTexture, Rectangle<i32, Physical>),
        size_prev: Size<f64, Logical>,
        texture_next: (GlesTexture, Rectangle<i32, Physical>),
        size_next: Size<f64, Logical>,
        progress: f32,
        clamped_progress: f32,
        corner_radius: CornerRadius,
        clip_to_geometry: bool,
        result_alpha: f32,
    ) -> Self {
        let curr_geo = area;

        let (texture_prev, tex_prev_geo) = texture_prev;
        let (texture_next, tex_next_geo) = texture_next;

        let scale_prev = area.size / size_prev;
        let scale_next = area.size / size_next;

        // Compute the area necessary to fit a crossfade.
        let tex_prev_geo_scaled = tex_prev_geo.to_f64().upscale(scale_prev);
        let tex_next_geo_scaled = tex_next_geo.to_f64().upscale(scale_next);
        let combined_geo = tex_prev_geo_scaled.merge(tex_next_geo_scaled).to_i32_up();

        let area = Rectangle::new(
            area.loc + combined_geo.loc.to_logical(scale),
            combined_geo.size.to_logical(scale),
        );

        // Convert Smithay types into glam types.
        let area_loc = Vec2::new(area.loc.x as f32, area.loc.y as f32);
        let area_size = Vec2::new(area.size.w as f32, area.size.h as f32);

        let curr_geo_loc = Vec2::new(curr_geo.loc.x as f32, curr_geo.loc.y as f32);
        let curr_geo_size = Vec2::new(curr_geo.size.w as f32, curr_geo.size.h as f32);

        let tex_prev_geo_loc = Vec2::new(tex_prev_geo.loc.x as f32, tex_prev_geo.loc.y as f32);
        let tex_prev_size = Vec2::new(texture_prev.width() as f32, texture_prev.height() as f32);

        let tex_next_geo_loc = Vec2::new(tex_next_geo.loc.x as f32, tex_next_geo.loc.y as f32);
        let tex_next_size = Vec2::new(texture_next.width() as f32, texture_next.height() as f32);

        let size_prev = Vec2::new(size_prev.w as f32, size_prev.h as f32);
        let size_next = Vec2::new(size_next.w as f32, size_next.h as f32);

        let scale = Vec2::new(scale.x as f32, scale.y as f32);

        // Compute the transformation matrices.
        let input_to_curr_geo = Mat3::from_scale(area_size / curr_geo_size)
            * Mat3::from_translation((area_loc - curr_geo_loc) / area_size);

        let curr_geo_to_prev_geo = Mat3::from_scale(curr_geo_size / size_prev);
        let curr_geo_to_next_geo = Mat3::from_scale(curr_geo_size / size_next);

        let geo_to_tex_prev = Mat3::from_translation(-tex_prev_geo_loc / tex_prev_size)
            * Mat3::from_scale(size_prev / tex_prev_size * scale);
        let geo_to_tex_next = Mat3::from_translation(-tex_next_geo_loc / tex_next_size)
            * Mat3::from_scale(size_next / tex_next_size * scale);

        let corner_radius = corner_radius.fit_to(curr_geo_size.x, curr_geo_size.y);
        let clip_to_geometry = if clip_to_geometry { 1. } else { 0. };

        // Create the shader.
        Self(
            ShaderRenderElement::new(
                ProgramType::Resize,
                area.size,
                None,
                scale.x,
                result_alpha,
                Rc::new([
                    mat3_uniform("niri_input_to_curr_geo", input_to_curr_geo),
                    mat3_uniform("niri_curr_geo_to_prev_geo", curr_geo_to_prev_geo),
                    mat3_uniform("niri_curr_geo_to_next_geo", curr_geo_to_next_geo),
                    Uniform::new("niri_curr_geo_size", curr_geo_size.to_array()),
                    mat3_uniform("niri_geo_to_tex_prev", geo_to_tex_prev),
                    mat3_uniform("niri_geo_to_tex_next", geo_to_tex_next),
                    Uniform::new("niri_progress", progress),
                    Uniform::new("niri_clamped_progress", clamped_progress),
                    Uniform::new("niri_corner_radius", <[f32; 4]>::from(corner_radius)),
                    Uniform::new("niri_clip_to_geometry", clip_to_geometry),
                ]),
                HashMap::from([
                    (String::from("niri_tex_prev"), texture_prev),
                    (String::from("niri_tex_next"), texture_next),
                ]),
                Kind::Unspecified,
            )
            .with_location(area.loc),
        )
    }

    pub fn has_shader(renderer: &mut impl NiriRenderer) -> bool {
        Shaders::get(renderer)
            .program(ProgramType::Resize)
            .is_some()
    }
}

impl Element for ResizeRenderElement {
    fn id(&self) -> &Id {
        self.0.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.0.current_commit()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.0.geometry(scale)
    }

    fn transform(&self) -> Transform {
        self.0.transform()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.0.src()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.0.damage_since(scale, commit)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.0.opaque_regions(scale)
    }

    fn alpha(&self) -> f32 {
        self.0.alpha()
    }

    fn kind(&self) -> Kind {
        self.0.kind()
    }
}

impl RenderElement<GlesRenderer> for ResizeRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let _span = tracy_client::span!("ResizeRenderElement::draw");
        frame.with_gpu_span(gpu_span_location!("ResizeRenderElement::draw"), |frame| {
            RenderElement::<GlesRenderer>::draw(&self.0, frame, src, dst, damage, opaque_regions)
        })
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.0.underlying_storage(renderer)
    }
}

impl<'render> RenderElement<TtyRenderer<'render>> for ResizeRenderElement {
    fn draw(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), TtyRendererError<'render>> {
        let frame = frame.as_gles_frame();
        RenderElement::<GlesRenderer>::draw(self, frame, src, dst, damage, opaque_regions)?;
        Ok(())
    }

    fn underlying_storage(
        &self,
        renderer: &mut TtyRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        self.0.underlying_storage(renderer)
    }
}
