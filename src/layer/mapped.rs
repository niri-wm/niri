use std::cell::RefCell;

use niri_config::utils::MergeWith as _;
use niri_config::{Config, LayerRule};
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::{LayerSurface, PopupManager};
use smithay::utils::{Logical, Point, Rectangle, Scale, Size};
use smithay::wayland::shell::wlr_layer::{ExclusiveZone, Layer};

use super::layer_animation::LayerAnimation;
use super::layer_animation::LayerAnimationRenderElement;
use super::ResolvedLayerRules;
use crate::animation::Animation as AnimationTrait;
use crate::animation::Clock;
use crate::layout::shadow::Shadow;
use crate::niri_render_elements;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::shadow::ShadowRenderElement;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use crate::render_helpers::surface::push_elements_from_surface_tree;
use crate::render_helpers::RenderTarget;
use crate::utils::{baba_is_float_offset, round_logical_in_physical};

#[derive(Debug)]
pub struct MappedLayer {
    /// The surface itself.
    surface: LayerSurface,

    /// Up-to-date rules.
    rules: ResolvedLayerRules,

    /// Buffer to draw instead of the surface when it should be blocked out.
    block_out_buffer: SolidColorBuffer,

    /// The shadow around the surface.
    shadow: Shadow,

    /// The view size for the layer surface's output.
    view_size: Size<f64, Logical>,

    /// Scale of the output the layer surface is on (and rounds its sizes to).
    scale: f64,

    /// Clock for driving animations.
    clock: Clock,

    /// Open animation state.
    open_animation: Option<LayerAnimation>,

    /// Close animation state.
    close_animation: Option<LayerAnimation>,

    closing_geometry: Option<Rectangle<i32, Logical>>,

    close_snapshot: RefCell<Option<Vec<WaylandSurfaceRenderElement<GlesRenderer>>>>,

    is_closing: bool,
}

niri_render_elements! {
    LayerSurfaceRenderElement<R> => {
        Wayland = WaylandSurfaceRenderElement<R>,
        SolidColor = SolidColorRenderElement,
        Shadow = ShadowRenderElement,
        Animation = LayerAnimationRenderElement,
    }
}

