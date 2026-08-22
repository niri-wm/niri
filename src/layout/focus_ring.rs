use std::iter::zip;

use niri_config::{BorderWidth, CornerRadius, Gradient, GradientRelativeTo};
use smithay::backend::renderer::element::{Element as _, Kind};
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::niri_render_elements;
use crate::render_helpers::border::BorderRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};

#[derive(Debug)]
pub struct FocusRing {
    buffers: [SolidColorBuffer; 8],
    locations: [Point<f64, Logical>; 8],
    sizes: [Size<f64, Logical>; 8],
    borders: [BorderRenderElement; 8],
    full_size: Size<f64, Logical>,
    draw_as_border: bool,
    use_border_shader: bool,
    config: niri_config::FocusRing,
    thicken_corners: bool,
}

niri_render_elements! {
    FocusRingRenderElement => {
        SolidColor = SolidColorRenderElement,
        Gradient = BorderRenderElement,
    }
}

impl FocusRing {
    pub fn new(config: niri_config::FocusRing) -> Self {
        Self {
            buffers: Default::default(),
            locations: Default::default(),
            sizes: Default::default(),
            borders: Default::default(),
            full_size: Default::default(),
            draw_as_border: false,
            use_border_shader: false,
            config,
            thicken_corners: true,
        }
    }

    pub fn update_config(&mut self, config: niri_config::FocusRing) {
        self.config = config;
    }

