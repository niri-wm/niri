use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::{import_surface, RendererSurfaceStateUserData};
use smithay::backend::renderer::{ImportAll, Renderer};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Scale};
use smithay::wayland::compositor::{with_surface_tree_downward, TraversalAction};

use super::texture::TextureBuffer;
use super::BakedBuffer;

/// Renders elements from a surface tree as textures into `storage`.
pub fn render_snapshot_from_surface_tree(
    renderer: &mut GlesRenderer,
    surface: &WlSurface,
    location: Point<f64, Logical>,
    storage: &mut Vec<BakedBuffer<TextureBuffer<GlesTexture>>>,
) {
    let _span = tracy_client::span!("render_snapshot_from_surface_tree");

    with_surface_tree_downward(
        surface,
        location,
        |_, states, location| {
            let mut location = *location;
            let data = states.data_map.get::<RendererSurfaceStateUserData>();

            if let Some(data) = data {
                let data = &*data.lock().unwrap();

                if let Some(view) = data.view() {
                    location += view.offset.to_f64();
                    TraversalAction::DoChildren(location)
                } else {
                    TraversalAction::SkipChildren
                }
            } else {
                TraversalAction::SkipChildren
            }
        },
        |_, states, location| {
            let mut location = *location;
            let data = states.data_map.get::<RendererSurfaceStateUserData>();

            if let Some(data) = data {
                let Some(view) = data.lock().unwrap().view() else {
                    return;
                };
                location += view.offset.to_f64();

                if let Err(err) = import_surface(renderer, states) {
                    warn!("failed to import surface: {err:?}");
                    return;
                }

                let data = data.lock().unwrap();
                let Some(texture) = data.texture(renderer.context_id()) else {
                    return;
                };

                let buffer = TextureBuffer::from_texture(
                    renderer,
                    texture.clone(),
                    f64::from(data.buffer_scale()),
                    data.buffer_transform(),
                    Vec::new(),
                );

                let baked = BakedBuffer {
                    buffer,
                    location,
                    src: Some(view.src),
                    dst: Some(view.dst),
                };

                storage.push(baked);
            }
        },
        |_, _, _| true,
    );
}

pub fn push_elements_from_surface_tree<R>(
    renderer: &mut R,
    surface: &WlSurface,
    // Fractional scale expects surface buffers to be aligned to physical pixels.
    location: Point<i32, Physical>,
    scale: Scale<f64>,
    alpha: f32,
    kind: Kind,
    push: &mut dyn FnMut(WaylandSurfaceRenderElement<R>),
) where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    let _span = tracy_client::span!("push_elements_from_surface_tree");

    let location = location.to_f64();

    with_surface_tree_downward(
        surface,
        location,
        |_, states, location| {
            let mut location = *location;
            let data = states.data_map.get::<RendererSurfaceStateUserData>();

            if let Some(data) = data {
                if let Some(view) = data.lock().unwrap().view() {
                    location += view.offset.to_f64().to_physical(scale);
                    TraversalAction::DoChildren(location)
                } else {
                    TraversalAction::SkipChildren
                }
            } else {
                TraversalAction::SkipChildren
            }
        },
        |surface, states, location| {
            let mut location = *location;
            let data = states.data_map.get::<RendererSurfaceStateUserData>();

            if let Some(data) = data {
                let has_view = if let Some(view) = data.lock().unwrap().view() {
                    location += view.offset.to_f64().to_physical(scale);
                    true
                } else {
                    false
                };

                if has_view {
                    match WaylandSurfaceRenderElement::from_surface(
                        renderer, surface, states, location, alpha, kind,
                    ) {
                        Ok(Some(surface)) => push(surface),
                        Ok(None) => {} // surface is not mapped
                        Err(err) => {
                            warn!("failed to import surface: {}", err);
                        }
                    };
                }
            }
        },
        |_, _, _| true,
    );
}
