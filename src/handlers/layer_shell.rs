use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::delegate_layer_shell;
use smithay::desktop::{layer_map_for_output, LayerSurface, PopupKind, WindowSurfaceType};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Rectangle, Scale};
use smithay::wayland::compositor::{
    add_pre_commit_hook, get_parent, with_states, BufferAssignment, HookId, SurfaceAttributes,
};
use smithay::wayland::shell::wlr_layer::{
    self, Anchor, ExclusiveZone, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceCachedState,
    LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::wayland::shell::xdg::PopupSurface;

use crate::animation::Animation;
use crate::layer::closing_layer::ClosingLayer;
use crate::layer::{MappedLayer, ResolvedLayerRules};
use crate::niri::{ClosingLayerState, State};
use crate::render_helpers::shaders::ProgramType;
use crate::utils::{is_mapped, output_size, send_scale_transform};

#[derive(Clone, Copy)]
enum LayerAnimationKind {
    Bar,
    Wallpaper,
    Launcher,
}

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
            self.niri.output_from_resource(wl_output)
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

        let found = self.niri.layout.outputs().find_map(|o| {
            let map = layer_map_for_output(o);
            let layer = map
                .layers()
                .find(|&layer| layer.layer_surface() == &surface)
                .cloned()?;
            Some((o.clone(), layer))
        });

        let output = if let Some((output, layer)) = found {
            let mut map = layer_map_for_output(&output);
            let geo = map.layer_geometry(&layer);

            if let Some(mapped) = self.niri.mapped_layer_surfaces.remove(&layer) {
                if let Some(geo) = geo {
                    self.start_close_animation_for_layer(&output, &layer, geo, mapped);
                }
            }

            map.unmap_layer(&layer);
            drop(map);
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

        if is_mapped(surface) {
            // Arrange mapped layers before handling commits so initial configures and geometry are
            // up-to-date for mapped surfaces.
            map.arrange();

            let layer = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .unwrap();

            let was_mapped = self.niri.mapped_layer_surfaces.contains_key(layer);

            if let Some(idx) = self
                .niri
                .closing_layers
                .iter()
                .position(|closing| &closing.surface == layer)
            {
                self.niri.closing_layers.remove(idx);
            }

            // Handle map edge: create state and start the open animation once.
            // And resolve rules for newly mapped layer surfaces.
            if !was_mapped {
                self.niri.unmapped_layer_surfaces.remove(surface);

                let config = self.niri.config.borrow();

                let rules = &config.layer_rules;
                let rules = ResolvedLayerRules::compute(rules, layer, self.niri.is_at_startup);

                let kind = resolve_layer_animation_kind(layer);
                let (anim_config, program) = match kind {
                    LayerAnimationKind::Bar => (
                        config
                            .animations
                            .layer_bar_open
                            .as_ref()
                            .unwrap_or(&config.animations.layer_open),
                        ProgramType::LayerBarOpen,
                    ),
                    LayerAnimationKind::Wallpaper => (
                        config
                            .animations
                            .layer_wallpaper_open
                            .as_ref()
                            .unwrap_or(&config.animations.layer_open),
                        ProgramType::LayerWallpaperOpen,
                    ),
                    LayerAnimationKind::Launcher => (
                        config
                            .animations
                            .layer_launcher_open
                            .as_ref()
                            .unwrap_or(&config.animations.layer_open),
                        ProgramType::LayerLauncherOpen,
                    ),
                };

                let output_size = output_size(&output);
                let scale = output.current_scale().fractional_scale();

                let hook = add_mapped_layer_pre_commit_hook(layer);
                let mut mapped = MappedLayer::new(
                    layer.clone(),
                    hook,
                    rules,
                    output_size,
                    scale,
                    self.niri.clock.clone(),
                    &config,
                );

                // Start the open animation immediately on map.
                mapped.start_open_animation(anim_config, program);

                let prev = self
                    .niri
                    .mapped_layer_surfaces
                    .insert(layer.clone(), mapped);
                if prev.is_some() {
                    error!("MappedLayer was present for an unmapped surface");
                }
            } else {
                // The surface remains mapped.
                if let Some(mapped) = self.niri.mapped_layer_surfaces.get_mut(layer) {
                    // Check if the layer changed.
                    if mapped.take_recompute_rules_on_commit() {
                        let config = self.niri.config.borrow();
                        if mapped
                            .recompute_layer_rules(&config.layer_rules, self.niri.is_at_startup)
                        {
                            mapped.update_config(&config);
                        }
                    }
                } else {
                    error!("MappedLayer missing for a mapped surface");
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
            if !was_mapped && on_demand {
                // I guess it'd make sense to check that no higher-layer on-demand surface
                // has focus, but Smithay's Layer doesn't implement Ord so this would be a
                // little annoying.
                self.niri.layer_shell_on_demand_focus = Some(layer.clone());
            }
        } else {
            let layer = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .unwrap();

            // The surface is unmapped.
            // Use the pre-arrange geometry for close animation; some clients clear/unmap content
            // quickly and may lose geometry if we arrange first.
            let geo = map.layer_geometry(layer);
            if let Some(mapped) = self.niri.mapped_layer_surfaces.remove(layer) {
                let mut mapped = mapped;
                if mapped.take_recompute_rules_on_commit() {
                    let config = self.niri.config.borrow();
                    if mapped.recompute_layer_rules(&config.layer_rules, self.niri.is_at_startup) {
                        mapped.update_config(&config);
                    }
                }

                if let Some(geo) = geo {
                    self.start_close_animation_for_layer(&output, layer, geo, mapped);
                } else {
                    warn!(
                        layer = ?layer.wl_surface(),
                        namespace = layer.namespace(),
                        "skipping layer close animation: missing geometry on unmap"
                    );
                }

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

    fn start_close_animation_for_layer(
        &mut self,
        output: &Output,
        layer: &LayerSurface,
        geo: Rectangle<i32, Logical>,
        mut mapped: MappedLayer,
    ) {
        let scale = Scale::from(output.current_scale().fractional_scale());
        let kind = resolve_layer_animation_kind(layer);
        let config = self.niri.config.borrow();

        let (anim_config, program) = match kind {
            LayerAnimationKind::Bar => (
                config
                    .animations
                    .layer_bar_close
                    .as_ref()
                    .unwrap_or(&config.animations.layer_close)
                    .anim,
                ProgramType::LayerBarClose,
            ),
            LayerAnimationKind::Wallpaper => (
                config
                    .animations
                    .layer_wallpaper_close
                    .as_ref()
                    .unwrap_or(&config.animations.layer_close)
                    .anim,
                ProgramType::LayerWallpaperClose,
            ),
            LayerAnimationKind::Launcher => (
                config
                    .animations
                    .layer_launcher_close
                    .as_ref()
                    .unwrap_or(&config.animations.layer_close)
                    .anim,
                ProgramType::LayerLauncherClose,
            ),
        };

        self.backend.with_primary_renderer(|renderer| {
            let snapshot = mapped.take_unmap_snapshot().or_else(|| {
                mapped.store_unmap_snapshot(renderer);
                mapped.take_unmap_snapshot()
            });

            let Some(snapshot) = snapshot else {
                warn!("error starting layer close animation: missing layer snapshot");
                return;
            };

            if snapshot.contents.is_empty() && snapshot.blocked_out_contents.is_empty() {
                warn!("error starting layer close animation: layer snapshot is empty");
                return;
            }

            let anim = Animation::new(self.niri.clock.clone(), 0., 1., 0., anim_config);

            let res = ClosingLayer::new(
                renderer,
                snapshot,
                scale,
                geo.size.to_f64(),
                geo.loc.to_f64(),
                anim,
                program,
            );

            let for_backdrop = mapped.place_within_backdrop();

            match res {
                Ok(animation) => {
                    self.niri.closing_layers.push(ClosingLayerState {
                        output: output.clone(),
                        surface: layer.clone(),
                        layer: layer.layer(),
                        for_backdrop,
                        animation,
                    });
                }
                Err(err) => warn!("error starting layer close animation: {err:?}"),
            }
        });
    }
}

/// Classifies a layer surface into one of three animation kinds based on its geometry hints.
///
/// The heuristic, in priority order:
/// - **Bar**: has an exclusive zone (reserves screen space) — e.g. panels, docks.
/// - **Wallpaper**: anchored to all four edges with no exclusive zone — e.g. desktop backgrounds.
/// - **Launcher**: everything else — e.g. app launchers, notification overlays.
fn resolve_layer_animation_kind(layer: &LayerSurface) -> LayerAnimationKind {
    let state = layer.cached_state();
    let anchor = state.anchor;
    let has_exclusive_zone = matches!(state.exclusive_zone, ExclusiveZone::Exclusive(_));
    let has_all_edges = anchor
        .contains(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT)
        && anchor.bits().count_ones() == 4;
    let is_edge = anchor.intersects(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT)
        && anchor.bits().count_ones() <= 3;

    if has_exclusive_zone && is_edge {
        LayerAnimationKind::Bar
    } else if has_all_edges && !has_exclusive_zone {
        LayerAnimationKind::Wallpaper
    } else {
        LayerAnimationKind::Launcher
    }
}

fn add_mapped_layer_pre_commit_hook(layer: &LayerSurface) -> HookId {
    add_pre_commit_hook::<State, _>(layer.wl_surface(), move |state, _dh, surface| {
        let (layer_changed, got_unmapped, got_new_buffer) = with_states(surface, |states| {
            let layer_changed = {
                let mut guard = states.cached_state.get::<LayerSurfaceCachedState>();
                let pending_layer = guard.pending().layer;
                let current_layer = guard.current().layer;
                pending_layer != current_layer
            };

            let (got_unmapped, got_new_buffer) = {
                let mut guard = states.cached_state.get::<SurfaceAttributes>();
                match guard.pending().buffer.as_ref() {
                    // Null commit: surface is about to lose its buffer entirely.
                    // Snapshot now while current state is still live.
                    Some(BufferAssignment::Removed) => (true, false),

                    // New buffer incoming: capture while we still have a valid current frame.
                    Some(BufferAssignment::NewBuffer(_)) => (false, true),

                    None => (false, false),
                }
            };

            (layer_changed, got_unmapped, got_new_buffer)
        });

        let current_has_buffer =
            with_renderer_surface_state(surface, |s| s.buffer().is_some()).unwrap_or(false);

        let should_snapshot = got_unmapped || (got_new_buffer && current_has_buffer);

        if layer_changed {
            for mapped in state.niri.mapped_layer_surfaces.values_mut() {
                if mapped.surface().wl_surface() == surface {
                    mapped.set_recompute_rules_on_commit();
                    break;
                }
            }
        }

        if should_snapshot {
            for mapped in state.niri.mapped_layer_surfaces.values_mut() {
                if mapped.surface().wl_surface() == surface {
                    if mapped.has_non_empty_unmap_snapshot() {
                        break;
                    }

                    state.backend.with_primary_renderer(|renderer| {
                        mapped.store_unmap_snapshot(renderer);
                    });
                    break;
                }
            }
        }
    })
}
