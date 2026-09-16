use std::cell::RefCell;

use glam::Mat3;
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
    UniformValue,
};

use super::renderer::NiriRenderer;
use super::shader_element::ShaderProgram;
use crate::render_helpers::blur::BlurProgram;

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

impl Shaders {
    fn compile(renderer: &mut GlesRenderer) -> Self {
        let _span = tracy_client::span!("Shaders::compile");

        let border = ShaderProgram::compile(
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
                UniformName::new("border_width", UniformType::_1f),
            ],
            &[],
        )
        .map_err(|err| {
            warn!("error compiling border shader: {err:?}");
        })
        .ok();

        // GLES2 only guarantees 16 fragment uniform vectors. Keep the shadow
        // geometry transforms packed as scale/translation vec4s.
        let shadow = ShaderProgram::compile(
            renderer,
            concat!(
                include_str!("shadow.frag"),
                include_str!("rounded_sdf.frag"),
                include_str!("rounding_alpha.frag")
            ),
            &[
                UniformName::new("shadow_color", UniformType::_4f),
                UniformName::new("sigma", UniformType::_1f),
                UniformName::new("input_to_geo", UniformType::_4f),
                UniformName::new("geo_size", UniformType::_2f),
                UniformName::new("corner_radius", UniformType::_4f),
                UniformName::new("window_input_to_geo", UniformType::_4f),
                UniformName::new("window_geo_size", UniformType::_2f),
                UniformName::new("window_corner_radius", UniformType::_4f),
                UniformName::new("feather_input_to_geo", UniformType::_4f),
                UniformName::new("feather_geo_size", UniformType::_2f),
                UniformName::new("feather_corner_radius", UniformType::_4f),
                UniformName::new("feather", UniformType::_1f),
            ],
            &[],
        )
        .map_err(|err| {
            warn!("error compiling shadow shader: {err:?}");
        })
        .ok();

        let clipped_surface = renderer
            .compile_custom_texture_shader(
                concat!(
                    include_str!("clipped_surface.frag"),
                    include_str!("rounded_sdf.frag"),
                    include_str!("rounding_alpha.frag"),
                    "\nvec4 postprocess(vec4 color) { return color; }",
                ),
                &[
                    UniformName::new("niri_scale", UniformType::_1f),
                    UniformName::new("geo_size", UniformType::_2f),
                    UniformName::new("corner_radius", UniformType::_4f),
                    UniformName::new("input_to_geo", UniformType::Matrix3x3),
                    UniformName::new("refraction_params", UniformType::_4f),
                    UniformName::new("effect_params", UniformType::_2f),
                ],
            )
            .map_err(|err| {
                warn!("error compiling clipped surface shader: {err:?}");
            })
            .ok();

        let postprocess_and_clip = renderer
            .compile_custom_texture_shader(
                concat!(
                    include_str!("clipped_surface.frag"),
                    include_str!("rounded_sdf.frag"),
                    include_str!("rounding_alpha.frag"),
                    include_str!("postprocess.frag"),
                ),
                &[
                    UniformName::new("niri_scale", UniformType::_1f),
                    UniformName::new("geo_size", UniformType::_2f),
                    UniformName::new("corner_radius", UniformType::_4f),
                    UniformName::new("input_to_geo", UniformType::Matrix3x3),
                    UniformName::new("noise", UniformType::_1f),
                    UniformName::new("saturation", UniformType::_1f),
                    UniformName::new("bg_color", UniformType::_4f),
                    UniformName::new("refraction_params", UniformType::_4f),
                    UniformName::new("effect_params", UniformType::_2f),
                ],
            )
            .map_err(|err| {
                warn!("error compiling postprocess_and_clip shader: {err:?}");
            })
            .ok();

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
    let mut program = include_str!("resize_prelude.frag").to_string();
    program.push_str(src);
    program.push_str(include_str!("resize_epilogue.frag"));
    program.push_str(include_str!("rounding_alpha.frag"));

    ShaderProgram::compile(
        renderer,
        &program,
        &[
            UniformName::new("niri_input_to_curr_geo", UniformType::Matrix3x3),
            UniformName::new("niri_curr_geo_to_prev_geo", UniformType::Matrix3x3),
            UniformName::new("niri_curr_geo_to_next_geo", UniformType::Matrix3x3),
            UniformName::new("niri_curr_geo_size", UniformType::_2f),
            UniformName::new("niri_geo_to_tex_prev", UniformType::Matrix3x3),
            UniformName::new("niri_geo_to_tex_next", UniformType::Matrix3x3),
            UniformName::new("niri_progress", UniformType::_1f),
            UniformName::new("niri_clamped_progress", UniformType::_1f),
            UniformName::new("niri_corner_radius", UniformType::_4f),
            UniformName::new("niri_clip_to_geometry", UniformType::_1f),
        ],
        &["niri_tex_prev", "niri_tex_next"],
    )
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    // GLES2 guarantees 16 fragment uniform vectors and 8 fragment texture
    // image units. Samplers are tracked separately from uniform vectors.
    const MIN_FRAGMENT_UNIFORM_VECTORS: usize = 16;
    const MIN_FRAGMENT_TEXTURE_IMAGE_UNITS: usize = 8;

