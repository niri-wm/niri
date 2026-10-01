use niri_config::layer_rule::{LayerRule, Match};
use niri_config::utils::MergeWith as _;
use niri_config::{BackgroundEffect, BlockOutFrom, CornerRadius, ResolvedPopupsRules, ShadowRule};
use smithay::desktop::LayerSurface;
use smithay::wayland::shell::wlr_layer::{ExclusiveZone, Layer};

pub mod mapped;
pub use mapped::MappedLayer;

/// Rules fully resolved for a layer-shell surface.
#[derive(Debug, Default, PartialEq)]
pub struct ResolvedLayerRules {
    /// Extra opacity to draw this layer surface with.
    pub opacity: Option<f32>,

    /// Whether to block out this layer surface from certain render targets.
    pub block_out_from: Option<BlockOutFrom>,

    /// Shadow overrides.
    pub shadow: ShadowRule,

    /// Corner radius to assume this layer surface has.
    pub geometry_corner_radius: Option<CornerRadius>,

    /// Whether to place this layer surface within the overview backdrop.
    pub place_within_backdrop: bool,

    /// Whether to bob this window up and down.
    pub baba_is_float: bool,

    /// Background effect configuration.
    pub background_effect: BackgroundEffect,

    /// Rules for this layer surface's popups.
    pub popups: ResolvedPopupsRules,
}

impl ResolvedLayerRules {
    pub fn compute(rules: &[LayerRule], surface: &LayerSurface, is_at_startup: bool) -> Self {
        let _span = tracy_client::span!("ResolvedLayerRules::compute");

        let mut resolved = ResolvedLayerRules::default();

        for rule in rules {
            let matches = |m: &Match| {
                if let Some(at_startup) = m.at_startup {
                    if at_startup != is_at_startup {
                        return false;
                    }
                }

                surface_matches(surface, m)
            };

            if !(rule.matches.is_empty() || rule.matches.iter().any(matches)) {
                continue;
            }

            if rule.excludes.iter().any(matches) {
                continue;
            }

            if let Some(x) = rule.opacity {
                resolved.opacity = Some(x);
            }
            if let Some(x) = rule.block_out_from {
                resolved.block_out_from = Some(x);
            }
            if let Some(x) = rule.geometry_corner_radius {
                resolved.geometry_corner_radius = Some(x);
            }
            if let Some(x) = rule.place_within_backdrop {
                resolved.place_within_backdrop = x;
            }
            if let Some(x) = rule.baba_is_float {
                resolved.baba_is_float = x;
            }

            resolved.shadow.merge_with(&rule.shadow);

            resolved
                .background_effect
                .merge_with(&rule.background_effect);

            resolved.popups.merge_with(&rule.popups);
        }

        resolved
    }
}

fn surface_matches(surface: &LayerSurface, m: &Match) -> bool {
    if let Some(namespace_re) = &m.namespace {
        if !namespace_re.0.is_match(surface.namespace()) {
            return false;
        }
    }

    if let Some(layer) = m.layer {
        let surface_layer = match surface.layer() {
            Layer::Background => niri_ipc::Layer::Background,
            Layer::Bottom => niri_ipc::Layer::Bottom,
            Layer::Top => niri_ipc::Layer::Top,
            Layer::Overlay => niri_ipc::Layer::Overlay,
        };
        if layer != surface_layer {
            return false;
        }
    }

    if let Some(anchors) = m.anchors {
        let surface_anchors = surface.cached_state().anchor;
        let same = anchors.contains(niri_config::layer_rule::Anchors::TOP)
            == surface_anchors.contains(smithay::wayland::shell::wlr_layer::Anchor::TOP)
            && anchors.contains(niri_config::layer_rule::Anchors::BOTTOM)
                == surface_anchors.contains(smithay::wayland::shell::wlr_layer::Anchor::BOTTOM)
            && anchors.contains(niri_config::layer_rule::Anchors::LEFT)
                == surface_anchors.contains(smithay::wayland::shell::wlr_layer::Anchor::LEFT)
            && anchors.contains(niri_config::layer_rule::Anchors::RIGHT)
                == surface_anchors.contains(smithay::wayland::shell::wlr_layer::Anchor::RIGHT);
        if !same {
            return false;
        }
    }

    if let Some(anchor_sides) = m.anchor_sides {
        let surface_anchor_sides = surface.cached_state().anchor.bits().count_ones() as u8;
        if anchor_sides != surface_anchor_sides {
            return false;
        }
    }

    if let Some(exclusive_zone) = m.exclusive_zone {
        let surface_exclusive = matches!(
            surface.cached_state().exclusive_zone,
            ExclusiveZone::Exclusive(_)
        );
        let matches = match exclusive_zone {
            niri_config::layer_rule::ExclusiveZone::Exclusive => surface_exclusive,
            niri_config::layer_rule::ExclusiveZone::Neutral => !surface_exclusive,
        };
        if !matches {
            return false;
        }
    }

    if let Some(keyboard_interactivity) = m.keyboard_interactivity {
        let matches = matches!(
            (
                keyboard_interactivity,
                surface.cached_state().keyboard_interactivity
            ),
            (
                niri_config::layer_rule::LayerKeyboardInteractivity::None,
                smithay::wayland::shell::wlr_layer::KeyboardInteractivity::None
            ) | (
                niri_config::layer_rule::LayerKeyboardInteractivity::Exclusive,
                smithay::wayland::shell::wlr_layer::KeyboardInteractivity::Exclusive
            ) | (
                niri_config::layer_rule::LayerKeyboardInteractivity::OnDemand,
                smithay::wayland::shell::wlr_layer::KeyboardInteractivity::OnDemand
            )
        );
        if !matches {
            return false;
        }
    }

    true
}
