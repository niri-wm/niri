use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use glam::{Mat3, Vec2};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesTexture, Uniform};
use smithay::backend::renderer::Texture;
use smithay::utils::{Buffer, Logical, Point, Scale, Size, Transform};

use crate::animation::{Animation, Clock};
use crate::niri_render_elements;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::shader_element::ShaderRenderElement;
use crate::render_helpers::shaders::{mat3_uniform, ProgramType, Shaders};
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::render_helpers::RenderTarget;

pub const DELAY: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub struct ScreenTransition {
    /// Texture to crossfade from for each render target.
    from_texture: [TextureBuffer<GlesTexture>; 3],
    anim: Animation,
    /// Random seed for the shader.
    random_seed: f32,
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
        let mut anim = Animation::new(clock, 1., 0., 0., config);
        // Freeze the screen for `delay` before the animation starts.
        anim.delay(delay);
        Self {
            from_texture,
            anim,
            random_seed: fastrand::f32(),
        }
    }

    pub fn is_done(&self) -> bool {
        self.anim.is_done()
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
        let alpha = self.anim.value();
        let clamped_alpha = self.anim.clamped_value().clamp(0., 1.);

        let progress = 1. - alpha;
        let clamped_progress = (1. - self.anim.clamped_value()).clamp(0., 1.);

        let idx = match target {
            RenderTarget::Output => 0,
            RenderTarget::Screencast => 1,
            RenderTarget::ScreenCapture => 2,
        };

        let texture_scale = self.from_texture[idx].texture_scale();

        let frozen_frame_tex =
            PrimaryGpuTextureRenderElement(TextureRenderElement::from_texture_buffer(
                self.from_texture[idx].clone(),
                (0., 0.),
                clamped_alpha as f32,
                None,
                None,
                Kind::Unspecified,
            ))
            .into();

        if !self.anim.has_started() {
            return frozen_frame_tex;
        }

        if Shaders::get(renderer)
            .program(ProgramType::ScreenTransition)
            .is_some()
        {
            let mouse_pos = mouse_pos
                .map(|pos| [pos.x as f32, pos.y as f32])
                .unwrap_or([-1., -1.]);

            // The element is the full output, so the input already spans the geometry
            // in [0, 1], matching the close/open convention.
            let input_to_geo = Mat3::IDENTITY;
            let logical_size = self.from_texture[idx].logical_size();
            let geo_size = [logical_size.w as f32, logical_size.h as f32];

            // The snapshot carries the output transform, so resolve it when sampling,
            // mirroring smithay's `build_texture_mat` like the fallback below does.
            let transform = self.from_texture[idx].texture_transform();
            let tex_size = self.from_texture[idx].texture().size();
            let geo_to_tex = fullscreen_geo_to_tex(transform, tex_size);

            return ShaderRenderElement::new(
                ProgramType::ScreenTransition,
                self.from_texture[idx].logical_size(),
                None,
                texture_scale.x as f32,
                1.,
                Rc::new([
                    mat3_uniform("niri_input_to_geo", input_to_geo),
                    Uniform::new("niri_geo_size", geo_size),
                    mat3_uniform("niri_geo_to_tex", geo_to_tex),
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

        frozen_frame_tex
    }
}

/// Matrix converting normalized output-geometry coordinates to snapshot-texture UVs.
fn fullscreen_geo_to_tex(transform: Transform, tex_size: Size<i32, Buffer>) -> Mat3 {
    // Source size with the transform applied, in buffer pixels.
    let dst_src_size = transform.transform_size(tex_size);
    let dst = Vec2::new(dst_src_size.w as f32, dst_src_size.h as f32);
    let tex = Vec2::new(tex_size.w as f32, tex_size.h as f32);

    // NOTE: offsets copied from smithay's `build_texture_mat`; keep in sync.
    let offset = match transform {
        Transform::Normal => Vec2::ZERO,
        Transform::_90 => Vec2::new(0., dst.x),
        Transform::_180 => Vec2::new(dst.x, dst.y),
        Transform::_270 => Vec2::new(dst.y, 0.),
        Transform::Flipped => Vec2::new(dst.x, 0.),
        Transform::Flipped90 => Vec2::ZERO,
        Transform::Flipped180 => Vec2::new(0., dst.y),
        Transform::Flipped270 => Vec2::new(dst.y, dst.x),
    };

    let transform_mat = transform.matrix();
    let matrix = Mat3::from_cols(
        transform_mat.x_axis.extend(0.),
        transform_mat.y_axis.extend(0.),
        transform_mat.translation.extend(1.),
    );

    Mat3::from_scale(Vec2::new(1. / tex.x, 1. / tex.y))
        * Mat3::from_translation(offset)
        * matrix
        * Mat3::from_scale(dst)
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    fn sample(matrix: Mat3, x: f32, y: f32) -> (f32, f32) {
        let out = matrix * Vec3::new(x, y, 1.);
        (out.x, out.y)
    }

    fn assert_approx(left: (f32, f32), right: (f32, f32)) {
        assert!(
            (left.0 - right.0).abs() < 1e-5 && (left.1 - right.1).abs() < 1e-5,
            "{left:?} != {right:?}"
        );
    }

    #[test]
    fn geo_to_tex_normal_is_identity() {
        let tex = Size::<i32, Buffer>::from((1920, 1080));
        let matrix = fullscreen_geo_to_tex(Transform::Normal, tex);
        assert_approx(sample(matrix, 0., 0.), (0., 0.));
        assert_approx(sample(matrix, 1., 0.), (1., 0.));
        assert_approx(sample(matrix, 0., 1.), (0., 1.));
        assert_approx(sample(matrix, 1., 1.), (1., 1.));
    }

    #[test]
    fn geo_to_tex_180_flips_both_axes() {
        let tex = Size::<i32, Buffer>::from((1920, 1080));
        let matrix = fullscreen_geo_to_tex(Transform::_180, tex);
        assert_approx(sample(matrix, 0., 0.), (1., 1.));
        assert_approx(sample(matrix, 1., 1.), (0., 0.));
    }

    #[test]
    fn geo_to_tex_90_known_mapping() {
        let tex = Size::<i32, Buffer>::from((1920, 1080));
        let matrix = fullscreen_geo_to_tex(Transform::_90, tex);
        assert_approx(sample(matrix, 0., 0.), (0., 1.));
        assert_approx(sample(matrix, 1., 1.), (1., 0.));
    }

    /// Every transform must permute the unit square onto itself; otherwise sampling
    /// would read outside the snapshot texture on rotated outputs.
    #[test]
    fn geo_to_tex_keeps_corners_in_unit_square() {
        let corners = [(0., 0.), (1., 0.), (0., 1.), (1., 1.)];
        let transforms = [
            Transform::Normal,
            Transform::_90,
            Transform::_180,
            Transform::_270,
            Transform::Flipped,
            Transform::Flipped90,
            Transform::Flipped180,
            Transform::Flipped270,
        ];
        for transform in transforms {
            for size in [(1920, 1080), (1080, 1080)] {
                let tex = Size::<i32, Buffer>::from(size);
                let matrix = fullscreen_geo_to_tex(transform, tex);
                let mut mapped: Vec<_> = corners
                    .iter()
                    .map(|&(x, y)| {
                        let (x, y) = sample(matrix, x, y);
                        (x.round() as i32, y.round() as i32)
                    })
                    .collect();
                mapped.sort_unstable();
                assert_eq!(mapped, [(0, 0), (0, 1), (1, 0), (1, 1)], "{transform:?}");
            }
        }
    }
}
