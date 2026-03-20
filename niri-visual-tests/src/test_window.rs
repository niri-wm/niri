use std::cell::RefCell;
use std::cmp::{max, min};
use std::rc::Rc;

use niri::layout::{
    ConfigureIntent, InteractiveResizeData, LayoutElement, LayoutElementRenderElement,
    LayoutElementRenderSnapshot, SizingMode,
};
use niri::render_helpers::offscreen::OffscreenData;
use niri::render_helpers::renderer::NiriRenderer;
use niri::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use niri::render_helpers::RenderTarget;
use niri::utils::transaction::Transaction;
use niri::window::ResolvedWindowRules;
use smithay::backend::renderer::element::Kind;
use smithay::output::{self, Output};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Scale, Serial, Size, Transform};

#[derive(Debug)]
struct TestWindowInner {
    size: Size<i32, Logical>,
    requested_size: Option<Size<i32, Logical>>,
    min_size: Size<i32, Logical>,
    max_size: Size<i32, Logical>,
    buffer: SolidColorBuffer,
    pending_sizing_mode: SizingMode,
    csd_shadow_width: i32,
    csd_shadow_buffer: SolidColorBuffer,
}

#[derive(Debug, Clone)]
pub struct TestWindow {
    id: usize,
    inner: Rc<RefCell<TestWindowInner>>,
    rules: ResolvedWindowRules,
}

impl TestWindow {
    pub fn freeform(id: usize) -> Self {
        let size = Size::from((100, 200));
        let min_size = Size::from((0, 0));
        let max_size = Size::from((0, 0));
        let buffer = SolidColorBuffer::new(size.to_f64(), [0.15, 0.64, 0.41, 1.]);

        Self {
            id,
            inner: Rc::new(RefCell::new(TestWindowInner {
                size,
                requested_size: None,
                min_size,
                max_size,
                buffer,
                pending_sizing_mode: SizingMode::Normal,
                csd_shadow_width: 0,
                csd_shadow_buffer: SolidColorBuffer::new((0., 0.), [0., 0., 0., 0.3]),
            })),
            rules: ResolvedWindowRules::default(),
        }
    }

    pub fn fixed_size(id: usize) -> Self {
        let rv = Self::freeform(id);
        rv.set_min_size((200, 400).into());
        rv.set_max_size((200, 400).into());
        rv.set_color([0.88, 0.11, 0.14, 1.]);
        rv.communicate();
        rv
    }

    pub fn set_min_size(&self, size: Size<i32, Logical>) {
        self.inner.borrow_mut().min_size = size;
    }

    pub fn set_max_size(&self, size: Size<i32, Logical>) {
        self.inner.borrow_mut().max_size = size;
    }

    pub fn set_color(&self, color: [f32; 4]) {
        self.inner.borrow_mut().buffer.set_color(color);
    }

    pub fn set_csd_shadow_width(&self, width: i32) {
        self.inner.borrow_mut().csd_shadow_width = width;
    }

    pub fn communicate(&self) -> bool {
        let mut rv = false;
        let mut inner = self.inner.borrow_mut();

        let mut new_size = inner.size;

        if let Some(size) = inner.requested_size {
            assert!(size.w >= 0);
            assert!(size.h >= 0);

            if size.w != 0 {
                new_size.w = size.w;
            }
            if size.h != 0 {
                new_size.h = size.h;
            }
        }

        if inner.max_size.w > 0 {
            new_size.w = min(new_size.w, inner.max_size.w);
        }
        if inner.max_size.h > 0 {
            new_size.h = min(new_size.h, inner.max_size.h);
        }
        if inner.min_size.w > 0 {
            new_size.w = max(new_size.w, inner.min_size.w);
        }
        if inner.min_size.h > 0 {
            new_size.h = max(new_size.h, inner.min_size.h);
        }

        if inner.size != new_size {
            inner.size = new_size;
            inner.buffer.resize(new_size.to_f64());
            rv = true;
        }

        let mut csd_shadow_size = new_size;
        csd_shadow_size.w += inner.csd_shadow_width * 2;
        csd_shadow_size.h += inner.csd_shadow_width * 2;
        inner.csd_shadow_buffer.resize(csd_shadow_size.to_f64());

        rv
    }
}

