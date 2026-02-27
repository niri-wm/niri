use std::cell::RefCell;
use std::rc::Rc;

use glam::{Mat3, Vec2};
use niri_config::CornerRadius;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::{Element, Id, RenderElement};
use smithay::backend::renderer::gles::{
    ffi, GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform,
};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{Frame as _, Texture as _};
use smithay::gpu_span_location;
use smithay::utils::{Buffer, Logical, Physical, Rectangle, Scale, Transform};

use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
use crate::render_helpers::background_effect::{EffectSubregion, RenderParams};
use crate::render_helpers::blur::{Blur, BlurOptions};
use crate::render_helpers::renderer::AsGlesFrame as _;
use crate::render_helpers::shaders::{mat3_uniform, Shaders};

#[derive(Debug)]
pub struct FramebufferEffect {
    id: Id,
    inner: Rc<RefCell<Option<Inner>>>,
}

#[derive(Debug)]
pub struct FramebufferEffectElement {
    id: Id,
    geometry: Rectangle<f64, Logical>,
    clip_geo: Rectangle<f64, Logical>,
    corner_radius: CornerRadius,
    subregion: Option<EffectSubregion>,
    scale: f32,
    blur_options: Option<BlurOptions>,
    noise: f32,
    saturation: f32,
    inner: Rc<RefCell<Option<Inner>>>,
}

#[derive(Debug)]
struct Inner {
    program: Option<GlesTexProgram>,
    framebuffer: Option<GlesTexture>,
    blur: Option<Blur>,
    intermediate: Option<GlesTexture>,
    /// Reusable storage for subregion-filtered damage rects.
    subregion_damage: Vec<Rectangle<i32, Physical>>,
}

impl FramebufferEffect {
    pub fn new() -> Self {
        Self {
            id: Id::new(),
            inner: Rc::new(RefCell::new(None)),
        }
    }

    pub fn render(
        &self,
        renderer: &mut GlesRenderer,
        params: RenderParams,
        blur_options: Option<BlurOptions>,
        noise: f32,
        saturation: f32,
    ) -> Option<FramebufferEffectElement> {
        let (clip_geo, corner_radius) = params
            .clip
            .unwrap_or((params.geometry, CornerRadius::default()));

        let element = FramebufferEffectElement {
            id: self.id.clone(),
            geometry: params.geometry,
            clip_geo,
            corner_radius,
            subregion: params.subregion,
            scale: params.scale as f32,
            blur_options,
            noise,
            saturation,
            inner: self.inner.clone(),
        };

        {
            let mut inner = element.inner.borrow_mut();

            let inner = if let Some(inner) = &*inner {
                inner
            } else {
                let blur = Blur::new(renderer);
                inner.insert(Inner::new(renderer, blur))
            };

            if blur_options.is_some() && inner.blur.is_none() {
                // Blur is requested but the shader is unavailable.
                return None;
            }
        }

        Some(element)
    }
}

impl FramebufferEffectElement {
    fn compute_uniforms(
        &self,
        crop: Rectangle<f64, Logical>,
        transform: Transform,
    ) -> [Uniform<'static>; 7] {
        let offset = crop.loc - (self.clip_geo.loc - self.geometry.loc);
        let offset = Vec2::new(offset.x as f32, offset.y as f32);
        let crop_size = Vec2::new(crop.size.w as f32, crop.size.h as f32);
        let clip_size = Vec2::new(self.clip_geo.size.w as f32, self.clip_geo.size.h as f32);

        // Our v_coords are [0, 1] inside crop. We want them to be [0, 1] inside clip_geo.
        let input_to_clip_geo =
            Mat3::from_scale(crop_size / clip_size) * Mat3::from_translation(offset / crop_size);

        // Revert the effect of the texture transform.
        let transform_mat = Mat3::from_translation(Vec2::new(0.5, 0.5))
            * Mat3::from_cols_array(transform.matrix().as_ref())
            * Mat3::from_translation(Vec2::new(-0.5, -0.5));
        let input_to_clip_geo = input_to_clip_geo * transform_mat;

        let clip_geo_size = (self.clip_geo.size.w as f32, self.clip_geo.size.h as f32);

        [
            Uniform::new("niri_scale", self.scale),
            Uniform::new("geo_size", clip_geo_size),
            Uniform::new("corner_radius", <[f32; 4]>::from(self.corner_radius)),
            mat3_uniform("input_to_geo", input_to_clip_geo),
            Uniform::new("noise", self.noise),
            Uniform::new("saturation", self.saturation),
            Uniform::new("bg_color", [0f32, 0., 0., 0.]),
        ]
    }
}

