use std::cell::RefCell;

use glam::Mat3;
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
    UniformValue,
};

use super::renderer::NiriRenderer;
use super::shader_element::ShaderProgram;
use crate::render_helpers::blur::BlurProgram;

const ROUNDING_ALPHA_IMPL_PLACEHOLDER: &str = "@ROUNDING_ALPHA_IMPL@";
const EXPONENT_SHADOW_X_IMPL_PLACEHOLDER: &str = "@EXPONENT_SHADOW_X_IMPL@";

const ROUNDING_ALPHA_IMPL: &str = include_str!("rounding_alpha_superellipse.frag");
const EXPONENT_SHADOW_X_IMPL: &str = "
float normalized = (-delta) / max(corner, 0.0001);
float remaining = max(0.0, 1.0 - pow(normalized, exponent));
curved = halfSize.x - corner + corner * pow(remaining, 1.0 / exponent);";

const ROUNDING_ALPHA_IMPL_FALLBACK: &str = "
float niri_rounding_alpha_impl(vec2 coords, vec2 center, float radius, float corner_exponent) {
    float dist = distance(coords, center);

    // Manual smoothstep() between radius - half_px and radius + half_px
    // to avoid a division in clamp().
    float t = clamp((dist - radius) * niri_scale + 0.5, 0.0, 1.0);
    return 1.0 - t * t * (3.0 - 2.0 * t);
}";
const EXPONENT_SHADOW_X_IMPL_FALLBACK: &str =
    "curved = halfSize.x - corner + sqrt(max(0.0, corner * corner - delta * delta));";

pub struct Shaders {
    pub border: Option<ShaderProgram>,
    pub shadow: Option<ShaderProgram>,
    pub clipped_surface: Option<GlesTexProgram>,
    pub postprocess_and_clip: Option<GlesTexProgram>,
    pub resize: Option<ShaderProgram>,
    pub gradient_fade: Option<GlesTexProgram>,
    pub blur: Option<BlurProgram>,
    pub custom_resize: RefCell<Option<ShaderProgram>>,
    pub custom_close: RefCell<Option<ShaderProgram>>,
    pub custom_open: RefCell<Option<ShaderProgram>>,
}

#[derive(Debug, Clone, Copy)]
pub enum ProgramType {
    Border,
    Shadow,
    Resize,
    Close,
    Open,
}

