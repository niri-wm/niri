use std::cell::OnceCell;
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Context as _;
use glam::{Mat3, Vec2};
use niri_config::BlockOutFrom;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::utils::{
    Relocate, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::{Kind, RenderElement};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture, Uniform};
use smithay::backend::renderer::Texture;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Size, Transform};

use super::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use super::shader_element::{ShaderProgram, ShaderRenderElement};
use super::shaders::mat3_uniform;
use super::texture::{TextureBuffer, TextureRenderElement};
use super::{render_to_encompassing_texture, ToRenderElement};
use crate::render_helpers::{RenderCtx, RenderTarget};

/// Snapshot of a render.
#[derive(Debug)]
pub struct RenderSnapshot<C, B> {
    /// Contents for a normal render.
    ///
    /// Relative to the geometry.
    pub contents: Vec<C>,

    /// Contents that are not blocked out, but the background is blocked out.
    ///
    /// If `None` then the background doesn't have any blocked-out surfaces, and normal `contents`
    /// can be used instead.
    pub contents_with_blocked_out_bg: Option<Vec<C>>,

    /// Blocked-out contents.
    ///
    /// Relative to the geometry.
    pub blocked_out_contents: Vec<B>,

    /// Where the contents were blocked out from at the time of the snapshot.
    pub block_out_from: Option<BlockOutFrom>,

    /// Visual size of the element at the point of the snapshot.
    pub size: Size<f64, Logical>,

    /// Contents rendered into a texture (lazily).
    pub texture: OnceCell<Option<(GlesTexture, Rectangle<i32, Physical>)>>,

    /// Contents with blocked-out bg rendered into a texture (lazily).
    pub texture_with_blocked_out_bg: OnceCell<Option<(GlesTexture, Rectangle<i32, Physical>)>>,

    /// Blocked-out contents rendered into a texture (lazily).
    pub blocked_out_texture: OnceCell<Option<(GlesTexture, Rectangle<i32, Physical>)>>,
}

/// A snapshot that has been baked into GPU texture buffers.
#[derive(Debug)]
pub struct BakedSnapshot {
    /// Contents of the element.
    pub buffer: TextureBuffer<GlesTexture>,

    /// Contents that are not blocked out, but the background is blocked out.
    pub buffer_with_blocked_out_bg: Option<TextureBuffer<GlesTexture>>,

    /// Blocked-out contents of the element.
    pub blocked_out_buffer: TextureBuffer<GlesTexture>,

    /// Where the element should be blocked out from.
    pub block_out_from: Option<BlockOutFrom>,

    /// Offset for the normal buffer.
    pub buffer_offset: Point<f64, Logical>,

    /// Offset for the contents-with-blocked-out-bg buffer.
    pub buffer_with_blocked_out_bg_offset: Point<f64, Logical>,

    /// Offset for the blocked-out buffer.
    pub blocked_out_buffer_offset: Point<f64, Logical>,

    /// Visual size of the element at the point of the snapshot.
    pub snapshot_size: Size<f64, Logical>,
}

