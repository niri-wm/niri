use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::{ImportMem, Renderer};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};

/// A cursor rasterized at its final physical size, independent of the output render scale.
#[derive(Debug)]
pub struct PhysicalCursorRenderElement<R: Renderer>(MemoryRenderBufferRenderElement<R>);

impl<R: Renderer + ImportMem> PhysicalCursorRenderElement<R> {
    pub fn from_buffer(
        renderer: &mut R,
        location: Point<i32, Physical>,
        buffer: &MemoryRenderBuffer,
    ) -> Result<Self, R::Error>
    where
        R::TextureId: Send + Clone + 'static,
    {
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location.to_f64(),
            buffer,
            None,
            None,
            None,
            Kind::Cursor,
        )
        .map(Self)
    }
}

impl<R: Renderer> Element for PhysicalCursorRenderElement<R> {
    fn id(&self) -> &Id {
        self.0.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.0.current_commit()
    }
    fn transform(&self) -> Transform {
        self.0.transform()
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        self.0.src()
    }
    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.0.geometry(Scale::from(1.))
    }
    fn damage_since(
        &self,
        _scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.0.damage_since(Scale::from(1.), commit)
    }
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.0.opaque_regions(Scale::from(1.))
    }
    fn alpha(&self) -> f32 {
        self.0.alpha()
    }
    fn kind(&self) -> Kind {
        self.0.kind()
    }
}

impl<R: Renderer + ImportMem> RenderElement<R> for PhysicalCursorRenderElement<R>
where
    R::TextureId: 'static,
{
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        self.0.draw(frame, src, dst, damage, opaque_regions, cache)
    }
    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        self.0.underlying_storage(renderer)
    }
}
