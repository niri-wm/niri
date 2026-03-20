use std::collections::HashMap;
use std::ffi::CString;
use std::rc::Rc;

use glam::{Mat3, Vec2};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    ffi, link_program, Capability, GlesError, GlesFrame, GlesRenderer, GlesTexture, Uniform,
    UniformDesc, UniformName,
};
use smithay::backend::renderer::utils::{CommitCounter, OpaqueRegions};
use smithay::backend::renderer::DebugFlags;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size};

use super::renderer::AsGlesFrame;
use super::resources::Resources;
use super::shaders::{ProgramType, Shaders};
use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};

/// Renders a shader with optional texture input, on the primary GPU.
#[derive(Debug, Clone)]
pub struct ShaderRenderElement {
    program: ProgramType,
    id: Id,
    commit_counter: CommitCounter,
    area: Rectangle<f64, Logical>,
    opaque_regions: Vec<Rectangle<f64, Logical>>,
    // Should only be used for visual improvements, i.e. corner radius anti-aliasing.
    scale: f32,
    alpha: f32,
    additional_uniforms: Rc<[Uniform<'static>]>,
    textures: HashMap<String, GlesTexture>,
    kind: Kind,
}

#[derive(Debug, Clone)]
pub struct ShaderProgram(Rc<ShaderProgramInner>);

#[derive(Debug)]
struct ShaderProgramInner {
    normal: ShaderProgramInternal,
    debug: ShaderProgramInternal,
    uniform_tint: ffi::types::GLint,
}

#[derive(Debug)]
struct ShaderProgramInternal {
    program: ffi::types::GLuint,
    uniform_tex_matrix: ffi::types::GLint,
    uniform_matrix: ffi::types::GLint,
    uniform_size: ffi::types::GLint,
    uniform_scale: ffi::types::GLint,
    uniform_alpha: ffi::types::GLint,
    attrib_vert: ffi::types::GLint,
    attrib_vert_position: ffi::types::GLint,
    additional_uniforms: HashMap<String, UniformDesc>,
    texture_uniforms: HashMap<String, ffi::types::GLint>,
}

impl PartialEq for ShaderProgram {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

unsafe fn compile_program(
    gl: &ffi::Gles2,
    src: &str,
    additional_uniforms: &[UniformName<'_>],
    texture_uniforms: &[&str],
    // destruction_callback_sender: Sender<CleanupResource>,
) -> Result<ShaderProgram, GlesError> {
    let shader = format!("#version 100\n{src}");
    let program = unsafe { link_program(gl, include_str!("shaders/texture.vert"), &shader)? };
    let debug_shader = format!("#version 100\n#define DEBUG_FLAGS\n{src}");
    let debug_program =
        unsafe { link_program(gl, include_str!("shaders/texture.vert"), &debug_shader)? };

    let vert = c"vert";
    let vert_position = c"vert_position";
    let matrix = c"matrix";
    let tex_matrix = c"tex_matrix";
    let size = c"niri_size";
    let scale = c"niri_scale";
    let alpha = c"niri_alpha";
    let tint = c"niri_tint";

    Ok(ShaderProgram(Rc::new(ShaderProgramInner {
        normal: ShaderProgramInternal {
            program,
            uniform_matrix: gl.GetUniformLocation(program, matrix.as_ptr()),
            uniform_tex_matrix: gl.GetUniformLocation(program, tex_matrix.as_ptr()),
            uniform_size: gl.GetUniformLocation(program, size.as_ptr()),
            uniform_scale: gl.GetUniformLocation(program, scale.as_ptr()),
            uniform_alpha: gl.GetUniformLocation(program, alpha.as_ptr()),
            attrib_vert: gl.GetAttribLocation(program, vert.as_ptr()),
            attrib_vert_position: gl.GetAttribLocation(program, vert_position.as_ptr()),
            additional_uniforms: additional_uniforms
                .iter()
                .map(|uniform| {
                    let name =
                        CString::new(uniform.name.as_bytes()).expect("Interior null in name");
                    let location = gl.GetUniformLocation(program, name.as_ptr());
                    (
                        uniform.name.clone().into_owned(),
                        UniformDesc {
                            location,
                            type_: uniform.type_,
                        },
                    )
                })
                .collect(),
            texture_uniforms: texture_uniforms
                .iter()
                .map(|name_| {
                    let name = CString::new(name_.as_bytes()).expect("Interior null in name");
                    let location = gl.GetUniformLocation(program, name.as_ptr());
                    (name_.to_string(), location)
                })
                .collect(),
        },
        debug: ShaderProgramInternal {
            program: debug_program,
            uniform_matrix: gl.GetUniformLocation(debug_program, matrix.as_ptr()),
            uniform_tex_matrix: gl.GetUniformLocation(debug_program, tex_matrix.as_ptr()),
            uniform_size: gl.GetUniformLocation(debug_program, size.as_ptr()),
            uniform_scale: gl.GetUniformLocation(debug_program, scale.as_ptr()),
            uniform_alpha: gl.GetUniformLocation(debug_program, alpha.as_ptr()),
            attrib_vert: gl.GetAttribLocation(debug_program, vert.as_ptr()),
            attrib_vert_position: gl.GetAttribLocation(debug_program, vert_position.as_ptr()),
            additional_uniforms: additional_uniforms
                .iter()
                .map(|uniform| {
                    let name =
                        CString::new(uniform.name.as_bytes()).expect("Interior null in name");
                    let location = gl.GetUniformLocation(debug_program, name.as_ptr());
                    (
                        uniform.name.clone().into_owned(),
                        UniformDesc {
                            location,
                            type_: uniform.type_,
                        },
                    )
                })
                .collect(),
            texture_uniforms: texture_uniforms
                .iter()
                .map(|name_| {
                    let name = CString::new(name_.as_bytes()).expect("Interior null in name");
                    let location = gl.GetUniformLocation(debug_program, name.as_ptr());
                    (name_.to_string(), location)
                })
                .collect(),
        },
        uniform_tint: gl.GetUniformLocation(debug_program, tint.as_ptr()),
    })))
}

impl ShaderProgram {
    pub fn compile(
        renderer: &mut GlesRenderer,
        src: &str,
        additional_uniforms: &[UniformName<'_>],
        texture_uniforms: &[&str],
    ) -> Result<Self, GlesError> {
        renderer.with_context(move |gl| unsafe {
            compile_program(gl, src, additional_uniforms, texture_uniforms)
        })?
    }

    pub fn destroy(self, renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        renderer.with_context(move |gl| unsafe {
            gl.DeleteProgram(self.0.normal.program);
            gl.DeleteProgram(self.0.debug.program);
        })
    }
}

impl ShaderRenderElement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        program: ProgramType,
        size: Size<f64, Logical>,
        opaque_regions: Option<Vec<Rectangle<f64, Logical>>>,
        // Should only be used for visual improvements, i.e. corner radius anti-aliasing.
        scale: f32,
        alpha: f32,
        additional_uniforms: Rc<[Uniform<'static>]>,
        textures: HashMap<String, GlesTexture>,
        kind: Kind,
    ) -> Self {
        Self {
            program,
            id: Id::new(),
            commit_counter: CommitCounter::default(),
            area: Rectangle::from_size(size),
            opaque_regions: opaque_regions.unwrap_or_default(),
            scale,
            alpha,
            additional_uniforms,
            textures,
            kind,
        }
    }

    pub fn empty(program: ProgramType, kind: Kind) -> Self {
        Self {
            program,
            id: Id::new(),
            commit_counter: CommitCounter::default(),
            area: Rectangle::default(),
            opaque_regions: vec![],
            scale: 1.,
            alpha: 1.,
            additional_uniforms: Rc::new([]),
            textures: HashMap::new(),
            kind,
        }
    }

    pub fn damage_all(&mut self) {
        self.commit_counter.increment();
    }

    pub fn update(
        &mut self,
        size: Size<f64, Logical>,
        opaque_regions: Option<Vec<Rectangle<f64, Logical>>>,
        scale: f32,
        alpha: f32,
        uniforms: Rc<[Uniform<'static>]>,
        textures: HashMap<String, GlesTexture>,
    ) {
        self.area.size = size;
        self.opaque_regions = opaque_regions.unwrap_or_default();
        self.scale = scale;
        self.alpha = alpha;
        self.additional_uniforms = uniforms;
        self.textures = textures;

        self.commit_counter.increment();
    }

    pub fn with_location(mut self, location: Point<f64, Logical>) -> Self {
        self.area.loc = location;
        self
    }

    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }
}

impl Element for ShaderRenderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit_counter
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(Size::from((1., 1.)))
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.area.to_physical_precise_round(scale)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.opaque_regions
            .iter()
            .map(|region| region.to_physical_precise_down(scale))
            .collect()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        self.kind
    }
}

impl RenderElement<GlesRenderer> for ShaderRenderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dest: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let _span = tracy_client::span!("ShaderRenderElement::draw");

