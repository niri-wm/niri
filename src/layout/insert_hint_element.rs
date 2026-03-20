use niri_config::CornerRadius;
use smithay::utils::{Logical, Point, Rectangle, Size};

use super::focus_ring::{FocusRing, FocusRingRenderElement};
use crate::render_helpers::renderer::NiriRenderer;

#[derive(Debug)]
pub struct InsertHintElement {
    inner: FocusRing,
}

pub type InsertHintRenderElement = FocusRingRenderElement;

impl InsertHintElement {
    pub fn new(config: niri_config::InsertHint) -> Self {
        Self {
            inner: FocusRing::new(niri_config::FocusRing {
                off: config.off,
                width: 0.,
                active_color: config.color,
                inactive_color: config.color,
                urgent_color: config.color,
                active_gradient: config.gradient,
                inactive_gradient: config.gradient,
                urgent_gradient: config.gradient,
            }),
        }
    }

    pub fn update_config(&mut self, config: niri_config::InsertHint) {
        self.inner.update_config(niri_config::FocusRing {
            off: config.off,
            width: 0.,
            active_color: config.color,
            inactive_color: config.color,
            urgent_color: config.color,
            active_gradient: config.gradient,
            inactive_gradient: config.gradient,
            urgent_gradient: config.gradient,
        });
    }

    pub fn update_shaders(&mut self) {
        self.inner.update_shaders();
    }

    pub fn update_render_elements(
        &mut self,
        size: Size<f64, Logical>,
        view_rect: Rectangle<f64, Logical>,
        radius: CornerRadius,
        scale: f64,
    ) {
        self.inner
            .update_render_elements(size, true, false, false, view_rect, radius, scale, 1.);
    }

    pub fn render(
        &self,
        renderer: &mut impl NiriRenderer,
        location: Point<f64, Logical>,
        push: &mut dyn FnMut(FocusRingRenderElement),
    ) {
        self.inner.render(renderer, location, push)
    }
}