    pub fn update_shaders(&mut self) {
        for elem in &mut self.borders {
            elem.damage_all();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_render_elements(
        &mut self,
        win_size: Size<f64, Logical>,
        is_active: bool,
        is_border: bool,
        is_urgent: bool,
        view_rect: Rectangle<f64, Logical>,
        radius: CornerRadius,
        scale: f64,
        alpha: f32,
    ) {
        // `width`: the actual pixel width of the border, not counting
        //     inset/outset - always >= 0
        // `inset`: how far inwards into the window content we move the border
        //     drawing algorithm
        // `inner_size`: size of content completely uncovered by the border;
        //     may be smaller than `win_size` if we draw the border *over* some of
        //     the content
        let (width, inset, is_inset) = match self.config.width {
            BorderWidth::Inset(width) => {
                // border exists inside the window content;
                // it must not be bigger than the window size
                let width = width.min(win_size.w / 2.).min(win_size.h / 2.);
                (width, width, true)
            }
            BorderWidth::Outset(width) => {
                // border exists outside the window content
                (width, 0., false)
            }
        };
        let inner_size = win_size - Size::from((inset, inset)).upscale(2.);
        let offset = Point::from((inset, inset));

        self.full_size = inner_size + Size::from((width, width)).upscale(2.);
        let draw_as_border = is_border || is_inset;
        self.draw_as_border = draw_as_border;

        let color = if is_urgent {
            self.config.urgent_color
        } else if is_active {
            self.config.active_color
        } else {
            self.config.inactive_color
        };

        for buf in &mut self.buffers {
            buf.set_color(color);
        }

        let radius = radius.fit_to(self.full_size.w as f32, self.full_size.h as f32);

        let gradient = if is_urgent {
            self.config.urgent_gradient
        } else if is_active {
            self.config.active_gradient
        } else {
            self.config.inactive_gradient
        };

        self.use_border_shader = radius != CornerRadius::default() || gradient.is_some();

        // Set the defaults for solid color + rounded corners.
        let gradient = gradient.unwrap_or_else(|| Gradient::from(color));

        let full_rect = Rectangle::new(Point::from((-width, -width)) + offset, self.full_size);
        let gradient_area = match gradient.relative_to {
            GradientRelativeTo::Window => full_rect,
            GradientRelativeTo::WorkspaceView => view_rect,
        };

        let rounded_corner_border_width = if draw_as_border {
            // HACK: increase the border width used for the inner rounded corners a tiny bit to
            // reduce background bleed.
            let extra = if self.thicken_corners { 0.5 } else { 0. };
            width as f32 + extra
        } else {
            0.
        };

        let ceil = |logical: f64| (logical * scale).ceil() / scale;

        // All of this stuff should end up aligned to physical pixels because:
        // * Window size and border width are rounded to physical pixels before being passed to this
        //   function.
        // * We will ceil the corner radii below.
        // * We do not divide anything, only add, subtract and multiply by integers.
        // * At rendering time, tile positions are rounded to physical pixels.

        if draw_as_border {
            let top_left = f64::max(width, ceil(f64::from(radius.top_left)));
            let top_right = f64::min(
                self.full_size.w - top_left,
                f64::max(width, ceil(f64::from(radius.top_right))),
            );
            let bottom_left = f64::min(
                self.full_size.h - top_left,
                f64::max(width, ceil(f64::from(radius.bottom_left))),
            );
            let bottom_right = f64::min(
                self.full_size.h - top_right,
                f64::min(
                    self.full_size.w - bottom_left,
                    f64::max(width, ceil(f64::from(radius.bottom_right))),
                ),
            );

            // Top edge.
            self.sizes[0] = Size::from((inner_size.w + width * 2. - top_left - top_right, width));
            self.locations[0] = offset + Point::from((-width + top_left, -width));

            // Bottom edge.
            self.sizes[1] = Size::from((
                inner_size.w + width * 2. - bottom_left - bottom_right,
                width,
            ));
            self.locations[1] = offset + Point::from((-width + bottom_left, inner_size.h));

            // Left edge.
            self.sizes[2] = Size::from((width, inner_size.h + width * 2. - top_left - bottom_left));
            self.locations[2] = offset + Point::from((-width, -width + top_left));

            // Right edge.
            self.sizes[3] =
                Size::from((width, inner_size.h + width * 2. - top_right - bottom_right));
            self.locations[3] = offset + Point::from((inner_size.w, -width + top_right));

            // Top-left corner.
            self.sizes[4] = Size::from((top_left, top_left));
            self.locations[4] = offset + Point::from((-width, -width));

            // Top-right corner.
            self.sizes[5] = Size::from((top_right, top_right));
            self.locations[5] = offset + Point::from((inner_size.w + width - top_right, -width));

            // Bottom-right corner.
            self.sizes[6] = Size::from((bottom_right, bottom_right));
            self.locations[6] = offset
                + Point::from((
                    inner_size.w + width - bottom_right,
                    inner_size.h + width - bottom_right,
                ));

            // Bottom-left corner.
            self.sizes[7] = Size::from((bottom_left, bottom_left));
            self.locations[7] = offset + Point::from((-width, inner_size.h + width - bottom_left));

            for (buf, size) in zip(&mut self.buffers, self.sizes) {
                buf.resize(size);
            }

            for (border, (loc, size)) in zip(&mut self.borders, zip(self.locations, self.sizes)) {
                border.update(
                    size,
                    Rectangle::new(gradient_area.loc - loc, gradient_area.size),
                    gradient.in_,
                    gradient.from,
                    gradient.to,
                    ((gradient.angle as f32) - 90.).to_radians(),
                    Rectangle::new(full_rect.loc - loc, full_rect.size),
                    rounded_corner_border_width,
                    radius,
                    scale as f32,
                    alpha,
                );
            }
        } else {
            self.sizes[0] = self.full_size;
            self.buffers[0].resize(self.sizes[0]);
            self.locations[0] = offset + Point::from((-width, -width));

            self.borders[0].update(
                self.sizes[0],
                Rectangle::new(gradient_area.loc - self.locations[0], gradient_area.size),
                gradient.in_,
                gradient.from,
                gradient.to,
                ((gradient.angle as f32) - 90.).to_radians(),
                Rectangle::new(full_rect.loc - self.locations[0], full_rect.size),
                rounded_corner_border_width,
                radius,
                scale as f32,
                alpha,
            );
        }
    }

    pub fn render(
        &self,
        renderer: &mut impl NiriRenderer,
        location: Point<f64, Logical>,
        push: &mut dyn FnMut(FocusRingRenderElement),
    ) {
        if self.config.off {
            return;
        }

        // If drawing as a border with width = 0, then there's nothing to draw.
        let width = self.config.width.pixels();
        if self.draw_as_border && width == 0. {
            return;
        }

        let has_border_shader = BorderRenderElement::has_shader(renderer);

        let mut push = |buffer, border: &BorderRenderElement, location: Point<f64, Logical>| {
            let elem = if self.use_border_shader && has_border_shader {
                border.clone().with_location(location).into()
            } else {
                let alpha = border.alpha();
                SolidColorRenderElement::from_buffer(buffer, location, alpha, Kind::Unspecified)
                    .into()
            };
            push(elem);
        };

        if self.draw_as_border {
            for ((buf, border), loc) in zip(zip(&self.buffers, &self.borders), self.locations) {
                push(buf, border, location + loc);
            }
        } else {
            push(
                &self.buffers[0],
                &self.borders[0],
                location + self.locations[0],
            );
        }
    }

    pub fn width(&self) -> BorderWidth {
        self.config.width
    }

    pub fn is_off(&self) -> bool {
        self.config.off
    }

    pub fn set_thicken_corners(&mut self, value: bool) {
        self.thicken_corners = value;
    }

    pub fn config(&self) -> &niri_config::FocusRing {
        &self.config
    }
}
