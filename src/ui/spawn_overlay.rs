use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use niri_config::Config;
use pangocairo::cairo::{self, ImageSurface};
use pangocairo::pango::{AttrInt, AttrList, FontDescription, Weight};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::output::{Output, WeakOutput};
use smithay::reexports::gbm::Format as Fourcc;
use smithay::utils::{Logical, Point, Transform};

use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::utils::{output_size, to_physical_precise_round};

const ARROW_SIZE: i32 = 64;
const ARROW_MARGIN: i32 = 48;
const ARROW_OPACITY: f32 = 0.85;
const LABEL_FONT: &str = "sans 11px";
const LABEL_OFFSET: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone)]
pub enum SpawnCommand {
    Args(Vec<String>),
    Shell(String),
}

pub struct SpawnOverlay {
    is_open: bool,
    command: Option<SpawnCommand>,
    config: Rc<RefCell<Config>>,
    buffers: RefCell<HashMap<WeakOutput, RenderedArrows>>,
}

struct RenderedArrows {
    // One texture per arrow: [left, right, up, down]
    arrows: [Option<TextureBuffer<GlesTexture>>; 4],
    scale: f64,
}

impl SpawnOverlay {
    pub fn new(config: Rc<RefCell<Config>>) -> Self {
        Self {
            is_open: false,
            command: None,
            config,
            buffers: RefCell::new(HashMap::new()),
        }
    }

    pub fn open(&mut self, command: SpawnCommand) -> bool {
        if !self.is_open {
            self.is_open = true;
            self.command = Some(command);
            true
        } else {
            false
        }
    }

