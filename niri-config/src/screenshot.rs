use std::str::FromStr;

use knuffel::errors::DecodeError;

use crate::appearance::Color;
use crate::misc::ScreenshotPath;
use crate::utils::MergeWith;

/// Background for screenshot captures.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ScreenshotBackground {
    #[default]
    Transparent,
    Desktop,
    Color(Color),
}

impl FromStr for ScreenshotBackground {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "transparent" => Ok(Self::Transparent),
            "desktop" | "wallpaper" => Ok(Self::Desktop),
            _ => {
                let color = Color::from_str(s)?;
                Ok(Self::Color(color))
            }
        }
    }
}

impl<S: knuffel::traits::ErrorSpan> knuffel::DecodeScalar<S> for ScreenshotBackground {
    fn type_check(
        type_name: &Option<knuffel::span::Spanned<knuffel::ast::TypeName, S>>,
        ctx: &mut knuffel::decode::Context<S>,
    ) {
        if let Some(type_name) = &type_name {
            ctx.emit_error(DecodeError::unexpected(
                type_name,
                "type name",
                "no type name expected for this node",
            ));
        }
    }

    fn raw_decode(
        val: &knuffel::span::Spanned<knuffel::ast::Literal, S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        let s = String::raw_decode(val, ctx)?;
        s.parse::<Self>()
            .map_err(|e| DecodeError::conversion(val, e))
    }
}

impl<S: knuffel::traits::ErrorSpan> knuffel::Decode<S> for ScreenshotBackground {
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        let mut iter = node.arguments.iter();
        let val = iter
            .next()
            .ok_or_else(|| DecodeError::missing(node, "argument is required"))?;
        let res = knuffel::traits::DecodeScalar::decode(val, ctx)?;
        if let Some(extra) = iter.next() {
            ctx.emit_error(DecodeError::unexpected(
                &extra.literal,
                "argument",
                "unexpected extra argument",
            ));
        }
        Ok(res)
    }
}

/// Padding around screenshot captures.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScreenshotPadding {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl ScreenshotPadding {
    pub const ZERO: Self = Self {
        top: 0.,
        right: 0.,
        bottom: 0.,
        left: 0.,
    };

    pub const fn uniform(all: f64) -> Self {
        let v = if all < 0. { 0. } else { all };
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    pub const fn symmetric(vertical: f64, horizontal: f64) -> Self {
        let v = if vertical < 0. { 0. } else { vertical };
        let h = if horizontal < 0. { 0. } else { horizontal };
        Self {
            top: v,
            right: h,
            bottom: v,
            left: h,
        }
    }

    pub const fn new(top: f64, right: f64, bottom: f64, left: f64) -> Self {
        Self {
            top: if top < 0. { 0. } else { top },
            right: if right < 0. { 0. } else { right },
            bottom: if bottom < 0. { 0. } else { bottom },
            left: if left < 0. { 0. } else { left },
        }
    }
}

impl FromStr for ScreenshotPadding {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(Self::ZERO);
        }

        if s.contains('=') {
            let mut padding = Self::ZERO;
            for token in s.split_whitespace() {
                let Some((k, v)) = token.split_once('=') else {
                    return Err(miette::miette!(
                        "invalid padding token '{token}', expected key=value"
                    ));
                };
                let val: f64 = v
                    .parse()
                    .map_err(|e| miette::miette!("invalid padding value '{v}': {e}"))?;
                let val = val.max(0.);
                match k {
                    "top" => padding.top = val,
                    "right" => padding.right = val,
                    "bottom" => padding.bottom = val,
                    "left" => padding.left = val,
                    _ => return Err(miette::miette!("unknown padding property '{k}'")),
                }
            }
            return Ok(padding);
        }

        let parts: Vec<&str> = s
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|p| !p.is_empty())
            .collect();

        match parts.len() {
            1 => {
                let v: f64 = parts[0]
                    .parse()
                    .map_err(|e| miette::miette!("invalid padding: {e}"))?;
                Ok(Self::uniform(v))
            }
            2 => {
                let v: f64 = parts[0]
                    .parse()
                    .map_err(|e| miette::miette!("invalid padding: {e}"))?;
                let h: f64 = parts[1]
                    .parse()
                    .map_err(|e| miette::miette!("invalid padding: {e}"))?;
                Ok(Self::symmetric(v, h))
            }
            4 => {
                let top: f64 = parts[0]
                    .parse()
                    .map_err(|e| miette::miette!("invalid padding: {e}"))?;
                let right: f64 = parts[1]
                    .parse()
                    .map_err(|e| miette::miette!("invalid padding: {e}"))?;
                let bottom: f64 = parts[2]
                    .parse()
                    .map_err(|e| miette::miette!("invalid padding: {e}"))?;
                let left: f64 = parts[3]
                    .parse()
                    .map_err(|e| miette::miette!("invalid padding: {e}"))?;
                Ok(Self::new(top, right, bottom, left))
            }
            _ => Err(miette::miette!(
                "expected 1, 2, or 4 padding values, got {}",
                parts.len()
            )),
        }
    }
}

