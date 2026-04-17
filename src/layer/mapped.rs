use std::cell::{Ref, RefCell};

use niri_config::animations::LayerOpenAnim;
use niri_config::utils::MergeWith as _;
use niri_config::{Config, LayerRule};
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::{LayerSurface, PopupKind, PopupManager};
use smithay::utils::{Logical, Point, Rectangle, Scale, Size};
use smithay::wayland::compositor::{remove_pre_commit_hook, HookId};
use smithay::wayland::shell::wlr_layer::{ExclusiveZone, Layer};

use super::ResolvedLayerRules;
use crate::animation::{Animation, Clock};
use crate::layer::closing_layer::ClosingLayerRenderElement;
use crate::layer::opening_layer::{OpenAnimation, OpeningLayerRenderElement};
use crate::layout::shadow::Shadow;
use crate::niri_render_elements;
use crate::render_helpers::background_effect::BackgroundEffectElement;
use crate::render_helpers::offscreen::OffscreenData;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::shaders::ProgramType;
use crate::render_helpers::shadow::ShadowRenderElement;
use crate::render_helpers::snapshot::RenderSnapshot;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use crate::render_helpers::surface::push_elements_from_surface_tree;
use crate::render_helpers::xray::XrayPos;
use crate::render_helpers::{background_effect, encompassing_geo, RenderCtx, RenderTarget};
use crate::utils::{baba_is_float_offset, round_logical_in_physical};

#[derive(Debug)]
pub struct MappedLayer {
    /// The surface itself.
    surface: LayerSurface,

    /// Pre-commit hook that we have on all mapped layer surfaces.
    pre_commit_hook: HookId,

    /// Up-to-date rules.
    rules: ResolvedLayerRules,

    /// Whether to recompute layer rules on the next commit.
    ///
    /// Set in the pre-commit hook when the layer changes; consumed in the commit handler.
    recompute_rules_on_commit: bool,

    /// Buffer to draw instead of the surface when it should be blocked out.
    block_out_buffer: SolidColorBuffer,

    /// The shadow around the surface.
    shadow: Shadow,

    /// The blur config, passed for background effect rendering.
    blur_config: niri_config::Blur,

    /// The view size for the layer surface's output.
    view_size: Size<f64, Logical>,

    /// Scale of the output the layer surface is on (and rounds its sizes to).
    scale: f64,

    /// The animation upon opening a layer.
    open_animation: Option<OpenAnimation>,

    /// Offscreen state from the current frame's opening animation render.
    offscreen_data: RefCell<Option<OffscreenData>>,

    /// The animation upon closing a layer.
    unmap_snapshot: Option<LayerSurfaceRenderSnapshot>,

    /// Clock for driving animations.
    clock: Clock,
}

niri_render_elements! {
    LayerSurfaceRenderElement<R> => {
        Wayland = WaylandSurfaceRenderElement<R>,
        SolidColor = SolidColorRenderElement,
        Shadow = ShadowRenderElement,
        BackgroundEffect = BackgroundEffectElement,
        Opening = OpeningLayerRenderElement,
        Closing = ClosingLayerRenderElement,
    }
}

pub type LayerSurfaceRenderSnapshot = RenderSnapshot<
    LayerSurfaceRenderElement<GlesRenderer>,
    LayerSurfaceRenderElement<GlesRenderer>,
>;

impl MappedLayer {
    pub fn new(
        surface: LayerSurface,
        pre_commit_hook: HookId,
        rules: ResolvedLayerRules,
        view_size: Size<f64, Logical>,
        scale: f64,
        clock: Clock,
        config: &Config,
    ) -> Self {
        let mut shadow_config = config.layout.shadow;
        // Shadows for layer surfaces need to be explicitly enabled.
        shadow_config.on = false;
        shadow_config.merge_with(&rules.shadow);

        Self {
            surface,
            pre_commit_hook,
            rules,
            recompute_rules_on_commit: false,
            block_out_buffer: SolidColorBuffer::new((0., 0.), [0., 0., 0., 1.]),
            view_size,
            scale,
            shadow: Shadow::new(shadow_config),
            open_animation: None,
            offscreen_data: RefCell::new(None),
            unmap_snapshot: None,
            blur_config: config.blur,
            clock,
        }
    }

    pub fn update_config(&mut self, config: &Config) {
        let mut shadow_config = config.layout.shadow;
        // Shadows for layer surfaces need to be explicitly enabled.
        shadow_config.on = false;
        shadow_config.merge_with(&self.rules.shadow);
        self.shadow.update_config(shadow_config);

        self.blur_config = config.blur;
    }