impl Element for FramebufferEffectElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        CommitCounter::default()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        // We don't use src for drawing but we can use it to figure out how we were cropped.
        let size = self.geometry.size.to_buffer(1., Transform::Normal);
        Rectangle::from_size(size)
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry.to_physical_precise_round(scale)
    }

    fn is_framebuffer_effect(&self) -> bool {
        true
    }
}

impl RenderElement<GlesRenderer> for FramebufferEffectElement {
    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
    ) -> Result<(), GlesError> {
        let mut inner = self.inner.borrow_mut();
        let Some(inner) = &mut *inner else {
            return Ok(());
        };
        let _span = tracy_client::span!("FramebufferEffectElement::capture_framebuffer");

        inner.intermediate = None;

        // We want clamp-to-edge behavior for out-of-bounds pixels. However, glBlitFramebuffer seems
        // to skip out-of-bounds pixels, even though my reading of the docs suggests otherwise (we
        // use GL_LINEAR filter). So, clamp dst to the framebuffer bounds ourselves.
        let output_rect = Rectangle::from_size(frame.output_size());
        let clamped_dst = match dst.intersection(output_rect) {
            Some(clamped) => clamped,
            None => return Ok(()),
        };
        let clamp_scale = clamped_dst.size.to_f64() / dst.size.to_f64();

        let transform = frame.transformation();
        let dst = transform.transform_rect_in(clamped_dst, &output_rect.size);

        // Compute size from our geometry and scale.
        //
        // The "correct" size is always dst.size since that's the pixel region we're actually
        // blitting. However, using dst.size causes two undesirable things when zooming out for the
        // overview:
        // 1. dst.size shrinks every frame, causing a texture realloaction for every fb effect
        //    element every frame.
        // 2. The underlying blur visually expands. This is technically correct, since the
        //    underlying contents shrink, but it's not what you visually expect: you expect the blur
        //    to also shrink as the windows zoom out, to give the zooming out effect.
        //
        // Using size computed from geometry and scale solves both of those problems (even though
        // there's a bit of a cost in that zoomed-out elements still blur the entire unzoomed
        // texture size, and even though the blur ends up slightly wrong as there's two layers of
        // texture resampling, up and back down).
        //
        // Here we use src.size rather than geometry directly because src takes into account
        // cropping.
        let size = src
            .size
            .to_logical(1., Transform::Normal)
            .upscale(clamp_scale)
            .to_physical_precise_round(self.scale);
        let size = transform.transform_size(size);

        let size = size.to_logical(1).to_buffer(1, Transform::Normal);

        let location = gpu_span_location!("FramebufferEffectElement::capture_framebuffer");
        frame.with_gpu_span(location, |frame| {
            // Recreate framebuffer if needed.
            if inner
                .framebuffer
                .as_ref()
                .is_some_and(|fb| fb.size() != size)
            {
                inner.framebuffer = None;
            }
            let framebuffer = if let Some(fb) = &inner.framebuffer {
                fb
            } else {
                trace!("creating framebuffer texture sized {} × {}", size.w, size.h);
                let texture = frame.create_texture(Fourcc::Abgr8888, size)?;
                inner.framebuffer.insert(texture)
            };

            // Prepare blur textures.
            let mut blur = Option::zip(inner.blur.as_mut(), self.blur_options);
            if let Some((b, options)) = &mut blur {
                if let Err(err) = b.prepare_textures(
                    |fourcc, size| frame.create_texture(fourcc, size),
                    framebuffer,
                    *options,
                ) {
                    warn!("error preparing blur textures: {err:?}");
                    blur = None;
                }
            }

            // Blit the framebuffer contents.
            frame.with_context(|gl| unsafe {
                while gl.GetError() != ffi::NO_ERROR {}

                let mut current_fbo = 0i32;
                gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &mut current_fbo as *mut _);

                // BlitFramebuffer is affected by the scissor test, we don't want that.
                gl.Disable(ffi::SCISSOR_TEST);

                let mut fbo = 0;
                gl.GenFramebuffers(1, &mut fbo as *mut _);
                gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, fbo);

                gl.FramebufferTexture2D(
                    ffi::DRAW_FRAMEBUFFER,
                    ffi::COLOR_ATTACHMENT0,
                    ffi::TEXTURE_2D,
                    framebuffer.tex_id(),
                    0,
                );

                gl.BlitFramebuffer(
                    dst.loc.x,
                    dst.loc.y,
                    dst.loc.x + dst.size.w,
                    dst.loc.y + dst.size.h,
                    0,
                    0,
                    size.w,
                    size.h,
                    ffi::COLOR_BUFFER_BIT,
                    ffi::LINEAR,
                );

                // Restore state set by GlesFrame that we just modified.
                gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, current_fbo as u32);
                gl.Enable(ffi::SCISSOR_TEST);

                gl.DeleteFramebuffers(1, &mut fbo as *mut _);

                if gl.GetError() != ffi::NO_ERROR {
                    Err(GlesError::BlitError)
                } else {
                    Ok(())
                }
            })??;

            // If blur is off, use the unblurred texture.
            if self.blur_options.is_none() {
                inner.intermediate = Some(framebuffer.clone());
                return Ok(());
            }

            if let Some((blur, options)) = blur {
                match blur.render(frame, framebuffer, options) {
                    Ok(blurred) => inner.intermediate = Some(blurred),
                    Err(err) => {
                        warn!("error rendering blur: {err:?}");
                    }
                }
            }

            Ok(())
        })
    }

    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let mut inner = self.inner.borrow_mut();
        let Some(inner) = &mut *inner else {
            return Ok(());
        };

        let Some(texture) = &inner.intermediate else {
            return Ok(());
        };

        // Clamp the same way as in capture_framebuffer().
        let output_rect = Rectangle::from_size(frame.output_size());
        let clamped_dst = match dst.intersection(output_rect) {
            Some(clamped) => clamped,
            None => return Ok(()),
        };
        let clamp_offset = clamped_dst.loc - dst.loc;

        // Filter damage by subregion, reusing the stored Vec to avoid allocation.
        let filtered = &mut inner.subregion_damage;
        filtered.clear();

        if let Some(subregion) = &self.subregion {
            // Convert to subregion coordinates.
            let mut crop = src.to_logical(1., Transform::Normal, &src.size);
            crop.loc += self.geometry.loc;
            subregion.filter_damage(crop, dst, damage, filtered);
        } else {
            filtered.extend(damage.iter());
        };

        // Adjust for clamped dst.
        if clamped_dst != dst {
            let r = Rectangle::new(clamp_offset, clamped_dst.size);
            filtered.retain_mut(|d| {
                if let Some(mut crop) = d.intersection(r) {
                    crop.loc -= clamp_offset;
                    *d = crop;
                    true
                } else {
                    false
                }
            });
        }

        if filtered.is_empty() {
            return Ok(());
        }
        let damage = &filtered[..];

        // Adjust src proportionally to the dst clamping.
        let src_loc = src.loc.to_logical(1., Transform::Normal, &src.size);
        let dst_to_src = src.size / dst.size.to_f64();
        let crop = Rectangle::new(
            src_loc + clamp_offset.to_f64().upscale(dst_to_src).to_logical(1.),
            clamped_dst.size.to_f64().upscale(dst_to_src).to_logical(1.),
        );

        let uniforms = inner
            .program
            .is_some()
            .then(|| self.compute_uniforms(crop, frame.transformation()));
        let uniforms = uniforms.as_ref().map_or(&[][..], |x| &x[..]);

        frame.render_texture_from_to(
            texture,
            Rectangle::from_size(texture.size().to_f64()),
            clamped_dst,
            damage,
            &[],
            // The intermediate texture has the same transform as the frame.
            frame.transformation().invert(),
            1.,
            inner.program.as_ref(),
            uniforms,
        )
    }
}

impl<'render> RenderElement<TtyRenderer<'render>> for FramebufferEffectElement {
    fn capture_framebuffer(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
    ) -> Result<(), TtyRendererError<'render>> {
        let gles_frame = frame.as_gles_frame();
        RenderElement::<GlesRenderer>::capture_framebuffer(&self, gles_frame, src, dst)?;
        Ok(())
    }

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

impl Inner {
    fn new(renderer: &mut GlesRenderer, blur: Option<Blur>) -> Self {
        let program = Shaders::get(renderer).postprocess_and_clip.clone();

        Self {
            program,
            framebuffer: None,
            blur,
            intermediate: None,
            subregion_damage: Vec::new(),
        }
    }
}
