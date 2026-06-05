use crate::appearance::{BackgroundEffectRule, BlockOutFrom, CornerRadius, ShadowRule};
use crate::utils::RegexEq;
use crate::window_rule::PopupsRule;

#[derive(knus::Decode, Debug, Default, Clone, PartialEq)]
pub struct LayerRule {
    #[knus(children(name = "match"))]
    pub matches: Vec<Match>,
    #[knus(children(name = "exclude"))]
    pub excludes: Vec<Match>,

    #[knus(child, unwrap(argument))]
    pub opacity: Option<f32>,
    #[knus(child, unwrap(argument))]
    pub block_out_from: Option<BlockOutFrom>,
    #[knus(child, default)]
    pub shadow: ShadowRule,
    #[knus(child)]
    pub geometry_corner_radius: Option<CornerRadius>,
    #[knus(child, unwrap(argument))]
    pub place_within_backdrop: Option<bool>,
    #[knus(child, unwrap(argument))]
    pub baba_is_float: Option<bool>,
    #[knus(child, default)]
    pub background_effect: BackgroundEffectRule,
    #[knus(child, default)]
    pub popups: PopupsRule,
}

#[derive(knus::Decode, Debug, Default, Clone, PartialEq)]
pub struct Match {
    #[knus(property, str)]
    pub namespace: Option<RegexEq>,
    #[knus(property)]
    pub at_startup: Option<bool>,
    #[knus(property, str)]
    pub layer: Option<niri_ipc::Layer>,
}