impl<S> knuffel::Decode<S> for ScreenshotPadding
where
    S: knuffel::traits::ErrorSpan,
{
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        // Check for unexpected type name.
        if let Some(type_name) = &node.type_name {
            ctx.emit_error(DecodeError::unexpected(
                type_name,
                "type name",
                "no type name expected for this node",
            ));
        }

        // Check for unexpected children.
        for child in node.children.as_ref().map(|lst| &lst[..]).unwrap_or(&[]) {
            ctx.emit_error(DecodeError::unexpected(
                child,
                "node",
                format!("unexpected node `{}`", child.node_name.escape_default()),
            ));
        }

        let decode_f64_val =
            |val: &knuffel::ast::Value<S>, ctx: &mut knuffel::decode::Context<S>| -> f64 {
                if let Some(typ) = &val.type_name {
                    ctx.emit_error(DecodeError::TypeName {
                        span: typ.span().clone(),
                        found: Some((**typ).clone()),
                        expected: knuffel::errors::ExpectedType::no_type(),
                        rust_type: "f64",
                    });
                }

                match *val.literal {
                    knuffel::ast::Literal::Int(ref x) => match i64::try_from(x) {
                        Ok(x) => x as f64,
                        Err(err) => {
                            ctx.emit_error(DecodeError::conversion(&val.literal, err));
                            0.
                        }
                    },
                    knuffel::ast::Literal::Decimal(ref x) => match x.try_into() {
                        Ok(x) => x,
                        Err(err) => {
                            ctx.emit_error(DecodeError::conversion(&val.literal, err));
                            0.
                        }
                    },
                    _ => {
                        ctx.emit_error(DecodeError::scalar_kind(
                            knuffel::decode::Kind::Int,
                            &val.literal,
                        ));
                        0.
                    }
                }
            };

        let mut padding = Self::ZERO;
        let mut iter_args = node.arguments.iter();

        if let Some(first_arg) = iter_args.next() {
            let v1 = decode_f64_val(first_arg, ctx);
            if let Some(second_arg) = iter_args.next() {
                let v2 = decode_f64_val(second_arg, ctx);
                if let Some(third_arg) = iter_args.next() {
                    let v3 = decode_f64_val(third_arg, ctx);
                    let fourth_arg = iter_args.next().ok_or_else(|| {
                        DecodeError::missing(node, "either 1, 2, or 4 arguments are required")
                    })?;
                    let v4 = decode_f64_val(fourth_arg, ctx);

                    if let Some(val) = iter_args.next() {
                        ctx.emit_error(DecodeError::unexpected(
                            &val.literal,
                            "argument",
                            "unexpected extra argument",
                        ));
                    }

                    padding = Self::new(v1, v2, v3, v4);
                } else {
                    padding = Self::symmetric(v1, v2);
                }
            } else {
                padding = Self::uniform(v1);
            }
        } else if node.properties.is_empty() {
            return Err(DecodeError::missing(
                node,
                "padding requires either 1, 2, or 4 arguments, or properties",
            ));
        }

        for (name, val) in &node.properties {
            match &***name {
                "top" => padding.top = decode_f64_val(val, ctx).max(0.),
                "right" => padding.right = decode_f64_val(val, ctx).max(0.),
                "bottom" => padding.bottom = decode_f64_val(val, ctx).max(0.),
                "left" => padding.left = decode_f64_val(val, ctx).max(0.),
                name_str => ctx.emit_error(DecodeError::unexpected(
                    name,
                    "property",
                    format!("unexpected property `{}`", name_str.escape_default()),
                )),
            }
        }

        Ok(padding)
    }
}

impl MergeWith<Self> for ScreenshotPadding {
    fn merge_with(&mut self, part: &Self) {
        *self = *part;
    }
}

/// Screenshot configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Screenshot {
    pub path: ScreenshotPath,
    pub window_background: ScreenshotBackground,
    pub include_shadows: bool,
    pub padding: ScreenshotPadding,
    pub padding_background: ScreenshotBackground,
}

