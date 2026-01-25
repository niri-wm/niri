use std::mem;

use anyhow::{ensure, Context as _};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::gles::{GlesFrame, GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{
    Bind as _, Color32F, ContextId, Offscreen as _, Renderer as _, Texture,
};
use smithay::utils::{Buffer, Logical, Physical, Scale, Size, Transform};

use crate::niri::OutputRenderElements;
use crate::render_helpers::blur::Blur;

#[derive(Debug)]
pub struct EffectBuffer {
    /// Id to be used for this effect buffer's elements.
    id: Id,

    /// Size of the effect buffer.
    size: Size<i32, Buffer>,
    /// Scale of the effect buffer.
    scale: Scale<f64>,
    /// Config for blurring.
    blur_config: niri_config::Blur,

    /// Elements to be rendered on demand.
    elements: Elements,
    /// Offscreen buffer where elements get rendered.
    offscreen: Option<Offscreen>,
    /// Blurring program, if available.
    blur: Option<Blur>,

    /// Commit counter that takes into account both original and blurred texture changes.
    commit_counter: CommitCounter,
}

#[derive(Debug)]
enum Elements {
    /// Contents remain unchanged.
    Unchanged(
        // Storage to avoid reallocating it every time.
        Vec<OutputRenderElements<GlesRenderer>>,
    ),
    /// New contents, need to check damage and render.
    New(Vec<OutputRenderElements<GlesRenderer>>),
}

#[derive(Debug)]
struct Offscreen {
    /// The texture with the offscreen contents.
    texture: GlesTexture,
    /// Id of the renderer context that the texture comes from.
    renderer_context_id: ContextId<GlesTexture>,
    /// Scale of the texture.
    scale: Scale<f64>,
    /// Damage tracker for drawing to the texture.
    damage: OutputDamageTracker,
    /// Rendered blurred version of the texture.
    ///
    /// When texture needs to be reblurred, this field must be reset to `None`.
    // TODO: store Option<Option<GlesTexture>> where Some(None) means we already tried rendering it
    // once this frame and failed.
    blurred: Option<GlesTexture>,
}

impl Default for Elements {
    fn default() -> Self {
        Self::Unchanged(Vec::new())
    }
}

impl EffectBuffer {
    pub fn new() -> Self {
        Self {
            id: Id::new(),
            size: Size::default(),
            scale: Scale::from(1.),
            blur_config: niri_config::Blur::default(),
            elements: Elements::default(),
            offscreen: None,
            blur: None,
            commit_counter: CommitCounter::default(),
        }
    }

    pub fn id(&self) -> &Id {
        &self.id
    }

    pub fn commit(&self) -> CommitCounter {
        self.commit_counter
    }

    pub fn blur_config(&self) -> niri_config::Blur {
        self.blur_config
    }

    pub fn logical_size(&self) -> Size<f64, Logical> {
        self.size.to_f64().to_logical(self.scale, Transform::Normal)
    }

    pub fn scale(&self) -> Scale<f64> {
        self.scale
    }

    pub fn update_size(&mut self, size: Size<i32, Physical>, scale: Scale<f64>) {
        self.size = size.to_logical(1).to_buffer(1, Transform::Normal);
        self.scale = scale;
    }

    pub fn update_blur_config(&mut self, config: niri_config::Blur) {
        if self.blur_config == config {
            return;
        }

        self.blur_config = config;

        if let Some(offscreen) = &mut self.offscreen {
            if offscreen.blurred.is_some() {
                offscreen.blurred = None;
                self.commit_counter.increment();
            }
        }
    }

    pub fn elements(&mut self) -> &mut Vec<OutputRenderElements<GlesRenderer>> {
        // Assume we're going to insert new elements, switch to New.
        match mem::take(&mut self.elements) {
            Elements::Unchanged(elements) | Elements::New(elements) => {
                self.elements = Elements::New(elements);
            }
        }
        let Elements::New(elements) = &mut self.elements else {
            unreachable!();
        };
        elements
    }

    pub fn prepare(&mut self, renderer: &mut GlesRenderer, blur: bool) -> Option<bool> {
        if let Err(err) = self.prepare_offscreen(renderer) {
            warn!("error preparing offscreen: {err:?}");
            return None;
        };

        // TODO: make sure it's known whether the blur is prepared or not. Reset the prepared flag
        // as necessary. During a frame, if any call to prepare() needs blur, prepare it.
        let blur = blur && !self.blur_config.off;
        if blur {
            if let Err(err) = self.prepare_blur(renderer) {
                warn!("error preparing blur: {err:?}");
                return None;
            }
        }

        Some(blur)
    }

    fn prepare_offscreen(&mut self, renderer: &mut GlesRenderer) -> anyhow::Result<()> {
        let _span = tracy_client::span!("EffectBuffer::prepare_offscreen");

        // Check if we need to create or recreate the texture.
        let size_string;
        let mut reason = "";
        if let Some(Offscreen {
            texture,
            renderer_context_id,
            ..
        }) = &mut self.offscreen
        {
            let old_size = texture.size();
            if old_size != self.size {
                size_string = format!(
                    "size changed from {} × {} to {} × {}",
                    old_size.w, old_size.h, self.size.w, self.size.h
                );
                reason = &size_string;

                self.offscreen = None;
            } else if !texture.is_unique_reference() {
                reason = "not unique";

                self.offscreen = None;
            } else if *renderer_context_id != renderer.context_id() {
                reason = "renderer id changed";

                self.offscreen = None;
            }
        } else {
            reason = "first render";
        }

        let offscreen = if let Some(offscreen) = &mut self.offscreen {
            offscreen
        } else {
            debug!("creating new offscreen texture: {reason}");
            let span = tracy_client::span!("creating effect offscreen texture");
            span.emit_text(reason);

            let texture: GlesTexture = renderer
                .create_buffer(Fourcc::Abgr8888, self.size)
                .context("error creating texture")?;

            let buffer_size = self.size.to_logical(1, Transform::Normal).to_physical(1);
            let damage = OutputDamageTracker::new(buffer_size, self.scale, Transform::Normal);

            self.offscreen.insert(Offscreen {
                texture,
                renderer_context_id: renderer.context_id(),
                scale: self.scale,
                damage,
                blurred: None,
            })
        };

        // Recreate the damage tracker if the scale changes. We already recreate it for buffer size
        // changes, and transform is always Normal.
        if offscreen.scale != self.scale {
            offscreen.scale = self.scale;

            trace!("recreating damage tracker due to scale change");
            let buffer_size = self.size.to_logical(1, Transform::Normal).to_physical(1);
            offscreen.damage = OutputDamageTracker::new(buffer_size, self.scale, Transform::Normal);

            self.commit_counter.increment();
            offscreen.blurred = None;
        }

        // Render the elements if any.
        let mut elements = match mem::take(&mut self.elements) {
            Elements::New(elements) => elements,
            x @ Elements::Unchanged(_) => {
                // No redrawing necessary.
                self.elements = x;
                return Ok(());
            }
        };

        let res = {
            let mut target = renderer
                .bind(&mut offscreen.texture)
                .context("error binding texture")?;
            offscreen
                .damage
                .render_output(renderer, &mut target, 1, &elements, Color32F::TRANSPARENT)
                .context("error rendering")?
        };

        if res.damage.is_some() {
            self.commit_counter.increment();

            // Original texture changed; reset the blurred texture.
            offscreen.blurred = None;
        }

        // Clear and put the storage back.
        elements.clear();
        self.elements = Elements::Unchanged(elements);

        Ok(())
    }

    fn prepare_blur(&mut self, renderer: &mut GlesRenderer) -> anyhow::Result<()> {
        let offscreen = self.offscreen.as_mut().context("missing offscreen")?;
        if offscreen.blurred.is_some() {
            // Already rendered.
            return Ok(());
        }

        if let Some(blur) = &self.blur {
            if blur.context_id() != renderer.context_id() {
                debug!("recreating blur: renderer changed");
                self.blur = None;
            }
        }

        let blur = if let Some(blur) = &mut self.blur {
            blur
        } else {
            let Some(blur) = Blur::new(renderer) else {
                // Missing blur shader.
                return Ok(());
            };
            self.blur.insert(blur)
        };

        ensure!(
            offscreen.renderer_context_id == renderer.context_id(),
            "wrong renderer context id"
        );

        blur.prepare_textures(renderer, &offscreen.texture, self.blur_config)
            .context("error preparing blur textures")?;

        Ok(())
    }

    pub fn render(&mut self, frame: &mut GlesFrame, blur: bool) -> anyhow::Result<GlesTexture> {
        let offscreen = self.offscreen.as_mut().context("offscreen is missing")?;

        if !blur {
            return Ok(offscreen.texture.clone());
        }

        let texture = if let Some(texture) = &offscreen.blurred {
            texture.clone()
        } else {
            let blur = self.blur.as_mut().context("blur is missing")?;
            let blurred = blur
                .render(frame, &offscreen.texture, self.blur_config)
                .context("error rendering blur")?;
            offscreen.blurred.insert(blurred).clone()
        };

        Ok(texture)
    }
}