    #[derive(Debug, Default, PartialEq, Eq)]
    struct FragmentBudget {
        vectors: usize,
        samplers: usize,
    }

    fn uniform_kind(type_name: &str) -> Option<(usize, bool)> {
        let vectors = match type_name {
            "float" | "int" | "bool" | "vec2" | "vec3" | "vec4" => 1,
            "mat2" => 2,
            "mat3" => 3,
            "mat4" => 4,
            "mat2x3" | "mat2x4" => 2,
            "mat3x2" | "mat3x4" => 3,
            "mat4x2" | "mat4x3" => 4,
            "sampler2D" | "samplerExternalOES" | "samplerCube" => return Some((0, true)),
            _ => return None,
        };
        Some((vectors, false))
    }

    fn source_budget(sources: &[&str]) -> FragmentBudget {
        let mut declarations = BTreeMap::<String, (usize, usize)>::new();

        for source in sources {
            for line in source.lines() {
                let line = line.split("//").next().unwrap_or_default().trim();
                let Some(rest) = line.strip_prefix("uniform ") else {
                    continue;
                };
                let rest = rest.trim_end_matches(';').trim();
                let mut tokens = rest.split_whitespace();
                let mut kind = None;
                let mut name = None;

                for token in tokens.by_ref() {
                    if kind.is_none() {
                        kind = uniform_kind(token);
                    } else {
                        name = Some(token);
                        break;
                    }
                }

                let Some((vectors, sampler)) = kind else {
                    continue;
                };
                let Some(name) = name else {
                    continue;
                };
                let (name, count) = name.split_once('[').map_or((name, 1), |(name, array)| {
                    let count = array.trim_end_matches(']').parse::<usize>().unwrap_or(1);
                    (name, count)
                });
                declarations
                    .entry(name.to_string())
                    .or_insert((vectors * count, if sampler { count } else { 0 }));
            }
        }

        declarations.into_values().fold(
            FragmentBudget::default(),
            |mut budget, (vectors, samplers)| {
                budget.vectors += vectors;
                budget.samplers += samplers;
                budget
            },
        )
    }

    fn assert_gles2_budget(name: &str, budget: FragmentBudget) {
        assert!(
            budget.vectors <= MIN_FRAGMENT_UNIFORM_VECTORS,
            "{name} uses {} fragment uniform vectors, minimum is {MIN_FRAGMENT_UNIFORM_VECTORS}",
            budget.vectors
        );
        assert!(
            budget.samplers <= MIN_FRAGMENT_TEXTURE_IMAGE_UNITS,
            "{name} uses {} fragment samplers, minimum is {MIN_FRAGMENT_TEXTURE_IMAGE_UNITS}",
            budget.samplers
        );
    }

    #[test]
    fn affected_fragment_variants_fit_gles2_minimum() {
        let shadow = source_budget(&[
            include_str!("shadow.frag"),
            include_str!("rounded_sdf.frag"),
            include_str!("rounding_alpha.frag"),
        ]);
        assert_eq!(
            shadow,
            FragmentBudget {
                vectors: 15,
                samplers: 0
            }
        );
        assert_gles2_budget("shadow", shadow);

        let clipped = source_budget(&[
            include_str!("clipped_surface.frag"),
            include_str!("rounded_sdf.frag"),
            include_str!("rounding_alpha.frag"),
        ]);
        assert_eq!(
            clipped,
            FragmentBudget {
                vectors: 10,
                samplers: 1
            }
        );
        assert_gles2_budget("clipped_surface", clipped);

        let postprocess = source_budget(&[
            include_str!("clipped_surface.frag"),
            include_str!("rounded_sdf.frag"),
            include_str!("rounding_alpha.frag"),
            include_str!("postprocess.frag"),
        ]);
        assert_eq!(
            postprocess,
            FragmentBudget {
                vectors: 13,
                samplers: 1
            }
        );
        assert_gles2_budget("postprocess_and_clip", postprocess);
    }
}
