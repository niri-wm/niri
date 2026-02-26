use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesTexture, Uniform};
use smithay::utils::{Logical, Point, Scale, Transform};

use crate::animation::{Animation, Clock};
use crate::niri_render_elements;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::shader_element::ShaderRenderElement;
use crate::render_helpers::shaders::{ProgramType, Shaders};
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::render_helpers::RenderTarget;

pub const DELAY: Duration = Duration::from_millis(250);
pub const DURATION: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub struct ScreenTransition {
    /// Texture to crossfade from for each render target.
    from_texture: [TextureBuffer<GlesTexture>; 3],
    delay: Duration,
    anim: Animation,
    /// Random seed for the shader.
    random_seed: f32,
    /// Clock to drive animations.
    clock: Clock,
}

niri_render_elements! {
    ScreenTransitionRenderElement => {
        Texture = PrimaryGpuTextureRenderElement,
        Shader = ShaderRenderElement,
    }
}

impl ScreenTransition {
    pub fn new(
        from_texture: [TextureBuffer<GlesTexture>; 3],
        delay: Duration,
        config: niri_config::Animation,
        clock: Clock,
    ) -> Self {
        let anim = Animation::new(clock.clone(), 1., 0., 0., config);
        Self {
            from_texture,
            delay,
            anim,
            random_seed: fastrand::f32(),
            clock,
        }
    }

    pub fn is_done(&self) -> bool {
        self.anim.end_time() <= self.clock.now().saturating_sub(self.delay)
    }

    pub fn update_render_elements(&mut self, scale: Scale<f64>, transform: Transform) {
        // These textures should remain full-screen, even if scale or transform changes.
        for buffer in &mut self.from_texture {
            buffer.set_texture_scale(scale);
            buffer.set_texture_transform(transform);
        }
    }

    pub fn render(
        &self,
        renderer: &mut impl NiriRenderer,
        target: RenderTarget,
        mouse_pos: Option<Point<f64, Logical>>,
    ) -> ScreenTransitionRenderElement {
        let now = self.clock.now().saturating_sub(self.delay);

        let alpha = self.anim.value_at(now);
        let clamped_alpha = alpha.clamp(0., 1.);

        let progress = 1. - alpha;
        let clamped_progress = progress.clamp(0., 1.);

        let idx = match target {
            RenderTarget::Output => 0,
            RenderTarget::Screencast => 1,
            RenderTarget::ScreenCapture => 2,
        };

        let texture_scale = self.from_texture[idx].texture_scale();

        if Shaders::get(renderer)
            .program(ProgramType::ScreenTransition)
            .is_some()
        {
            let mouse_pos = mouse_pos
                .map(|pos| [pos.x as f32, pos.y as f32])
                .unwrap_or([-1., -1.]);

            return ShaderRenderElement::new(
                ProgramType::ScreenTransition,
                self.from_texture[idx].logical_size(),
                None,
                texture_scale.x as f32,
                1.,
                Rc::new([
                    Uniform::new("niri_progress", progress as f32),
                    Uniform::new("niri_clamped_progress", clamped_progress as f32),
                    Uniform::new("niri_mouse_pos", mouse_pos),
                    Uniform::new("niri_random_seed", self.random_seed),
                ]),
                HashMap::from([(
                    String::from("niri_tex_from"),
                    self.from_texture[idx].texture().clone(),
                )]),
                Kind::Unspecified,
            )
            .with_location(Point::from((0., 0.)))
            .into();
        }

        PrimaryGpuTextureRenderElement(TextureRenderElement::from_texture_buffer(
            self.from_texture[idx].clone(),
            (0., 0.),
            clamped_alpha as f32,
            None,
            None,
            Kind::Unspecified,
        ))
        .into()
    }
}
