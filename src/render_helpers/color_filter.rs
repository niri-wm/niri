use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Transform};

use super::renderer::{AsGlesFrame as _, NiriRenderer};
use super::shaders::Shaders;
use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};

#[derive(Debug)]
pub struct ColorFilterRenderElement<R: NiriRenderer> {
    inner: WaylandSurfaceRenderElement<R>,
    program: GlesTexProgram,
    uniforms: Vec<Uniform<'static>>,
}

impl<R: NiriRenderer> ColorFilterRenderElement<R> {
    pub fn new(
        elem: WaylandSurfaceRenderElement<R>,
        program: GlesTexProgram,
        uniforms: Vec<Uniform<'static>>,
    ) -> Self {
        Self {
            inner: elem,
            program,
            uniforms,
        }
    }

    pub fn saturation_shader(renderer: &mut R) -> Option<&GlesTexProgram> {
        Shaders::get(renderer).saturated_surface.as_ref()
    }
}

impl<R: NiriRenderer> Element for ColorFilterRenderElement<R> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.inner.opaque_regions(scale)
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl RenderElement<GlesRenderer> for ColorFilterRenderElement<GlesRenderer> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        frame.override_default_tex_program(self.program.clone(), self.uniforms.clone());
        RenderElement::<GlesRenderer>::draw(&self.inner, frame, src, dst, damage, opaque_regions)?;
        frame.clear_tex_program_override();
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

impl<'render> RenderElement<TtyRenderer<'render>>
    for ColorFilterRenderElement<TtyRenderer<'render>>
{
    fn draw(
        &self,
        frame: &mut TtyFrame<'render, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), TtyRendererError<'render>> {
        frame
            .as_gles_frame()
            .override_default_tex_program(self.program.clone(), self.uniforms.clone());
        RenderElement::draw(&self.inner, frame, src, dst, damage, opaque_regions)?;
        frame.as_gles_frame().clear_tex_program_override();
        Ok(())
    }

    fn underlying_storage(
        &self,
        _renderer: &mut TtyRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
