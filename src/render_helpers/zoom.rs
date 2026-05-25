use smithay::backend::renderer::element::utils::Relocate;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::{FrameContext, Renderer, TextureFilter};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};

use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
use crate::render_helpers::renderer::AsGlesFrame;

/// Helper macro: wrap a draw/capture_framebuffer call with filter set/restore.
///
/// `$get_guard` is an expression that yields a renderer guard (e.g. `frame.renderer()` or
/// `frame.as_gles_frame().renderer()`). `$filter` is `self.filter` (or any
/// `Option<TextureFilter>`). `$body` is the draw or capture_framebuffer call expression (which
/// returns a `Result`).
macro_rules! with_filter {
    ($get_guard:expr, $filter:expr, $body:expr $(,)?) => {{
        if let Some(filter) = $filter {
            let mut guard = $get_guard;
            guard.as_mut().upscale_filter(filter)?;
            guard.as_mut().downscale_filter(filter)?;
            drop(guard);

            $body?;

            let mut guard = $get_guard;
            guard.as_mut().upscale_filter(TextureFilter::Linear)?;
            guard.as_mut().downscale_filter(TextureFilter::Linear)?;
            Ok(())
        } else {
            $body
        }
    }};
}

/// Linear below threshold, nearest-neighbour at or above.
pub fn zoom_filter(zoom_factor: f64, threshold: f64) -> Option<TextureFilter> {
    (zoom_factor > 1.0).then_some(match zoom_factor < threshold {
        true => TextureFilter::Linear,
        false => TextureFilter::Nearest,
    })
}

#[derive(Debug)]
pub struct ZoomElement<E> {
    element: E,
    origin: Point<f64, Physical>,
    scale: Scale<f64>,
    location: Point<f64, Physical>,
    relocate: Relocate,
    filter: Option<TextureFilter>,
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
            filter: None,
        }
    }

    pub fn with_filter(mut self, filter: Option<TextureFilter>) -> Self {
        self.filter = filter;
        self
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
        // Damage is relative to the element, so only the zoom scale applies
        // here; focal-origin anchoring and relocation are reflected by the
        // element geometry, not by its relative damage rectangles.
        self.element
            .damage_since(scale, commit)
            .into_iter()
            .map(|rect| rect.to_f64().upscale(self.scale).to_i32_up())
            .collect()
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // Opaque regions are also element-relative and therefore only need the
        // zoom scale propagated.
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

impl<E: RenderElement<GlesRenderer>> RenderElement<GlesRenderer> for ZoomElement<E> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        with_filter!(
            frame.renderer(),
            self.filter,
            self.element
                .draw(frame, src, dst, damage, opaque_regions, cache),
        )
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.element.underlying_storage(renderer)
    }

    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        with_filter!(
            frame.renderer(),
            self.filter,
            self.element.capture_framebuffer(frame, src, dst, cache),
        )
    }
}

impl<'render, E: RenderElement<TtyRenderer<'render>>> RenderElement<TtyRenderer<'render>>
    for ZoomElement<E>
{
    fn draw(
        &self,
        frame: &mut TtyFrame<'render, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), TtyRendererError<'render>> {
        with_filter!(
            frame.as_gles_frame().renderer(),
            self.filter,
            self.element
                .draw(frame, src, dst, damage, opaque_regions, cache),
        )
    }

    fn underlying_storage(
        &self,
        renderer: &mut TtyRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        self.element.underlying_storage(renderer)
    }

    fn capture_framebuffer(
        &self,
        frame: &mut TtyFrame<'render, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), TtyRendererError<'render>> {
        with_filter!(
            frame.as_gles_frame().renderer(),
            self.filter,
            self.element.capture_framebuffer(frame, src, dst, cache),
        )
    }
}
