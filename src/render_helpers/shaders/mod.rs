use std::cell::RefCell;

use glam::Mat3;
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
    UniformValue,
};

use super::renderer::NiriRenderer;
use super::shader_element::ShaderProgram;

pub struct Shaders {
    pub border: Option<ShaderProgram>,
    pub shadow: Option<ShaderProgram>,
    pub clipped_surface: Option<GlesTexProgram>,
    pub resize: Option<ShaderProgram>,
    pub gradient_fade: Option<GlesTexProgram>,
    pub custom_window_resize: RefCell<Option<ShaderProgram>>,
    pub custom_window_close: RefCell<Option<ShaderProgram>>,
    pub custom_window_open: RefCell<Option<ShaderProgram>>,
    pub custom_layer_close: RefCell<Option<ShaderProgram>>,
    pub custom_layer_open: RefCell<Option<ShaderProgram>>,
    pub custom_layer_bar_close: RefCell<Option<ShaderProgram>>,
    pub custom_layer_bar_open: RefCell<Option<ShaderProgram>>,
    pub custom_layer_wallpaper_close: RefCell<Option<ShaderProgram>>,
    pub custom_layer_wallpaper_open: RefCell<Option<ShaderProgram>>,
    pub custom_layer_launcher_close: RefCell<Option<ShaderProgram>>,
    pub custom_layer_launcher_open: RefCell<Option<ShaderProgram>>,
}

#[derive(Debug, Clone, Copy)]
pub enum ProgramType {
    Border,
    Shadow,
    WindowResize,
    WindowClose,
    WindowOpen,
    LayerClose,
    LayerOpen,
    LayerBarClose,
    LayerBarOpen,
    LayerWallpaperClose,
    LayerWallpaperOpen,
    LayerLauncherClose,
    LayerLauncherOpen,
}

impl Shaders {
    fn compile(renderer: &mut GlesRenderer) -> Self {
        let _span = tracy_client::span!("Shaders::compile");

        let border = ShaderProgram::compile(
            renderer,
            include_str!("border.frag"),
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

        let shadow = ShaderProgram::compile(
            renderer,
            include_str!("shadow.frag"),
            &[
                UniformName::new("shadow_color", UniformType::_4f),
                UniformName::new("sigma", UniformType::_1f),
                UniformName::new("input_to_geo", UniformType::Matrix3x3),
                UniformName::new("geo_size", UniformType::_2f),
                UniformName::new("corner_radius", UniformType::_4f),
                UniformName::new("window_input_to_geo", UniformType::Matrix3x3),
                UniformName::new("window_geo_size", UniformType::_2f),
                UniformName::new("window_corner_radius", UniformType::_4f),
            ],
            &[],
        )
        .map_err(|err| {
            warn!("error compiling shadow shader: {err:?}");
        })
        .ok();

        let clipped_surface = renderer
            .compile_custom_texture_shader(
                include_str!("clipped_surface.frag"),
                &[
                    UniformName::new("niri_scale", UniformType::_1f),
                    UniformName::new("geo_size", UniformType::_2f),
                    UniformName::new("corner_radius", UniformType::_4f),
                    UniformName::new("input_to_geo", UniformType::Matrix3x3),
                ],
            )
            .map_err(|err| {
                warn!("error compiling clipped surface shader: {err:?}");
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

        Self {
            border,
            shadow,
            clipped_surface,
            resize,
            gradient_fade,
            custom_window_resize: RefCell::new(None),
            custom_window_close: RefCell::new(None),
            custom_window_open: RefCell::new(None),
            custom_layer_close: RefCell::new(None),
            custom_layer_open: RefCell::new(None),
            custom_layer_bar_close: RefCell::new(None),
            custom_layer_bar_open: RefCell::new(None),
            custom_layer_wallpaper_close: RefCell::new(None),
            custom_layer_wallpaper_open: RefCell::new(None),
            custom_layer_launcher_close: RefCell::new(None),
            custom_layer_launcher_open: RefCell::new(None),
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

    pub fn replace_custom_window_resize_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_window_resize.replace(program)
    }

    pub fn replace_custom_window_close_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_window_close.replace(program)
    }

    pub fn replace_custom_window_open_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_window_open.replace(program)
    }

    pub fn replace_custom_layer_close_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_layer_close.replace(program)
    }

    pub fn replace_custom_layer_open_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_layer_open.replace(program)
    }

