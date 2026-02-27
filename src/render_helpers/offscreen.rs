use std::cell::RefCell;

use anyhow::Context as _;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
use smithay::backend::renderer::element::{
    Element, Id, Kind, RenderElement, RenderElementStates, UnderlyingStorage,
};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture};
use smithay::backend::renderer::sync::SyncPoint;
use smithay::backend::renderer::utils::{
    CommitCounter, DamageBag, DamageSet, DamageSnapshot, OpaqueRegions,
};
use smithay::backend::renderer::{
    Bind as _, Color32F, ContextId, Frame as _, Offscreen as _, Renderer, Texture as _,
};
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform};

use super::encompassing_geo;
use super::renderer::AsGlesFrame as _;
use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};

/// Buffer for offscreen rendering.
#[derive(Debug)]
pub struct OffscreenBuffer {
    id: Id,

    /// The cached texture buffer.
    ///
    /// Lazily created when `render` is called. Recreated when necessary.
    inner: RefCell<Option<Inner>>,
}

#[derive(Debug)]
struct Inner {
    /// The texture with offscreened contents.
    texture: GlesTexture,
    /// Id of the renderer context that the texture comes from.
    renderer_context_id: ContextId<GlesTexture>,
    /// Scale of the texture.
    scale: Scale<f64>,
    /// Damage tracker for drawing to the texture.
    damage: OutputDamageTracker,
    /// Damage of this offscreen element itself facing outside.
    outer_damage: DamageBag<i32, Buffer>,
}

#[derive(Debug, Clone)]
pub struct OffscreenRenderElement {
    id: Id,
    texture: GlesTexture,
    renderer_context_id: ContextId<GlesTexture>,
    scale: Scale<f64>,
    damage: DamageSnapshot<i32, Buffer>,
    offset: Point<f64, Logical>,
    src_size: Size<i32, Buffer>,
    alpha: f32,
    kind: Kind,
}

#[derive(Debug)]
pub struct OffscreenData {
    /// Id of the offscreen element.
    pub id: Id,
    /// States for the render into the offscreen buffer.
    pub states: RenderElementStates,
}

impl OffscreenBuffer {
    pub fn render(
        &self,
        renderer: &mut GlesRenderer,
        scale: Scale<f64>,
        elements: &[impl RenderElement<GlesRenderer>],
    ) -> anyhow::Result<(OffscreenRenderElement, SyncPoint, OffscreenData)> {
        let _span = tracy_client::span!("OffscreenBuffer::render");

        let geo = encompassing_geo(scale, elements.iter());
        // TODO: check for zero size.
        let elements = Vec::from_iter(elements.iter().map(|ele| {
            RelocateRenderElement::from_element(ele, geo.loc.upscale(-1), Relocate::Relative)
        }));

        let src_size = geo.size;
        let src_size = src_size.to_logical(1).to_buffer(1, Transform::Normal);
        let offset = geo.loc.to_f64().to_logical(scale);

        let mut inner = self.inner.borrow_mut();

        // Check if we need to create or recreate the texture.
        let size_string;
        let mut reason = "";
        if let Some(Inner {
            texture,
            renderer_context_id,
            ..
        }) = inner.as_mut()
        {
            let old_size = texture.size();
            if old_size.w < src_size.w || old_size.h < src_size.h {
                size_string = format!(
                    "size increased from {} × {} to {} × {}",
                    old_size.w, old_size.h, src_size.w, src_size.h
                );
                reason = &size_string;

                *inner = None;
            } else if !texture.is_unique_reference() {
                reason = "not unique";

                *inner = None;
            } else if *renderer_context_id != renderer.context_id() {
                reason = "renderer id changed";

                *inner = None;
            }
        } else {
            reason = "first render";
        }

        let inner = if let Some(inner) = inner.as_mut() {
            inner
        } else {
            trace!("creating new texture: {reason}");
            let span = tracy_client::span!("creating offscreen buffer");
            span.emit_text(reason);

            let texture: GlesTexture = renderer
                .create_buffer(Fourcc::Abgr8888, src_size)
                .context("error creating texture")?;

            let buffer_size = src_size.to_logical(1, Transform::Normal).to_physical(1);
            let damage = OutputDamageTracker::new(buffer_size, scale, Transform::Normal);

            inner.insert(Inner {
                texture,
                renderer_context_id: renderer.context_id(),
                scale,
                damage,
                outer_damage: DamageBag::default(),
            })
        };

        // When leaving the old texture as is, its size might be bigger than src_size.
        let texture_size = inner.texture.size();
        let buffer_size = texture_size.to_logical(1, Transform::Normal).to_physical(1);

        // Recreate the damage tracker if the scale changes. We already recreate it for buffer size
        // changes, and transform is always Normal.
        if inner.scale != scale {
            inner.scale = scale;

            trace!("recreating damage tracker due to scale change");
            inner.damage = OutputDamageTracker::new(buffer_size, scale, Transform::Normal);
            inner.outer_damage = DamageBag::default();
        }

        let res = {
            let mut target = renderer.bind(&mut inner.texture)?;
            inner
                .damage
                .render_output(renderer, &mut target, 1, &elements, Color32F::TRANSPARENT)
                .context("error rendering")?
        };

        // Add the resulting damage to the outer tracker.
        if let Some(damage) = res.damage {
            // OutputDamageTracker gives us Physical coordinate space, but it's actually the Buffer
            // space because we were rendering to a texture.
            let size = buffer_size.to_logical(1);
            let damage = damage
                .iter()
                .map(|rect| rect.to_logical(1).to_buffer(1, Transform::Normal, &size));
            inner.outer_damage.add(damage);
        }

        let elem = OffscreenRenderElement {
            id: self.id.clone(),
            texture: inner.texture.clone(),
            renderer_context_id: inner.renderer_context_id.clone(),
            scale,
            damage: inner.outer_damage.snapshot(),
            offset,
            src_size,
            alpha: 1.,
            kind: Kind::Unspecified,
        };

        let data = OffscreenData {
            id: self.id.clone(),
            states: res.states,
        };

        Ok((elem, res.sync, data))
    }
}

