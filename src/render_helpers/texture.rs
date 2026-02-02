use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::GlesTexture;
use smithay::backend::renderer::utils::{CommitCounter, OpaqueRegions};
use smithay::backend::renderer::{ContextId, Frame as _, ImportMem, Renderer, Texture};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform};

use super::memory::MemoryBuffer;

/// Smithay's texture buffer, but with fractional scale.
#[derive(Debug, Clone)]
pub struct TextureBuffer<T: Texture> {
    id: Id,
    commit_counter: CommitCounter,
    renderer_context_id: ContextId<T>,
    texture: T,
    scale: Scale<f64>,
    transform: Transform,
    opaque_regions: Vec<Rectangle<i32, Buffer>>,
}

/// Render element for a [`TextureBuffer`].
#[derive(Debug, Clone)]
pub struct TextureRenderElement<T: Texture> {
    buffer: TextureBuffer<T>,
    location: Point<f64, Logical>,
    alpha: f32,
    src: Option<Rectangle<f64, Logical>>,
    size: Option<Size<f64, Logical>>,
    kind: Kind,
}

impl<T: Texture> TextureBuffer<T> {
    pub fn from_texture<R: Renderer<TextureId = T>>(
        renderer: &R,
        texture: T,
        scale: impl Into<Scale<f64>>,
        transform: Transform,
        opaque_regions: Vec<Rectangle<i32, Buffer>>,
    ) -> Self {
        TextureBuffer {
            id: Id::new(),
            commit_counter: CommitCounter::default(),
            renderer_context_id: renderer.context_id(),
            texture,
            scale: scale.into(),
            transform,
            opaque_regions,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_memory<R: Renderer<TextureId = T> + ImportMem>(
        renderer: &mut R,
        data: &[u8],
        format: Fourcc,
        size: impl Into<Size<i32, Buffer>>,
        flipped: bool,
        scale: impl Into<Scale<f64>>,
        transform: Transform,
        opaque_regions: Vec<Rectangle<i32, Buffer>>,
    ) -> Result<Self, R::Error> {
        let texture = renderer.import_memory(data, format, size.into(), flipped)?;
        Ok(TextureBuffer::from_texture(
            renderer,
            texture,
            scale,
            transform,
            opaque_regions,
        ))
    }

    pub fn from_memory_buffer<R: Renderer<TextureId = T> + ImportMem>(
        renderer: &mut R,
        buffer: &MemoryBuffer,
    ) -> Result<Self, R::Error> {
        Self::from_memory(
            renderer,
            buffer.data(),
            buffer.format(),
            buffer.size(),
            false,
            buffer.scale(),
            buffer.transform(),
            Vec::new(),
        )
    }

    pub fn texture(&self) -> &T {
        &self.texture
    }

    pub fn texture_scale(&self) -> Scale<f64> {
        self.scale
    }

    pub fn set_texture_scale(&mut self, scale: impl Into<Scale<f64>>) {
        self.scale = scale.into();
    }

    pub fn texture_transform(&self) -> Transform {
        self.transform
    }

    pub fn set_texture_transform(&mut self, transform: Transform) {
        self.transform = transform;
    }
}

impl<T: Texture> TextureBuffer<T> {
    pub fn logical_size(&self) -> Size<f64, Logical> {
        self.texture
            .size()
            .to_f64()
            .to_logical(self.scale, self.transform)
    }
}

impl TextureBuffer<GlesTexture> {
    pub fn is_texture_reference_unique(&mut self) -> bool {
        self.texture.is_unique_reference()
    }
}

impl<T: Texture> TextureRenderElement<T> {
    pub fn from_texture_buffer(
        buffer: TextureBuffer<T>,
        location: impl Into<Point<f64, Logical>>,
        alpha: f32,
        src: Option<Rectangle<f64, Logical>>,
        size: Option<Size<f64, Logical>>,
        kind: Kind,
    ) -> Self {
        TextureRenderElement {
            buffer,
            location: location.into(),
            alpha,
            src,
            size,
            kind,
        }
    }

    pub fn buffer(&self) -> &TextureBuffer<T> {
        &self.buffer
    }
}

impl<T: Texture> TextureRenderElement<T> {
    pub fn logical_size(&self) -> Size<f64, Logical> {
        self.size
            .or_else(|| self.src.map(|src| src.size))
            .unwrap_or_else(|| self.buffer.logical_size())
    }

    pub fn logical_src(&self) -> Rectangle<f64, Logical> {
        self.src
            .unwrap_or_else(|| Rectangle::from_size(self.logical_size()))
    }
}

impl<T: Texture> Element for TextureRenderElement<T> {
    fn id(&self) -> &Id {
        &self.buffer.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.buffer.commit_counter
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        let logical_geo = Rectangle::new(self.location, self.logical_size());
        logical_geo.to_physical_precise_round(scale)
    }

    fn transform(&self) -> Transform {
        self.buffer.transform
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.src
            .map(|src| {
                src.to_buffer(
                    self.buffer.scale,
                    self.buffer.transform,
                    &self.buffer.logical_size(),
                )
            })
            .unwrap_or_else(|| Rectangle::from_size(self.buffer.texture.size()).to_f64())
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        let texture_size = self.buffer.texture.size().to_f64();
        let src = self.src();

        self.buffer
            .opaque_regions
            .iter()
            .filter_map(|region| {
                let mut region = region.to_f64().intersection(src)?;

                region.loc -= src.loc;
                region = region.upscale(texture_size / src.size);

                let logical =
                    region.to_logical(self.buffer.scale, self.buffer.transform, &src.size);
                Some(logical.to_physical_precise_down(scale))
            })
            .collect()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        self.kind
    }
}

impl<R, T> RenderElement<R> for TextureRenderElement<T>
where
    R: Renderer<TextureId = T>,
    T: Texture,
{
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dest: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        if frame.context_id() != self.buffer.renderer_context_id {
            warn!("trying to render texture from different renderer");
            return Ok(());
        }

        frame.render_texture_from_to(
            &self.buffer.texture,
            src,
            dest,
            damage,
            opaque_regions,
            self.buffer.transform,
            self.alpha,
        )
    }

    fn underlying_storage(&self, _renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
