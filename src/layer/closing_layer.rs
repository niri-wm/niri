use smithay::backend::renderer::element::utils::{RelocateRenderElement, RescaleRenderElement};
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::Texture;
use smithay::desktop::LayerSurface;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle, Scale, Size};
use smithay::wayland::shell::wlr_layer::Layer;

use crate::animation::Animation;
use crate::niri_render_elements;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::shader_element::{ShaderProgram, ShaderRenderElement};
use crate::render_helpers::shaders::{layer_close_program_for_source, ProgramType, Shaders};
use crate::render_helpers::snapshot::{
    render_close_fallback, render_close_shader, BakedSnapshot, RenderSnapshot,
};
use crate::render_helpers::RenderTarget;

#[derive(Debug)]
pub struct ClosingLayer {
    /// The output this layer is on.
    output: Output,

    /// The layer surface.
    surface: LayerSurface,

    /// The layer level (background, bottom, top, overlay).
    layer: Layer,

    /// Whether this is a backdrop animation.
    for_backdrop: bool,

    /// Baked snapshot textures.
    snapshot: BakedSnapshot,

    /// Size of the layer geometry.
    geo_size: Size<f64, Logical>,

    /// Position in the workspace.
    pos: Point<f64, Logical>,

    /// The closing animation.
    anim: Animation,

    /// Program type for shader selection.
    program: ProgramType,

    /// Random seed for the shader.
    random_seed: f32,

    /// Optional custom shader source from layer rules.
    custom_shader: Option<String>,
}

niri_render_elements! {
    ClosingLayerRenderElement => {
        Texture = RelocateRenderElement<RescaleRenderElement<PrimaryGpuTextureRenderElement>>,
        Shader = ShaderRenderElement,
    }
}

impl ClosingLayer {
    #[allow(clippy::too_many_arguments)]
    pub fn new<E: RenderElement<GlesRenderer>>(
        renderer: &mut GlesRenderer,
        snapshot: RenderSnapshot<E, E>,
        scale: Scale<f64>,
        mut geo_size: Size<f64, Logical>,
        pos: Point<f64, Logical>,
        anim: Animation,
        program: ProgramType,
        custom_shader: Option<String>,
        output: Output,
        surface: LayerSurface,
        for_backdrop: bool,
    ) -> anyhow::Result<Self> {
        let _span = tracy_client::span!("ClosingLayer::new");

        let layer = surface.layer();

        let baked = snapshot.bake(renderer, scale)?;

        // Some layer-shell clients can race unmap/teardown such that close-time geometry becomes
        // invalid or no longer matches the captured snapshot. The custom shader path relies on
        // geometry transforms, so keep it stable by falling back to snapshot-derived size.
        if geo_size.w <= 0. || geo_size.h <= 0. {
            geo_size = baked.snapshot_size;
        }

        if geo_size.w <= 0. || geo_size.h <= 0. {
            let tex_size = baked.buffer.texture().size().to_f64();
            geo_size = Size::new(
                (tex_size.w / scale.x).max(1.),
                (tex_size.h / scale.y).max(1.),
            );
        }

        Ok(Self {
            output,
            surface,
            layer,
            for_backdrop,
            snapshot: baked,
            geo_size,
            pos,
            anim,
            program,
            custom_shader,
            random_seed: fastrand::f32(),
        })
    }

    /// Whether this closing layer matches the given output, layer, and backdrop settings.
    pub fn matches(&self, output: &Output, layer: Layer, for_backdrop: bool) -> bool {
        self.output == *output && self.layer == layer && self.for_backdrop == for_backdrop
    }

    /// Whether this closing layer matches the given surface.
    pub fn matches_surface(&self, surface: &LayerSurface) -> bool {
        self.surface == *surface
    }

    /// Accessor for the output.
    pub fn output(&self) -> &Output {
        &self.output
    }

    pub fn advance_animations(&mut self) {
        // We don't need to do anything here since the animation is time-based, but we still want to
        // call this to trigger the end of the animation when it finishes.
        self.anim.value();
    }

    pub fn are_animations_ongoing(&self) -> bool {
        !self.anim.is_done()
    }

    pub fn render(
        &self,
        renderer: &mut GlesRenderer,
        view_rect: Rectangle<f64, Logical>,
        scale: Scale<f64>,
        target: RenderTarget,
    ) -> ClosingLayerRenderElement {
        let (buffer, offset) = self.snapshot.pick_buffer(target);

        let anim = &self.anim;

        let progress = anim.value();
        let clamped_progress = anim.clamped_value().clamp(0., 1.);
        if let Some(shader) = self.resolve_shader(renderer) {
            // ClosingLayer uses the picked buffer for tex coord calculation (same as the
            // texture buffer), unlike ClosingWindow which uses the normal buffer.
            let elem = render_close_shader(
                shader,
                buffer,
                buffer,
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

        Shaders::get(renderer).program(self.program)
    }
}
