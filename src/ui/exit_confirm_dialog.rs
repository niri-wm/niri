use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;

use niri_config::Config;
use ordered_float::NotNan;
use pangocairo::cairo::{self, ImageSurface};
use pangocairo::pango::{Alignment, FontDescription};
use smithay::backend::renderer::element::utils::RescaleRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::output::Output;
use smithay::reexports::gbm::Format as Fourcc;
use smithay::utils::{Point, Transform};

use crate::animation::{Animation, Clock};
use crate::niri_render_elements;
use crate::render_helpers::memory::MemoryBuffer;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::utils::{output_size, to_physical_precise_round};

const KEY_NAME: &str = "Enter";
const PADDING: i32 = 16;
const FONT: &str = "sans 14px";
const BORDER: i32 = 8;
const BACKDROP_COLOR: [f32; 4] = [0., 0., 0., 0.4];

pub struct ExitConfirmDialog {
    state: State,
    buffers: RefCell<HashMap<NotNan<f64>, Option<MemoryBuffer>>>,

    clock: Clock,
    config: Rc<RefCell<Config>>,
}

niri_render_elements! {
    ExitConfirmDialogRenderElement => {
        Texture = RescaleRenderElement<PrimaryGpuTextureRenderElement>,
        SolidColor = SolidColorRenderElement,
    }
}

struct OutputData {
    backdrop: SolidColorBuffer,
}

enum State {
    Hidden,
    Showing(Animation),
    Visible,
    Hiding(Animation),
}

impl ExitConfirmDialog {
    pub fn new(clock: Clock, config: Rc<RefCell<Config>>) -> Self {
        let buffer = match render(1.) {
            Ok(x) => Some(x),
            Err(err) => {
                warn!("error creating the exit confirm dialog: {err:?}");
                None
            }
        };

        Self {
            state: State::Hidden,
            buffers: RefCell::new(HashMap::from([(NotNan::new(1.).unwrap(), buffer)])),
            clock,
            config,
        }
    }

    pub fn can_show(&self) -> bool {
        let buffers = self.buffers.borrow();
        let fallback = &buffers[&NotNan::new(1.).unwrap()];
        fallback.is_some()
    }

    fn animation(&self, from: f64, to: f64) -> Animation {
        let c = self.config.borrow();
        Animation::new(
            self.clock.clone(),
            from,
            to,
            0.,
            c.animations.exit_confirmation_open_close.0,
        )
    }

    fn value(&self) -> f64 {
        match &self.state {
            State::Hidden => 0.,
            State::Showing(anim) | State::Hiding(anim) => anim.value(),
            State::Visible => 1.,
        }
    }

    /// Returns true if the dialog will be shown (even if it is already shown).
    pub fn show(&mut self) -> bool {
        if !self.can_show() {
            return false;
        }

        if self.is_open() {
            return true;
        }

        self.state = State::Showing(self.animation(self.value(), 1.));
        true
    }

