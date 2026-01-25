use std::cell::RefCell;
use std::cmp::max;
use std::iter::{once, zip};
use std::rc::Rc;

use anyhow::{ensure, Context as _};
use glam::Mat3;
use niri_config::CornerRadius;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::{Element, Id, RenderElement};
use smithay::backend::renderer::gles::{
    ffi, link_program, GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform,
};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{
    ContextId, Frame as _, Offscreen as _, Renderer as _, Texture as _,
};
use smithay::gpu_span_location;
use smithay::utils::{Buffer, Logical, Physical, Rectangle, Scale, Size, Transform};

use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
use crate::render_helpers::background_effect::{Options, RenderParams};
use crate::render_helpers::renderer::AsGlesFrame as _;
use crate::render_helpers::shaders::{mat3_uniform, Shaders};

#[derive(Debug)]
pub struct Blur {
    program: BlurProgram,
    /// Context ID of the renderer that created the program and the textures.
    renderer_context_id: ContextId<GlesTexture>,
    /// Output texture followed by intermediate textures, large to small.
    ///
    /// Created lazily and stored here to avoid recreating blur textures frequently.
    textures: Vec<GlesTexture>,
    /// Config to use for rendering.
    config: niri_config::Blur,
}

#[derive(Debug, Clone)]
pub struct BlurElement {
    id: Id,
    commit: CommitCounter,
    geometry: Rectangle<f64, Logical>,
    clip_geo: Rectangle<f64, Logical>,
    corner_radius: CornerRadius,
    scale: f32,
    config: niri_config::Blur,
    inner: Rc<RefCell<Option<BlurElementInner>>>,
}

#[derive(Debug)]
struct BlurElementInner {
    program: Option<GlesTexProgram>,
    framebuffer: GlesTexture,
    blur: Blur,
    blurred: Option<GlesTexture>,
}

#[derive(Debug, Clone)]
pub struct BlurProgram(Rc<BlurProgramInner>);

#[derive(Debug)]
struct BlurProgramInner {
    down: BlurProgramInternal,
    up: BlurProgramInternal,
}

#[derive(Debug)]
struct BlurProgramInternal {
    program: ffi::types::GLuint,
    uniform_tex: ffi::types::GLint,
    uniform_half_pixel: ffi::types::GLint,
    uniform_offset: ffi::types::GLint,
    attrib_vert: ffi::types::GLint,
}

impl BlurElement {
    pub fn new() -> Self {
        Self {
            id: Id::new(),
            commit: CommitCounter::default(),
            geometry: Rectangle::zero(),
            clip_geo: Rectangle::zero(),
            corner_radius: CornerRadius::default(),
            scale: 1.,
            config: niri_config::Blur::default(),
            inner: Rc::new(RefCell::new(None)),
        }
    }

    pub fn update_config(&mut self, config: niri_config::Blur) {
        if self.config == config {
            return;
        }

        self.config = config;

        let mut inner = self.inner.borrow_mut();
        if let Some(inner) = &mut *inner {
            inner.blurred = None;
        }

        self.commit.increment();
    }

    pub fn render(
        &self,
        renderer: &mut GlesRenderer,
        options: Options,
        params: RenderParams,
    ) -> Option<Self> {
        // TODO: expand geo by blur offset?
        let mut this = self.clone();
        this.config.off |= !options.blur;
        if this.config.off || options.noise.is_some() {
            this.config.noise = options.noise.unwrap_or(0.);
        }
        if this.config.off || options.saturation.is_some() {
            this.config.saturation = options.saturation.unwrap_or(1.5);
        }
        this.geometry = params.geometry;
        this.scale = params.scale as f32;

        let (clip_geo, corner_radius) = params
            .clip
            .unwrap_or((params.geometry, CornerRadius::default()));
        this.clip_geo = clip_geo;
        this.corner_radius = corner_radius;

        let size = this.geometry.size.to_physical_precise_round(params.scale);

        {
            let mut inner = this.inner.borrow_mut();

            if let Some(x) = &mut *inner {
                if x.framebuffer.size() != size.to_logical(1).to_buffer(1, Transform::Normal) {
                    info!("recreating inner due to size mismatch");
                    *inner = None;
                }
            }

            let inner = if let Some(inner) = &mut *inner {
                inner
            } else {
                let Some(blur) = Blur::new(renderer) else {
                    // Missing blur shader.
                    return None;
                };
                let x = match BlurElementInner::new(renderer, blur, size) {
                    Ok(x) => x,
                    Err(err) => {
                        warn!("error creating blur element: {err:?}");
                        return None;
                    }
                };
                info!("created blur inner");
                inner.insert(x)
            };

            if inner.blurred.is_none() {
                if let Err(err) =
                    inner
                        .blur
                        .prepare_textures(renderer, &inner.framebuffer, self.config)
                {
                    warn!("error preparing blur textures: {err:?}");
                    return None;
                }
            }
        }

        Some(this)
    }

