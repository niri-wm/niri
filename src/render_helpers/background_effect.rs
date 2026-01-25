use std::sync::Arc;

use niri_config::CornerRadius;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Scale};

use crate::niri_render_elements;
use crate::render_helpers::blur::BlurElement;
use crate::render_helpers::damage::ExtraDamage;
use crate::render_helpers::xray::XrayElement;
use crate::render_helpers::RenderCtx;

#[derive(Debug)]
pub struct BackgroundEffect {
    nonxray: BlurElement,
    /// Damage when options change.
    damage: ExtraDamage,
    options: Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Options {
    pub blur: bool,
    pub xray: bool,
    pub noise: Option<f64>,
    pub saturation: Option<f64>,
}

impl Options {
    fn is_visible(&self) -> bool {
        self.xray
            || self.blur
            || self.noise.is_some_and(|x| x > 0.)
            || self.saturation.is_some_and(|x| x != 1.)
    }
}

/// Render-time parameters.
#[derive(Debug)]
pub struct RenderParams {
    /// Geometry of the background effect.
    pub geometry: Rectangle<f64, Logical>,
    /// Effect subregion, will be clipped to `geometry`.
    pub subregion: Option<EffectSubregion>,
    /// Position of `geometry` relative to the backdrop.
    pub pos_in_backdrop: Point<f64, Logical>,
    /// Geometry and radius for clipping in the same coordinate space as `geometry`.
    pub clip: Option<(Rectangle<f64, Logical>, CornerRadius)>,
    /// Zoom factor between backdrop coordinates and geometry.
    pub zoom: f64,
    /// Scale to use for rounding to physical pixels.
    pub scale: f64,
}

impl RenderParams {
    fn fit_clip_radius(&mut self) {
        if let Some((geo, radius)) = &mut self.clip {
            *radius = radius.fit_to(geo.size.w as f32, geo.size.h as f32);
        }
    }
}

#[derive(Debug)]
pub struct EffectSubregion {
    /// Non-overlapping rects in surface-local coordinates.
    pub rects: Arc<Vec<Rectangle<i32, Logical>>>,
    /// Scale to apply to each rect.
    pub scale: Scale<f64>,
    /// Translation to apply to each rect after scaling.
    pub offset: Point<f64, Logical>,
}

impl EffectSubregion {
    pub fn iter(&self) -> impl Iterator<Item = Rectangle<f64, Logical>> + '_ {
        self.rects.iter().map(|r| {
            let mut r = r.to_f64();
            r = r.upscale(self.scale);
            r.loc += self.offset;
            r
        })
    }
}

niri_render_elements! {
    BackgroundEffectElement => {
        Blur = BlurElement,
        Xray = XrayElement,
        ExtraDamage = ExtraDamage,
    }
}

impl BackgroundEffect {
    pub fn new() -> Self {
        Self {
            nonxray: BlurElement::new(),
            damage: ExtraDamage::new(),
            options: Options::default(),
        }
    }

    pub fn update_config(&mut self, config: niri_config::Blur) {
        self.nonxray.update_config(config);
    }

    pub fn update_render_elements(
        &mut self,
        effect: niri_config::BackgroundEffect,
        has_blur_region: bool,
    ) {
        // If the surface explicitly requests a blur region, default blur to true.
        let blur = if has_blur_region {
            effect.blur != Some(false)
        } else {
            effect.blur == Some(true)
        };

        let mut options = Options {
            blur,
            xray: effect.xray == Some(true),
            noise: effect.noise,
            saturation: effect.saturation,
        };

        // If we have some background effect but xray wasn't explicitly set, default it to true
        // since it's cheaper.
        if options.is_visible() && effect.xray.is_none() {
            options.xray = true;
        }

        if self.options != options {
            self.options = options;
            self.damage.damage_all();
        }
    }

    pub fn is_visible(&self) -> bool {
        self.options.is_visible()
    }

    pub fn render(
        &self,
        ctx: RenderCtx<GlesRenderer>,
        mut params: RenderParams,
        push: &mut dyn FnMut(BackgroundEffectElement),
    ) {
        if !self.is_visible() {
            return;
        }

        params.fit_clip_radius();

        let damage = self.damage.render(params.geometry);

        if self.options.xray {
            let Some(xray) = ctx.xray else {
                return;
            };

            push(damage.into());
            xray.render(ctx, self.options, params, &mut |elem| push(elem.into()));
        } else {
            // Render non-xray effect.
            if let Some(elem) = self.nonxray.render(ctx.renderer, self.options, params) {
                push(damage.into());
                push(elem.into());
            }
        }
    }
}