    pub fn replace_custom_layer_bar_close_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_layer_bar_close.replace(program)
    }

    pub fn replace_custom_layer_bar_open_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_layer_bar_open.replace(program)
    }

    pub fn replace_custom_layer_wallpaper_close_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_layer_wallpaper_close.replace(program)
    }

    pub fn replace_custom_layer_wallpaper_open_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_layer_wallpaper_open.replace(program)
    }

    pub fn replace_custom_layer_launcher_close_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_layer_launcher_close.replace(program)
    }

    pub fn replace_custom_layer_launcher_open_program(
        &self,
        program: Option<ShaderProgram>,
    ) -> Option<ShaderProgram> {
        self.custom_layer_launcher_open.replace(program)
    }

    pub fn program(&self, program: ProgramType) -> Option<ShaderProgram> {
        match program {
            ProgramType::Border => self.border.clone(),
            ProgramType::Shadow => self.shadow.clone(),
            ProgramType::WindowResize => self
                .custom_window_resize
                .borrow()
                .clone()
                .or_else(|| self.resize.clone()),
            ProgramType::WindowClose => self.custom_window_close.borrow().clone(),
            ProgramType::WindowOpen => self.custom_window_open.borrow().clone(),
            ProgramType::LayerClose => self.custom_layer_close.borrow().clone(),
            ProgramType::LayerOpen => self.custom_layer_open.borrow().clone(),
            ProgramType::LayerBarClose => self
                .custom_layer_bar_close
                .borrow()
                .clone()
                .or_else(|| self.custom_layer_close.borrow().clone()),
            ProgramType::LayerBarOpen => self
                .custom_layer_bar_open
                .borrow()
                .clone()
                .or_else(|| self.custom_layer_open.borrow().clone()),
            ProgramType::LayerWallpaperClose => self
                .custom_layer_wallpaper_close
                .borrow()
                .clone()
                .or_else(|| self.custom_layer_close.borrow().clone()),
            ProgramType::LayerWallpaperOpen => self
                .custom_layer_wallpaper_open
                .borrow()
                .clone()
                .or_else(|| self.custom_layer_open.borrow().clone()),
            ProgramType::LayerLauncherClose => self
                .custom_layer_launcher_close
                .borrow()
                .clone()
                .or_else(|| self.custom_layer_close.borrow().clone()),
            ProgramType::LayerLauncherOpen => self
                .custom_layer_launcher_open
                .borrow()
                .clone()
                .or_else(|| self.custom_layer_open.borrow().clone()),
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

    if let Some(prev) = Shaders::get(renderer).replace_custom_window_resize_program(program) {
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

pub fn set_custom_window_close_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_close_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom window close shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_window_close_program(program) {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom window close shader: {err:?}");
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

pub fn set_custom_window_open_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_open_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom window open shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_window_open_program(program) {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom window open shader: {err:?}");
        }
    }
}

pub fn set_custom_layer_open_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_open_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom layer open shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_layer_open_program(program) {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom layer open shader: {err:?}");
        }
    }
}

pub fn set_custom_layer_close_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_close_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom layer close shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_layer_close_program(program) {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom layer close shader: {err:?}");
        }
    }
}

pub fn set_custom_layer_bar_open_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_open_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom layer bar open shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_layer_bar_open_program(program) {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom layer bar open shader: {err:?}");
        }
    }
}

pub fn set_custom_layer_bar_close_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_close_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom layer bar close shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_layer_bar_close_program(program) {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom layer bar close shader: {err:?}");
        }
    }
}

pub fn set_custom_layer_wallpaper_open_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_open_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom layer wallpaper open shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_layer_wallpaper_open_program(program)
    {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom layer wallpaper open shader: {err:?}");
        }
    }
}

pub fn set_custom_layer_wallpaper_close_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_close_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom layer wallpaper close shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_layer_wallpaper_close_program(program)
    {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom layer wallpaper close shader: {err:?}");
        }
    }
}

pub fn set_custom_layer_launcher_open_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_open_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom layer launcher open shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_layer_launcher_open_program(program) {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom layer launcher open shader: {err:?}");
        }
    }
}

pub fn set_custom_layer_launcher_close_program(renderer: &mut GlesRenderer, src: Option<&str>) {
    let program = if let Some(src) = src {
        match compile_close_program(renderer, src) {
            Ok(program) => Some(program),
            Err(err) => {
                warn!("error compiling custom layer launcher close shader: {err:?}");
                return;
            }
        }
    } else {
        None
    };

    if let Some(prev) = Shaders::get(renderer).replace_custom_layer_launcher_close_program(program)
    {
        if let Err(err) = prev.destroy(renderer) {
            warn!("error destroying previous custom layer launcher close shader: {err:?}");
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
