use smithay::delegate_layer_shell;
use smithay::desktop::{layer_map_for_output, LayerSurface, PopupKind, WindowSurfaceType};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{get_parent, with_states};
use smithay::wayland::shell::wlr_layer::{
    self, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler,
    WlrLayerShellState,
};
use smithay::wayland::shell::xdg::PopupSurface;

use crate::layer::{MappedLayer, ResolvedLayerRules};
use crate::niri::State;
use crate::utils::{is_mapped, output_size, send_scale_transform};

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.niri.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let output = if let Some(wl_output) = &wl_output {
            Output::from_resource(wl_output)
        } else {
            self.niri.layout.active_output().cloned()
        };
        let Some(output) = output else {
            warn!("no output for new layer surface, closing");
            surface.send_close();
            return;
        };

        let wl_surface = surface.wl_surface().clone();
        let is_new = self.niri.unmapped_layer_surfaces.insert(wl_surface);
        assert!(is_new);

        let mut map = layer_map_for_output(&output);
        map.map_layer(&LayerSurface::new(surface, namespace))
            .unwrap();
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let wl_surface = surface.wl_surface();
        self.niri.unmapped_layer_surfaces.remove(wl_surface);

        let output = if let Some((output, mut map, layer)) =
            self.niri.layout.outputs().find_map(|o| {
                let map = layer_map_for_output(o);
                let layer = map
                    .layers()
                    .find(|&layer| layer.layer_surface() == &surface)
                    .cloned();
                layer.map(|layer| (o.clone(), map, layer))
            }) {
            map.unmap_layer(&layer);
            self.niri.mapped_layer_surfaces.remove(&layer);
            Some(output)
        } else {
            None
        };
        if let Some(output) = output {
            self.niri.output_resized(&output);
        }
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: PopupSurface) {
        self.unconstrain_popup(&PopupKind::Xdg(popup));
    }
}
delegate_layer_shell!(State);

impl State {
    pub fn layer_shell_handle_commit(&mut self, surface: &WlSurface) -> bool {
        let mut root_surface = surface.clone();
        while let Some(parent) = get_parent(&root_surface) {
            root_surface = parent;
        }

        let output = self
            .niri
            .layout
            .outputs()
            .find(|o| {
                let map = layer_map_for_output(o);
                map.layer_for_surface(&root_surface, WindowSurfaceType::TOPLEVEL)
                    .is_some()
            })
            .cloned();
        let Some(output) = output else {
            return false;
        };

        if surface != &root_surface {
            // This is an unsync layer-shell subsurface.
            self.niri.queue_redraw(&output);
            return true;
        }

        let mut map = layer_map_for_output(&output);

        // Arrange the layers before sending the initial configure to respect any size the
        // client may have sent.
        map.arrange();

        let layer = map
            .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
            .unwrap();

        if is_mapped(surface) {
            let was_unmapped = self.niri.unmapped_layer_surfaces.remove(surface);

            // Resolve rules for newly mapped layer surfaces.
            if was_unmapped {
                let config = self.niri.config.borrow();

                let rules = &config.layer_rules;
                let rules = ResolvedLayerRules::compute(rules, layer, self.niri.is_at_startup);

                let output_size = output_size(&output);
                let scale = output.current_scale().fractional_scale();

                let mapped = MappedLayer::new(
                    layer.clone(),
                    rules,
                    output_size,
                    scale,
                    self.niri.clock.clone(),
                    &config,
                );

                let prev = self
                    .niri
                    .mapped_layer_surfaces
                    .insert(layer.clone(), mapped);
                if prev.is_some() {
                    error!("MappedLayer was present for an unmapped surface");
                }
            }

            // Give focus to newly mapped on-demand surfaces. Some launchers like lxqt-runner rely
            // on this behavior. While this behavior doesn't make much sense for other clients like
            // panels, the consensus seems to be that it's not a big deal since panels generally
            // only open once at the start of the session.
            //
            // Note that:
            // 1) Exclusive layer surfaces already get focus automatically in
            //    update_keyboard_focus().
            // 2) Same-layer exclusive layer surfaces are already preferred to on-demand surfaces in
            //    update_keyboard_focus(), so we don't need to check for that here.
            //
            // https://github.com/niri-wm/niri/issues/641
            let on_demand = layer.cached_state().keyboard_interactivity
                == wlr_layer::KeyboardInteractivity::OnDemand;
            if was_unmapped && on_demand {
                // I guess it'd make sense to check that no higher-layer on-demand surface
                // has focus, but Smithay's Layer doesn't implement Ord so this would be a
                // little annoying.
                self.niri.layer_shell_on_demand_focus = Some(layer.clone());
            }
        } else {
            // The surface is unmapped.
            if self.niri.mapped_layer_surfaces.remove(layer).is_some() {
                // A mapped surface got unmapped via a null commit. Now it needs to do a new
                // initial commit again.
                self.niri.unmapped_layer_surfaces.insert(surface.clone());
            } else {
                // An unmapped surface remains unmapped. If we haven't sent an initial configure
                // yet, we should do so.
                let initial_configure_sent = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<LayerSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .initial_configure_sent
                });
                if !initial_configure_sent {
                    let scale = output.current_scale();
                    let transform = output.current_transform();
                    with_states(surface, |data| {
                        send_scale_transform(surface, data, scale, transform);
                    });

                    layer.layer_surface().send_configure();
                }
                // If we already sent an initial configure, then map.arrange() above had just sent
                // it a new configure, if needed.
            }
        }

        drop(map);

        // This will call queue_redraw() inside.
        self.niri.output_resized(&output);

        true
    }
}