    fn compute_uniforms(&self) -> [Uniform<'static>; 7] {
        // TODO when geometry doesn't match clip_geo
        let input_to_geo = Mat3::IDENTITY;

        let clip_geo_size = (self.clip_geo.size.w as f32, self.clip_geo.size.h as f32);

        [
            Uniform::new("niri_scale", self.scale),
            Uniform::new("geo_size", clip_geo_size),
            Uniform::new("corner_radius", <[f32; 4]>::from(self.corner_radius)),
            mat3_uniform("input_to_geo", input_to_geo),
            Uniform::new("noise", self.config.noise as f32),
            Uniform::new("saturation", self.config.saturation as f32),
            Uniform::new("bg_color", [0f32, 0., 0., 0.]),
        ]
    }
}

impl BlurElementInner {
    fn new(
        renderer: &mut GlesRenderer,
        blur: Blur,
        size: Size<i32, Physical>,
    ) -> anyhow::Result<Self> {
        let program = Shaders::get(renderer).postprocess_and_clip.clone();

        let size = size.to_logical(1).to_buffer(1, Transform::Normal);
        let framebuffer: GlesTexture = renderer
            .create_buffer(Fourcc::Abgr8888, size)
            .context("error creating texture")?;

        Ok(Self {
            program,
            framebuffer,
            blur,
            blurred: None,
        })
    }
}

unsafe fn compile_program(gl: &ffi::Gles2, src: &str) -> Result<BlurProgramInternal, GlesError> {
    let program = unsafe { link_program(gl, include_str!("shaders/blur.vert"), src)? };

    let vert = c"vert";
    let tex = c"tex";
    let half_pixel = c"half_pixel";
    let offset = c"offset";

    Ok(BlurProgramInternal {
        program,
        uniform_tex: gl.GetUniformLocation(program, tex.as_ptr()),
        uniform_half_pixel: gl.GetUniformLocation(program, half_pixel.as_ptr()),
        uniform_offset: gl.GetUniformLocation(program, offset.as_ptr()),
        attrib_vert: gl.GetAttribLocation(program, vert.as_ptr()),
    })
}

impl BlurProgram {
    pub fn compile(renderer: &mut GlesRenderer) -> anyhow::Result<Self> {
        renderer
            .with_context(move |gl| unsafe {
                let down = compile_program(gl, include_str!("shaders/blur_down.frag"))
                    .context("error compiling blur_down shader")?;
                let up = compile_program(gl, include_str!("shaders/blur_up.frag"))
                    .context("error compiling blur_up shader")?;
                Ok(Self(Rc::new(BlurProgramInner { down, up })))
            })
            .context("error making GL context current")?
    }

    pub fn destroy(self, renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        renderer.with_context(move |gl| unsafe {
            gl.DeleteProgram(self.0.down.program);
            gl.DeleteProgram(self.0.up.program);
        })
    }
}

impl Blur {
    pub fn new(renderer: &mut GlesRenderer) -> Option<Self> {
        let program = Shaders::get(renderer).blur.clone()?;
        Some(Self {
            program,
            renderer_context_id: renderer.context_id(),
            textures: Vec::new(),
            config: niri_config::Blur::default(),
        })
    }

    pub fn context_id(&self) -> ContextId<GlesTexture> {
        self.renderer_context_id.clone()
    }

    pub fn prepare_textures(
        &mut self,
        renderer: &mut GlesRenderer,
        source: &GlesTexture,
        config: niri_config::Blur,
    ) -> anyhow::Result<()> {
        let _span = tracy_client::span!("Blur::prepare_textures");

        ensure!(
            renderer.context_id() == self.renderer_context_id,
            "wrong renderer"
        );

        self.config = config;

        let passes = config.passes.clamp(1, 31) as usize;
        let size = source.size();

        if let Some(output) = self.textures.first_mut() {
            let old_size = output.size();
            if old_size != size {
                debug!(
                    "recreating textures: output size changed from {} × {} to {} × {}",
                    old_size.w, old_size.h, size.w, size.h
                );
                self.textures.clear();
            } else if !output.is_unique_reference() {
                debug!("recreating textures: not unique",);
                // We only need to recreate the output texture here, but this case shouldn't really
                // happen anyway, and this is simpler.
                self.textures.clear();
            }
        }

        // Create any missing textures.
        let mut w = size.w;
        let mut h = size.h;
        for i in 0..=passes {
            let size = Size::new(w, h);
            w = max(1, w / 2);
            h = max(1, h / 2);

            if self.textures.len() > i {
                // This texture already exists.
                continue;
            }

            // debug!("creating texture for step {i} sized {w} × {h}");

            let texture: GlesTexture = renderer
                .create_buffer(Fourcc::Abgr8888, size)
                .context("error creating texture")?;
            self.textures.push(texture);
        }

        // Drop any no longer needed textures.
        self.textures.drain(passes + 1..);

        Ok(())
    }

