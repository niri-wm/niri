use std::cell::RefCell;
use std::rc::Rc;

use smithay::backend::renderer::gles::{ffi, Capability, GlesError, GlesFrame, GlesRenderer};

pub struct Resources {
    pub vertices: Vec<f32>,
    pub vbos: [ffi::types::GLuint; 2],
}

static INSTANCED_VERTS: [ffi::types::GLfloat; 8] = [
    1.0, 0.0, // top right
    0.0, 0.0, // top left
    1.0, 1.0, // bottom right
    0.0, 1.0, // bottom left
];

/// Vertices for rendering individual triangles.
const MAX_RECTS_PER_DRAW: usize = 10;
const TRIANGLE_VERTS: [ffi::types::GLfloat; 12 * MAX_RECTS_PER_DRAW] = triangle_verts();
const fn triangle_verts() -> [ffi::types::GLfloat; 12 * MAX_RECTS_PER_DRAW] {
    let mut verts = [0.; 12 * MAX_RECTS_PER_DRAW];
    let mut i = 0;
    loop {
        // Top Left.
        verts[i * 12] = 0.0;
        verts[i * 12 + 1] = 0.0;

        // Bottom left.
        verts[i * 12 + 2] = 0.0;
        verts[i * 12 + 3] = 1.0;

        // Bottom right.
        verts[i * 12 + 4] = 1.0;
        verts[i * 12 + 5] = 1.0;

        // Top left.
        verts[i * 12 + 6] = 0.0;
        verts[i * 12 + 7] = 0.0;

        // Bottom right.
        verts[i * 12 + 8] = 1.0;
        verts[i * 12 + 9] = 1.0;

        // Top right.
        verts[i * 12 + 10] = 1.0;
        verts[i * 12 + 11] = 0.0;

        i += 1;
        if i == MAX_RECTS_PER_DRAW {
            break;
        }
    }
    verts
}

impl Resources {
    fn create(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let _span = tracy_client::span!("Resources::init");

        let supports_instancing = renderer.capabilities().contains(&Capability::Instancing);
        renderer.with_context(|gl| unsafe {
            let vertices: &[ffi::types::GLfloat] = if supports_instancing {
                &INSTANCED_VERTS
            } else {
                &TRIANGLE_VERTS
            };

            let mut vbos = [0; 2];
            gl.GenBuffers(vbos.len() as i32, vbos.as_mut_ptr());
            gl.BindBuffer(ffi::ARRAY_BUFFER, vbos[0]);
            gl.BufferData(
                ffi::ARRAY_BUFFER,
                std::mem::size_of_val(vertices) as isize,
                vertices.as_ptr() as *const _,
                ffi::STATIC_DRAW,
            );

            gl.BindBuffer(ffi::ARRAY_BUFFER, 0);

            Self {
                vertices: vec![],
                vbos,
            }
        })
    }

    pub fn get(frame: &mut GlesFrame) -> Option<Rc<RefCell<Self>>> {
        let data = frame.egl_context().user_data();
        data.get().cloned()
    }
}

pub fn init(renderer: &mut GlesRenderer) {
    match Resources::create(renderer) {
        Ok(resources) => {
            let data = renderer.egl_context().user_data();
            if !data.insert_if_missing(|| Rc::new(RefCell::new(resources))) {
                error!("resources were already initialized");
            }
        }
        Err(err) => {
            warn!("error creating resources for rendering: {err:?}");
        }
    }
}