        let frame = frame.as_gles_frame();

        let Some(shader) = Shaders::get_from_frame(frame).program(self.program) else {
            return Ok(());
        };

        let Some(resources) = Resources::get(frame) else {
            return Ok(());
        };
        let mut resources = resources.borrow_mut();

        let supports_instancing = frame.capabilities().contains(&Capability::Instancing);

        // prepare the vertices
        resources.vertices.clear();
        if supports_instancing {
            resources.vertices.extend(damage.iter().flat_map(|rect| {
                let dest_size = dest.size;

                let rect_constrained_loc = rect
                    .loc
                    .constrain(Rectangle::from_extremities((0, 0), dest_size.to_point()));
                let rect_clamped_size = rect.size.clamp(
                    (0, 0),
                    (dest_size.to_point() - rect_constrained_loc).to_size(),
                );

                let rect = Rectangle::new(rect_constrained_loc, rect_clamped_size);
                [
                    rect.loc.x as f32,
                    rect.loc.y as f32,
                    rect.size.w as f32,
                    rect.size.h as f32,
                ]
            }));
        } else {
            resources.vertices.extend(damage.iter().flat_map(|rect| {
                let dest_size = dest.size;

                let rect_constrained_loc = rect
                    .loc
                    .constrain(Rectangle::from_extremities((0, 0), dest_size.to_point()));
                let rect_clamped_size = rect.size.clamp(
                    (0, 0),
                    (dest_size.to_point() - rect_constrained_loc).to_size(),
                );

                let rect = Rectangle::new(rect_constrained_loc, rect_clamped_size);
                // Add the 4 f32s per damage rectangle for each of the 6 vertices.
                (0..6).flat_map(move |_| {
                    [
                        rect.loc.x as f32,
                        rect.loc.y as f32,
                        rect.size.w as f32,
                        rect.size.h as f32,
                    ]
                })
            }));
        }