    pub fn render(
        &mut self,
        frame: &mut GlesFrame,
        source: &GlesTexture,
        config: niri_config::Blur,
    ) -> anyhow::Result<GlesTexture> {
        let _span = tracy_client::span!("Blur::render");
        trace!("rendering blur");

        ensure!(
            frame.context_id() == self.renderer_context_id,
            "wrong renderer"
        );

        let passes = config.passes.clamp(1, 31) as usize;
        let size = source.size();

        ensure!(
            self.textures.len() == passes + 1,
            "wrong textures len: expected {}, got {}",
            passes + 1,
            self.textures.len()
        );

        let output = &mut self.textures[0];
        ensure!(
            output.size() == size,
            "wrong output texture size: expected {size:?}, got {:?}",
            output.size()
        );

        ensure!(
            output.is_unique_reference(),
            "output texture has a non-unique reference"
        );

        frame.with_profiled_context(gpu_span_location!("Blur::render"), |gl| unsafe {
            while gl.GetError() != ffi::NO_ERROR {}

            let mut current_fbo = 0i32;
            let mut viewport = [0i32; 4];
            gl.GetIntegerv(ffi::FRAMEBUFFER_BINDING, &mut current_fbo as *mut _);
            gl.GetIntegerv(ffi::VIEWPORT, viewport.as_mut_ptr());

            gl.Disable(ffi::BLEND);
            gl.Disable(ffi::SCISSOR_TEST);

            gl.ActiveTexture(ffi::TEXTURE0);

            let mut fbos = [0; 2];
            gl.GenFramebuffers(fbos.len() as _, fbos.as_mut_ptr());
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, fbos[0]);

            let program = &self.program.0.down;
            gl.UseProgram(program.program);
            gl.Uniform1i(program.uniform_tex, 0);
            gl.Uniform1f(program.uniform_offset, config.offset as f32);

            let vertices: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0];
            gl.EnableVertexAttribArray(program.attrib_vert as u32);
            gl.BindBuffer(ffi::ARRAY_BUFFER, 0);
            gl.VertexAttribPointer(
                program.attrib_vert as u32,
                2,
                ffi::FLOAT,
                ffi::FALSE,
                0,
                vertices.as_ptr().cast(),
            );

            let src = once(source).chain(&self.textures[1..]);
            let dst = &self.textures[1..];
            for (src, dst) in zip(src, dst) {
                let dst_size = dst.size();
                let w = dst_size.w;
                let h = dst_size.h;
                gl.Viewport(0, 0, w, h);

                // During downsampling, half_pixel is half of the destination pixel.
                gl.Uniform2f(program.uniform_half_pixel, 0.5 / w as f32, 0.5 / h as f32);

                let src = src.tex_id();
                let dst = dst.tex_id();

                trace!("drawing down {src} to {dst}");
                gl.FramebufferTexture2D(
                    ffi::DRAW_FRAMEBUFFER,
                    ffi::COLOR_ATTACHMENT0,
                    ffi::TEXTURE_2D,
                    dst,
                    0,
                );

                gl.BindTexture(ffi::TEXTURE_2D, src);
                gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
                gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
                gl.TexParameteri(
                    ffi::TEXTURE_2D,
                    ffi::TEXTURE_WRAP_S,
                    ffi::CLAMP_TO_EDGE as i32,
                );
                gl.TexParameteri(
                    ffi::TEXTURE_2D,
                    ffi::TEXTURE_WRAP_T,
                    ffi::CLAMP_TO_EDGE as i32,
                );

                gl.DrawArrays(ffi::TRIANGLES, 0, 6);
            }

            gl.DisableVertexAttribArray(program.attrib_vert as u32);

            // Up
            let program = &self.program.0.up;
            gl.UseProgram(program.program);
            gl.Uniform1i(program.uniform_tex, 0);
            gl.Uniform1f(program.uniform_offset, config.offset as f32);

            let vertices: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0];
            gl.EnableVertexAttribArray(program.attrib_vert as u32);
            gl.BindBuffer(ffi::ARRAY_BUFFER, 0);
            gl.VertexAttribPointer(
                program.attrib_vert as u32,
                2,
                ffi::FLOAT,
                ffi::FALSE,
                0,
                vertices.as_ptr().cast(),
            );