impl<C, B, EC, EB> RenderSnapshot<C, B>
where
    C: ToRenderElement<RenderElement = EC>,
    B: ToRenderElement<RenderElement = EB>,
    EC: RenderElement<GlesRenderer>,
    EB: RenderElement<GlesRenderer>,
{
    pub fn texture(
        &self,
        ctx: RenderCtx<GlesRenderer>,
        scale: Scale<f64>,
    ) -> Option<&(GlesTexture, Rectangle<i32, Physical>)> {
        if ctx.target.should_block_out(self.block_out_from) {
            self.blocked_out_texture.get_or_init(|| {
                let _span = tracy_client::span!("RenderSnapshot::texture");

                let elements: Vec<_> = self
                    .blocked_out_contents
                    .iter()
                    .map(|baked| {
                        baked.to_render_element(Point::from((0., 0.)), scale, 1., Kind::Unspecified)
                    })
                    .collect();

                match render_to_encompassing_texture(
                    ctx.renderer,
                    scale,
                    Transform::Normal,
                    Fourcc::Abgr8888,
                    &elements,
                ) {
                    Ok((texture, _sync_point, geo)) => Some((texture, geo)),
                    Err(err) => {
                        warn!("error rendering blocked-out contents to texture: {err:?}");
                        None
                    }
                }
            })
        } else if ctx.target != RenderTarget::Output && self.contents_with_blocked_out_bg.is_some()
        {
            let contents = self.contents_with_blocked_out_bg.as_ref().unwrap();
            self.texture_with_blocked_out_bg.get_or_init(|| {
                let _span = tracy_client::span!("RenderSnapshot::texture");

                let elements: Vec<_> = contents
                    .iter()
                    .map(|baked| {
                        baked.to_render_element(Point::from((0., 0.)), scale, 1., Kind::Unspecified)
                    })
                    .collect();

                match render_to_encompassing_texture(
                    ctx.renderer,
                    scale,
                    Transform::Normal,
                    Fourcc::Abgr8888,
                    &elements,
                ) {
                    Ok((texture, _sync_point, geo)) => Some((texture, geo)),
                    Err(err) => {
                        warn!("error rendering contents with blocked-out bg to texture: {err:?}");
                        None
                    }
                }
            })
        } else {
            self.texture.get_or_init(|| {
                let _span = tracy_client::span!("RenderSnapshot::texture");

                let elements: Vec<_> = self
                    .contents
                    .iter()
                    .map(|baked| {
                        baked.to_render_element(Point::from((0., 0.)), scale, 1., Kind::Unspecified)
                    })
                    .collect();

                match render_to_encompassing_texture(
                    ctx.renderer,
                    scale,
                    Transform::Normal,
                    Fourcc::Abgr8888,
                    &elements,
                ) {
                    Ok((texture, _sync_point, geo)) => Some((texture, geo)),
                    Err(err) => {
                        warn!("error rendering contents to texture: {err:?}");
                        None
                    }
                }
            })
        }
        .as_ref()
    }
}

impl BakedSnapshot {
    /// Pick the appropriate buffer and offset for the given render target.
    pub fn pick_buffer(
        &self,
        target: RenderTarget,
    ) -> (&TextureBuffer<GlesTexture>, Point<f64, Logical>) {
        if target.should_block_out(self.block_out_from) {
            (&self.blocked_out_buffer, self.blocked_out_buffer_offset)
        } else if target != RenderTarget::Output && self.buffer_with_blocked_out_bg.is_some() {
            (
                self.buffer_with_blocked_out_bg.as_ref().unwrap(),
                self.buffer_with_blocked_out_bg_offset,
            )
        } else {
            (&self.buffer, self.buffer_offset)
        }
    }
}

impl<E: RenderElement<GlesRenderer>> RenderSnapshot<E, E> {
    /// Bake a `RenderSnapshot` into GPU texture buffers.
    pub fn bake(
        self,
        renderer: &mut GlesRenderer,
        scale: Scale<f64>,
    ) -> anyhow::Result<BakedSnapshot> {
        let _span = tracy_client::span!("RenderSnapshot::bake");

        let mut render_to_texture = |elements: Vec<E>| -> anyhow::Result<_> {
            let (texture, _sync_point, geo) = render_to_encompassing_texture(
                renderer,
                scale,
                Transform::Normal,
                Fourcc::Abgr8888,
                &elements,
            )
            .context("error rendering to texture")?;

            let buffer = TextureBuffer::from_texture(
                renderer,
                texture,
                scale,
                Transform::Normal,
                Vec::new(),
            );

            let offset = geo.loc.to_f64().to_logical(scale);

            Ok((buffer, offset))
        };

        let (buffer, buffer_offset) =
            render_to_texture(self.contents).context("error rendering contents")?;

        let (buffer_with_blocked_out_bg, buffer_with_blocked_out_bg_offset) =
            if let Some(contents) = self.contents_with_blocked_out_bg {
                let (buffer, offset) = render_to_texture(contents)
                    .context("error rendering contents with blocked-out bg")?;
                (Some(buffer), offset)
            } else {
                (None, Point::default())
            };

        let (blocked_out_buffer, blocked_out_buffer_offset) =
            render_to_texture(self.blocked_out_contents)
                .context("error rendering blocked-out contents")?;

        Ok(BakedSnapshot {
            buffer,
            buffer_with_blocked_out_bg,
            blocked_out_buffer,
            block_out_from: self.block_out_from,
            buffer_offset,
            buffer_with_blocked_out_bg_offset,
            blocked_out_buffer_offset,
            snapshot_size: self.size,
        })
    }
}

