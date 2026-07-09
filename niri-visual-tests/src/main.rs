#[macro_use]
extern crate tracing;

use std::env;

use adw::prelude::{AdwApplicationWindowExt, NavigationPageExt};
use cases::Args;
use gtk::prelude::{ApplicationExt, ApplicationExtManual, BoxExt, GtkWindowExt, WidgetExt};
use gtk::{gdk, gio, glib};
use smithay_view::SmithayView;
use tracing_subscriber::EnvFilter;

use crate::cases::gradient_angle::GradientAngle;
use crate::cases::gradient_area::GradientArea;
use crate::cases::gradient_oklab::GradientOklab;
use crate::cases::gradient_oklab_alpha::GradientOklabAlpha;
use crate::cases::gradient_oklch_alpha::GradientOklchAlpha;
use crate::cases::gradient_oklch_decreasing::GradientOklchDecreasing;
use crate::cases::gradient_oklch_increasing::GradientOklchIncreasing;
use crate::cases::gradient_oklch_longer::GradientOklchLonger;
use crate::cases::gradient_oklch_shorter::GradientOklchShorter;
use crate::cases::gradient_srgb::GradientSrgb;
use crate::cases::gradient_srgb_alpha::GradientSrgbAlpha;
use crate::cases::gradient_srgblinear::GradientSrgbLinear;
use crate::cases::gradient_srgblinear_alpha::GradientSrgbLinearAlpha;
use crate::cases::layout::Layout;
use crate::cases::tile::Tile;
use crate::cases::window::Window;
use crate::cases::TestCase;

mod cases;
mod smithay_view;
mod test_window;

fn main() -> glib::ExitCode {
    let directives =
        env::var("RUST_LOG").unwrap_or_else(|_| "niri-visual-tests=debug,niri=debug".to_owned());
    let env_filter = EnvFilter::builder().parse_lossy(directives);
    tracing_subscriber::fmt()
        .compact()
        .with_env_filter(env_filter)
        .init();

    let app = adw::Application::new(None::<&str>, gio::ApplicationFlags::NON_UNIQUE);
    app.connect_startup(on_startup);
    app.connect_activate(build_ui);
    app.run()
}

fn on_startup(_app: &adw::Application) {
    // Load our CSS.
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../resources/style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn build_ui(app: &adw::Application) {
    let stack = gtk::Stack::new();
    let anim_adjustment = gtk::Adjustment::new(1., 0., 10., 0.1, 0.5, 0.);

    struct S {
        stack: gtk::Stack,
        anim_adjustment: gtk::Adjustment,
    }

    impl S {
        fn add<T: TestCase + 'static>(&self, make: impl Fn(Args) -> T + 'static, title: &str) {
            let view = SmithayView::new(make, &self.anim_adjustment);
            self.stack.add_titled(&view, None, title);
        }
    }

    let s = S {
        stack: stack.clone(),
        anim_adjustment: anim_adjustment.clone(),
    };

    s.add(Window::freeform, "Freeform Window");
    s.add(Window::fixed_size, "Fixed Size Window");
    s.add(
        Window::fixed_size_with_csd_shadow,
        "Fixed Size Window - CSD Shadow",
    );

    s.add(Tile::freeform, "Freeform Tile");
    s.add(Tile::fixed_size, "Fixed Size Tile");
    s.add(
        Tile::fixed_size_with_csd_shadow,
        "Fixed Size Tile - CSD Shadow",
    );
    s.add(Tile::freeform_open, "Freeform Tile - Open");
    s.add(Tile::fixed_size_open, "Fixed Size Tile - Open");
    s.add(
        Tile::fixed_size_with_csd_shadow_open,
        "Fixed Size Tile - CSD Shadow - Open",
    );

    s.add(Layout::open_in_between, "Layout - Open In-Between");
    s.add(
        Layout::open_multiple_quickly,
        "Layout - Open Multiple Quickly",
    );
    s.add(
        Layout::open_multiple_quickly_big,
        "Layout - Open Multiple Quickly - Big",
    );
    s.add(Layout::open_to_the_left, "Layout - Open To The Left");
    s.add(
        Layout::open_to_the_left_big,
        "Layout - Open To The Left - Big",
    );

    s.add(GradientAngle::new, "Gradient - Angle");
    s.add(GradientArea::new, "Gradient - Area");
    s.add(GradientSrgb::new, "Gradient - Srgb");
    s.add(GradientSrgbLinear::new, "Gradient - SrgbLinear");
    s.add(GradientOklab::new, "Gradient - Oklab");
    s.add(GradientOklchShorter::new, "Gradient - Oklch Shorter");
    s.add(GradientOklchLonger::new, "Gradient - Oklch Longer");
    s.add(GradientOklchIncreasing::new, "Gradient - Oklch Increasing");
    s.add(GradientOklchDecreasing::new, "Gradient - Oklch Decreasing");
    s.add(GradientSrgbAlpha::new, "Gradient - Srgb Alpha");
    s.add(GradientSrgbLinearAlpha::new, "Gradient - SrgbLinear Alpha");
    s.add(GradientOklabAlpha::new, "Gradient - Oklab Alpha");
    s.add(GradientOklchAlpha::new, "Gradient - Oklch Alpha");

    let content_headerbar = adw::HeaderBar::new();

    let anim_scale = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&anim_adjustment));
    anim_scale.set_hexpand(true);

    let anim_control_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    anim_control_bar.add_css_class("anim-control-bar");
    anim_control_bar.append(&gtk::Label::new(Some("Slowdown")));
    anim_control_bar.append(&anim_scale);

    let content_view = adw::ToolbarView::new();
    content_view.set_top_bar_style(adw::ToolbarStyle::RaisedBorder);
    content_view.set_bottom_bar_style(adw::ToolbarStyle::RaisedBorder);
    content_view.add_top_bar(&content_headerbar);
    content_view.add_bottom_bar(&anim_control_bar);
    content_view.set_content(Some(&stack));
    let content = adw::NavigationPage::new(
        &content_view,
        stack
            .page(&stack.visible_child().unwrap())
            .title()
            .as_deref()
            .unwrap(),
    );

    let sidebar_header = adw::HeaderBar::new();
    let stack_sidebar = gtk::StackSidebar::new();
    stack_sidebar.set_stack(&stack);
    let sidebar_view = adw::ToolbarView::new();
    sidebar_view.add_top_bar(&sidebar_header);
    sidebar_view.set_content(Some(&stack_sidebar));
    let sidebar = adw::NavigationPage::new(&sidebar_view, "Tests");

    let split_view = adw::NavigationSplitView::new();
    split_view.set_content(Some(&content));
    split_view.set_sidebar(Some(&sidebar));

    stack.connect_visible_child_notify(move |stack| {
        content.set_title(
            stack
                .visible_child()
                .and_then(|c| stack.page(&c).title())
                .as_deref()
                .unwrap_or_default(),
        )
    });

    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some("niri visual tests"));
    window.set_content(Some(&split_view));
    window.present();
}
