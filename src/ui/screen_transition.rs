use std::time::Duration;

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesTexture;
use smithay::utils::{Scale, Transform};

use crate::animation::Clock;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::render_helpers::RenderTarget;

pub const DELAY: Duration = Duration::from_millis(250);
pub const DURATION: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub struct ScreenTransition {
    /// Texture to crossfade from for each render target.
    from_texture: [TextureBuffer<GlesTexture>; 3],
    /// Monotonic time when to start the crossfade.
    start_at: Duration,
    /// Clock to drive animations.
    clock: Clock,
}

impl ScreenTransition {
    pub fn new(
        from_texture: [TextureBuffer<GlesTexture>; 3],
        delay: Duration,
        clock: Clock,
    ) -> Self {
        Self {
            from_texture,
            start_at: clock.now_unadjusted() + delay,
            clock,
        }
    }

    pub fn is_done(&self) -> bool {
        self.start_at + DURATION <= self.clock.now_unadjusted()
    }

    pub fn update_render_elements(&mut self, scale: Scale<f64>, transform: Transform) {
        // These textures should remain full-screen, even if scale or transform changes.
        for buffer in &mut self.from_texture {
            buffer.set_texture_scale(scale);
            buffer.set_texture_transform(transform);
        }
    }

    pub fn render(&self, target: RenderTarget) -> PrimaryGpuTextureRenderElement {
        // Screen transition ignores animation slowdown.
        let now = self.clock.now_unadjusted();

        let alpha = if self.start_at + DURATION <= now {
            0.
        } else if self.start_at <= now {
            1. - (now - self.start_at).as_secs_f32() / DURATION.as_secs_f32()
        } else {
            1.
        };

        let idx = match target {
            RenderTarget::Output => 0,
            RenderTarget::Screencast => 1,
            RenderTarget::ScreenCapture => 2,
        };

        PrimaryGpuTextureRenderElement(TextureRenderElement::from_texture_buffer(
            self.from_texture[idx].clone(),
            (0., 0.),
            alpha,
            None,
            None,
            Kind::Unspecified,
        ))
    }
}
