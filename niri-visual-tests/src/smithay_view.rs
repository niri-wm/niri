use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use smithay::utils::Size;

use crate::cases::{Args, TestCase};

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::ptr::null;
    use std::time::Duration;

    use anyhow::{ensure, Context};
    use gtk::gdk;
    use gtk::prelude::*;
    use niri::animation::Clock;
    use niri::render_helpers::{resources, shaders};
    use smithay::backend::egl::ffi::egl;
    use smithay::backend::egl::EGLContext;
    use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
    use smithay::backend::renderer::{Bind, Color32F, Frame, Offscreen, Renderer};
    use smithay::reexports::gbm::Format as Fourcc;
    use smithay::utils::{Physical, Rectangle, Scale, Transform};

    use super::*;

    type DynMakeTestCase = Box<dyn Fn(Args) -> Box<dyn TestCase>>;

    struct RendererData {
        renderer: GlesRenderer,
        dummy_texture: GlesTexture,
    }

    #[derive(Default)]
    pub struct SmithayView {
        gl_area: gtk::GLArea,
        size: Cell<(i32, i32)>,
        renderer: RefCell<Option<Result<RendererData, ()>>>,
        pub make_test_case: OnceCell<DynMakeTestCase>,
        test_case: RefCell<Option<Box<dyn TestCase>>>,
        pub clock: RefCell<Clock>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SmithayView {
        const NAME: &'static str = "NiriSmithayView";
        type Type = super::SmithayView;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for SmithayView {
        fn constructed(&self) {
            let obj = self.obj();

            self.parent_constructed();

            self.gl_area.set_allowed_apis(gdk::GLAPI::GLES);
            self.gl_area.set_parent(&*obj);

            self.gl_area.connect_resize({
                let imp = self.downgrade();
                move |_, width, height| {
                    if let Some(imp) = imp.upgrade() {
                        imp.resize(width, height);
                    }
                }
            });

            self.gl_area.connect_render({
                let imp = self.downgrade();
                move |_, gl_context| {
                    if let Some(imp) = imp.upgrade() {
                        if let Err(err) = imp.render(gl_context) {
                            warn!("error rendering: {err:?}");
                        }
                    }
                    glib::Propagation::Stop
                }
            });

            obj.add_tick_callback(|obj, _frame_clock| {
                let imp = obj.imp();

                if let Some(case) = &mut *imp.test_case.borrow_mut() {
                    if case.are_animations_ongoing() {
                        imp.gl_area.queue_draw();
                    }
                }

                glib::ControlFlow::Continue
            });
        }

        fn dispose(&self) {
            self.gl_area.unparent();
        }
    }

    impl WidgetImpl for SmithayView {
        fn unmap(&self) {
            self.test_case.replace(None);
            self.parent_unmap();
        }

        fn unrealize(&self) {
            self.renderer.replace(None);
            self.parent_unrealize();
        }
    }

    impl SmithayView {
        fn resize(&self, width: i32, height: i32) {
            self.size.set((width, height));

            if let Some(case) = &mut *self.test_case.borrow_mut() {
                case.resize(width, height);
            }
        }

        fn render(&self, _gl_context: &gdk::GLContext) -> anyhow::Result<()> {
            // Set up the Smithay renderer.
            let mut renderer = self.renderer.borrow_mut();
            let renderer = renderer.get_or_insert_with(|| {
                unsafe { create_renderer() }
                    .map_err(|err| warn!("error creating a Smithay renderer: {err:?}"))
            });
            let Ok(renderer) = renderer else {
                return Ok(());
            };
            let RendererData {
                renderer,
                dummy_texture,
            } = renderer;

            let size = self.size.get();

            let frame_clock = self.obj().frame_clock().unwrap();
            let time = Duration::from_micros(frame_clock.frame_time() as u64);
            self.clock.borrow_mut().set_unadjusted(time);

            // Create the test case if missing.
            let mut case = self.test_case.borrow_mut();
            let case = case.get_or_insert_with(|| {
                let make = self.make_test_case.get().unwrap();
                let args = Args {
                    size: Size::from(size),
                    clock: self.clock.borrow().clone(),
                };
                make(args)
            });

            case.advance_animations(self.clock.borrow_mut().now());

            let rect: Rectangle<i32, Physical> = Rectangle::from_size(Size::from(size));

            // Fetch GtkGLArea's framebuffer binding.
            let mut framebuffer = 0;
            renderer
                .with_context(|gl| unsafe {
                    gl.GetIntegerv(
                        smithay::backend::renderer::gles::ffi::FRAMEBUFFER_BINDING,
                        &mut framebuffer,
                    );
                })
                .context("error running closure in GL context")?;
            ensure!(framebuffer != 0, "error getting the framebuffer");

            // This call will already change the framebuffer binding (offscreen elements will bind
            // intermediate textures during rendering).
            let elements = case.render(renderer, Size::from(size));

            // HACK: there's currently no way to "just" render into an externally bound framebuffer
            // (like we have in this case). The render() call requires a valid target. So what
            // we'll do is use a dummy texture as a target, then swap the framebuffer binding right
            // before rendering.
            let mut dummy_target = renderer
                .bind(dummy_texture)
                .context("error binding dummy texture")?;

            let mut frame = renderer
                .render(&mut dummy_target, rect.size, Transform::Normal)
                .context("error creating frame")?;

            // Now that render() bound the dummy texture, change the binding underneath it back to
            // GtkGLArea's framebuffer, to render there instead.
            frame
                .with_context(|gl| unsafe {
                    gl.BindFramebuffer(
                        smithay::backend::renderer::gles::ffi::FRAMEBUFFER,
                        framebuffer as u32,
                    );
                })
                .context("error running closure in GL context")?;

            frame
                .clear(Color32F::from([0.3, 0.3, 0.3, 1.]), &[rect])
                .context("error clearing")?;

            for element in elements.iter().rev() {
                let src = element.src();
                let dst = element.geometry(Scale::from(1.));

                if let Some(mut damage) = rect.intersection(dst) {
                    damage.loc -= dst.loc;
                    element
                        .draw(&mut frame, src, dst, &[damage], &[])
                        .context("error drawing element")?;
                }
            }

            Ok(())
        }
    }

    unsafe fn create_renderer() -> anyhow::Result<RendererData> {
        smithay::backend::egl::ffi::make_sure_egl_is_loaded()
            .context("error loading EGL symbols in Smithay")?;

        let egl_display = egl::GetCurrentDisplay();
        ensure!(egl_display != egl::NO_DISPLAY, "no current EGL display");

        let egl_context = egl::GetCurrentContext();
        ensure!(egl_context != egl::NO_CONTEXT, "no current EGL context");

        // There's no config ID on the EGL context and there's no current EGL surface, but we don't
        // really use it anyway so just get some random one.
        let mut egl_config_id = null();
        let mut num_configs = 0;
        let res = egl::GetConfigs(egl_display, &mut egl_config_id, 1, &mut num_configs);
        ensure!(res == egl::TRUE, "error choosing EGL config");
        ensure!(num_configs != 0, "no EGL config");

        let egl_context = EGLContext::from_raw(egl_display, egl_config_id as *const _, egl_context)
            .context("error creating EGL context")?;

        let mut renderer = GlesRenderer::new(egl_context).context("error creating GlesRenderer")?;

        let dummy_texture = renderer
            .create_buffer(Fourcc::Abgr8888, Size::from((1, 1)))
            .context("error creating dummy texture")?;

        resources::init(&mut renderer);
        shaders::init(&mut renderer);

        Ok(RendererData {
            renderer,
            dummy_texture,
        })
    }
}

glib::wrapper! {
    pub struct SmithayView(ObjectSubclass<imp::SmithayView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SmithayView {
    pub fn new<T: TestCase + 'static>(
        make_test_case: impl Fn(Args) -> T + 'static,
        anim_adjustment: &gtk::Adjustment,
    ) -> Self {
        let obj: Self = glib::Object::builder().build();

        let make = move |args| Box::new(make_test_case(args)) as Box<dyn TestCase>;
        let make_test_case = Box::new(make) as _;
        let _ = obj.imp().make_test_case.set(make_test_case);

        anim_adjustment.connect_value_changed({
            let obj = obj.downgrade();
            move |adj| {
                if let Some(obj) = obj.upgrade() {
                    let mut clock = obj.imp().clock.borrow_mut();
                    let instantly = adj.value() == 0.0;
                    let rate = if instantly {
                        1.0
                    } else {
                        1.0 / adj.value().max(0.001)
                    };
                    clock.set_rate(rate);
                    clock.set_complete_instantly(instantly);
                }
            }
        });

        obj
    }
}