    pub fn update_shaders(&mut self) {
        self.shadow.update_shaders();
    }

    pub fn update_sizes(&mut self, view_size: Size<f64, Logical>, scale: f64) {
        self.view_size = view_size;
        self.scale = scale;
    }

    pub fn update_render_elements(&mut self, size: Size<f64, Logical>) {
        // Round to physical pixels.
        let size = size
            .to_physical_precise_round(self.scale)
            .to_logical(self.scale);

        self.block_out_buffer.resize(size);

        let radius = self.rules.geometry_corner_radius.unwrap_or_default();
        // FIXME: is_active based on keyboard focus?
        self.shadow
            .update_render_elements(size, true, radius, self.scale, 1.);
    }

    pub fn store_unmap_snapshot(&mut self, renderer: &mut GlesRenderer) {
        let _span = tracy_client::span!("MappedLayer::store_unmap_snapshot");
        let mut contents = Vec::new();
        self.render_normal_inner(
            RenderCtx {
                renderer,
                target: RenderTarget::Output,
                xray: None,
            },
            None,
            Point::from((0., 0.)),
            XrayPos::default(),
            RenderTarget::Output.should_block_out(self.rules.block_out_from),
            &mut |elem| contents.push(elem),
        );
        self.render_popups(
            RenderCtx {
                renderer,
                target: RenderTarget::Output,
                xray: None,
            },
            None,
            Point::from((0., 0.)),
            XrayPos::default(),
            &mut |elem| contents.push(elem),
        );

        let mut contents_with_blocked_out_bg = None;
        if self.rules.block_out_from.is_some() {
            let mut with_blocked_out_bg = Vec::new();
            self.render_normal_inner(
                RenderCtx {
                    renderer,
                    target: RenderTarget::Output,
                    xray: None,
                },
                None,
                Point::from((0., 0.)),
                XrayPos::default(),
                true,
                &mut |elem| with_blocked_out_bg.push(elem),
            );
            self.render_popups(
                RenderCtx {
                    renderer,
                    target: RenderTarget::Output,
                    xray: None,
                },
                None,
                Point::from((0., 0.)),
                XrayPos::default(),
                &mut |elem| with_blocked_out_bg.push(elem),
            );

            contents_with_blocked_out_bg = Some(with_blocked_out_bg);
        }

        // A bit of a hack to render blocked out as for screencast, but I think it's fine here as
        // well.
        let mut blocked_out_contents = Vec::new();
        self.render_normal_inner(
            RenderCtx {
                renderer,
                target: RenderTarget::Screencast,
                xray: None,
            },
            None,
            Point::from((0., 0.)),
            XrayPos::default(),
            RenderTarget::Screencast.should_block_out(self.rules.block_out_from),
            &mut |elem| blocked_out_contents.push(elem),
        );
        self.render_popups(
            RenderCtx {
                renderer,
                target: RenderTarget::Screencast,
                xray: None,
            },
            None,
            Point::from((0., 0.)),
            XrayPos::default(),
            &mut |elem| blocked_out_contents.push(elem),
        );

        let is_empty = contents.is_empty() && blocked_out_contents.is_empty();
        if is_empty {
            // Preserve the last good snapshot if this capture raced with content teardown.
            if self.unmap_snapshot.as_ref().is_some_and(|snapshot| {
                !snapshot.contents.is_empty() || !snapshot.blocked_out_contents.is_empty()
            }) {
                return;
            }

            // Keep this as missing so close-start can attempt one last capture.
            self.unmap_snapshot = None;
            return;
        }

        let size = self.surface.cached_state().size.to_f64();

        self.unmap_snapshot = Some(LayerSurfaceRenderSnapshot {
            contents,
            blocked_out_contents,
            contents_with_blocked_out_bg,
            block_out_from: self.rules.block_out_from,
            size,
            texture: Default::default(),
            texture_with_blocked_out_bg: Default::default(),
            blocked_out_texture: Default::default(),
        });
    }

    pub fn take_unmap_snapshot(&mut self) -> Option<LayerSurfaceRenderSnapshot> {
        self.unmap_snapshot.take()
    }

