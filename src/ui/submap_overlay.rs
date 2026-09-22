use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use pangocairo::cairo::{self, ImageSurface};
use pangocairo::pango::FontDescription;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::output::Output;
use smithay::reexports::gbm::Format as Fourcc;
use smithay::utils::{Point, Transform};

use crate::animation::{Animation, Clock};
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::utils::{output_size, to_physical_precise_round};

const PADDING: i32 = 8;
const FONT: &str = "sans 14px";
const BORDER: i32 = 4;
const ANIM_DURATION: niri_config::Animation = niri_config::Animation {
    off: false,
    kind: niri_config::animations::Kind::Easing(niri_config::animations::EasingParams {
        duration_ms: 150,
        curve: niri_config::animations::Curve::EaseOutCubic,
    }),
};

pub struct SubmapOverlay {
    state: State,
    buffers: RefCell<HashMap<String, Option<TextureBuffer<GlesTexture>>>>,
    clock: Clock,
}

enum State {
    Hidden,
    Showing(String, Animation),
    Shown(String, Duration),
    Hiding(String, Animation),
}

impl SubmapOverlay {
    pub fn new(clock: Clock) -> Self {
        Self {
            state: State::Hidden,
            buffers: RefCell::new(HashMap::new()),
            clock,
        }
    }

    pub fn show(&mut self, name: &str) {
        self.state = State::Showing(
            name.to_string(),
            Animation::new(self.clock.clone(), 0., 1., 0., ANIM_DURATION),
        );
    }

    pub fn hide(&mut self) {
        if let State::Shown(name, _) | State::Showing(name, _) = &self.state {
            let name = name.clone();
            self.state = State::Hiding(
                name,
                Animation::new(self.clock.clone(), 1., 0., 0., ANIM_DURATION),
            );
        }
    }

    pub fn advance_animations(&mut self) {
        match &mut self.state {
            State::Hidden => (),
            State::Showing(name, anim) => {
                if anim.is_done() {
                    let name = std::mem::take(name);
                    self.state = State::Shown(name, self.clock.now_unadjusted() + Duration::from_secs(2));
                }
            }
            State::Shown(_, deadline) => {
                if self.clock.now_unadjusted() >= *deadline {
                    self.hide();
                }
            }
            State::Hiding(_, anim) => {
                if anim.is_clamped_done() {
                    self.state = State::Hidden;
                }
            }
        }
    }

    pub fn are_animations_ongoing(&self) -> bool {
        !matches!(self.state, State::Hidden)
    }

    pub fn render<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        output: &Output,
    ) -> Option<PrimaryGpuTextureRenderElement> {
        let (name, _alpha) = match &self.state {
            State::Hidden => return None,
            State::Showing(name, anim) => (name.clone(), anim.value() as f32),
            State::Shown(name, _) => (name.clone(), 1.0),
            State::Hiding(name, anim) => (name.clone(), anim.value() as f32),
        };

        let scale = output.current_scale().fractional_scale();
        let output_size = output_size(output);

        let mut buffers = self.buffers.borrow_mut();
        let buffer = buffers
            .entry(name.clone())
            .or_insert_with(move || render(renderer.as_gles_renderer(), scale, &name).ok());

        let buffer = buffer.clone()?;

        let size = buffer.logical_size();
        let y_range = size.h + f64::from(PADDING) * 2.;

        let x = (output_size.w - size.w).max(0.) / 2.;
        let (y, alpha_f64) = match &self.state {
            State::Hidden => unreachable!(),
            State::Showing(_, anim) => (-size.h + anim.value() * y_range, anim.value()),
            State::Shown(_, _) => (f64::from(PADDING) * 2., 1.0),
            State::Hiding(_, anim) => (-size.h + anim.value() * y_range, anim.value()),
        };

        let location = Point::from((x, y));
        let location = location.to_physical_precise_round(scale).to_logical(scale);

        let elem = TextureRenderElement::from_texture_buffer(
            buffer,
            location,
            alpha_f64 as f32,
            None,
            None,
            Kind::Unspecified,
        );
        Some(PrimaryGpuTextureRenderElement(elem))
    }
}

fn render(
    renderer: &mut GlesRenderer,
    scale: f64,
    name: &str,
) -> anyhow::Result<TextureBuffer<GlesTexture>> {
    let _span = tracy_client::span!("submap_overlay::render");

    let padding: i32 = to_physical_precise_round(scale, PADDING);

    let text = format!("Submap: <b>{name}</b>");

    let mut font = FontDescription::from_string(FONT);
    font.set_absolute_size(to_physical_precise_round(scale, font.size()));

    let surface = ImageSurface::create(cairo::Format::ARgb32, 0, 0)?;
    let cr = cairo::Context::new(&surface)?;
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));
    layout.set_markup(&text);

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
    layout.set_markup(&text);

    cr.set_source_rgb(1., 1., 1.);
    pangocairo::functions::show_layout(&cr, &layout);

    cr.move_to(0., 0.);
    cr.line_to(width.into(), 0.);
    cr.line_to(width.into(), height.into());
    cr.line_to(0., height.into());
    cr.line_to(0., 0.);
    cr.set_source_rgb(0.3, 0.6, 1.0);
    cr.set_line_width((f64::from(BORDER) / 2. * scale).round() * 2.);
    cr.stroke()?;
    drop(cr);

    let data = surface.take_data().unwrap();
    let buffer = TextureBuffer::from_memory(
        renderer,
        &data,
        Fourcc::Argb8888,
        (width, height),
        false,
        scale,
        Transform::Normal,
        Vec::new(),
    )?;

    Ok(buffer)
}
