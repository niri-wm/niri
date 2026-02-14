use std::str::FromStr;

use crate::animations::AnimationsPart;
use crate::{LayoutPart, OverviewPart};

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct AppearanceRule {
    #[knuffel(children(name = "match"))]
    pub matches: Vec<Match>,
    #[knuffel(children(name = "exclude"))]
    pub excludes: Vec<Match>,

    #[knuffel(child)]
    pub layout: Option<LayoutPart>,
    #[knuffel(child)]
    pub overview: Option<OverviewPart>,
    #[knuffel(child)]
    pub animations: Option<AnimationsPart>,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    #[knuffel(property(name = "color-scheme"), str)]
    pub color_scheme: Option<ColorScheme>,
    #[knuffel(property, str)]
    pub contrast: Option<Contrast>,
    #[knuffel(property(name = "reduced-motion"))]
    pub reduced_motion: Option<bool>,
}

impl AppearanceRule {
    pub fn matches(&self, appearance: AppearanceSettings) -> bool {
        let include_matches =
            self.matches.is_empty() || self.matches.iter().any(|m| m.matches(appearance));
        if !include_matches {
            return false;
        }

        if self.excludes.iter().any(|m| m.matches(appearance)) {
            return false;
        }

        true
    }
}

impl Match {
    pub fn matches(&self, appearance: AppearanceSettings) -> bool {
        if let Some(expected) = self.color_scheme {
            if appearance.color_scheme != Some(expected) {
                return false;
            }
        }

        if let Some(expected) = self.contrast {
            if appearance.contrast != Some(expected) {
                return false;
            }
        }

        if let Some(expected) = self.reduced_motion {
            if appearance.reduced_motion != expected {
                return false;
            }
        }

        true
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AppearanceSettings {
    pub color_scheme: Option<ColorScheme>,
    pub contrast: Option<Contrast>,
    pub reduced_motion: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    Light,
    Dark,
}

impl FromStr for ColorScheme {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            _ => Err(format!(
                "invalid color scheme {s:?}, expected \"light\" or \"dark\""
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contrast {
    High,
}

impl FromStr for Contrast {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "high" => Ok(Self::High),
            _ => Err(format!("invalid contrast {s:?}, expected \"high\"")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn parses_appearance_rule_block() {
        let config = Config::parse_mem(
            r#"
            appearance-rule {
                match color-scheme="dark"
                layout {
                    gaps 8
                }
            }
            "#,
        )
        .unwrap();

        assert_eq!(config.appearance_rules.len(), 1);
        let rule = &config.appearance_rules[0];
        assert_eq!(rule.matches.len(), 1);
        assert!(rule.layout.is_some());
    }

    #[test]
    fn match_semantics() {
        let appearance = AppearanceSettings {
            color_scheme: Some(ColorScheme::Dark),
            contrast: None,
            reduced_motion: false,
        };

        let m = Match {
            color_scheme: Some(ColorScheme::Dark),
            contrast: None,
            reduced_motion: Some(false),
        };
        assert!(m.matches(appearance));

        let m = Match {
            color_scheme: Some(ColorScheme::Light),
            contrast: None,
            reduced_motion: None,
        };
        assert!(!m.matches(appearance));

        let rule = AppearanceRule {
            matches: vec![
                Match {
                    color_scheme: Some(ColorScheme::Light),
                    contrast: None,
                    reduced_motion: None,
                },
                Match {
                    color_scheme: Some(ColorScheme::Dark),
                    contrast: None,
                    reduced_motion: None,
                },
            ],
            excludes: vec![Match {
                color_scheme: None,
                contrast: None,
                reduced_motion: Some(true),
            }],
            layout: None,
            overview: None,
            animations: None,
        };
        assert!(rule.matches(appearance));
    }
}
