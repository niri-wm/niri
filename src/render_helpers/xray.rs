use std::array;
use std::cell::RefCell;
use std::rc::Rc;

use glam::{Mat3, Vec2};
use niri_config::CornerRadius;
use smithay::backend::renderer::element::{Element, Id, RenderElement};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform,
};
use smithay::backend::renderer::utils::{CommitCounter, OpaqueRegions};
use smithay::backend::renderer::Color32F;
use smithay::utils::{Buffer, Logical, Physical, Rectangle, Scale, Size, Transform};

use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
use crate::render_helpers::background_effect::{EffectSubregion, RenderParams};
use crate::render_helpers::effect_buffer::EffectBuffer;
use crate::render_helpers::renderer::AsGlesFrame as _;
use crate::render_helpers::shaders::{mat3_uniform, Shaders};
use crate::render_helpers::{RenderCtx, RenderTarget};

#[derive(Debug)]
pub struct Xray {
    // The buffers are per-render-target to avoid constant rerendering when screencasting.
    pub background: [Rc<RefCell<EffectBuffer>>; RenderTarget::COUNT],
    pub backdrop: [Rc<RefCell<EffectBuffer>>; RenderTarget::COUNT],
    pub backdrop_color: Color32F,
    pub workspaces: Vec<(Rectangle<f64, Logical>, Color32F)>,
}

#[derive(Debug)]
pub struct XrayElement {
    buffer: Rc<RefCell<EffectBuffer>>,
    id: Id,
    geometry: Rectangle<f64, Logical>,
    src: Rectangle<f64, Buffer>,
    subregion: Option<EffectSubregion>,
    input_to_clip_geo: Mat3,
    clip_geo_size: Vec2,
    corner_radius: CornerRadius,
    scale: f32,
    blur: bool,
    noise: f32,
    saturation: f32,
    bg_color: Color32F,
    program: Option<GlesTexProgram>,
}

impl Xray {
    pub fn new() -> Self {
        Self {
            background: array::from_fn(|_| Rc::new(RefCell::new(EffectBuffer::new()))),
            backdrop: array::from_fn(|_| Rc::new(RefCell::new(EffectBuffer::new()))),
            backdrop_color: Color32F::TRANSPARENT,
            workspaces: Vec::new(),
        }
    }

    pub fn render(
        &self,
        ctx: RenderCtx<GlesRenderer>,
        params: RenderParams,
        blur: bool,
        noise: f32,
        saturation: f32,
        push: &mut dyn FnMut(XrayElement),
    ) {
        let program = Shaders::get(ctx.renderer).postprocess_and_clip.clone();

        let (clip_geo, corner_radius) = params
            .clip
            .unwrap_or((params.geometry, CornerRadius::default()));

        let clip_pos_in_backdrop =
            params.pos_in_backdrop + (clip_geo.loc - params.geometry.loc).upscale(params.zoom);

        let geo_in_backdrop = Rectangle::new(
            params.pos_in_backdrop,
            params.geometry.size.upscale(params.zoom),
        );

        let mut skip_backdrop = false;

        let mut background = self.background[ctx.target as usize].borrow_mut();
        let prev = background.commit();
        if background.prepare(ctx.renderer, blur) {
            if background.commit() != prev {
                debug!("background damaged");
            }

            let clip_geo_size = Vec2::new(clip_geo.size.w as f32, clip_geo.size.h as f32);
            let buf_size = background.logical_size();

            for (ws_geo, bg_color) in &self.workspaces {
                // If the background color is opaque, check if the workspace fully covers the
                // element. In this case, we will skip the backdrop element since it's fully
                // covered.
                //
                // FIXME: also implement some way to check if the background elements are fully
                // covered in opaque regions, and not just the niri background color is opaque
                let crop = if bg_color.is_opaque() && ws_geo.contains_rect(geo_in_backdrop) {
                    skip_backdrop = true;
                    // No need to intersect, we know it's fully covered.
                    Some(geo_in_backdrop)
                } else {
                    ws_geo.intersection(geo_in_backdrop)
                };

                let Some(crop) = crop else {
                    continue;
                };

                // This can be different from params.zoom for surfaces that do not scale with
                // workspaces, e.g. layer-shell top and overlay layer.
                let ws_zoom = ws_geo.size / buf_size;

                let buf_size = Vec2::new(buf_size.w as f32, buf_size.h as f32);
                let pos_against_buf = (clip_pos_in_backdrop - ws_geo.loc).downscale(ws_zoom);
                let pos_against_buf = Vec2::new(pos_against_buf.x as f32, pos_against_buf.y as f32);
                let ws_zoom_vec = Vec2::new(ws_zoom.x as f32, ws_zoom.y as f32);
                let input_to_clip_geo = Mat3::from_scale(ws_zoom_vec / params.zoom as f32)
                    * Mat3::from_scale(buf_size / clip_geo_size)
                    * Mat3::from_translation(-pos_against_buf / buf_size);

                let src = Rectangle::new(crop.loc - ws_geo.loc, crop.size).downscale(ws_zoom);
                let src = src.to_buffer(
                    background.scale(),
                    Transform::Normal,
                    &background.logical_size(),
                );

                let mut geometry = Rectangle::new(crop.loc - params.pos_in_backdrop, crop.size)
                    .downscale(params.zoom);
                geometry.loc += params.geometry.loc;

                let elem = XrayElement {
                    buffer: self.background[ctx.target as usize].clone(),
                    id: background.id().clone(),
                    geometry,
                    src,
                    subregion: params.subregion.clone(),
                    input_to_clip_geo,
                    clip_geo_size,
                    corner_radius,
                    scale: params.scale as f32,
                    blur,
                    noise,
                    saturation,
                    bg_color: *bg_color,
                    program: program.clone(),
                };
                push(elem);
            }
        }

        // If the backdrop is fully covered by opaque background, we can skip it.
        if skip_backdrop {
            return;
        }

        let mut backdrop = self.backdrop[ctx.target as usize].borrow_mut();
        let prev = backdrop.commit();
        if backdrop.prepare(ctx.renderer, blur) {
            if backdrop.commit() != prev {
                debug!("backdrop damaged");
            }

            let src = geo_in_backdrop.to_buffer(
                backdrop.scale(),
                Transform::Normal,
                &backdrop.logical_size(),
            );

            let clip_pos_in_backdrop =
                Vec2::new(clip_pos_in_backdrop.x as f32, clip_pos_in_backdrop.y as f32);

            let clip_size_in_backdrop = clip_geo.size.upscale(params.zoom);
            let clip_geo_size = Vec2::new(
                clip_size_in_backdrop.w as f32,
                clip_size_in_backdrop.h as f32,
            );
            let buf_size = backdrop.logical_size();
            let buf_size = Vec2::new(buf_size.w as f32, buf_size.h as f32);
            let input_to_clip_geo = Mat3::from_scale(buf_size / clip_geo_size)
                * Mat3::from_translation(-clip_pos_in_backdrop / buf_size);

            let elem = XrayElement {
                buffer: self.backdrop[ctx.target as usize].clone(),
                id: backdrop.id().clone(),
                geometry: params.geometry,
                src,
                subregion: params.subregion.clone(),
                input_to_clip_geo,
                clip_geo_size,
                corner_radius: corner_radius.scaled_by(params.zoom as f32),
                scale: params.scale as f32,
                blur,
                noise,
                saturation,
                bg_color: self.backdrop_color,
                program: program.clone(),
            };
            push(elem);
        }
    }
}