fn try_compile_shader(
    shader_name: &str,
    renderer: &mut GlesRenderer,
    src: &str,
    additional_uniforms: &[UniformName<'_>],
    texture_uniforms: &[&str],
) -> Option<ShaderProgram> {
    let replaced_src = src
        .replace(ROUNDING_ALPHA_IMPL_PLACEHOLDER, ROUNDING_ALPHA_IMPL)
        .replace(EXPONENT_SHADOW_X_IMPL_PLACEHOLDER, EXPONENT_SHADOW_X_IMPL);
    match ShaderProgram::compile(
        renderer,
        &replaced_src,
        additional_uniforms,
        texture_uniforms,
    ) {
        Ok(p) => Some(p),
        Err(err) => {
            warn!("error compiling {shader_name} shader with support for corner radius exponent: {err:?}");
            let fallback_src = src
                .replace(
                    ROUNDING_ALPHA_IMPL_PLACEHOLDER,
                    ROUNDING_ALPHA_IMPL_FALLBACK,
                )
                .replace(
                    EXPONENT_SHADOW_X_IMPL_PLACEHOLDER,
                    EXPONENT_SHADOW_X_IMPL_FALLBACK,
                );
            ShaderProgram::compile(
                renderer,
                &fallback_src,
                additional_uniforms,
                texture_uniforms,
            )
            .inspect_err(|e| warn!("error compiling fallback {shader_name} shader: {e:?}"))
            .ok()
        }
    }
}

fn try_compile_custom_texture_shader(
    shader_name: &str,
    renderer: &mut GlesRenderer,
    src: &str,
    additional_uniforms: &[UniformName<'_>],
) -> Option<GlesTexProgram> {
    let replaced_src = src
        .replace(ROUNDING_ALPHA_IMPL_PLACEHOLDER, ROUNDING_ALPHA_IMPL)
        .replace(EXPONENT_SHADOW_X_IMPL_PLACEHOLDER, EXPONENT_SHADOW_X_IMPL);
    match renderer.compile_custom_texture_shader(replaced_src, additional_uniforms) {
        Ok(s) => Some(s),
        Err(err) => {
            warn!("error compiling {shader_name} shader with support for corner radius exponent: {err:?}");
            let fallback_src = src
                .replace(
                    ROUNDING_ALPHA_IMPL_PLACEHOLDER,
                    ROUNDING_ALPHA_IMPL_FALLBACK,
                )
                .replace(
                    EXPONENT_SHADOW_X_IMPL_PLACEHOLDER,
                    EXPONENT_SHADOW_X_IMPL_FALLBACK,
                );
            renderer
                .compile_custom_texture_shader(fallback_src, additional_uniforms)
                .inspect_err(|e| warn!("error compiling fallback {shader_name} shader: {e:?}"))
                .ok()
        }
    }
}

impl Shaders {
    fn compile(renderer: &mut GlesRenderer) -> Self {
        let _span = tracy_client::span!("Shaders::compile");

        let border = try_compile_shader(
            "border",
            renderer,
            concat!(
                include_str!("border.frag"),
                include_str!("rounding_alpha.frag")
            ),
            &[
                UniformName::new("colorspace", UniformType::_1f),
                UniformName::new("hue_interpolation", UniformType::_1f),
                UniformName::new("color_from", UniformType::_4f),
                UniformName::new("color_to", UniformType::_4f),
                UniformName::new("grad_offset", UniformType::_2f),
                UniformName::new("grad_width", UniformType::_1f),
                UniformName::new("grad_vec", UniformType::_2f),
                UniformName::new("input_to_geo", UniformType::Matrix3x3),
                UniformName::new("geo_size", UniformType::_2f),
                UniformName::new("outer_radius", UniformType::_4f),
                UniformName::new("corner_exponent", UniformType::_1f),
                UniformName::new("border_width", UniformType::_1f),
            ],
            &[],
        );

        let shadow = try_compile_shader(
            "shadow",
            renderer,
            concat!(
                include_str!("shadow.frag"),
                include_str!("rounding_alpha.frag")
            ),
            &[
                UniformName::new("shadow_color", UniformType::_4f),
                UniformName::new("sigma", UniformType::_1f),
                UniformName::new("input_to_geo", UniformType::Matrix3x3),
                UniformName::new("geo_size", UniformType::_2f),
                UniformName::new("corner_radius", UniformType::_4f),
                UniformName::new("corner_exponent", UniformType::_1f),
                UniformName::new("window_input_to_geo", UniformType::Matrix3x3),
                UniformName::new("window_geo_size", UniformType::_2f),
                UniformName::new("window_corner_radius", UniformType::_4f),
                UniformName::new("window_corner_exponent", UniformType::_1f),
            ],
            &[],
        );

        let clipped_surface = try_compile_custom_texture_shader(
            "clipped surface",
            renderer,
            concat!(
                include_str!("clipped_surface.frag"),
                include_str!("rounding_alpha.frag"),
                "\nvec4 postprocess(vec4 color) { return color; }",
            ),
            &[
                UniformName::new("niri_scale", UniformType::_1f),
                UniformName::new("geo_size", UniformType::_2f),
                UniformName::new("corner_radius", UniformType::_4f),
                UniformName::new("corner_exponent", UniformType::_1f),
                UniformName::new("input_to_geo", UniformType::Matrix3x3),
            ],
        );

        let postprocess_and_clip = try_compile_custom_texture_shader(
            "postprocess_and_clip",
            renderer,
            concat!(
                include_str!("clipped_surface.frag"),
                include_str!("rounding_alpha.frag"),
                include_str!("postprocess.frag"),
            ),
            &[
                UniformName::new("niri_scale", UniformType::_1f),
                UniformName::new("geo_size", UniformType::_2f),
                UniformName::new("corner_radius", UniformType::_4f),
                UniformName::new("corner_exponent", UniformType::_1f),
                UniformName::new("input_to_geo", UniformType::Matrix3x3),
                UniformName::new("noise", UniformType::_1f),
                UniformName::new("saturation", UniformType::_1f),
                UniformName::new("bg_color", UniformType::_4f),
            ],
        );

        let resize = compile_resize_program(renderer, include_str!("resize.frag"))
            .map_err(|err| {
                warn!("error compiling resize shader: {err:?}");
            })
            .ok();

        let gradient_fade = renderer
            .compile_custom_texture_shader(
                include_str!("gradient_fade.frag"),
                &[UniformName::new("cutoff", UniformType::_2f)],
            )
            .map_err(|err| {
                warn!("error compiling gradient fade shader: {err:?}");
            })
            .ok();

        let blur = BlurProgram::compile(renderer)
            .map_err(|err| {
                warn!("error compiling blur shaders: {err:?}");
            })
            .ok();

        Self {
            border,
            shadow,
            clipped_surface,
            postprocess_and_clip,
            resize,
            gradient_fade,
            blur,
            custom_resize: RefCell::new(None),
            custom_close: RefCell::new(None),
            custom_open: RefCell::new(None),
        }
    }

    pub fn get_from_frame<'a>(frame: &'a mut GlesFrame<'_, '_>) -> &'a Self {
        let data = frame.egl_context().user_data();
        data.get()
            .expect("shaders::init() must be called when creating the renderer")
    }

    pub fn get(renderer: &mut impl NiriRenderer) -> &Self {
        let renderer = renderer.as_gles_renderer();
        let data = renderer.egl_context().user_data();
        data.get()
            .expect("shaders::init() must be called when creating the renderer")
    }

    pub fn replace_custom_resize_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_resize.replace(program)
    }

    pub fn replace_custom_close_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_close.replace(program)
    }

    pub fn replace_custom_open_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_open.replace(program)
    }

    pub fn program(&self, program: ProgramType) -> Option<ShaderProgram> {
        match program {
            ProgramType::Border => self.border.clone(),
            ProgramType::Shadow => self.shadow.clone(),
            ProgramType::Resize => self
                .custom_resize
                .borrow()
                .clone()
                .or_else(|| self.resize.clone()),
            ProgramType::Close => self.custom_close.borrow().clone(),
            ProgramType::Open => self.custom_open.borrow().clone(),
        }
    }
}

