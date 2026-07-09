use std::iter::zip;

use niri_config::CornerRadius;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::shadow::ShadowRenderElement;

#[derive(Debug)]
pub struct Shadow {
    shader_rects: Vec<Rectangle<f64, Logical>>,
    shaders: Vec<ShadowRenderElement>,
    config: niri_config::Shadow,
}

impl Shadow {
    pub fn new(config: niri_config::Shadow) -> Self {
        Self {
            shader_rects: Vec::new(),
            shaders: Vec::new(),
            config,
        }
    }

    pub fn update_config(&mut self, config: niri_config::Shadow) {
        self.config = config;
    }

    pub fn update_shaders(&mut self) {
        for elem in &mut self.shaders {
            elem.damage_all();
        }
    }

    pub fn update_render_elements(
        &mut self,
        win_size: Size<f64, Logical>,
        is_active: bool,
        radius: CornerRadius,
        scale: f64,
        alpha: f32,
    ) {
        let ceil = |logical: f64| (logical * scale).ceil() / scale;

        // All of this stuff should end up aligned to physical pixels because:
        // * Window size is rounded to physical pixels before being passed to this function.
        // * We will ceil the corner radii below.
        // * We do not divide anything, only add, subtract and multiply by integers.
        // * At rendering time, tile positions are rounded to physical pixels.

        let width = self.config.softness;
        // Like in CSS box-shadow.
        let sigma = width / 2.;
        // Adjust width to draw all necessary pixels.
        let width = ceil(sigma * 3.);

        let offset = self.config.offset;
        let offset = Point::from((ceil(offset.x.0), ceil(offset.y.0)));

        let spread = self.config.spread;
        let spread = ceil(spread.abs()).copysign(spread);
        let offset = offset - Point::from((spread, spread));

        let win_radius = radius.fit_to(win_size.w as f32, win_size.h as f32);

        let box_size = if spread >= 0. {
            win_size + Size::from((spread, spread)).upscale(2.)
        } else {
            // This is a saturating sub.
            win_size - Size::from((-spread, -spread)).upscale(2.)
        };
        let radius = win_radius.expanded_by(spread as f32);

        let shader_size = box_size + Size::from((width, width)).upscale(2.);

        let color = if is_active {
            self.config.color
        } else {
            // Default to slightly more transparent.
            self.config
                .inactive_color
                .unwrap_or(self.config.color * 0.75)
        };

        let shader_geo = Rectangle::new(Point::from((-width, -width)), shader_size);

        // This is actually offset relative to shader_geo, this is handled below.
        let window_geo = Rectangle::new(Point::from((0., 0.)), win_size);

        if !self.config.draw_behind_window {
            let top_left = ceil(f64::from(win_radius.top_left));
            let top_right = f64::min(win_size.w - top_left, ceil(f64::from(win_radius.top_right)));
            let bottom_left = f64::min(
                win_size.h - top_left,
                ceil(f64::from(win_radius.bottom_left)),
            );
            let bottom_right = f64::min(
                win_size.h - top_right,
                f64::min(
                    win_size.w - bottom_left,
                    ceil(f64::from(win_radius.bottom_right)),
                ),
            );

            let top_left = Rectangle::new(Point::from((0., 0.)), Size::from((top_left, top_left)));
            let top_right = Rectangle::new(
                Point::from((win_size.w - top_right, 0.)),
                Size::from((top_right, top_right)),
            );
            let bottom_right = Rectangle::new(
                Point::from((win_size.w - bottom_right, win_size.h - bottom_right)),
                Size::from((bottom_right, bottom_right)),
            );
            let bottom_left = Rectangle::new(
                Point::from((0., win_size.h - bottom_left)),
                Size::from((bottom_left, bottom_left)),
            );

            let mut background =
                window_geo.subtract_rects([top_left, top_right, bottom_right, bottom_left]);
            for rect in &mut background {
                rect.loc -= offset;
            }

            self.shader_rects = shader_geo.subtract_rects(background);
            self.shaders
                .resize_with(self.shader_rects.len(), Default::default);

            for (shader, rect) in zip(&mut self.shaders, &mut self.shader_rects) {
                shader.update(
                    rect.size,
                    Rectangle::new(rect.loc.upscale(-1.), box_size),
                    color,
                    sigma as f32,
                    radius,
                    scale as f32,
                    Rectangle::new(window_geo.loc - offset - rect.loc, window_geo.size),
                    win_radius,
                    alpha,
                );

                rect.loc += offset;
            }
        } else {
            self.shader_rects.resize_with(1, Default::default);
            self.shader_rects[0] = shader_geo;

            self.shaders.resize_with(1, Default::default);
            self.shaders[0].update(
                shader_geo.size,
                Rectangle::new(shader_geo.loc.upscale(-1.), box_size),
                color,
                sigma as f32,
                radius,
                scale as f32,
                Rectangle::zero(),
                Default::default(),
                alpha,
            );

            self.shader_rects[0].loc += offset;
        }
    }

    pub fn render(
        &self,
        renderer: &mut impl NiriRenderer,
        location: Point<f64, Logical>,
        push: &mut dyn FnMut(ShadowRenderElement),
    ) {
        if !self.config.on {
            return;
        }

        let has_shadow_shader = ShadowRenderElement::has_shader(renderer);
        if !has_shadow_shader {
            return;
        }

        for (shader, rect) in zip(&self.shaders, &self.shader_rects) {
            push(shader.clone().with_location(location + rect.loc));
        }
    }
}
