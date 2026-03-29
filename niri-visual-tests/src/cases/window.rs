use niri::layout::{LayoutElement, SizingMode};
use niri::render_helpers::RenderTarget;
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Scale, Size};

use super::{Args, TestCase};
use crate::test_window::TestWindow;

pub struct Window {
    window: TestWindow,
}

impl Window {
    pub fn freeform(args: Args) -> Self {
        let mut window = TestWindow::freeform(0);
        window.request_size(args.size, SizingMode::Normal, false, None);
        window.communicate();
        Self { window }
    }

    pub fn fixed_size(args: Args) -> Self {
        let mut window = TestWindow::fixed_size(0);
        window.request_size(args.size, SizingMode::Normal, false, None);
        window.communicate();
        Self { window }
    }

    pub fn fixed_size_with_csd_shadow(args: Args) -> Self {
        let mut window = TestWindow::fixed_size(0);
        window.set_csd_shadow_width(64);
        window.request_size(args.size, SizingMode::Normal, false, None);
        window.communicate();
        Self { window }
    }
}

impl TestCase for Window {
    fn resize(&mut self, width: i32, height: i32) {
        self.window
            .request_size(Size::from((width, height)), SizingMode::Normal, false, None);
        self.window.communicate();
    }

    fn render(
        &mut self,
        renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Vec<Box<dyn RenderElement<GlesRenderer>>> {
        let win_size = self.window.size().to_physical(1);
        let location = Point::from((size.w - win_size.w, size.h - win_size.h))
            .to_f64()
            .downscale(2.);

        let mut rv = Vec::new();
        self.window.render_normal(
            renderer,
            location,
            Scale::from(1.),
            1.,
            RenderTarget::Output,
            &mut |elem| rv.push(Box::new(elem) as _),
        );
        rv
    }
}
