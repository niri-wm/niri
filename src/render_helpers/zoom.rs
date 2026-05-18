use smithay::backend::renderer::element::utils::Relocate;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::Renderer;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};

#[derive(Debug)]
pub struct ZoomElement<E> {
    element: E,
    origin: Point<f64, Physical>,
    scale: Scale<f64>,
    location: Point<f64, Physical>,
    relocate: Relocate,
}

impl<E: Element> ZoomElement<E> {
    pub fn from_element(
        element: E,
        origin: Point<f64, Physical>,
        scale: impl Into<Scale<f64>>,
        location: Point<f64, Physical>,
        relocate: Relocate,
    ) -> Self {
        Self {
            element,
            origin,
            scale: scale.into(),
            location,
            relocate,
        }
    }
}

impl<E: Element> Element for ZoomElement<E> {
    fn id(&self) -> &Id {
        self.element.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.element.current_commit()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.element.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        let mut geometry = self.element.geometry(scale).to_f64();
        geometry.loc -= self.origin;
        geometry = geometry.upscale(self.scale);
        geometry.loc += self.origin;

        match self.relocate {
            Relocate::Absolute => geometry.loc = self.location,
            Relocate::Relative => geometry.loc += self.location,
        }

        let loc = geometry.loc.to_i32_round();
        let bottom_right = (geometry.loc + geometry.size.to_f64()).to_i32_round();
        Rectangle::new(loc, (bottom_right - loc).to_size())
    }

    fn transform(&self) -> Transform {
        self.element.transform()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.element
            .damage_since(scale, commit)
            .into_iter()
            .map(|rect| rect.to_f64().upscale(self.scale).to_i32_up())
            .collect()
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.element
            .opaque_regions(scale)
            .into_iter()
            .map(|rect| rect.to_f64().upscale(self.scale).to_i32_round())
            .collect()
    }

    fn alpha(&self) -> f32 {
        self.element.alpha()
    }

    fn kind(&self) -> Kind {
        self.element.kind()
    }

    fn is_framebuffer_effect(&self) -> bool {
        self.element.is_framebuffer_effect()
    }
}

impl<R: Renderer, E: RenderElement<R>> RenderElement<R> for ZoomElement<E> {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        self.element
            .draw(frame, src, dst, damage, opaque_regions, cache)
    }

    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        self.element.underlying_storage(renderer)
    }

    fn capture_framebuffer(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), R::Error> {
        self.element.capture_framebuffer(frame, src, dst, cache)
    }
}
