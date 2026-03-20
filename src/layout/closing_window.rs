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
use smithay::utils::{Logical, Point, Rectangle, Scale, Size, Transform};
use smithay::wayland::compositor::{Blocker, BlockerState};

use crate::animation::Animation;
use crate::niri_render_elements;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::shader_element::ShaderRenderElement;
use crate::render_helpers::shaders::{mat3_uniform, ProgramType, Shaders};
use crate::render_helpers::snapshot::RenderSnapshot;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::render_helpers::{render_to_encompassing_texture, RenderTarget};
use crate::utils::transaction::TransactionBlocker;

#[derive(Debug)]
pub struct ClosingWindow {
    /// Contents of the window.
    buffer: TextureBuffer<GlesTexture>,

    /// Blocked-out contents of the window.
    blocked_out_buffer: TextureBuffer<GlesTexture>,

    /// Where the window should be blocked out from.
    block_out_from: Option<BlockOutFrom>,

    /// Size of the window geometry.
    geo_size: Size<f64, Logical>,

    /// Position in the workspace.
    pos: Point<f64, Logical>,

    /// How much the texture should be offset.
    buffer_offset: Point<f64, Logical>,

    /// How much the blocked-out texture should be offset.
    blocked_out_buffer_offset: Point<f64, Logical>,

    /// The closing animation.
    anim_state: AnimationState,

    /// Random seed for the shader.
    random_seed: f32,
}

niri_render_elements! {
    ClosingWindowRenderElement => {
        Texture = RelocateRenderElement<RescaleRenderElement<PrimaryGpuTextureRenderElement>>,
        Shader = ShaderRenderElement,
    }
}

#[derive(Debug)]
enum AnimationState {
    Waiting {
        /// Blocker for a transaction before starting the animation.
        blocker: TransactionBlocker,
        anim: Animation,
    },
    Animating(Animation),
}

impl AnimationState {
    pub fn new(blocker: TransactionBlocker, anim: Animation) -> Self {
        if blocker.state() == BlockerState::Pending {
            Self::Waiting { blocker, anim }
        } else {
            // This actually doesn't normally happen because the window is removed only after the
            // closing animation is created. Though, it does happen with disable-transactions debug
            // flag.
            Self::Animating(anim)
        }
    }
}

impl ClosingWindow {
    pub fn new<E: RenderElement<GlesRenderer>>(
        renderer: &mut GlesRenderer,
        snapshot: RenderSnapshot<E, E>,
        scale: Scale<f64>,
        geo_size: Size<f64, Logical>,
        pos: Point<f64, Logical>,
        blocker: TransactionBlocker,
        anim: Animation,
    ) -> anyhow::Result<Self> {
        let _span = tracy_client::span!("ClosingWindow::new");

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
            render_to_texture(snapshot.contents).context("error rendering contents")?;
        let (blocked_out_buffer, blocked_out_buffer_offset) =
            render_to_texture(snapshot.blocked_out_contents)
                .context("error rendering blocked-out contents")?;

        Ok(Self {
            buffer,
            blocked_out_buffer,
            block_out_from: snapshot.block_out_from,
            geo_size,
            pos,
            buffer_offset,
            blocked_out_buffer_offset,
            anim_state: AnimationState::new(blocker, anim),
            random_seed: fastrand::f32(),
        })
    }

    pub fn advance_animations(&mut self) {
        match &mut self.anim_state {
            AnimationState::Waiting { blocker, anim } => {
                if blocker.state() != BlockerState::Pending {
                    let anim = anim.restarted(0., 1., 0.);
                    self.anim_state = AnimationState::Animating(anim);
                }
            }
            AnimationState::Animating(_anim) => (),
        }
    }

    pub fn are_animations_ongoing(&self) -> bool {
        match &self.anim_state {
            AnimationState::Waiting { .. } => true,
            AnimationState::Animating(anim) => !anim.is_done(),
        }
    }