    pub fn close(&mut self) -> Option<SpawnCommand> {
        if self.is_open {
            self.is_open = false;
            self.command.take()
        } else {
            None
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn take_command(&mut self) -> Option<SpawnCommand> {
        self.is_open = false;
        self.command.take()
    }

    pub fn on_config_updated(&mut self) {
        self.buffers.borrow_mut().clear();
    }

    pub fn render<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        output: &Output,
    ) -> Vec<PrimaryGpuTextureRenderElement> {
        if !self.is_open {
            return vec![];
        }

        let scale = output.current_scale().fractional_scale();
        let output_sz = output_size(output);

        let mut buffers = self.buffers.borrow_mut();
        buffers.retain(|output, _| output.is_alive());

        let weak = output.downgrade();
        if let Some(rendered) = buffers.get(&weak) {
            if rendered.scale != scale {
                buffers.remove(&weak);
            }
        }

        let config = self.config.borrow();
        let rendered = buffers.entry(weak).or_insert_with(|| {
            let gles = renderer.as_gles_renderer();
            render_arrows(gles, &config, scale).unwrap_or_else(|err| {
                warn!("error rendering spawn overlay arrows: {err:?}");
                RenderedArrows {
                    arrows: [None, None, None, None],
                    scale,
                }
            })
        });

        let mut elements = Vec::new();

        // Position each arrow at a screen edge, centered on the opposite axis.
        let arrow_logical = ARROW_SIZE as f64;
        let margin_logical = ARROW_MARGIN as f64;

        let positions: [Point<f64, Logical>; 4] = [
            // Left: left edge, vertically centered
            Point::from((margin_logical, (output_sz.h - arrow_logical) / 2.)),
            // Right: right edge, vertically centered
            Point::from((
                output_sz.w - arrow_logical - margin_logical,
                (output_sz.h - arrow_logical) / 2.,
            )),
            // Up: top edge, horizontally centered
            Point::from(((output_sz.w - arrow_logical) / 2., margin_logical)),
            // Down: bottom edge, horizontally centered
            Point::from((
                (output_sz.w - arrow_logical) / 2.,
                output_sz.h - arrow_logical - margin_logical,
            )),
        ];

        for (i, pos) in positions.iter().enumerate() {
            if let Some(buffer) = &rendered.arrows[i] {
                let location = pos.to_physical_precise_round(scale).to_logical(scale);
                let elem = TextureRenderElement::from_texture_buffer(
                    buffer.clone(),
                    location,
                    ARROW_OPACITY,
                    None,
                    None,
                    Kind::Unspecified,
                );
                elements.push(PrimaryGpuTextureRenderElement(elem));
            }
        }

        elements
    }
}

fn render_arrows(
    renderer: &mut GlesRenderer,
    config: &Config,
    scale: f64,
) -> anyhow::Result<RenderedArrows> {
    let _span = tracy_client::span!("spawn_overlay::render_arrows");

    // Use active color from border (if enabled) or focus ring for theme compliance.
    let color = if !config.layout.border.off {
        config.layout.border.active_color
    } else {
        config.layout.focus_ring.active_color
    };

    let r = color.r as f64;
    let g = color.g as f64;
    let b = color.b as f64;
    let a = color.a as f64;

    let size: i32 = to_physical_precise_round(scale, ARROW_SIZE);

    let labels = ["New Column\nLeft", "New Column\nRight", "Above in\nColumn", "Below in\nColumn"];

    let arrows = [
        render_single_arrow(renderer, size, scale, r, g, b, a, ArrowDir::Left, labels[0])?,
        render_single_arrow(renderer, size, scale, r, g, b, a, ArrowDir::Right, labels[1])?,
        render_single_arrow(renderer, size, scale, r, g, b, a, ArrowDir::Up, labels[2])?,
        render_single_arrow(renderer, size, scale, r, g, b, a, ArrowDir::Down, labels[3])?,
    ];

    Ok(RenderedArrows {
        arrows: arrows.map(Some),
        scale,
    })
}

#[derive(Clone, Copy)]
enum ArrowDir {
    Left,
    Right,
    Up,
    Down,
}

fn render_single_arrow(
    renderer: &mut GlesRenderer,
    size: i32,
    scale: f64,
    r: f64,
    g: f64,
    b: f64,
    a: f64,
    direction: ArrowDir,
    label: &str,
) -> anyhow::Result<TextureBuffer<GlesTexture>> {
    // Measure the label text first so we can size the surface accordingly.
    let mut font = FontDescription::from_string(LABEL_FONT);
    font.set_absolute_size(to_physical_precise_round(scale, font.size()));

    let measure_surface = ImageSurface::create(cairo::Format::ARgb32, 0, 0)?;
    let measure_cr = cairo::Context::new(&measure_surface)?;
    let measure_layout = pangocairo::functions::create_layout(&measure_cr);
    measure_layout.context().set_round_glyph_positions(false);
    measure_layout.set_font_description(Some(&font));

    let bold = AttrList::new();
    bold.insert(AttrInt::new_weight(Weight::Bold));
    measure_layout.set_attributes(Some(&bold));
    measure_layout.set_alignment(pangocairo::pango::Alignment::Center);
    measure_layout.set_text(label);
    let (label_w, label_h) = measure_layout.pixel_size();
    drop(measure_cr);

    let label_offset: i32 = to_physical_precise_round(scale, LABEL_OFFSET);

    // Compute total surface size: arrow + gap + label
    let total_w;
    let total_h;
    let arrow_x;
    let arrow_y;
    let label_x;
    let label_y;

    match direction {
        ArrowDir::Left | ArrowDir::Right => {
            total_w = size + label_offset + label_w;
            total_h = size.max(label_h);
            arrow_y = (total_h - size) / 2;
            label_y = (total_h - label_h) / 2;
            if matches!(direction, ArrowDir::Left) {
                arrow_x = 0;
                label_x = size + label_offset;
            } else {
                label_x = 0;
                arrow_x = label_w + label_offset;
            }
        }
        ArrowDir::Up | ArrowDir::Down => {
            total_w = size.max(label_w);
            total_h = size + label_offset + label_h;
            arrow_x = (total_w - size) / 2;
            label_x = (total_w - label_w) / 2;
            if matches!(direction, ArrowDir::Up) {
                arrow_y = 0;
                label_y = size + label_offset;
            } else {
                label_y = 0;
                arrow_y = label_h + label_offset;
            }
        }
    }

    let surface = ImageSurface::create(cairo::Format::ARgb32, total_w, total_h)?;
    let cr = cairo::Context::new(&surface)?;

    // Draw rounded rectangle background behind the arrow.
    let bg_radius = (size as f64) * 0.15;
    let pi = std::f64::consts::PI;

    let bx = arrow_x as f64;
    let by = arrow_y as f64;
    let bw = size as f64;
    let bh = size as f64;

    cr.new_path();
    cr.arc(
        bx + bg_radius,
        by + bg_radius,
        bg_radius,
        pi,
        1.5 * pi,
    );
    cr.arc(
        bx + bw - bg_radius,
        by + bg_radius,
        bg_radius,
        1.5 * pi,
        2. * pi,
    );
    cr.arc(
        bx + bw - bg_radius,
        by + bh - bg_radius,
        bg_radius,
        0.,
        0.5 * pi,
    );
    cr.arc(
        bx + bg_radius,
        by + bh - bg_radius,
        bg_radius,
        0.5 * pi,
        pi,
    );
    cr.close_path();
    cr.set_source_rgba(0.1, 0.1, 0.1, 0.9);
    cr.fill()?;

    // Draw arrow triangle using the theme color.
    let margin = bw * 0.25;

    draw_arrow_triangle(&cr, bx, by, bw, margin, direction);
    cr.set_source_rgba(r, g, b, a);
    cr.fill()?;

    // Draw border on the arrow triangle.
    draw_arrow_triangle(&cr, bx, by, bw, margin, direction);
    let lighter = |c: f64| (c + (1. - c) * 0.3).min(1.);
    cr.set_source_rgba(lighter(r), lighter(g), lighter(b), a);
    cr.set_line_width(1.5 * scale);
    cr.stroke()?;

    // Draw label text.
    cr.move_to(label_x.into(), label_y.into());
    cr.set_source_rgba(0.9, 0.9, 0.9, 0.95);
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));
    layout.set_attributes(Some(&bold));
    layout.set_alignment(pangocairo::pango::Alignment::Center);
    layout.set_text(label);
    pangocairo::functions::show_layout(&cr, &layout);

    drop(cr);

    let data = surface.take_data().unwrap();
    let buffer = TextureBuffer::from_memory(
        renderer,
        &data,
        Fourcc::Argb8888,
        (total_w, total_h),
        false,
        scale,
        Transform::Normal,
        Vec::new(),
    )?;

    Ok(buffer)
}

fn draw_arrow_triangle(
    cr: &cairo::Context,
    bx: f64,
    by: f64,
    bw: f64,
    margin: f64,
    direction: ArrowDir,
) {
    cr.new_path();
    match direction {
        ArrowDir::Left => {
            cr.move_to(bx + margin, by + bw / 2.);
            cr.line_to(bx + bw - margin, by + margin);
            cr.line_to(bx + bw - margin, by + bw - margin);
        }
        ArrowDir::Right => {
            cr.move_to(bx + bw - margin, by + bw / 2.);
            cr.line_to(bx + margin, by + margin);
            cr.line_to(bx + margin, by + bw - margin);
        }
        ArrowDir::Up => {
            cr.move_to(bx + bw / 2., by + margin);
            cr.line_to(bx + bw - margin, by + bw - margin);
            cr.line_to(bx + margin, by + bw - margin);
        }
        ArrowDir::Down => {
            cr.move_to(bx + bw / 2., by + bw - margin);
            cr.line_to(bx + bw - margin, by + margin);
            cr.line_to(bx + margin, by + margin);
        }
    }
    cr.close_path();
}