// Silence, Clippy.
// A Smithay agent is coding.
/// Render a close animation element using a shader.
///
/// `tex_calc_buffer` is the buffer used to compute texture coordinates (may differ from `buffer`
/// which provides the actual texture data for the shader).
#[allow(clippy::too_many_arguments)]
pub fn render_close_shader(
    shader: ShaderProgram,
    buffer: &TextureBuffer<GlesTexture>,
    tex_calc_buffer: &TextureBuffer<GlesTexture>,
    tex_offset: Point<f64, Logical>,
    geo_size: Size<f64, Logical>,
    pos: Point<f64, Logical>,
    progress: f32,
    clamped_progress: f32,
    random_seed: f32,
    view_rect: Rectangle<f64, Logical>,
    scale: Scale<f64>,
) -> ShaderRenderElement {
    let area_loc = Vec2::new(view_rect.loc.x as f32, view_rect.loc.y as f32);
    let area_size = Vec2::new(view_rect.size.w as f32, view_rect.size.h as f32);

    // Round to physical pixels relative to the view position. This is similar to what
    // happens when rendering normal windows.
    let relative = pos - view_rect.loc;
    let pos = view_rect.loc + relative.to_physical_precise_round(scale).to_logical(scale);

    let geo_loc = Vec2::new(pos.x as f32, pos.y as f32);
    let geo_size = Vec2::new(geo_size.w as f32, geo_size.h as f32);

    let input_to_geo = Mat3::from_scale(area_size / geo_size)
        * Mat3::from_translation((area_loc - geo_loc) / area_size);

    let tex_scale = tex_calc_buffer.texture_scale();
    let tex_scale = Vec2::new(tex_scale.x as f32, tex_scale.y as f32);
    let tex_loc = Vec2::new(tex_offset.x as f32, tex_offset.y as f32);
    let tex_size = tex_calc_buffer.texture().size();
    let tex_size = Vec2::new(tex_size.w as f32, tex_size.h as f32) / tex_scale;

    let geo_to_tex =
        Mat3::from_translation(-tex_loc / tex_size) * Mat3::from_scale(geo_size / tex_size);

    ShaderRenderElement::new_with_shader(
        shader,
        view_rect.size,
        None,
        scale.x as f32,
        1.,
        Rc::new([
            mat3_uniform("niri_input_to_geo", input_to_geo),
            Uniform::new("niri_geo_size", geo_size.to_array()),
            mat3_uniform("niri_geo_to_tex", geo_to_tex),
            Uniform::new("niri_progress", progress),
            Uniform::new("niri_clamped_progress", clamped_progress),
            Uniform::new("niri_random_seed", random_seed),
        ]),
        HashMap::from([(String::from("niri_tex"), buffer.texture().clone())]),
        Kind::Unspecified,
    )
    .with_location(Point::from((0., 0.)))
}

/// Render a close animation element using the fallback (non-shader) fade+scale path.
pub fn render_close_fallback(
    buffer: &TextureBuffer<GlesTexture>,
    offset: Point<f64, Logical>,
    geo_size: Size<f64, Logical>,
    pos: Point<f64, Logical>,
    clamped_progress: f64,
    view_rect: Rectangle<f64, Logical>,
    scale: Scale<f64>,
) -> RelocateRenderElement<RescaleRenderElement<PrimaryGpuTextureRenderElement>> {
    let elem = TextureRenderElement::from_texture_buffer(
        buffer.clone(),
        Point::from((0., 0.)),
        1. - clamped_progress as f32,
        None,
        None,
        Kind::Unspecified,
    );

    let elem = PrimaryGpuTextureRenderElement(elem);

    let center = geo_size.to_point().downscale(2.);
    let elem = RescaleRenderElement::from_element(
        elem,
        (center - offset).to_physical_precise_round(scale),
        ((1. - clamped_progress) / 5. + 0.8).max(0.),
    );

    let mut location = pos + offset;
    location.x -= view_rect.loc.x;
    RelocateRenderElement::from_element(
        elem,
        location.to_physical_precise_round(scale),
        Relocate::Relative,
    )
}