    pub fn offscreen_data(&self) -> Ref<'_, Option<OffscreenData>> {
        self.offscreen_data.borrow()
    }

    pub fn advance_animations(&mut self) {
        if self
            .open_animation
            .as_ref()
            .is_some_and(|open_anim| open_anim.is_done())
        {
            self.open_animation = None;
        }
    }

    pub fn start_open_animation(&mut self, anim_config: &LayerOpenAnim, program: ProgramType) {
        if self.open_animation.is_some() {
            return;
        }

        self.open_animation = Some(OpenAnimation::new(
            Animation::new(self.clock.clone(), 0., 1., 0., anim_config.anim),
            program,
        ));
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.rules.baba_is_float
            || self
                .open_animation
                .as_ref()
                .is_some_and(|open| !open.is_done())
    }

    pub fn surface(&self) -> &LayerSurface {
        &self.surface
    }

    pub fn rules(&self) -> &ResolvedLayerRules {
        &self.rules
    }

    /// Recomputes the resolved layer rules and returns whether they changed.
    pub fn recompute_layer_rules(&mut self, rules: &[LayerRule], is_at_startup: bool) -> bool {
        let new_rules = ResolvedLayerRules::compute(rules, &self.surface, is_at_startup);
        if new_rules == self.rules {
            return false;
        }

        self.rules = new_rules;
        true
    }

    pub fn set_recompute_rules_on_commit(&mut self) {
        self.recompute_rules_on_commit = true;
    }

    pub fn take_recompute_rules_on_commit(&mut self) -> bool {
        std::mem::take(&mut self.recompute_rules_on_commit)
    }

    pub fn place_within_backdrop(&self) -> bool {
        if !self.rules.place_within_backdrop {
            return false;
        }

        if self.surface.layer() != Layer::Background {
            return false;
        }

        let state = self.surface.cached_state();
        if state.exclusive_zone != ExclusiveZone::DontCare {
            return false;
        }

        true
    }

    pub fn bob_offset(&self) -> Point<f64, Logical> {
        if !self.rules.baba_is_float {
            return Point::from((0., 0.));
        }

        let y = baba_is_float_offset(self.clock.now(), self.view_size.h);
        let y = round_logical_in_physical(self.scale, y);
        Point::from((0., y))
    }

    pub fn render_normal<R: NiriRenderer>(
        &self,
        ctx: RenderCtx<R>,
        ns: Option<usize>,
        location: Point<f64, Logical>,
        xray_pos: XrayPos,
        push: &mut dyn FnMut(LayerSurfaceRenderElement<R>),
    ) {
        let scale = Scale::from(self.scale);
        let alpha = self.rules.opacity.unwrap_or(1.).clamp(0., 1.);
        let bob_offset = self.bob_offset();
        let location = location + bob_offset;
        let xray_pos = xray_pos.offset(bob_offset);
        let should_block_out = ctx.target.should_block_out(self.rules.block_out_from);

        self.set_offscreen_data(None);

        if let Some(open) = &self.open_animation {
            let mut elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
            push_elements_from_surface_tree(
                ctx.renderer.as_gles_renderer(),
                self.surface.wl_surface(),
                Point::from((0, 0)),
                scale,
                alpha,
                Kind::ScanoutCandidate,
                &mut |elem| elements.push(elem),
            );

            if !elements.is_empty() {
                let mut geo_size = self.surface.cached_state().size.to_f64();
                if geo_size.w <= 0. || geo_size.h <= 0. {
                    geo_size = encompassing_geo(scale, elements.iter())
                        .size
                        .to_f64()
                        .to_logical(scale);
                }

                if geo_size.w > 0. && geo_size.h > 0. {
                    match open.render(
                        ctx.renderer.as_gles_renderer(),
                        &elements,
                        geo_size,
                        location,
                        scale,
                        alpha,
                    ) {
                        Ok((elem, data)) => {
                            self.set_offscreen_data(Some(data));
                            push(elem.into());
                            return;
                        }
                        Err(err) => {
                            warn!("error rendering layer opening animation: {err:?}");
                        }
                    }
                }
            }
        }

        self.render_normal_inner(ctx, ns, location, xray_pos, should_block_out, push);
    }

    fn render_normal_inner<R: NiriRenderer>(
        &self,
        mut ctx: RenderCtx<R>,
        ns: Option<usize>,
        location: Point<f64, Logical>,
        xray_pos: XrayPos,
        should_block_out: bool,
        push: &mut dyn FnMut(LayerSurfaceRenderElement<R>),
    ) {
        let scale = Scale::from(self.scale);
        let alpha = self.rules.opacity.unwrap_or(1.).clamp(0., 1.);
        let surface = self.surface.wl_surface();

        if should_block_out {
            // Round to physical pixels.
            let location = location.to_physical_precise_round(scale).to_logical(scale);

            // FIXME: take geometry-corner-radius into account.
            let elem = SolidColorRenderElement::from_buffer(
                &self.block_out_buffer,
                location,
                alpha,
                Kind::Unspecified,
            );
            push(elem.into());
        } else {
            // Layer surfaces don't have extra geometry like windows.
            push_elements_from_surface_tree(
                ctx.renderer,
                surface,
                location.to_physical_precise_round(scale),
                scale,
                alpha,
                Kind::ScanoutCandidate,
                &mut |elem| push(elem.into()),
            );
        }

        let location = location.to_physical_precise_round(scale).to_logical(scale);
        self.shadow
            .render(ctx.renderer, location, &mut |elem| push(elem.into()));

        let geometry = Rectangle::new(location, self.block_out_buffer.size());
        let surface_off = Point::new(0., 0.); // No geometry on layer surfaces.
        let surface_anim_scale = Scale::from(1.);
        let radius = self.rules.geometry_corner_radius.unwrap_or_default();
        background_effect::render_for_tile(
            ctx.as_gles(),
            ns,
            geometry,
            self.scale,
            false,
            surface,
            surface_off,
            surface_anim_scale,
            self.blur_config,
            radius,
            self.rules.background_effect,
            should_block_out,
            xray_pos,
            &mut |elem| push(elem.into()),
        );
    }

    pub fn render_popups<R: NiriRenderer>(
        &self,
        mut ctx: RenderCtx<R>,
        ns: Option<usize>,
        location: Point<f64, Logical>,
        xray_pos: XrayPos,
        push: &mut dyn FnMut(LayerSurfaceRenderElement<R>),
    ) {
        if ctx.target.should_block_out(self.rules.block_out_from) {
            return;
        }

        let scale = Scale::from(self.scale);
        let alpha = self.rules.opacity.unwrap_or(1.).clamp(0., 1.);

        let bob_offset = self.bob_offset();
        let location = location + bob_offset;
        let xray_pos = xray_pos.offset(bob_offset);

        let surface = self.surface.wl_surface();
        for (popup, offset) in PopupManager::popups_for_surface(surface) {
            let popup_rules = match popup {
                PopupKind::Xdg(_) => self.rules.popups,
                // IME popups aren't affected by rules for regular popups.
                PopupKind::InputMethod(_) => niri_config::ResolvedPopupsRules::default(),
            };
            let alpha = alpha * popup_rules.opacity.unwrap_or(1.).clamp(0., 1.);

            let surface = popup.wl_surface();
            let popup_geo = popup.geometry();
            let surface_loc = location + (offset - popup_geo.loc).to_f64();

            push_elements_from_surface_tree(
                ctx.renderer,
                surface,
                surface_loc.to_physical_precise_round(scale),
                scale,
                alpha,
                Kind::ScanoutCandidate,
                &mut |elem| push(elem.into()),
            );

            let geometry = Rectangle::new(location + offset.to_f64(), popup_geo.size.to_f64());
            let surface_off = popup_geo.loc.upscale(-1).to_f64();
            let surface_anim_scale = Scale::from(1.);
            let mut effect = popup_rules.background_effect;
            // Default xray to false for pop-ups since they're always on top of something.
            if effect.xray.is_none() {
                effect.xray = Some(false);
            }
            let xray_pos = xray_pos.offset(offset.to_f64());
            background_effect::render_for_tile(
                ctx.as_gles(),
                ns,
                geometry,
                self.scale,
                false,
                surface,
                surface_off,
                surface_anim_scale,
                self.blur_config,
                popup_rules.geometry_corner_radius.unwrap_or_default(),
                effect,
                false,
                xray_pos,
                &mut |elem| push(elem.into()),
            );
        }
    }

    fn set_offscreen_data(&self, data: Option<OffscreenData>) {
        let Some(data) = data else {
            self.offscreen_data.replace(None);
            return;
        };

        let mut offscreen_data = self.offscreen_data.borrow_mut();
        match &mut *offscreen_data {
            None => {
                *offscreen_data = Some(data);
            }
            Some(existing) => {
                existing.id = data.id;
                existing.states.states.extend(data.states.states);
            }
        }
    }
}

impl Drop for MappedLayer {
    fn drop(&mut self) {
        remove_pre_commit_hook(self.surface.wl_surface(), &self.pre_commit_hook);
    }
}