            let src = self.textures.iter().rev();
            let dst = self.textures.iter().rev().skip(1);
            for (src, dst) in zip(src, dst) {
                let dst_size = dst.size();
                let w = dst_size.w;
                let h = dst_size.h;
                gl.Viewport(0, 0, w, h);

                // During upsampling, half_pixel is half of the source pixel.
                let src_size = src.size();
                let src_w = src_size.w as f32;
                let src_h = src_size.h as f32;
                gl.Uniform2f(program.uniform_half_pixel, 0.5 / src_w, 0.5 / src_h);

                let src = src.tex_id();
                let dst = dst.tex_id();

                trace!("drawing up {src} to {dst}");
                gl.FramebufferTexture2D(
                    ffi::DRAW_FRAMEBUFFER,
                    ffi::COLOR_ATTACHMENT0,
                    ffi::TEXTURE_2D,
                    dst,
                    0,
                );

                gl.BindTexture(ffi::TEXTURE_2D, src);
                gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
                gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
                gl.TexParameteri(
                    ffi::TEXTURE_2D,
                    ffi::TEXTURE_WRAP_S,
                    ffi::CLAMP_TO_EDGE as i32,
                );
                gl.TexParameteri(
                    ffi::TEXTURE_2D,
                    ffi::TEXTURE_WRAP_T,
                    ffi::CLAMP_TO_EDGE as i32,
                );

                gl.DrawArrays(ffi::TRIANGLES, 0, 6);
            }

            gl.DisableVertexAttribArray(program.attrib_vert as u32);

            gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);
            gl.DeleteFramebuffers(fbos.len() as _, fbos.as_ptr());

            // Restore state set by GlesFrame that we just modified.
            gl.Enable(ffi::BLEND);
            gl.Enable(ffi::SCISSOR_TEST);
            gl.BindFramebuffer(ffi::FRAMEBUFFER, current_fbo as u32);
            gl.Viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
        })?;

        Ok(self.textures[0].clone())
    }
}

impl Element for BlurElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(Size::new(1., 1.))
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry.to_physical_precise_round(scale)
    }

    fn is_framebuffer_effect(&self) -> bool {
        true
    }
}

impl RenderElement<GlesRenderer> for BlurElement {
    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        // TODO: how can we handle cropping?
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
    ) -> Result<(), GlesError> {
        let mut inner = self.inner.borrow_mut();
        let Some(inner) = &mut *inner else {
            return Ok(());
        };
        let _span = tracy_client::span!("BlurElement::capture_framebuffer");

        inner.blurred = None;

        // info!("capturing framebuffer");
        // info!("dst before {dst:?}, transform {transform:?}");
        // let dst = transform.transform_rect_in(dst, &dst.size);
        // info!("dst after {dst:?}");

        let size = inner.framebuffer.size();
        if size.w != dst.size.w || size.h != dst.size.h {
            trace!(
                "size mismatch, blur will look wrong: size={size:?}, dst.size={:?}",
                dst.size
            );
        }

        let location = gpu_span_location!("BlurElement::capture_framebuffer");
        frame.with_gpu_span(location, |frame| {
            frame.with_context(|gl| unsafe {
                while gl.GetError() != ffi::NO_ERROR {}

                let mut current_fbo = 0i32;
                gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &mut current_fbo as *mut _);

                let mut fbo = 0;
                gl.GenFramebuffers(1, &mut fbo as *mut _);
                gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, fbo);

                gl.FramebufferTexture2D(
                    ffi::DRAW_FRAMEBUFFER,
                    ffi::COLOR_ATTACHMENT0,
                    ffi::TEXTURE_2D,
                    inner.framebuffer.tex_id(),
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

                gl.DeleteFramebuffers(1, &mut fbo as *mut _);

                if gl.GetError() != ffi::NO_ERROR {
                    Err(GlesError::BlitError)
                } else {
                    Ok(())
                }
            })??;

            if self.config.off {
                inner.blurred = Some(inner.framebuffer.clone());
                return Ok(());
            }

            match inner.blur.render(frame, &inner.framebuffer, self.config) {
                Ok(blurred) => inner.blurred = Some(blurred),
                Err(err) => {
                    warn!("error rendering blur: {err:?}");
                }
            }

            Ok(())
        })
    }

    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        // TODO: element getting cropped messes things up because dst no longer corresponds to the
        // actual dst, but rather is cropped.
        let inner = self.inner.borrow();
        let Some(inner) = &*inner else {
            return Ok(());
        };

        let Some(blurred) = &inner.blurred else {
            return Ok(());
        };
        let src = Rectangle::from_size(blurred.size()).to_f64();

        let uniforms = inner.program.is_some().then(|| self.compute_uniforms());
        let uniforms = uniforms.as_ref().map_or(&[][..], |x| &x[..]);
        // let uniforms = &[];

        frame.render_texture_from_to(
            blurred,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            1.,
            inner.program.as_ref(),
            // None,
            uniforms,
        )
    }
}

impl<'render> RenderElement<TtyRenderer<'render>> for BlurElement {
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
