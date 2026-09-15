use smithay::backend::renderer::element::utils::{
    Relocate, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::{Kind, RenderElement};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Scale, Size};
use smithay::wayland::compositor::{Blocker, BlockerState};

use crate::animation::Animation;
use crate::niri_render_elements;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::shader_element::{ShaderProgram, ShaderRenderElement};
use crate::render_helpers::shaders::{layer_close_program_for_source, ProgramType, Shaders};
use crate::render_helpers::snapshot::{
    render_close_fallback, render_close_shader, BakedSnapshot, RenderSnapshot,
};
use crate::render_helpers::texture::TextureRenderElement;
use crate::render_helpers::RenderCtx;
use crate::utils::transaction::TransactionBlocker;

#[derive(Debug)]
pub struct ClosingWindow {
    /// Baked snapshot textures.
    snapshot: BakedSnapshot,

    /// Size of the window geometry.
    geo_size: Size<f64, Logical>,

    /// Position in the workspace.
    pos: Point<f64, Logical>,

    /// The closing animation.
    anim_state: AnimationState,

    /// Random seed for the shader.
    random_seed: f32,

    /// Optional custom shader source from window rules.
    custom_shader: Option<String>,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new<E: RenderElement<GlesRenderer>>(
        renderer: &mut GlesRenderer,
        snapshot: RenderSnapshot<E, E>,
        scale: Scale<f64>,
        geo_size: Size<f64, Logical>,
        pos: Point<f64, Logical>,
        blocker: TransactionBlocker,
        anim: Animation,
        custom_shader: Option<String>,
    ) -> anyhow::Result<Self> {
        let _span = tracy_client::span!("ClosingWindow::new");

        let snapshot = snapshot.bake(renderer, scale)?;

        Ok(Self {
            snapshot,
            geo_size,
            pos,
            anim_state: AnimationState::new(blocker, anim),
            random_seed: fastrand::f32(),
            custom_shader,
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
        ctx: RenderCtx<GlesRenderer>,
        view_rect: Rectangle<f64, Logical>,
        scale: Scale<f64>,
    ) -> ClosingWindowRenderElement {
        let (buffer, offset) = self.snapshot.pick_buffer(ctx.target);

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

        if let Some(shader) = self.resolve_shader(ctx.renderer) {
            // ClosingWindow uses the normal buffer for tex coord calculation (not the picked
            // buffer) because the blocked-out buffer may have different dimensions.
            let elem = render_close_shader(
                shader,
                buffer,
                &self.snapshot.buffer,
                offset,
                self.geo_size,
                self.pos,
                progress as f32,
                clamped_progress as f32,
                self.random_seed,
                view_rect,
                scale,
            );
            return elem.into();
        }

        render_close_fallback(
            buffer,
            offset,
            self.geo_size,
            self.pos,
            clamped_progress,
            view_rect,
            scale,
        )
        .into()
    }

    fn resolve_shader(&self, renderer: &mut GlesRenderer) -> Option<ShaderProgram> {
        if let Some(src) = self.custom_shader.as_deref() {
            return layer_close_program_for_source(renderer, src);
        }

        Shaders::get(renderer).program(ProgramType::LayerClose)
    }
}