impl MappedLayer {
    pub fn new(
        surface: LayerSurface,
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
            rules,
            block_out_buffer: SolidColorBuffer::new((0., 0.), [0., 0., 0., 1.]),
            view_size,
            scale,
            shadow: Shadow::new(shadow_config),
            clock,
            open_animation: None,
            close_animation: None,
            closing_geometry: None,
            close_snapshot: RefCell::new(None),
            is_closing: false,
        }
    }

    pub fn update_config(&mut self, config: &Config) {
        let mut shadow_config = config.layout.shadow;
        // Shadows for layer surfaces need to be explicitly enabled.
        shadow_config.on = false;
        shadow_config.merge_with(&self.rules.shadow);
        self.shadow.update_config(shadow_config);
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

    pub fn are_animations_ongoing(&self) -> bool {
        self.open_animation.is_some() || self.close_animation.is_some() || self.rules.baba_is_float
    }

    pub fn start_open_animation(&mut self, config: &niri_config::Animations) {
        self.open_animation = Some(LayerAnimation::new_open(AnimationTrait::new(
            self.clock.clone(),
            0.,
            1.,
            0.,
            config.layer_open.anim.clone(),
        )));
    }

    pub fn start_close_animation(&mut self, config: &niri_config::Animations) {
        self.close_animation = Some(LayerAnimation::new_close(AnimationTrait::new(
            self.clock.clone(),
            1.,
            0.,
            0.,
            config.layer_close.anim.clone(),
        )));
    }

    pub fn capture_close_snapshot(&self, renderer: &mut GlesRenderer) {
        if self.close_snapshot.borrow().is_some() {
            return;
        }

        let _span = tracy_client::span!("MappedLayer::capture_close_snapshot");

        let scale = Scale::from(self.scale);
        let alpha = self.rules.opacity.unwrap_or(1.).clamp(0., 1.) as f32;
        let location = Point::from((0., 0.));
        let buf_pos = location + self.bob_offset();

        let mut elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
        let surface = self.surface.wl_surface();
        push_elements_from_surface_tree(
            renderer,
            surface,
            buf_pos.to_physical_precise_round(scale),
            scale,
            alpha,
            Kind::ScanoutCandidate,
            &mut |elem| elements.push(elem),
        );

        *self.close_snapshot.borrow_mut() = Some(elements);
    }

    pub fn set_closing(&mut self, closing: bool) {
        self.is_closing = closing;
    }

    pub fn set_closing_geometry(&mut self, geo: Rectangle<i32, Logical>) {
        self.closing_geometry = Some(geo);
    }

    pub fn closing_geometry(&self) -> Option<Rectangle<i32, Logical>> {
        self.closing_geometry
    }

    pub fn clear_close_animation(&mut self) {
        self.close_animation = None;
        self.is_closing = false;
        self.closing_geometry = None;
        *self.close_snapshot.borrow_mut() = None;
    }

    pub fn should_remove(&self) -> bool {
        self.is_closing && self.close_animation.is_none()
    }

    pub fn is_close_animation_done(&self) -> bool {
        self.close_animation.is_none()
    }

    pub fn advance_animations(&mut self) {
        if let Some(open) = &mut self.open_animation {
            if open.is_done() {
                self.open_animation = None;
            }
        }

        if let Some(close) = &mut self.close_animation {
            if close.is_done() {
                self.close_animation = None;
            }
        }
    }

    pub fn surface(&self) -> &LayerSurface {
        &self.surface
    }

    pub fn rules(&self) -> &ResolvedLayerRules {
        &self.rules
    }

    pub fn scale(&self) -> f64 {
        self.scale
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
        renderer: &mut R,
        location: Point<f64, Logical>,
        target: RenderTarget,
        push: &mut dyn FnMut(LayerSurfaceRenderElement<R>),
    ) {
        let scale = Scale::from(self.scale);
        let alpha = self.rules.opacity.unwrap_or(1.).clamp(0., 1.);
        let location = location + self.bob_offset();

        if target.should_block_out(self.rules.block_out_from) {
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
            let buf_pos = location;

            let surface = self.surface.wl_surface();
            push_elements_from_surface_tree(
                renderer,
                surface,
                buf_pos.to_physical_precise_round(scale),
                scale,
                alpha,
                Kind::ScanoutCandidate,
                &mut |elem| push(elem.into()),
            );

            let mut snapshot = Vec::new();
            let gles_renderer = renderer.as_gles_renderer();
            push_elements_from_surface_tree(
                gles_renderer,
                surface,
                buf_pos.to_physical_precise_round(scale),
                scale,
                alpha as f32,
                Kind::ScanoutCandidate,
                &mut |elem| snapshot.push(elem),
            );
            *self.close_snapshot.borrow_mut() = Some(snapshot);
        }

        let location = location.to_physical_precise_round(scale).to_logical(scale);
        self.shadow
            .render(renderer, location, &mut |elem| push(elem.into()));
    }

    pub fn render_popups<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        location: Point<f64, Logical>,
        target: RenderTarget,
        push: &mut dyn FnMut(LayerSurfaceRenderElement<R>),
    ) {
        let scale = Scale::from(self.scale);
        let alpha = self.rules.opacity.unwrap_or(1.).clamp(0., 1.);
        let location = location + self.bob_offset();

        if target.should_block_out(self.rules.block_out_from) {
            return;
        }

        // Layer surfaces don't have extra geometry like windows.
        let buf_pos = location;

        let surface = self.surface.wl_surface();
        for (popup, popup_offset) in PopupManager::popups_for_surface(surface) {
            // Layer surfaces don't have extra geometry like windows.
            let offset = popup_offset - popup.geometry().loc;

            push_elements_from_surface_tree(
                renderer,
                popup.wl_surface(),
                (buf_pos + offset.to_f64()).to_physical_precise_round(scale),
                scale,
                alpha,
                Kind::ScanoutCandidate,
                &mut |elem| push(elem.into()),
            );
        }
    }

    /// Render with animation if an open or close animation is ongoing.
    ///
    /// Returns true if animation was rendered, false to indicate fallback to normal rendering needed.
    pub fn render_with_animation<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        location: Point<f64, Logical>,
        scale: Scale<f64>,
        target: RenderTarget,
        geo_size: Size<f64, Logical>,
        push: &mut dyn FnMut(LayerSurfaceRenderElement<R>),
    ) -> bool {
        let animation = self
            .open_animation
            .as_ref()
            .or(self.close_animation.as_ref());

        let Some(animation) = animation else {
            return false;
        };

        let alpha = self.rules.opacity.unwrap_or(1.).clamp(0., 1.) as f32;
        let location = location + self.bob_offset();

        // Blocked out layers fall back to normal rendering
        if target.should_block_out(self.rules.block_out_from) {
            return false;
        }

        let gles_renderer = renderer.as_gles_renderer();

        let mut live_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();

        if self.close_animation.is_some() {
            if self.close_snapshot.borrow().is_none() {
                self.capture_close_snapshot(gles_renderer);
            }

            let snapshot = self.close_snapshot.borrow();
            if let Some(snapshot) = snapshot.as_ref() {
                match animation.render(
                    gles_renderer,
                    snapshot.as_slice(),
                    geo_size,
                    location,
                    scale,
                    alpha,
                ) {
                    Ok((elem, _data)) => {
                        push(elem.into());
                        return true;
                    }
                    Err(err) => {
                        warn!("error rendering layer close animation: {:?}", err);
                        return false;
                    }
                }
            } else {
                return false;
            }
        }

        let buf_pos = location;
        let surface = self.surface.wl_surface();
        push_elements_from_surface_tree(
            gles_renderer,
            surface,
            buf_pos.to_physical_precise_round(scale),
            scale,
            alpha,
            Kind::ScanoutCandidate,
            &mut |e| live_elements.push(e),
        );

        match animation.render(
            gles_renderer,
            &live_elements,
            geo_size,
            location,
            scale,
            alpha,
        ) {
            Ok((elem, _data)) => {
                push(elem.into());
                true
            }
            Err(err) => {
                warn!("error rendering layer animation: {:?}", err);
                false
            }
        }
    }
}
