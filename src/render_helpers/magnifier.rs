use std::cell::RefCell;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::{Element, Id, RenderElement};
use smithay::backend::renderer::gles::{ffi, GlesError, GlesFrame, GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{Frame as _, FrameContext, Offscreen, Texture as _};
use smithay::gpu_span_location;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Transform};

use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
use crate::render_helpers::renderer::AsGlesFrame as _;

/// A whole-output live magnifier effect: crops a small area of the already-composited
/// framebuffer around the cursor and blits it back scaled up to fill the entire output.
///
/// Modeled on [`super::framebuffer_effect::FramebufferEffect`]'s `capture_framebuffer` trick
/// (grab whatever was already drawn to the output before this element runs), but much
/// simpler: no blur/shader postprocessing, and the captured source rectangle is decoupled
/// from the element's own draw destination (framebuffer_effect always captures from `dst`
/// itself, i.e. draws in place; we deliberately capture from a *different*, smaller,
/// cursor-centered rectangle and let a single `glBlitFramebuffer` do the crop and the
/// upscale together).
#[derive(Debug)]
pub struct Magnifier {
    id: Id,
    commit: CommitCounter,
}

#[derive(Debug)]
pub struct MagnifierElement {
    id: Id,
    commit: CommitCounter,
    /// This element's own draw destination, in output-local logical coordinates: always the
    /// whole output.
    geometry: Rectangle<f64, Logical>,
    /// Cursor position, in output-local physical coordinates, at the time this element was
    /// built. Precomputed by the caller (which already has the output scale to hand) rather
    /// than recomputed here, since `capture_framebuffer`/`draw` only see physical space.
    pointer_pos: Point<i32, Physical>,
    zoom: f64,
}

#[derive(Debug)]
struct Inner {
    intermediate: Option<GlesTexture>,
}

impl Magnifier {
    pub fn new() -> Self {
        Self {
            id: Id::new(),
            commit: CommitCounter::default(),
        }
    }

    /// Marks the effect as changed, forcing a fresh capture on the next render. Call this
    /// every frame while the magnifier is active: the cursor moving (or the content behind
    /// it changing) both require a fresh crop of the framebuffer, and this element has no
    /// other way to know that happened.
    pub fn damage(&mut self) {
        self.commit.increment();
    }

    pub fn render(
        &self,
        geometry: Rectangle<f64, Logical>,
        pointer_pos: Point<i32, Physical>,
        zoom: f64,
    ) -> MagnifierElement {
        MagnifierElement {
            id: self.id.clone(),
            commit: self.commit,
            geometry,
            pointer_pos,
            zoom,
        }
    }
}

impl Default for Magnifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Element for MagnifierElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
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

impl RenderElement<GlesRenderer> for MagnifierElement {
    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        let _span = tracy_client::span!("MagnifierElement::capture_framebuffer");
        let location = gpu_span_location!("MagnifierElement::capture_framebuffer");
        frame.with_gpu_span(location, |frame| {
            let output_rect = Rectangle::from_size(frame.output_size());
            let transform = frame.transformation();

            let mut guard = frame.renderer();

            let inner = cache.get_or_insert::<RefCell<Inner>, _>(|| RefCell::new(Inner::new()));
            let mut inner = inner.borrow_mut();
            let inner = &mut *inner;

            inner.intermediate = None;

            // The area of the framebuffer we actually want to sample: a small box centered
            // on the cursor, sized so that blitting it up to the destination (the whole
            // output) gives the requested zoom factor. Clamp so it never reads outside the
            // output (the cursor near an edge shifts the box instead of letting it go out of
            // bounds).
            let zoom = self.zoom.max(1.);
            let src_w = ((dst.size.w as f64) / zoom).round().max(1.) as i32;
            let src_h = ((dst.size.h as f64) / zoom).round().max(1.) as i32;
            let max_x = (output_rect.size.w - src_w).max(0);
            let max_y = (output_rect.size.h - src_h).max(0);
            let src_x = (self.pointer_pos.x - src_w / 2).clamp(0, max_x);
            let src_y = (self.pointer_pos.y - src_h / 2).clamp(0, max_y);
            let capture_rect = Rectangle::new((src_x, src_y).into(), (src_w, src_h).into());
            // Account for output transform/rotation the same way framebuffer_effect.rs does,
            // since these are raw framebuffer pixel coordinates for the GL blit below.
            let capture_rect = transform.transform_rect_in(capture_rect, &output_rect.size);

            // The intermediate texture is sized to match `dst`, so capturing straight into it
            // is what performs the crop-and-upscale in a single blit. `capture_rect` above was
            // transformed into raw (rotated) framebuffer pixel space, so the texture we blit
            // it into needs to be sized to match that same space (swapping width and height
            // under a 90°/270° output transform), or the blit stretches the image unevenly.
            let tex_size = transform.transform_size(dst.size);

            let renderer = guard.as_mut();
            let texture: GlesTexture = renderer.create_buffer(
                Fourcc::Abgr8888,
                tex_size.to_logical(1).to_buffer(1, Transform::Normal),
            )?;

            drop(guard);

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
                    texture.tex_id(),
                    0,
                );

                gl.BlitFramebuffer(
                    capture_rect.loc.x,
                    capture_rect.loc.y,
                    capture_rect.loc.x + capture_rect.size.w,
                    capture_rect.loc.y + capture_rect.size.h,
                    0,
                    0,
                    tex_size.w,
                    tex_size.h,
                    ffi::COLOR_BUFFER_BIT,
                    ffi::LINEAR,
                );

                gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, current_fbo as u32);
                gl.Enable(ffi::SCISSOR_TEST);

                gl.DeleteFramebuffers(1, &mut fbo as *mut _);

                if gl.GetError() != ffi::NO_ERROR {
                    Err(GlesError::BlitError)
                } else {
                    Ok(())
                }
            })??;

            inner.intermediate = Some(texture);

            Ok(())
        })
    }

    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let Some(cache) = cache else {
            return Ok(());
        };
        let Some(inner) = cache.get::<RefCell<Inner>>() else {
            return Ok(());
        };
        let inner = inner.borrow();

        let Some(texture) = &inner.intermediate else {
            return Ok(());
        };

        // The intermediate texture was already captured at exactly `dst`'s size, so this is
        // a plain 1:1 blit — no cropping needed, unlike framebuffer_effect.rs.
        frame.render_texture_from_to(
            texture,
            Rectangle::from_size(texture.size().to_f64()),
            dst,
            damage,
            opaque_regions,
            frame.transformation().invert(),
            1.,
            None,
            &[],
        )
    }
}

impl<'render> RenderElement<TtyRenderer<'render>> for MagnifierElement {
    fn capture_framebuffer(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), TtyRendererError<'render>> {
        let gles_frame = frame.as_gles_frame();
        RenderElement::<GlesRenderer>::capture_framebuffer(&self, gles_frame, src, dst, cache)?;
        Ok(())
    }

    fn draw(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), TtyRendererError<'render>> {
        let gles_frame = frame.as_gles_frame();
        RenderElement::<GlesRenderer>::draw(
            &self,
            gles_frame,
            src,
            dst,
            damage,
            opaque_regions,
            cache,
        )?;
        Ok(())
    }
}

impl Inner {
    fn new() -> Self {
        Inner { intermediate: None }
    }
}
