use std::rc::Rc;
use std::time::Duration;

use niri::layout::Options;
use niri::render_helpers::RenderTarget;
use niri_config::Color;
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Rectangle, Size};

use super::{Args, TestCase};
use crate::test_window::TestWindow;

pub struct Tile {
    window: TestWindow,
    tile: niri::layout::tile::Tile<TestWindow>,
}

impl Tile {
    pub fn freeform(args: Args) -> Self {
        let window = TestWindow::freeform(0);
        Self::with_window(args, window)
    }

    pub fn fixed_size(args: Args) -> Self {
        let window = TestWindow::fixed_size(0);
        Self::with_window(args, window)
    }

    pub fn fixed_size_with_csd_shadow(args: Args) -> Self {
        let window = TestWindow::fixed_size(0);
        window.set_csd_shadow_width(64);
        Self::with_window(args, window)
    }

    pub fn freeform_open(args: Args) -> Self {
        let mut rv = Self::freeform(args);
        rv.window.set_color([0.1, 0.1, 0.1, 1.]);
        rv.tile.start_open_animation();
        rv
    }

    pub fn fixed_size_open(args: Args) -> Self {
        let mut rv = Self::fixed_size(args);
        rv.window.set_color([0.1, 0.1, 0.1, 1.]);
        rv.tile.start_open_animation();
        rv
    }

    pub fn fixed_size_with_csd_shadow_open(args: Args) -> Self {
        let mut rv = Self::fixed_size_with_csd_shadow(args);
        rv.window.set_color([0.1, 0.1, 0.1, 1.]);
        rv.tile.start_open_animation();
        rv
    }

    pub fn with_window(args: Args, window: TestWindow) -> Self {
        let Args { size, clock } = args;

        let options = Options {
            layout: niri_config::Layout {
                focus_ring: niri_config::FocusRing {
                    off: true,
                    ..Default::default()
                },
                border: niri_config::Border {
                    off: false,
                    width: 32.,
                    active_color: Color::from_rgba8_unpremul(255, 163, 72, 255),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let mut tile = niri::layout::tile::Tile::new(
            window.clone(),
            size.to_f64(),
            1.,
            clock,
            Rc::new(options),
        );

        tile.request_tile_size(size.to_f64(), false, None);
        window.communicate();

        Self { window, tile }
    }
}

impl TestCase for Tile {
    fn resize(&mut self, width: i32, height: i32) {
        let size = Size::from((width, height)).to_f64();
        self.tile
            .update_config(size, 1., self.tile.options().clone());
        self.tile.request_tile_size(size, false, None);
        self.window.communicate();
    }

    fn are_animations_ongoing(&self) -> bool {
        self.tile.are_animations_ongoing()
    }

    fn advance_animations(&mut self, _current_time: Duration) {
        self.tile.advance_animations();
    }

    fn render(
        &mut self,
        renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Vec<Box<dyn RenderElement<GlesRenderer>>> {
        let size = size.to_f64();
        let tile_size = self.tile.tile_size().to_physical(1.);
        let location = Point::from((size.w - tile_size.w, size.h - tile_size.h)).downscale(2.);

        self.tile.update_render_elements(
            true,
            Rectangle::new(Point::from((-location.x, -location.y)), size.to_logical(1.)),
        );

        let mut rv = Vec::new();
        self.tile.render(
            renderer,
            location,
            true,
            RenderTarget::Output,
            &mut |elem| rv.push(Box::new(elem) as _),
        );
        rv
    }
}
