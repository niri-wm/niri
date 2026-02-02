use std::array;
use std::sync::Arc;

use niri_config::CornerRadius;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale};

use crate::niri_render_elements;
use crate::render_helpers::blur::BlurOptions;
use crate::render_helpers::damage::ExtraDamage;
use crate::render_helpers::framebuffer_effect::{FramebufferEffect, FramebufferEffectElement};
use crate::render_helpers::xray::XrayElement;
use crate::render_helpers::{RenderCtx, RenderTarget};

#[derive(Debug)]
pub struct BackgroundEffect {
    // Framebuffer effects are per-render-target because they store the framebuffer contents in a
    // texture, and those differ per render target.
    nonxray: [FramebufferEffect; RenderTarget::COUNT],
    /// Damage when options change.
    damage: ExtraDamage,
    /// Corner radius for clipping.
    ///
    /// Stored here in addition to `RenderParams` to damage when it changes.
    // FIXME: would be good to remove this duplication of radius.
    corner_radius: CornerRadius,
    blur_config: niri_config::Blur,
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
    ///
    /// `subregion.iter()` should return `geometry`-relative rectangles.
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
            // HACK: increase radius to avoid slight bleed on rounded corners.
            *radius = radius.expanded_by(1.);

            *radius = radius.fit_to(geo.size.w as f32, geo.size.h as f32);
        }
    }
}

#[derive(Debug, Clone)]
pub struct EffectSubregion {
    /// Non-overlapping rects in surface-local coordinates.
    pub rects: Arc<Vec<Rectangle<i32, Logical>>>,
    /// Scale to apply to each rect.
    pub scale: Scale<f64>,
    /// Translation to apply to each rect after scaling.
    pub offset: Point<f64, Logical>,
}

impl EffectSubregion {
    /// Returns an iterator over the top-left and bottom-right corners of transformed rects.
    pub fn iter(&self) -> impl Iterator<Item = (Point<f64, Logical>, Point<f64, Logical>)> + '_ {
        self.rects.iter().map(|r| {
            // Here we start in a happy i32 world where everything lines up, and rectangle loc +
            // size is exactly equal to the adjacent rectangle's loc.
            //
            // Unfortunately, we're about to descend to the floating point hell. And we *really*
            // want adjacent rects to remain adjacent no matter what. So we'll convert our rects to
            // their extremities (rather than loc and size), and operate on those. Coordinates from
            // adjacent rects will undergo exactly the same floating point operations, so when
            // they're ultimately rounded to physical pixels, they will remain adjacent.
            let r = r.to_f64();

            let mut a = r.loc;
            // f64 is enough to represent this i32 addition exactly.
            let mut b = r.loc + r.size.to_point();

            a = a.upscale(self.scale);
            b = b.upscale(self.scale);

            a += self.offset;
            b += self.offset;

            (a, b)
        })
    }

    pub fn filter_damage(
        &self,
        // Same coordinate space as self.iter().
        crop: Rectangle<f64, Logical>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        filtered: &mut Vec<Rectangle<i32, Physical>>,
    ) {
        let scale = dst.size.to_f64() / crop.size;

        let cs = crop.size.to_point();

        for (mut a, mut b) in self.iter() {
            // Convert to dst-relative.
            a -= crop.loc;
            b -= crop.loc;

            // Intersect with crop.
            let ia = Point::new(f64::max(a.x, 0.), f64::max(a.y, 0.));
            let ib = Point::new(f64::min(b.x, cs.x), f64::min(b.y, cs.y));
            if ib.x <= ia.x || ib.y <= ia.y {
                // No intersection.
                continue;
            }

            // Round extremities to physical pixels, ensuring that adjacent rectangles stay adjacent
            // at fractional scales.
            let ia = ia.to_physical_precise_round(scale);
            let ib = ib.to_physical_precise_round(scale);

            let r = Rectangle::from_extremities(ia, ib);

            // Intersect with each damage rect.
            for d in damage {
                if let Some(intersection) = r.intersection(*d) {
                    filtered.push(intersection);
                }
            }
        }
    }
}

niri_render_elements! {
    BackgroundEffectElement => {
        FramebufferEffect = FramebufferEffectElement,
        Xray = XrayElement,
        ExtraDamage = ExtraDamage,
    }
}

impl BackgroundEffect {
    pub fn new() -> Self {
        Self {
            nonxray: array::from_fn(|_| FramebufferEffect::new()),
            damage: ExtraDamage::new(),
            corner_radius: CornerRadius::default(),
            blur_config: niri_config::Blur::default(),
            options: Options::default(),
        }
    }

    pub fn update_config(&mut self, config: niri_config::Blur) {
        if self.blur_config == config {
            return;
        }

        self.blur_config = config;
        self.damage.damage_all();
    }

    pub fn update_render_elements(
        &mut self,
        corner_radius: CornerRadius,
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

        // FIXME: do we also need to damage when subregion changes? Then we'll need to pass
        // subregion in update_render_elements().
        if self.options == options && self.corner_radius == corner_radius {
            return;
        }

        self.options = options;
        self.corner_radius = corner_radius;
        self.damage.damage_all();
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

        if let Some(clip) = &mut params.clip {
            clip.1 = self.corner_radius;
        }
        params.fit_clip_radius();

        let damage = self.damage.render(params.geometry);

        // Use noise/saturation from options, falling back to blur defaults if blurred, and
        // to no effect if not blurred.
        let blur = self.options.blur && !self.blur_config.off;
        let blur_options = blur.then_some(BlurOptions::from(self.blur_config));
        let noise = if blur { self.blur_config.noise } else { 0. };
        let noise = self.options.noise.unwrap_or(noise) as f32;
        let saturation = if blur {
            self.blur_config.saturation
        } else {
            1.
        };
        let saturation = self.options.saturation.unwrap_or(saturation) as f32;

        if self.options.xray {
            let Some(xray) = ctx.xray else {
                return;
            };

            push(damage.into());
            xray.render(ctx, params, blur, noise, saturation, &mut |elem| {
                push(elem.into())
            });
        } else {
            // Render non-xray effect.
            let elem = &self.nonxray[ctx.target as usize];
            if let Some(elem) = elem.render(ctx.renderer, params, blur_options, noise, saturation) {
                push(damage.into());
                push(elem.into());
            }
        }
    }
}