    pub fn render(
        &self,
        renderer: &mut GlesRenderer,
        view_rect: Rectangle<f64, Logical>,
        scale: Scale<f64>,
        target: RenderTarget,
    ) -> ClosingWindowRenderElement {
        let (buffer, offset) = if target.should_block_out(self.block_out_from) {
            (&self.blocked_out_buffer, self.blocked_out_buffer_offset)
        } else {
            (&self.buffer, self.buffer_offset)
        };

        let anim = match &self.anim_state {
            AnimationState::Waiting { .. } => {
                let elem = TextureRenderElement::from_texture_buffer(
                    buffer.clone(),
                    Point::from((0., 0.)),
                    1.,
                    None,
                    None,
                    Kind::Unspecified,
                );

                let elem = PrimaryGpuTextureRenderElement(elem);
                let elem = RescaleRenderElement::from_element(elem, Point::from((0, 0)), 1.);

                let mut location = self.pos + offset;
                location.x -= view_rect.loc.x;
                let elem = RelocateRenderElement::from_element(
                    elem,
                    location.to_physical_precise_round(scale),
                    Relocate::Relative,
                );

                return elem.into();
            }
            AnimationState::Animating(anim) => anim,
        };

        let progress = anim.value();
        let clamped_progress = anim.clamped_value().clamp(0., 1.);

        if Shaders::get(renderer).program(ProgramType::Close).is_some() {
            let area_loc = Vec2::new(view_rect.loc.x as f32, view_rect.loc.y as f32);
            let area_size = Vec2::new(view_rect.size.w as f32, view_rect.size.h as f32);

            // Round to physical pixels relative to the view position. This is similar to what
            // happens when rendering normal windows.
            let relative = self.pos - view_rect.loc;
            let pos = view_rect.loc + relative.to_physical_precise_round(scale).to_logical(scale);

            let geo_loc = Vec2::new(pos.x as f32, pos.y as f32);
            let geo_size = Vec2::new(self.geo_size.w as f32, self.geo_size.h as f32);

            let input_to_geo = Mat3::from_scale(area_size / geo_size)
                * Mat3::from_translation((area_loc - geo_loc) / area_size);

            let tex_scale = self.buffer.texture_scale();
            let tex_scale = Vec2::new(tex_scale.x as f32, tex_scale.y as f32);
            let tex_loc = Vec2::new(offset.x as f32, offset.y as f32);
            let tex_size = self.buffer.texture().size();
            let tex_size = Vec2::new(tex_size.w as f32, tex_size.h as f32) / tex_scale;

            let geo_to_tex =
                Mat3::from_translation(-tex_loc / tex_size) * Mat3::from_scale(geo_size / tex_size);

            return ShaderRenderElement::new(
                ProgramType::Close,
                view_rect.size,
                None,
                scale.x as f32,
                1.,
                Rc::new([
                    mat3_uniform("niri_input_to_geo", input_to_geo),
                    Uniform::new("niri_geo_size", geo_size.to_array()),
                    mat3_uniform("niri_geo_to_tex", geo_to_tex),
                    Uniform::new("niri_progress", progress as f32),
                    Uniform::new("niri_clamped_progress", clamped_progress as f32),
                    Uniform::new("niri_random_seed", self.random_seed),
                ]),
                HashMap::from([(String::from("niri_tex"), buffer.texture().clone())]),
                Kind::Unspecified,
            )
            .with_location(Point::from((0., 0.)))
            .into();
        }

        let elem = TextureRenderElement::from_texture_buffer(
            buffer.clone(),
            Point::from((0., 0.)),
            1. - clamped_progress as f32,
            None,
            None,
            Kind::Unspecified,
        );

        let elem = PrimaryGpuTextureRenderElement(elem);

        let center = self.geo_size.to_point().downscale(2.);
        let elem = RescaleRenderElement::from_element(
            elem,
            (center - offset).to_physical_precise_round(scale),
            ((1. - clamped_progress) / 5. + 0.8).max(0.),
        );

        let mut location = self.pos + offset;
        location.x -= view_rect.loc.x;
        let elem = RelocateRenderElement::from_element(
            elem,
            location.to_physical_precise_round(scale),
            Relocate::Relative,
        );

        elem.into()
    }
}