impl LayoutElement for TestWindow {
    type Id = usize;

    fn id(&self) -> &Self::Id {
        &self.id
    }

    fn size(&self) -> Size<i32, Logical> {
        self.inner.borrow().size
    }

    fn buf_loc(&self) -> Point<i32, Logical> {
        (0, 0).into()
    }

    fn is_in_input_region(&self, _point: Point<f64, Logical>) -> bool {
        false
    }

    fn render_normal<R: NiriRenderer>(
        &self,
        _renderer: &mut R,
        location: Point<f64, Logical>,
        _scale: Scale<f64>,
        alpha: f32,
        _target: RenderTarget,
        push: &mut dyn FnMut(LayoutElementRenderElement<R>),
    ) {
        let inner = self.inner.borrow();

        push(
            SolidColorRenderElement::from_buffer(&inner.buffer, location, alpha, Kind::Unspecified)
                .into(),
        );
        push(
            SolidColorRenderElement::from_buffer(
                &inner.csd_shadow_buffer,
                location - Point::from((inner.csd_shadow_width, inner.csd_shadow_width)).to_f64(),
                alpha,
                Kind::Unspecified,
            )
            .into(),
        );
    }

    fn request_size(
        &mut self,
        size: Size<i32, Logical>,
        mode: SizingMode,
        _animate: bool,
        _transaction: Option<Transaction>,
    ) {
        self.inner.borrow_mut().requested_size = Some(size);
        self.inner.borrow_mut().pending_sizing_mode = mode;
    }

    fn min_size(&self) -> Size<i32, Logical> {
        self.inner.borrow().min_size
    }

    fn max_size(&self) -> Size<i32, Logical> {
        self.inner.borrow().max_size
    }

    fn is_wl_surface(&self, _wl_surface: &WlSurface) -> bool {
        false
    }

    fn set_preferred_scale_transform(&self, _scale: output::Scale, _transform: Transform) {}

    fn has_ssd(&self) -> bool {
        false
    }

    fn output_enter(&self, _output: &Output) {}

    fn output_leave(&self, _output: &Output) {}

    fn set_offscreen_data(&self, _data: Option<OffscreenData>) {}

    fn set_activated(&mut self, _active: bool) {}

    fn set_active_in_column(&mut self, _active: bool) {}

    fn set_floating(&mut self, _floating: bool) {}

    fn set_bounds(&self, _bounds: Size<i32, Logical>) {}

    fn is_ignoring_opacity_window_rule(&self) -> bool {
        false
    }

    fn configure_intent(&self) -> ConfigureIntent {
        ConfigureIntent::CanSend
    }

    fn send_pending_configure(&mut self) {}

    fn pending_sizing_mode(&self) -> SizingMode {
        self.inner.borrow().pending_sizing_mode
    }

    fn sizing_mode(&self) -> SizingMode {
        SizingMode::Normal
    }

    fn requested_size(&self) -> Option<Size<i32, Logical>> {
        self.inner.borrow().requested_size
    }

    fn is_child_of(&self, _parent: &Self) -> bool {
        false
    }

    fn refresh(&self) {}

    fn rules(&self) -> &ResolvedWindowRules {
        &self.rules
    }

    fn take_animation_snapshot(&mut self) -> Option<LayoutElementRenderSnapshot> {
        None
    }

    fn set_interactive_resize(&mut self, _data: Option<InteractiveResizeData>) {}

    fn cancel_interactive_resize(&mut self) {}

    fn on_commit(&mut self, _serial: Serial) {}

    fn interactive_resize_data(&self) -> Option<InteractiveResizeData> {
        None
    }

    fn is_urgent(&self) -> bool {
        false
    }
}