pub fn init(renderer: &mut GlesRenderer) {
    let shaders = Shaders::compile(renderer);
    let data = renderer.egl_context().user_data();
    if !data.insert_if_missing(|| shaders) {
        error!("shaders were already compiled");
    }
}

fn compile_resize_program(
    renderer: &mut GlesRenderer,
    src: &str,
) -> Result<ShaderProgram, GlesError> {
    let additional_uniforms = &[
        UniformName::new("niri_input_to_curr_geo", UniformType::Matrix3x3),
        UniformName::new("niri_curr_geo_to_prev_geo", UniformType::Matrix3x3),
        UniformName::new("niri_curr_geo_to_next_geo", UniformType::Matrix3x3),
        UniformName::new("niri_curr_geo_size", UniformType::_2f),
        UniformName::new("niri_geo_to_tex_prev", UniformType::Matrix3x3),
        UniformName::new("niri_geo_to_tex_next", UniformType::Matrix3x3),
        UniformName::new("niri_progress", UniformType::_1f),
        UniformName::new("niri_clamped_progress", UniformType::_1f),
        UniformName::new("niri_corner_radius", UniformType::_4f),
        UniformName::new("niri_corner_exponent", UniformType::_1f),
        UniformName::new("niri_clip_to_geometry", UniformType::_1f),
    ];
    let texture_uniforms = &["niri_tex_prev", "niri_tex_next"];

    let mut program = include_str!("resize_prelude.frag").to_string();
    program.push_str(src);
    program.push_str(include_str!("resize_epilogue.frag"));

    let replaced_program = program.clone()
        + "\n".into()
        + &include_str!("rounding_alpha.frag")
            .replace(ROUNDING_ALPHA_IMPL_PLACEHOLDER, ROUNDING_ALPHA_IMPL);

    match ShaderProgram::compile(
        renderer,
        &replaced_program,
        additional_uniforms,
        texture_uniforms,
    ) {
        Ok(p) => Ok(p),
        Err(err) => {
            warn!("error compiling resize shader with support for corner radius exponent: {err:?}");
            let fallback_program = program
                + "\n".into()
                + &include_str!("rounding_alpha.frag").replace(
                    ROUNDING_ALPHA_IMPL_PLACEHOLDER,
                    ROUNDING_ALPHA_IMPL_FALLBACK,
                );
            ShaderProgram::compile(
                renderer,
                &fallback_program,
                additional_uniforms,
                texture_uniforms,
            )
        }
    }
}

pub fn set_custom_resize_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_resize_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom resize shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_resize_program(program) {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom resize shader: {err:?}");
        }
    }
}

fn compile_close_program(
    renderer: &mut GlesRenderer,
    src: &str,
) -> Result<ShaderProgram, GlesError> {
    let mut program = include_str!("close_prelude.frag").to_string();
    program.push_str(src);
    program.push_str(include_str!("close_epilogue.frag"));

    ShaderProgram::compile(
        renderer,
        &program,
        &[
            UniformName::new("niri_input_to_geo", UniformType::Matrix3x3),
            UniformName::new("niri_geo_size", UniformType::_2f),
            UniformName::new("niri_geo_to_tex", UniformType::Matrix3x3),
            UniformName::new("niri_progress", UniformType::_1f),
            UniformName::new("niri_clamped_progress", UniformType::_1f),
            UniformName::new("niri_random_seed", UniformType::_1f),
        ],
        &["niri_tex"],
    )
}

pub fn set_custom_close_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_close_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom close shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_close_program(program) {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom close shader: {err:?}");
        }
    }
}

fn compile_open_program(
    renderer: &mut GlesRenderer,
    src: &str,
) -> Result<ShaderProgram, GlesError> {
    let mut program = include_str!("open_prelude.frag").to_string();
    program.push_str(src);
    program.push_str(include_str!("open_epilogue.frag"));

    ShaderProgram::compile(
        renderer,
        &program,
        &[
            UniformName::new("niri_input_to_geo", UniformType::Matrix3x3),
            UniformName::new("niri_geo_size", UniformType::_2f),
            UniformName::new("niri_geo_to_tex", UniformType::Matrix3x3),
            UniformName::new("niri_progress", UniformType::_1f),
            UniformName::new("niri_clamped_progress", UniformType::_1f),
            UniformName::new("niri_random_seed", UniformType::_1f),
        ],
        &["niri_tex"],
    )
}

pub fn set_custom_open_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_open_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom open shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_open_program(program) {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom open shader: {err:?}");
        }
    }
}

pub fn mat3_uniform(name: &str, mat: Mat3) -> Uniform<'_> {
    Uniform::new(
        name,
        UniformValue::Matrix3x3 {
            matrices: vec![mat.to_cols_array()],
            transpose: false,
        },
    )
}
