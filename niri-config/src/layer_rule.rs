use std::str::FromStr;

use bitflags::bitflags;
use miette::miette;

use crate::animations::{LayerCloseAnim, LayerOpenAnim};
use crate::appearance::{BackgroundEffectRule, BlockOutFrom, CornerRadius, ShadowRule};
use crate::utils::RegexEq;
use crate::window_rule::PopupsRule;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Anchors: u8 {
        const TOP = 1;
        const BOTTOM = 1 << 1;
        const LEFT = 1 << 2;
        const RIGHT = 1 << 3;
    }
}

impl FromStr for Anchors {
    type Err = miette::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut anchors = Anchors::empty();
        for raw in s.split(',') {
            let part = raw.trim();
            if part.is_empty() {
                return Err(miette!("anchor cannot be empty"));
            }

            let next = match part {
                "top" => Anchors::TOP,
                "bottom" => Anchors::BOTTOM,
                "left" => Anchors::LEFT,
                "right" => Anchors::RIGHT,
                _ => return Err(miette!("unknown anchor: {part:?}")),
            };

            if anchors.contains(next) {
                return Err(miette!("duplicate anchor: {part:?}"));
            }

            anchors |= next;
        }

        Ok(anchors)
    }
}

#[derive(knuffel::DecodeScalar, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusiveZone {
    Exclusive,
    Neutral,
}

#[derive(knuffel::DecodeScalar, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKeyboardInteractivity {
    None,
    Exclusive,
    OnDemand,
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct LayerRule {
    #[knuffel(children(name = "match"))]
    pub matches: Vec<Match>,
    #[knuffel(children(name = "exclude"))]
    pub excludes: Vec<Match>,

    #[knuffel(child, unwrap(argument))]
    pub opacity: Option<f32>,
    #[knuffel(child, unwrap(argument))]
    pub block_out_from: Option<BlockOutFrom>,
    #[knuffel(child, default)]
    pub shadow: ShadowRule,
    #[knuffel(child)]
    pub geometry_corner_radius: Option<CornerRadius>,
    #[knuffel(child, unwrap(argument))]
    pub place_within_backdrop: Option<bool>,
    #[knuffel(child, unwrap(argument))]
    pub baba_is_float: Option<bool>,
    #[knuffel(child, default)]
    pub background_effect: BackgroundEffectRule,
    #[knuffel(child, default)]
    pub popups: PopupsRule,
    #[knuffel(child)]
    pub animations: Option<LayerAnimationsRule>,
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct LayerAnimationsRule {
    #[knuffel(child)]
    pub layer_open: Option<LayerOpenAnim>,
    #[knuffel(child)]
    pub layer_close: Option<LayerCloseAnim>,
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct Match {
    #[knuffel(property, str)]
    pub namespace: Option<RegexEq>,
    #[knuffel(property)]
    pub at_startup: Option<bool>,
    #[knuffel(property, str)]
    pub layer: Option<niri_ipc::Layer>,
    #[knuffel(property, str)]
    pub anchors: Option<Anchors>,
    #[knuffel(property(name = "anchor-sides"))]
    pub anchor_sides: Option<u8>,
    #[knuffel(property(name = "exclusive-zone"))]
    pub exclusive_zone: Option<ExclusiveZone>,
    #[knuffel(property(name = "keyboard-interactivity"))]
    pub keyboard_interactivity: Option<LayerKeyboardInteractivity>,
}