impl Default for Screenshot {
    fn default() -> Self {
        Self {
            path: ScreenshotPath::default(),
            window_background: ScreenshotBackground::Transparent,
            include_shadows: false,
            padding: ScreenshotPadding::ZERO,
            padding_background: ScreenshotBackground::Transparent,
        }
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub(crate) struct ScreenshotPart {
    #[knuffel(child)]
    pub(crate) path: Option<ScreenshotPath>,
    #[knuffel(child, unwrap(argument))]
    pub(crate) window_background: Option<ScreenshotBackground>,
    #[knuffel(child, unwrap(argument))]
    pub(crate) include_shadows: Option<bool>,
    #[knuffel(child)]
    pub(crate) padding: Option<ScreenshotPadding>,
    #[knuffel(child, unwrap(argument))]
    pub(crate) padding_background: Option<ScreenshotBackground>,
}

impl MergeWith<ScreenshotPart> for Screenshot {
    fn merge_with(&mut self, part: &ScreenshotPart) {
        if let Some(path) = &part.path {
            self.path = path.clone();
        }
        if let Some(wb) = part.window_background {
            self.window_background = wb;
        }
        if let Some(is) = part.include_shadows {
            self.include_shadows = is;
        }
        if let Some(pad) = part.padding {
            self.padding = pad;
        }
        if let Some(pb) = part.padding_background {
            self.padding_background = pb;
        }
    }
}

impl MergeWith<crate::window_rule::WindowScreenshotRule> for Screenshot {
    fn merge_with(&mut self, part: &crate::window_rule::WindowScreenshotRule) {
        if let Some(wb) = part.window_background {
            self.window_background = wb;
        }
        if let Some(is) = part.include_shadows {
            self.include_shadows = is;
        }
        if let Some(pad) = part.padding {
            self.padding = pad;
        }
        if let Some(pb) = part.padding_background {
            self.padding_background = pb;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn test_screenshot_background_from_str() {
        assert_eq!(
            "transparent".parse::<ScreenshotBackground>().unwrap(),
            ScreenshotBackground::Transparent
        );
        assert_eq!(
            "desktop".parse::<ScreenshotBackground>().unwrap(),
            ScreenshotBackground::Desktop
        );
        assert_eq!(
            "wallpaper".parse::<ScreenshotBackground>().unwrap(),
            ScreenshotBackground::Desktop
        );
        assert_eq!(
            "#123456".parse::<ScreenshotBackground>().unwrap(),
            ScreenshotBackground::Color(Color::from_rgba8_unpremul(0x12, 0x34, 0x56, 0xff))
        );
        assert!("invalid_color_xyz".parse::<ScreenshotBackground>().is_err());
    }

    #[test]
    fn test_screenshot_padding_from_str() {
        assert_eq!(
            "32".parse::<ScreenshotPadding>().unwrap(),
            ScreenshotPadding::uniform(32.0)
        );
        assert_eq!(
            "24 32".parse::<ScreenshotPadding>().unwrap(),
            ScreenshotPadding::symmetric(24.0, 32.0)
        );
        assert_eq!(
            "10 20 30 40".parse::<ScreenshotPadding>().unwrap(),
            ScreenshotPadding::new(10.0, 20.0, 30.0, 40.0)
        );
        assert_eq!(
            "top=16 right=32 bottom=24 left=32"
                .parse::<ScreenshotPadding>()
                .unwrap(),
            ScreenshotPadding::new(16.0, 32.0, 24.0, 32.0)
        );
        assert_eq!(
            "top=-10 left=-5 right=20 bottom=0"
                .parse::<ScreenshotPadding>()
                .unwrap(),
            ScreenshotPadding::new(0.0, 20.0, 0.0, 0.0)
        );
        assert!("top=10 invalid_param".parse::<ScreenshotPadding>().is_err());
        assert!("10 20 30".parse::<ScreenshotPadding>().is_err());
    }

    #[test]
    fn test_parse_screenshot_config_all_options() {
        let config = Config::parse_mem(
            r##"
            screenshot {
                path "~/Pictures/Custom/shot.png"
                window-background "desktop"
                include-shadows true
                padding 32
                padding-background "#123456"
            }
            "##,
        )
        .unwrap();

        assert_eq!(
            config.screenshot.path.0.as_deref(),
            Some("~/Pictures/Custom/shot.png")
        );
        assert_eq!(
            config.screenshot_path.0.as_deref(),
            Some("~/Pictures/Custom/shot.png")
        );
        assert_eq!(
            config.screenshot.window_background,
            ScreenshotBackground::Desktop
        );
        assert!(config.screenshot.include_shadows);
        assert_eq!(config.screenshot.padding, ScreenshotPadding::uniform(32.0));
        assert_eq!(
            config.screenshot.padding_background,
            ScreenshotBackground::Color(Color::from_rgba8_unpremul(0x12, 0x34, 0x56, 0xff))
        );
    }

    #[test]
    fn test_parse_screenshot_padding_variations() {
        let uniform = Config::parse_mem(
            r##"
            screenshot {
                padding 40
            }
            "##,
        )
        .unwrap();
        assert_eq!(uniform.screenshot.padding, ScreenshotPadding::uniform(40.0));

        let symmetric = Config::parse_mem(
            r##"
            screenshot {
                padding 20 30
            }
            "##,
        )
        .unwrap();
        assert_eq!(
            symmetric.screenshot.padding,
            ScreenshotPadding::symmetric(20.0, 30.0)
        );

        let four_args = Config::parse_mem(
            r##"
            screenshot {
                padding 10 20 30 40
            }
            "##,
        )
        .unwrap();
        assert_eq!(
            four_args.screenshot.padding,
            ScreenshotPadding::new(10.0, 20.0, 30.0, 40.0)
        );

        let properties = Config::parse_mem(
            r##"
            screenshot {
                padding top=15 right=25 bottom=35 left=45
            }
            "##,
        )
        .unwrap();
        assert_eq!(
            properties.screenshot.padding,
            ScreenshotPadding::new(15.0, 25.0, 35.0, 45.0)
        );

        let override_props = Config::parse_mem(
            r##"
            screenshot {
                padding 50 top=12 left=14
            }
            "##,
        )
        .unwrap();
        assert_eq!(
            override_props.screenshot.padding,
            ScreenshotPadding::new(12.0, 50.0, 50.0, 14.0)
        );
    }

    #[test]
    fn test_screenshot_path_backward_compat() {
        let config = Config::parse_mem(
            r##"
            screenshot-path "~/OldPath/shot.png"
            "##,
        )
        .unwrap();

        assert_eq!(
            config.screenshot_path.0.as_deref(),
            Some("~/OldPath/shot.png")
        );
        assert_eq!(
            config.screenshot.path.0.as_deref(),
            Some("~/OldPath/shot.png")
        );

        let config_null = Config::parse_mem(
            r##"
            screenshot-path null
            "##,
        )
        .unwrap();

        assert_eq!(config_null.screenshot_path.0, None);
        assert_eq!(config_null.screenshot.path.0, None);
    }

    #[test]
    fn test_window_rule_screenshot() {
        let config = Config::parse_mem(
            r##"
            window-rule {
                match app-id="test-app"
                screenshot {
                    window-background "wallpaper"
                    include-shadows false
                    padding 16 24
                    padding-background "transparent"
                }
            }
            "##,
        )
        .unwrap();

        assert_eq!(config.window_rules.len(), 1);
        let rule = &config.window_rules[0];
        assert_eq!(
            rule.screenshot.window_background,
            Some(ScreenshotBackground::Desktop)
        );
        assert_eq!(rule.screenshot.include_shadows, Some(false));
        assert_eq!(
            rule.screenshot.padding,
            Some(ScreenshotPadding::symmetric(16.0, 24.0))
        );
        assert_eq!(
            rule.screenshot.padding_background,
            Some(ScreenshotBackground::Transparent)
        );
    }

    #[test]
    fn test_merge_window_screenshot_rule_into_screenshot() {
        let mut screenshot = Screenshot::default();
        let rule = crate::window_rule::WindowScreenshotRule {
            window_background: Some(ScreenshotBackground::Desktop),
            include_shadows: Some(true),
            padding: Some(ScreenshotPadding::uniform(24.0)),
            padding_background: Some(ScreenshotBackground::Color(Color::from_rgba8_unpremul(
                1, 2, 3, 255,
            ))),
        };

        screenshot.merge_with(&rule);

        assert_eq!(screenshot.window_background, ScreenshotBackground::Desktop);
        assert!(screenshot.include_shadows);
        assert_eq!(screenshot.padding, ScreenshotPadding::uniform(24.0));
        assert_eq!(
            screenshot.padding_background,
            ScreenshotBackground::Color(Color::from_rgba8_unpremul(1, 2, 3, 255))
        );
    }
}
