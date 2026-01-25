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
use smithay::utils::{Buffer, Logical, Physical, Rectangle, Scale, Transform};

use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
use crate::render_helpers::background_effect::{Options, RenderParams};
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
    blur: bool,
    geometry: Rectangle<f64, Logical>,
    src: Rectangle<f64, Buffer>,
    input_to_clip_geo: Mat3,
    clip_geo_size: Vec2,
    corner_radius: CornerRadius,
    scale: f32,
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
        options: Options,
        params: RenderParams,
        push: &mut dyn FnMut(XrayElement),
    ) {
        let program = Shaders::get(ctx.renderer).postprocess_and_clip.clone();

        let (clip_geo, corner_radius) = params
            .clip
            .unwrap_or((params.geometry, CornerRadius::default()));

        let clip_pos_in_backdrop =
            params.pos_in_backdrop + (clip_geo.loc - params.geometry.loc).upscale(params.zoom);
        let clip_geo_in_backdrop =
            Rectangle::new(clip_pos_in_backdrop, clip_geo.size.upscale(params.zoom));

        // Iterate over subregions. They are non-overlapping by construction, so the element
        // ordering across them doesn't matter.
        let mut main;
        let mut fallback;
        let rects: &mut dyn Iterator<Item = _> = if let Some(region) = &params.subregion {
            main = region.iter().filter_map(|r| {
                // Round extremities (rather than loc and size) to physical pixels to ensure that
                // adjacent rectangles stay adjacent at fractional scales.
                let mut a = r.loc;
                let mut b = r.loc + r.size.to_point();

                a = a
                    .to_physical_precise_round(params.scale)
                    .to_logical(params.scale);
                b = b
                    .to_physical_precise_round(params.scale)
                    .to_logical(params.scale);

                let r = Rectangle::from_extremities(a, b);

                // And intersect with geometry.
                r.intersection(params.geometry)
            });
            &mut main
        } else {
            fallback = [params.geometry].into_iter();
            &mut fallback
        };

        for rect in rects {
            let rect_pos_in_backdrop =
                params.pos_in_backdrop + (rect.loc - params.geometry.loc).upscale(params.zoom);
            let rect_geo_in_backdrop =
                Rectangle::new(rect_pos_in_backdrop, rect.size.upscale(params.zoom));

            let mut background = self.background[ctx.target as usize].borrow_mut();
            let prev = background.commit();
            if let Some(blur) = background.prepare(ctx.renderer, options.blur) {
                if background.commit() != prev {
                    debug!("background damaged");
                }

                // Use noise/saturation from options, falling back to blur defaults if blurred, and
                // to no effect if not blurred.
                let blur_config = background.blur_config();
                let noise = options
                    .noise
                    .unwrap_or(if blur { blur_config.noise } else { 0. })
                    as f32;
                let saturation =
                    options
                        .saturation
                        .unwrap_or(if blur { blur_config.saturation } else { 1. })
                        as f32;

                let clip_geo_size = Vec2::new(clip_geo.size.w as f32, clip_geo.size.h as f32);
                let buf_size = background.logical_size();

                for (ws_geo, bg_color) in &self.workspaces {
                    let Some(crop) = ws_geo.intersection(rect_geo_in_backdrop) else {
                        continue;
                    };

                    // This can be different from params.zoom for surfaces that do not scale with
                    // workspaces, e.g. layer-shell top and overlay layer.
                    let ws_zoom = ws_geo.size / buf_size;

                    let buf_size = Vec2::new(buf_size.w as f32, buf_size.h as f32);
                    let pos_against_buf = (clip_pos_in_backdrop - ws_geo.loc).downscale(ws_zoom);
                    let pos_against_buf =
                        Vec2::new(pos_against_buf.x as f32, pos_against_buf.y as f32);
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
                        blur,
                        geometry,
                        src,
                        input_to_clip_geo,
                        clip_geo_size,
                        corner_radius,
                        scale: params.scale as f32,
                        noise,
                        saturation,
                        bg_color: *bg_color,
                        program: program.clone(),
                    };
                    push(elem);
                }
            }
            // TODO: we can try to compute when background fully covers the geometry and has a fully
            // opaque bg color, and skip pushing the backdrop element.

            let mut backdrop = self.backdrop[ctx.target as usize].borrow_mut();
            let prev = backdrop.commit();
            if let Some(blur) = backdrop.prepare(ctx.renderer, options.blur) {
                if backdrop.commit() != prev {
                    debug!("backdrop damaged");
                }

                // Use noise/saturation from options, falling back to blur defaults if blurred, and
                // to no effect if not blurred.
                let blur_config = backdrop.blur_config();
                let noise = options
                    .noise
                    .unwrap_or(if blur { blur_config.noise } else { 0. })
                    as f32;
                let saturation =
                    options
                        .saturation
                        .unwrap_or(if blur { blur_config.saturation } else { 1. })
                        as f32;

                let src = rect_geo_in_backdrop.to_buffer(
                    backdrop.scale(),
                    Transform::Normal,
                    &backdrop.logical_size(),
                );

                let clip_pos_in_backdrop = Vec2::new(
                    clip_geo_in_backdrop.loc.x as f32,
                    clip_geo_in_backdrop.loc.y as f32,
                );

                let clip_geo_size = Vec2::new(
                    clip_geo_in_backdrop.size.w as f32,
                    clip_geo_in_backdrop.size.h as f32,
                );
                let buf_size = backdrop.logical_size();
                let buf_size = Vec2::new(buf_size.w as f32, buf_size.h as f32);
                let input_to_clip_geo = Mat3::from_scale(buf_size / clip_geo_size)
                    * Mat3::from_translation(-clip_pos_in_backdrop / buf_size);

                let elem = XrayElement {
                    buffer: self.backdrop[ctx.target as usize].clone(),
                    id: backdrop.id().clone(),
                    blur,
                    geometry: rect,
                    src,
                    input_to_clip_geo,
                    clip_geo_size,
                    corner_radius: corner_radius.scaled_by(params.zoom as f32),
                    scale: params.scale as f32,
                    noise,
                    saturation,
                    bg_color: self.backdrop_color,
                    program: program.clone(),
                };
                push(elem);
            }
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
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let mut buffer = self.buffer.borrow_mut();
        let texture = match buffer.render(frame, self.blur) {
            Ok(x) => x,
            Err(err) => {
                warn!("error rendering effect buffer: {err:?}");
                return Ok(());
            }
        };

        let uniforms = self.program.is_some().then(|| self.compute_uniforms());
        let uniforms = uniforms.as_ref().map_or(&[][..], |x| &x[..]);

        frame.render_texture_from_to(
            &texture,
            src,
            dst,
            damage,
            opaque_regions,
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