impl XrayElement {
    fn compute_uniforms(&self) -> [Uniform<'static>; 7] {
        [
            Uniform::new("niri_scale", self.scale),
            Uniform::new("geo_size", <[f32; 2]>::from(self.clip_geo_size)),
            Uniform::new("corner_radius", <[f32; 4]>::from(self.corner_radius)),
            mat3_uniform("input_to_geo", self.input_to_clip_geo),
            Uniform::new("noise", self.noise),
            Uniform::new("saturation", self.saturation),
            Uniform::new("bg_color", self.bg_color.components()),
        ]
    }
}

impl Element for XrayElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.buffer.borrow().commit()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.src
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry.to_physical_precise_round(scale)
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // TODO: if bg_color alpha is 1 then compute opaque regions here taking corners into account
        OpaqueRegions::default()
    }
}

impl RenderElement<GlesRenderer> for XrayElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let mut buffer = self.buffer.borrow_mut();
        let texture = match buffer.render(frame, self.blur) {
            Ok(x) => x,
            Err(err) => {
                warn!("error rendering effect buffer: {err:?}");
                return Ok(());
            }
        };

        let mut filtered_damage = Vec::new();
        let damage = if let Some(subregion) = &self.subregion {
            let src_to_geo = self.geometry.size / self.src.size;

            // Compute crop in geometry coordinates.
            let mut crop = src;
            crop.loc -= self.src.loc;
            crop = crop.upscale(src_to_geo);
            let mut crop = crop.to_logical(1., Transform::Normal, &Size::default());

            // Then convert to subregion coordinates.
            crop.loc += self.geometry.loc;

            subregion.filter_damage(crop, dst, damage, &mut filtered_damage);

            if filtered_damage.is_empty() {
                return Ok(());
            }
            &filtered_damage[..]
        } else {
            damage
        };

        let uniforms = self.program.is_some().then(|| self.compute_uniforms());
        let uniforms = uniforms.as_ref().map_or(&[][..], |x| &x[..]);

        frame.render_texture_from_to(
            &texture,
            src,
            dst,
            damage,
            // TODO: opaque regions need to be filtered like damage.
            &[],
            Transform::Normal,
            1.,
            self.program.as_ref(),
            uniforms,
        )
    }
}

impl<'render> RenderElement<TtyRenderer<'render>> for XrayElement {
    fn draw(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), TtyRendererError<'render>> {
        let gles_frame = frame.as_gles_frame();
        RenderElement::<GlesRenderer>::draw(&self, gles_frame, src, dst, damage, opaque_regions)?;
        Ok(())
    }
}