impl Default for OffscreenBuffer {
    fn default() -> Self {
        OffscreenBuffer {
            inner: RefCell::new(None),
            id: Id::new(),
        }
    }
}

impl OffscreenRenderElement {
    pub fn texture(&self) -> &GlesTexture {
        &self.texture
    }

    pub fn offset(&self) -> Point<f64, Logical> {
        self.offset
    }

    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }

    pub fn with_offset(mut self, offset: Point<f64, Logical>) -> Self {
        self.offset = offset;
        self
    }

    pub fn logical_size(&self) -> Size<f64, Logical> {
        self.src_size
            .to_f64()
            .to_logical(self.scale, Transform::Normal)
    }

    fn damage_since(&self, commit: Option<CommitCounter>) -> DamageSet<i32, Buffer> {
        self.damage
            .damage_since(commit)
            .unwrap_or_else(|| DamageSet::from_slice(&[Rectangle::from_size(self.texture.size())]))
    }
}

impl Element for OffscreenRenderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.damage.current_commit()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        let logical_geo = Rectangle::new(self.offset, self.logical_size());
        logical_geo.to_physical_precise_round(scale)
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(self.src_size).to_f64()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        let texture_size = self.texture.size().to_f64();
        let src = self.src();

        self.damage_since(commit)
            .into_iter()
            .filter_map(|region| {
                let mut region = region.to_f64().intersection(src)?;

                region.loc -= src.loc;
                region = region.upscale(texture_size / src.size);

                let logical = region.to_logical(self.scale, Transform::Normal, &src.size);
                Some(logical.to_physical_precise_up(scale))
            })
            .collect()
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        self.kind
    }
}

impl RenderElement<GlesRenderer> for OffscreenRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dest: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        if frame.context_id() != self.renderer_context_id {
            warn!("trying to render texture from different renderer");
            return Ok(());
        }

        frame.render_texture_from_to(
            &self.texture,
            src,
            dest,
            damage,
            opaque_regions,
            Transform::Normal,
            self.alpha,
            None,
            &[],
        )
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // If scanout for things other than Wayland buffers is implemented, this will need to take
        // the target GPU into account.
        None
    }
}

impl<'render> RenderElement<TtyRenderer<'render>> for OffscreenRenderElement {
    fn draw(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), TtyRendererError<'render>> {
        let gles_frame = frame.as_gles_frame();
        RenderElement::<GlesRenderer>::draw(&self, gles_frame, src, dst, damage, opaque_regions)?;
        Ok(())
    }

    fn underlying_storage(
        &self,
        _renderer: &mut TtyRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        // If scanout for things other than Wayland buffers is implemented, this will need to take
        // the target GPU into account.
        None
    }
}