    /// Returns true if started the hide animation.
    pub fn hide(&mut self) -> bool {
        if !self.is_open() {
            return false;
        }

        self.state = State::Hiding(self.animation(self.value(), 0.));
        true
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state, State::Showing(_) | State::Visible)
    }

    pub fn advance_animations(&mut self) {
        match &mut self.state {
            State::Hidden => (),
            State::Showing(anim) => {
                if anim.is_done() {
                    self.state = State::Visible;
                }
            }
            State::Visible => (),
            State::Hiding(anim) => {
                if anim.is_clamped_done() {
                    self.state = State::Hidden;
                }
            }
        }
    }

    pub fn are_animations_ongoing(&self) -> bool {
        matches!(self.state, State::Showing(_) | State::Hiding(_))
    }

    pub fn render<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        output: &Output,
        push: &mut dyn FnMut(ExitConfirmDialogRenderElement),
    ) {
        let (value, clamped_value) = match &self.state {
            State::Hidden => return,
            State::Showing(anim) | State::Hiding(anim) => (anim.value(), anim.clamped_value()),
            State::Visible => (1., 1.),
        };
        let _span = tracy_client::span!("ExitConfirmDialog::render");

        // Can be out of range when starting from past 0. or 1. from a spring bounce.
        let clamped_value = clamped_value.clamp(0., 1.);

        let scale = output.current_scale().fractional_scale();
        let output_size = output_size(output);

        let mut buffers = self.buffers.borrow_mut();
        let Some(fallback) = buffers[&NotNan::new(1.).unwrap()].clone() else {
            error!("exit confirm dialog opened without fallback buffer");
            return;
        };

        let buffer = buffers
            .entry(NotNan::new(scale).unwrap())
            .or_insert_with(|| render(scale).ok());
        let buffer = buffer.as_ref().unwrap_or(&fallback);

        let size = buffer.logical_size();
        let Ok(buffer) = TextureBuffer::from_memory_buffer(renderer.as_gles_renderer(), buffer)
        else {
            return;
        };

        let location = (output_size.to_point() - size.to_point()).downscale(2.);
        let mut location = location.to_physical_precise_round(scale).to_logical(scale);
        location.x = f64::max(0., location.x);
        location.y = f64::max(0., location.y);

        let elem = TextureRenderElement::from_texture_buffer(
            buffer,
            location,
            clamped_value as f32,
            None,
            None,
            Kind::Unspecified,
        );
        let elem = PrimaryGpuTextureRenderElement(elem);
        let elem = RescaleRenderElement::from_element(
            elem,
            (location + size.downscale(2.)).to_physical_precise_round(scale),
            value.max(0.) * 0.2 + 0.8,
        );
        push(ExitConfirmDialogRenderElement::Texture(elem));

        // Backdrop.
        let data = output.user_data().get_or_insert(|| {
            Mutex::new(OutputData {
                backdrop: SolidColorBuffer::new(output_size, BACKDROP_COLOR),
            })
        });
        let mut data = data.lock().unwrap();
        data.backdrop.resize(output_size);

        let elem = SolidColorRenderElement::from_buffer(
            &data.backdrop,
            Point::new(0., 0.),
            clamped_value as f32,
            Kind::Unspecified,
        );
        push(ExitConfirmDialogRenderElement::SolidColor(elem));
    }
}

fn render(scale: f64) -> anyhow::Result<MemoryBuffer> {
    let _span = tracy_client::span!("exit_confirm_dialog::render");

    let markup = text(true);

    let padding: i32 = to_physical_precise_round(scale, PADDING);

    let mut font = FontDescription::from_string(FONT);
    font.set_absolute_size(to_physical_precise_round(scale, font.size()));

    let surface = ImageSurface::create(cairo::Format::ARgb32, 0, 0)?;
    let cr = cairo::Context::new(&surface)?;
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));
    layout.set_alignment(Alignment::Center);
    layout.set_markup(&markup);

    let (mut width, mut height) = layout.pixel_size();
    width += padding * 2;
    height += padding * 2;

    let surface = ImageSurface::create(cairo::Format::ARgb32, width, height)?;
    let cr = cairo::Context::new(&surface)?;
    cr.set_source_rgb(0.1, 0.1, 0.1);
    cr.paint()?;

    cr.move_to(padding.into(), padding.into());
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));
    layout.set_alignment(Alignment::Center);
    layout.set_markup(&markup);

    cr.set_source_rgb(1., 1., 1.);
    pangocairo::functions::show_layout(&cr, &layout);

    cr.move_to(0., 0.);
    cr.line_to(width.into(), 0.);
    cr.line_to(width.into(), height.into());
    cr.line_to(0., height.into());
    cr.line_to(0., 0.);
    cr.set_source_rgb(1., 0.3, 0.3);
    // Keep the border width even to avoid blurry edges.
    cr.set_line_width((f64::from(BORDER) / 2. * scale).round() * 2.);
    cr.stroke()?;
    drop(cr);

    let data = surface.take_data().unwrap();
    let buffer = MemoryBuffer::new(
        data.to_vec(),
        Fourcc::Argb8888,
        (width, height),
        scale,
        Transform::Normal,
    );

    Ok(buffer)
}

fn text(markup: bool) -> String {
    let key = if markup {
        format!("<span face='mono' bgcolor='#2C2C2C'> {KEY_NAME} </span>")
    } else {
        String::from(KEY_NAME)
    };

    format!(
        "Are you sure you want to exit niri?\n\n\
         Press {key} to confirm."
    )
}

#[cfg(feature = "dbus")]
pub fn a11y_node() -> accesskit::Node {
    let mut node = accesskit::Node::new(accesskit::Role::AlertDialog);
    node.set_label("Exit niri");
    node.set_description(text(false));
    node.set_modal();
    node
}