        if resources.vertices.is_empty() {
            return Ok(());
        }

        // dest position and scale
        let mut matrix = Mat3::from_translation(Vec2::new(dest.loc.x as f32, dest.loc.y as f32));

        let scale = src.size.to_f64() / dest.size.to_f64();
        let tex_matrix = Mat3::from_scale(Vec2::new(scale.x as f32, scale.y as f32));
        let tex_matrix =
            Mat3::from_translation(Vec2::new(src.loc.x as f32, src.loc.y as f32)) * tex_matrix;

        //apply output transformation
        matrix = Mat3::from_cols_array(frame.projection()) * matrix;

        let has_debug = !frame.debug_flags().is_empty();
        let has_tint = frame.debug_flags().contains(DebugFlags::TINT);

        // render
        let span_loc = smithay::gpu_span_location!("draw shader");
        frame.with_profiled_context(span_loc, move |gl| -> Result<(), GlesError> {
            let program = if has_debug {
                &shader.0.debug
            } else {
                &shader.0.normal
            };

            unsafe {
                for (i, texture) in self.textures.values().enumerate() {
                    gl.ActiveTexture(ffi::TEXTURE0 + i as u32);
                    gl.BindTexture(ffi::TEXTURE_2D, texture.tex_id());
                    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
                    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
                    gl.TexParameteri(
                        ffi::TEXTURE_2D,
                        ffi::TEXTURE_WRAP_S,
                        ffi::CLAMP_TO_BORDER as i32,
                    );
                    gl.TexParameteri(
                        ffi::TEXTURE_2D,
                        ffi::TEXTURE_WRAP_T,
                        ffi::CLAMP_TO_BORDER as i32,
                    );
                }

                gl.UseProgram(program.program);

                for (i, name) in self.textures.keys().enumerate() {
                    gl.Uniform1i(program.texture_uniforms[name], i as i32);
                }

                gl.UniformMatrix3fv(
                    program.uniform_matrix,
                    1,
                    ffi::FALSE,
                    matrix.as_ref().as_ptr(),
                );
                gl.UniformMatrix3fv(
                    program.uniform_tex_matrix,
                    1,
                    ffi::FALSE,
                    tex_matrix.as_ref().as_ptr(),
                );
                gl.Uniform2f(program.uniform_size, dest.size.w as f32, dest.size.h as f32);
                gl.Uniform1f(program.uniform_scale, self.scale);
                gl.Uniform1f(program.uniform_alpha, self.alpha);

                let tint = if has_tint { 1.0f32 } else { 0.0f32 };
                if has_debug {
                    gl.Uniform1f(shader.0.uniform_tint, tint);
                }

                for uniform in &*self.additional_uniforms {
                    let desc =
                        program
                            .additional_uniforms
                            .get(&*uniform.name)
                            .ok_or_else(|| {
                                GlesError::UnknownUniform(uniform.name.clone().into_owned())
                            })?;
                    uniform.value.set(gl, desc)?;
                }

                gl.EnableVertexAttribArray(program.attrib_vert as u32);
                gl.BindBuffer(ffi::ARRAY_BUFFER, resources.vbos[0]);
                gl.VertexAttribPointer(
                    program.attrib_vert as u32,
                    2,
                    ffi::FLOAT,
                    ffi::FALSE,
                    0,
                    std::ptr::null(),
                );

                // vert_position
                gl.EnableVertexAttribArray(program.attrib_vert_position as u32);
                gl.BindBuffer(ffi::ARRAY_BUFFER, resources.vbos[1]);
                gl.BufferData(
                    ffi::ARRAY_BUFFER,
                    (std::mem::size_of::<ffi::types::GLfloat>() * resources.vertices.len())
                        as isize,
                    resources.vertices.as_ptr() as *const _,
                    ffi::STREAM_DRAW,
                );

                gl.VertexAttribPointer(
                    program.attrib_vert_position as u32,
                    4,
                    ffi::FLOAT,
                    ffi::FALSE,
                    0,
                    std::ptr::null(),
                );

                let damage_len = damage.len() as i32;
                if supports_instancing {
                    gl.VertexAttribDivisor(program.attrib_vert as u32, 0);
                    gl.VertexAttribDivisor(program.attrib_vert_position as u32, 1);
                    gl.DrawArraysInstanced(ffi::TRIANGLE_STRIP, 0, 4, damage_len);
                } else {
                    // When we have more than 10 rectangles, draw them in batches of 10.
                    for i in 0..(damage_len - 1) / 10 {
                        gl.DrawArrays(ffi::TRIANGLES, 0, 6);

                        // Set damage pointer to the next 10 rectangles.
                        let offset =
                            (i + 1) as usize * 6 * 4 * std::mem::size_of::<ffi::types::GLfloat>();
                        gl.VertexAttribPointer(
                            program.attrib_vert_position as u32,
                            4,
                            ffi::FLOAT,
                            ffi::FALSE,
                            0,
                            offset as *const _,
                        );
                    }

                    // Draw the up to 10 remaining rectangles.
                    let count = ((damage_len - 1) % 10 + 1) * 6;
                    gl.DrawArrays(ffi::TRIANGLES, 0, count);
                }

                gl.BindBuffer(ffi::ARRAY_BUFFER, 0);
                gl.BindTexture(ffi::TEXTURE_2D, 0);
                gl.ActiveTexture(ffi::TEXTURE0);
                gl.BindTexture(ffi::TEXTURE_2D, 0);
                gl.DisableVertexAttribArray(program.attrib_vert as u32);
                gl.DisableVertexAttribArray(program.attrib_vert_position as u32);
            }

            Ok(())
        })??;

        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // If scanout for things other than Wayland buffers is implemented, this will need to take
        // the target GPU into account.
        None
    }
}

impl<'render> RenderElement<TtyRenderer<'render>> for ShaderRenderElement {
    fn draw(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), TtyRendererError<'render>> {
        let frame = frame.as_gles_frame();

        RenderElement::<GlesRenderer>::draw(self, frame, src, dst, damage, opaque_regions)?;

        Ok(())
    }

    fn underlying_storage(
        &self,
        _renderer: &mut TtyRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        // If scanout for things other than Wayland buffers is implemented, this will need to take
        // the target GPU into account.
        None
    }
}
