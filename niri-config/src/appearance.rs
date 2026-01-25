use std::ops::{Mul, MulAssign};
use std::str::FromStr;

use knuffel::errors::DecodeError;
use miette::{miette, IntoDiagnostic as _};
use smithay::backend::renderer::Color32F;

use crate::utils::{Flag, MergeWith};
use crate::FloatOrInt;

pub const DEFAULT_BACKGROUND_COLOR: Color = Color::from_array_unpremul([0.25, 0.25, 0.25, 1.]);
pub const DEFAULT_BACKDROP_COLOR: Color = Color::from_array_unpremul([0.15, 0.15, 0.15, 1.]);

/// RGB color in [0, 1] with unpremultiplied alpha.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn new_unpremul(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_rgba8_unpremul(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::from_array_unpremul([r, g, b, a].map(|x| x as f32 / 255.))
    }

    pub fn from_array_premul([r, g, b, a]: [f32; 4]) -> Self {
        let a = a.clamp(0., 1.);

        if a == 0. {
            Self::new_unpremul(0., 0., 0., 0.)
        } else {
            Self {
                r: (r / a).clamp(0., 1.),
                g: (g / a).clamp(0., 1.),
                b: (b / a).clamp(0., 1.),
                a,
            }
        }
    }

    pub const fn from_array_unpremul([r, g, b, a]: [f32; 4]) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_color32f(color: Color32F) -> Self {
        Self::from_array_premul(color.components())
    }

    pub fn to_array_unpremul(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub fn to_array_premul(self) -> [f32; 4] {
        let [r, g, b, a] = [self.r, self.g, self.b, self.a];
        [r * a, g * a, b * a, a]
    }
}

impl Mul<f32> for Color {
    type Output = Self;

    fn mul(mut self, rhs: f32) -> Self::Output {
        self.a *= rhs;
        self
    }
}

impl MulAssign<f32> for Color {
    fn mul_assign(&mut self, rhs: f32) {
        self.a *= rhs;
    }
}

impl From<Color> for Color32F {
    fn from(value: Color) -> Self {
        Color32F::from(value.to_array_premul())
    }
}

#[derive(knuffel::Decode, Debug, Clone, Copy, PartialEq)]
pub struct Gradient {
    #[knuffel(property, str)]
    pub from: Color,
    #[knuffel(property, str)]
    pub to: Color,
    #[knuffel(property, default = 180)]
    pub angle: i16,
    #[knuffel(property, default)]
    pub relative_to: GradientRelativeTo,
    #[knuffel(property(name = "in"), str, default)]
    pub in_: GradientInterpolation,
}

impl From<Color> for Gradient {
    fn from(value: Color) -> Self {
        Self {
            from: value,
            to: value,
            angle: 0,
            relative_to: GradientRelativeTo::Window,
            in_: GradientInterpolation::default(),
        }
    }
}

#[derive(knuffel::DecodeScalar, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum GradientRelativeTo {
    #[default]
    Window,
    WorkspaceView,
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct GradientInterpolation {
    pub color_space: GradientColorSpace,
    pub hue_interpolation: HueInterpolation,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum GradientColorSpace {
    #[default]
    Srgb,
    SrgbLinear,
    Oklab,
    Oklch,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum HueInterpolation {
    #[default]
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct CornerRadius {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl From<CornerRadius> for [f32; 4] {
    fn from(value: CornerRadius) -> Self {
        [
            value.top_left,
            value.top_right,
            value.bottom_right,
            value.bottom_left,
        ]
    }
}

impl From<f32> for CornerRadius {
    fn from(value: f32) -> Self {
        Self {
            top_left: value,
            top_right: value,
            bottom_right: value,
            bottom_left: value,
        }
    }
}

impl CornerRadius {
    pub fn fit_to(self, width: f32, height: f32) -> Self {
        // Like in CSS: https://drafts.csswg.org/css-backgrounds/#corner-overlap
        let reduction = f32::min(
            f32::min(
                width / (self.top_left + self.top_right),
                width / (self.bottom_left + self.bottom_right),
            ),
            f32::min(
                height / (self.top_left + self.bottom_left),
                height / (self.top_right + self.bottom_right),
            ),
        );
        let reduction = f32::min(1., reduction);

        Self {
            top_left: self.top_left * reduction,
            top_right: self.top_right * reduction,
            bottom_right: self.bottom_right * reduction,
            bottom_left: self.bottom_left * reduction,
        }
    }

    pub fn expanded_by(mut self, width: f32) -> Self {
        // Radius = 0 is preserved, so that square corners remain square.
        if self.top_left > 0. {
            self.top_left += width;
        }
        if self.top_right > 0. {
            self.top_right += width;
        }
        if self.bottom_right > 0. {
            self.bottom_right += width;
        }
        if self.bottom_left > 0. {
            self.bottom_left += width;
        }

        if width < 0. {
            self.top_left = self.top_left.max(0.);
            self.top_right = self.top_right.max(0.);
            self.bottom_left = self.bottom_left.max(0.);
            self.bottom_right = self.bottom_right.max(0.);
        }

        self
    }

    pub fn scaled_by(self, scale: f32) -> Self {
        Self {
            top_left: self.top_left * scale,
            top_right: self.top_right * scale,
            bottom_right: self.bottom_right * scale,
            bottom_left: self.bottom_left * scale,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FocusRing {
    pub off: bool,
    pub width: f64,
    pub active_color: Color,
    pub inactive_color: Color,
    pub urgent_color: Color,
    pub active_gradient: Option<Gradient>,
    pub inactive_gradient: Option<Gradient>,
    pub urgent_gradient: Option<Gradient>,
}

impl Default for FocusRing {
    fn default() -> Self {
        Self {
            off: false,
            width: 4.,
            active_color: Color::from_rgba8_unpremul(127, 200, 255, 255),
            inactive_color: Color::from_rgba8_unpremul(80, 80, 80, 255),
            urgent_color: Color::from_rgba8_unpremul(155, 0, 0, 255),
            active_gradient: None,
            inactive_gradient: None,
            urgent_gradient: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    pub off: bool,
    pub width: f64,
    pub active_color: Color,
    pub inactive_color: Color,
    pub urgent_color: Color,
    pub active_gradient: Option<Gradient>,
    pub inactive_gradient: Option<Gradient>,
    pub urgent_gradient: Option<Gradient>,
}

impl Default for Border {
    fn default() -> Self {
        Self {
            off: true,
            width: 4.,
            active_color: Color::from_rgba8_unpremul(255, 200, 127, 255),
            inactive_color: Color::from_rgba8_unpremul(80, 80, 80, 255),
            urgent_color: Color::from_rgba8_unpremul(155, 0, 0, 255),
            active_gradient: None,
            inactive_gradient: None,
            urgent_gradient: None,
        }
    }
}

impl From<Border> for FocusRing {
    fn from(value: Border) -> Self {
        Self {
            off: value.off,
            width: value.width,
            active_color: value.active_color,
            inactive_color: value.inactive_color,
            urgent_color: value.urgent_color,
            active_gradient: value.active_gradient,
            inactive_gradient: value.inactive_gradient,
            urgent_gradient: value.urgent_gradient,
        }
    }
}

impl From<FocusRing> for Border {
    fn from(value: FocusRing) -> Self {
        Self {
            off: value.off,
            width: value.width,
            active_color: value.active_color,
            inactive_color: value.inactive_color,
            urgent_color: value.urgent_color,
            active_gradient: value.active_gradient,
            inactive_gradient: value.inactive_gradient,
            urgent_gradient: value.urgent_gradient,
        }
    }
}

impl MergeWith<BorderRule> for Border {
    fn merge_with(&mut self, part: &BorderRule) {
        self.off |= part.off;
        if part.on {
            self.off = false;
        }

        merge!((self, part), width);

        merge_color_gradient!(
            (self, part),
            (active_color, active_gradient),
            (inactive_color, inactive_gradient),
            (urgent_color, urgent_gradient),
        );
    }
}

impl MergeWith<BorderRule> for FocusRing {
    fn merge_with(&mut self, part: &BorderRule) {
        let mut x = Border::from(*self);
        x.merge_with(part);
        *self = FocusRing::from(x);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub on: bool,
    pub offset: ShadowOffset,
    pub softness: f64,
    pub spread: f64,
    pub draw_behind_window: bool,
    pub color: Color,
    pub inactive_color: Option<Color>,
}

impl Default for Shadow {
    fn default() -> Self {
        Self {
            on: false,
            offset: ShadowOffset {
                x: FloatOrInt(0.),
                y: FloatOrInt(5.),
            },
            softness: 30.,
            spread: 5.,
            draw_behind_window: false,
            color: Color::from_rgba8_unpremul(0, 0, 0, 0x77),
            inactive_color: None,
        }
    }
}

impl MergeWith<ShadowRule> for Shadow {
    fn merge_with(&mut self, part: &ShadowRule) {
        self.on |= part.on;
        if part.off {
            self.on = false;
        }

        merge!((self, part), softness, spread);

        merge_clone!((self, part), offset, draw_behind_window, color);

        merge_clone_opt!((self, part), inactive_color);
    }
}

#[derive(knuffel::Decode, Debug, Clone, Copy, PartialEq)]
pub struct ShadowOffset {
    #[knuffel(property, default)]
    pub x: FloatOrInt<-65535, 65535>,
    #[knuffel(property, default)]
    pub y: FloatOrInt<-65535, 65535>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkspaceShadow {
    pub off: bool,
    pub offset: ShadowOffset,
    pub softness: f64,
    pub spread: f64,
    pub color: Color,
}

impl Default for WorkspaceShadow {
    fn default() -> Self {
        Self {
            off: false,
            offset: ShadowOffset {
                x: FloatOrInt(0.),
                y: FloatOrInt(10.),
            },
            softness: 40.,
            spread: 10.,
            color: Color::from_rgba8_unpremul(0, 0, 0, 0x50),
        }
    }
}

impl From<WorkspaceShadow> for Shadow {
    fn from(value: WorkspaceShadow) -> Self {
        Self {
            on: !value.off,
            offset: value.offset,
            softness: value.softness,
            spread: value.spread,
            draw_behind_window: false,
            color: value.color,
            inactive_color: None,
        }
    }
}

#[derive(knuffel::Decode, Debug, Clone, Copy, PartialEq)]
pub struct WorkspaceShadowPart {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child)]
    pub offset: Option<ShadowOffset>,
    #[knuffel(child, unwrap(argument))]
    pub softness: Option<FloatOrInt<0, 1024>>,
    #[knuffel(child, unwrap(argument))]
    pub spread: Option<FloatOrInt<-1024, 1024>>,
    #[knuffel(child)]
    pub color: Option<Color>,
}

impl MergeWith<WorkspaceShadowPart> for WorkspaceShadow {
    fn merge_with(&mut self, part: &WorkspaceShadowPart) {
        self.off |= part.off;
        if part.on {
            self.off = false;
        }

        merge_clone!((self, part), offset, color);
        merge!((self, part), softness, spread);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabIndicator {
    pub off: bool,
    pub hide_when_single_tab: bool,
    pub place_within_column: bool,
    pub gap: f64,
    pub width: f64,
    pub length: TabIndicatorLength,
    pub position: TabIndicatorPosition,
    pub gaps_between_tabs: f64,
    pub corner_radius: f64,
    pub active_color: Option<Color>,
    pub inactive_color: Option<Color>,
    pub urgent_color: Option<Color>,
    pub active_gradient: Option<Gradient>,
    pub inactive_gradient: Option<Gradient>,
    pub urgent_gradient: Option<Gradient>,
}

impl Default for TabIndicator {
    fn default() -> Self {
        Self {
            off: false,
            hide_when_single_tab: false,
            place_within_column: false,
            gap: 5.,
            width: 4.,
            length: TabIndicatorLength {
                total_proportion: Some(0.5),
            },
            position: TabIndicatorPosition::Left,
            gaps_between_tabs: 0.,
            corner_radius: 0.,
            active_color: None,
            inactive_color: None,
            urgent_color: None,
            active_gradient: None,
            inactive_gradient: None,
            urgent_gradient: None,
        }
    }
}

impl MergeWith<TabIndicatorPart> for TabIndicator {
    fn merge_with(&mut self, part: &TabIndicatorPart) {
        self.off |= part.off;
        if part.on {
            self.off = false;
        }

        merge!(
            (self, part),
            hide_when_single_tab,
            place_within_column,
            gap,
            width,
            gaps_between_tabs,
            corner_radius,
        );

        merge_clone!((self, part), length, position);

        merge_color_gradient_opt!(
            (self, part),
            (active_color, active_gradient),
            (inactive_color, inactive_gradient),
            (urgent_color, urgent_gradient),
        );
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct TabIndicatorPart {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child)]
    pub hide_when_single_tab: Option<Flag>,
    #[knuffel(child)]
    pub place_within_column: Option<Flag>,
    #[knuffel(child, unwrap(argument))]
    pub gap: Option<FloatOrInt<-65535, 65535>>,
    #[knuffel(child, unwrap(argument))]
    pub width: Option<FloatOrInt<0, 65535>>,
    #[knuffel(child)]
    pub length: Option<TabIndicatorLength>,
    #[knuffel(child, unwrap(argument))]
    pub position: Option<TabIndicatorPosition>,
    #[knuffel(child, unwrap(argument))]
    pub gaps_between_tabs: Option<FloatOrInt<0, 65535>>,
    #[knuffel(child, unwrap(argument))]
    pub corner_radius: Option<FloatOrInt<0, 65535>>,
    #[knuffel(child)]
    pub active_color: Option<Color>,
    #[knuffel(child)]
    pub inactive_color: Option<Color>,
    #[knuffel(child)]
    pub urgent_color: Option<Color>,
    #[knuffel(child)]
    pub active_gradient: Option<Gradient>,
    #[knuffel(child)]
    pub inactive_gradient: Option<Gradient>,
    #[knuffel(child)]
    pub urgent_gradient: Option<Gradient>,
}

#[derive(knuffel::Decode, Debug, Clone, Copy, PartialEq)]
pub struct TabIndicatorLength {
    #[knuffel(property)]
    pub total_proportion: Option<f64>,
}

#[derive(knuffel::DecodeScalar, Debug, Clone, Copy, PartialEq)]
pub enum TabIndicatorPosition {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InsertHint {
    pub off: bool,
    pub color: Color,
    pub gradient: Option<Gradient>,
}

impl Default for InsertHint {
    fn default() -> Self {
        Self {
            off: false,
            color: Color::from_rgba8_unpremul(127, 200, 255, 128),
            gradient: None,
        }
    }
}

impl MergeWith<InsertHintPart> for InsertHint {
    fn merge_with(&mut self, part: &InsertHintPart) {
        self.off |= part.off;
        if part.on {
            self.off = false;
        }

        merge_color_gradient!((self, part), (color, gradient));
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct InsertHintPart {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child)]
    pub color: Option<Color>,
    #[knuffel(child)]
    pub gradient: Option<Gradient>,
}

#[derive(knuffel::DecodeScalar, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockOutFrom {
    Screencast,
    ScreenCapture,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct BorderRule {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child, unwrap(argument))]
    pub width: Option<FloatOrInt<0, 65535>>,
    #[knuffel(child)]
    pub active_color: Option<Color>,
    #[knuffel(child)]
    pub inactive_color: Option<Color>,
    #[knuffel(child)]
    pub urgent_color: Option<Color>,
    #[knuffel(child)]
    pub active_gradient: Option<Gradient>,
    #[knuffel(child)]
    pub inactive_gradient: Option<Gradient>,
    #[knuffel(child)]
    pub urgent_gradient: Option<Gradient>,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct ShadowRule {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child)]
    pub offset: Option<ShadowOffset>,
    #[knuffel(child, unwrap(argument))]
    pub softness: Option<FloatOrInt<0, 1024>>,
    #[knuffel(child, unwrap(argument))]
    pub spread: Option<FloatOrInt<-1024, 1024>>,
    #[knuffel(child, unwrap(argument))]
    pub draw_behind_window: Option<bool>,
    #[knuffel(child)]
    pub color: Option<Color>,
    #[knuffel(child)]
    pub inactive_color: Option<Color>,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct TabIndicatorRule {
    #[knuffel(child)]
    pub active_color: Option<Color>,
    #[knuffel(child)]
    pub inactive_color: Option<Color>,
    #[knuffel(child)]
    pub urgent_color: Option<Color>,
    #[knuffel(child)]
    pub active_gradient: Option<Gradient>,
    #[knuffel(child)]
    pub inactive_gradient: Option<Gradient>,
    #[knuffel(child)]
    pub urgent_gradient: Option<Gradient>,
}

impl MergeWith<Self> for BorderRule {
    fn merge_with(&mut self, part: &Self) {
        merge_on_off!((self, part));

        merge_clone_opt!((self, part), width);

        merge_color_gradient_opt!(
            (self, part),
            (active_color, active_gradient),
            (inactive_color, inactive_gradient),
            (urgent_color, urgent_gradient),
        );
    }
}

impl MergeWith<Self> for ShadowRule {
    fn merge_with(&mut self, part: &Self) {
        merge_on_off!((self, part));

        merge_clone_opt!(
            (self, part),
            offset,
            softness,
            spread,
            draw_behind_window,
            color,
            inactive_color,
        );
    }
}

impl MergeWith<Self> for TabIndicatorRule {
    fn merge_with(&mut self, part: &Self) {
        merge_color_gradient_opt!(
            (self, part),
            (active_color, active_gradient),
            (inactive_color, inactive_gradient),
            (urgent_color, urgent_gradient),
        );
    }
}

impl FromStr for GradientInterpolation {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut iter = s.split_whitespace();
        let in_part1 = iter.next();
        let in_part2 = iter.next();
        let in_part3 = iter.next();

        let Some(in_part1) = in_part1 else {
            return Err(miette!("missing color space"));
        };

        let color = match in_part1 {
            "srgb" => GradientColorSpace::Srgb,
            "srgb-linear" => GradientColorSpace::SrgbLinear,
            "oklab" => GradientColorSpace::Oklab,
            "oklch" => GradientColorSpace::Oklch,
            x => {
                return Err(miette!(
                    "invalid color space {x}; can be srgb, srgb-linear, oklab or oklch"
                ))
            }
        };

        let interpolation = if let Some(in_part2) = in_part2 {
            if color != GradientColorSpace::Oklch {
                return Err(miette!("only oklch color space can have hue interpolation"));
            }

            if in_part3 != Some("hue") {
                return Err(miette!(
                    "interpolation must end with \"hue\", like \"oklch shorter hue\""
                ));
            } else if iter.next().is_some() {
                return Err(miette!("unexpected text after hue interpolation"));
            } else {
                match in_part2 {
                    "shorter" => HueInterpolation::Shorter,
                    "longer" => HueInterpolation::Longer,
                    "increasing" => HueInterpolation::Increasing,
                    "decreasing" => HueInterpolation::Decreasing,
                    x => {
                        return Err(miette!(
                            "invalid hue interpolation {x}; \
                             can be shorter, longer, increasing, decreasing"
                        ))
                    }
                }
            }
        } else {
            HueInterpolation::default()
        };

        Ok(Self {
            color_space: color,
            hue_interpolation: interpolation,
        })
    }
}

impl FromStr for Color {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let color = csscolorparser::parse(s)
            .into_diagnostic()?
            .clamp()
            .to_array();
        Ok(Self::from_array_unpremul(color))
    }
}

#[derive(knuffel::Decode)]
struct ColorRgba {
    #[knuffel(argument)]
    r: u8,
    #[knuffel(argument)]
    g: u8,
    #[knuffel(argument)]
    b: u8,
    #[knuffel(argument)]
    a: u8,
}

impl From<ColorRgba> for Color {
    fn from(value: ColorRgba) -> Self {
        let ColorRgba { r, g, b, a } = value;
        Self::from_array_unpremul([r, g, b, a].map(|x| x as f32 / 255.))
    }
}

// Manual impl to allow both one-argument string and 4-argument RGBA forms.
impl<S> knuffel::Decode<S> for Color
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

        // Get the first argument.
        let mut iter_args = node.arguments.iter();
        let val = iter_args
            .next()
            .ok_or_else(|| DecodeError::missing(node, "additional argument is required"))?;

        // Check for unexpected type name.
        if let Some(typ) = &val.type_name {
            ctx.emit_error(DecodeError::TypeName {
                span: typ.span().clone(),
                found: Some((**typ).clone()),
                expected: knuffel::errors::ExpectedType::no_type(),
                rust_type: "str",
            });
        }

        // Check the argument type.
        let rv = match *val.literal {
            // If it's a string, use FromStr.
            knuffel::ast::Literal::String(ref s) => {
                Color::from_str(s).map_err(|e| DecodeError::conversion(&val.literal, e))
            }
            // Otherwise, fall back to the 4-argument RGBA form.
            _ => return ColorRgba::decode_node(node, ctx).map(Color::from),
        }?;

        // Check for unexpected following arguments.
        if let Some(val) = iter_args.next() {
            ctx.emit_error(DecodeError::unexpected(
                &val.literal,
                "argument",
                "unexpected argument",
            ));
        }

        // Check for unexpected properties and children.
        for name in node.properties.keys() {
            ctx.emit_error(DecodeError::unexpected(
                name,
                "property",
                format!("unexpected property `{}`", name.escape_default()),
            ));
        }
        for child in node.children.as_ref().map(|lst| &lst[..]).unwrap_or(&[]) {
            ctx.emit_error(DecodeError::unexpected(
                child,
                "node",
                format!("unexpected node `{}`", child.node_name.escape_default()),
            ));
        }

        Ok(rv)
    }
}

impl<S> knuffel::Decode<S> for CornerRadius
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

        let decode_radius = |ctx: &mut knuffel::decode::Context<S>,
                             val: &knuffel::ast::Value<S>| {
            // Check for unexpected type name.
            if let Some(typ) = &val.type_name {
                ctx.emit_error(DecodeError::TypeName {
                    span: typ.span().clone(),
                    found: Some((**typ).clone()),
                    expected: knuffel::errors::ExpectedType::no_type(),
                    rust_type: "str",
                });
            }

            // Decode both integers and floats.
            let radius = match *val.literal {
                knuffel::ast::Literal::Int(ref x) => f32::from(match x.try_into() {
                    Ok(x) => x,
                    Err(err) => {
                        ctx.emit_error(DecodeError::conversion(&val.literal, err));
                        0i16
                    }
                }),
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
            };

            if radius < 0. {
                ctx.emit_error(DecodeError::conversion(&val.literal, "radius must be >= 0"));
            }

            radius
        };

        // Get the first argument.
        let mut iter_args = node.arguments.iter();
        let val = iter_args
            .next()
            .ok_or_else(|| DecodeError::missing(node, "additional argument is required"))?;

        let top_left = decode_radius(ctx, val);

        let mut rv = CornerRadius {
            top_left,
            top_right: top_left,
            bottom_right: top_left,
            bottom_left: top_left,
        };

        if let Some(val) = iter_args.next() {
            rv.top_right = decode_radius(ctx, val);

            let val = iter_args.next().ok_or_else(|| {
                DecodeError::missing(node, "either 1 or 4 arguments are required")
            })?;
            rv.bottom_right = decode_radius(ctx, val);

            let val = iter_args.next().ok_or_else(|| {
                DecodeError::missing(node, "either 1 or 4 arguments are required")
            })?;
            rv.bottom_left = decode_radius(ctx, val);

            // Check for unexpected following arguments.
            if let Some(val) = iter_args.next() {
                ctx.emit_error(DecodeError::unexpected(
                    &val.literal,
                    "argument",
                    "unexpected argument",
                ));
            }
        }

        // Check for unexpected properties and children.
        for name in node.properties.keys() {
            ctx.emit_error(DecodeError::unexpected(
                name,
                "property",
                format!("unexpected property `{}`", name.escape_default()),
            ));
        }
        for child in node.children.as_ref().map(|lst| &lst[..]).unwrap_or(&[]) {
            ctx.emit_error(DecodeError::unexpected(
                child,
                "node",
                format!("unexpected node `{}`", child.node_name.escape_default()),
            ));
        }

        Ok(rv)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blur {
    pub off: bool,
    pub passes: u8,
    pub offset: f64,
    pub noise: f64,
    pub saturation: f64,
}

impl Default for Blur {
    fn default() -> Self {
        Self {
            off: false,
            // TODO: tune, reduce passes
            passes: 3,
            offset: 3.,
            noise: 0.02,
            saturation: 1.5,
        }
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct BlurPart {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child, unwrap(argument))]
    pub passes: Option<u8>,
    #[knuffel(child, unwrap(argument))]
    pub offset: Option<FloatOrInt<0, 100>>,
    #[knuffel(child, unwrap(argument))]
    pub noise: Option<FloatOrInt<0, 1>>,
    #[knuffel(child, unwrap(argument))]
    pub saturation: Option<FloatOrInt<0, 1000>>,
}

impl MergeWith<BlurPart> for Blur {
    fn merge_with(&mut self, part: &BlurPart) {
        self.off |= part.off;
        if part.on {
            self.off = false;
        }

        merge_clone!((self, part), passes);
        merge!((self, part), offset, noise, saturation);
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct BlurRule {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct BackgroundEffectRule {
    #[knuffel(child, unwrap(argument))]
    pub xray: Option<bool>,
    #[knuffel(child, default)]
    pub blur: BlurRule,
    #[knuffel(child, unwrap(argument))]
    pub noise: Option<FloatOrInt<0, 1>>,
    #[knuffel(child, unwrap(argument))]
    pub saturation: Option<FloatOrInt<0, 1000>>,
}

impl MergeWith<Self> for BackgroundEffectRule {
    fn merge_with(&mut self, part: &Self) {
        merge_clone_opt!((self, part), xray, noise, saturation);
        merge_on_off!((self.blur, part.blur));
    }
}

/// Resolved background effect rule.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct BackgroundEffect {
    /// Whether to render with xray effect (see through).
    pub xray: Option<bool>,

    /// Whether to blur the background.
    ///
    /// - `None`: blur when the window/layer requests it (e.g. through ext-background-effect
    ///   protocol)
    /// - `Some(false)`: never blur
    /// - `Some(true)`: always blur
    pub blur: Option<bool>,

    pub noise: Option<f64>,
    pub saturation: Option<f64>,
}

impl MergeWith<BackgroundEffectRule> for BackgroundEffect {
    fn merge_with(&mut self, part: &BackgroundEffectRule) {
        merge_clone_opt!((self, part), xray);

        if part.blur.on {
            self.blur = Some(true);
        }
        if part.blur.off {
            self.blur = Some(false);
        }

        if let Some(x) = part.noise {
            self.noise = Some(x.0);
        }

        if let Some(x) = part.saturation {
            self.saturation = Some(x.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use insta::{assert_debug_snapshot, assert_snapshot};

    use super::*;
    use crate::Config;

    #[test]
    fn parse_gradient_interpolation() {
        assert_eq!(
            "srgb".parse::<GradientInterpolation>().unwrap(),
            GradientInterpolation {
                color_space: GradientColorSpace::Srgb,
                ..Default::default()
            }
        );
        assert_eq!(
            "srgb-linear".parse::<GradientInterpolation>().unwrap(),
            GradientInterpolation {
                color_space: GradientColorSpace::SrgbLinear,
                ..Default::default()
            }
        );
        assert_eq!(
            "oklab".parse::<GradientInterpolation>().unwrap(),
            GradientInterpolation {
                color_space: GradientColorSpace::Oklab,
                ..Default::default()
            }
        );
        assert_eq!(
            "oklch".parse::<GradientInterpolation>().unwrap(),
            GradientInterpolation {
                color_space: GradientColorSpace::Oklch,
                ..Default::default()
            }
        );
        assert_eq!(
            "oklch shorter hue"
                .parse::<GradientInterpolation>()
                .unwrap(),
            GradientInterpolation {
                color_space: GradientColorSpace::Oklch,
                hue_interpolation: HueInterpolation::Shorter,
            }
        );
        assert_eq!(
            "oklch longer hue".parse::<GradientInterpolation>().unwrap(),
            GradientInterpolation {
                color_space: GradientColorSpace::Oklch,
                hue_interpolation: HueInterpolation::Longer,
            }
        );
        assert_eq!(
            "oklch decreasing hue"
                .parse::<GradientInterpolation>()
                .unwrap(),
            GradientInterpolation {
                color_space: GradientColorSpace::Oklch,
                hue_interpolation: HueInterpolation::Decreasing,
            }
        );
        assert_eq!(
            "oklch increasing hue"
                .parse::<GradientInterpolation>()
                .unwrap(),
            GradientInterpolation {
                color_space: GradientColorSpace::Oklch,
                hue_interpolation: HueInterpolation::Increasing,
            }
        );

        assert!("".parse::<GradientInterpolation>().is_err());
        assert!("srgb shorter hue".parse::<GradientInterpolation>().is_err());
        assert!("oklch shorter".parse::<GradientInterpolation>().is_err());
        assert!("oklch shorter h".parse::<GradientInterpolation>().is_err());
        assert!("oklch a hue".parse::<GradientInterpolation>().is_err());
        assert!("oklch shorter hue a"
            .parse::<GradientInterpolation>()
            .is_err());
    }

    #[test]
    fn test_border_rule_on_off_merging() {
        fn is_on(config: &str, rules: &[&str]) -> String {
            let mut resolved = Border {
                off: config == "off",
                ..Default::default()
            };

            for rule in rules.iter().copied() {
                let rule = BorderRule {
                    off: rule == "off" || rule == "off,on",
                    on: rule == "on" || rule == "off,on",
                    ..Default::default()
                };

                resolved.merge_with(&rule);
            }

            if resolved.off { "off" } else { "on" }.to_owned()
        }

        assert_snapshot!(is_on("off", &[]), @"off");
        assert_snapshot!(is_on("off", &["off"]), @"off");
        assert_snapshot!(is_on("off", &["on"]), @"on");
        assert_snapshot!(is_on("off", &["off,on"]), @"on");

        assert_snapshot!(is_on("on", &[]), @"on");
        assert_snapshot!(is_on("on", &["off"]), @"off");
        assert_snapshot!(is_on("on", &["on"]), @"on");
        assert_snapshot!(is_on("on", &["off,on"]), @"on");

        assert_snapshot!(is_on("off", &["off", "off"]), @"off");
        assert_snapshot!(is_on("off", &["off", "on"]), @"on");
        assert_snapshot!(is_on("off", &["on", "off"]), @"off");
        assert_snapshot!(is_on("off", &["on", "on"]), @"on");

        assert_snapshot!(is_on("on", &["off", "off"]), @"off");
        assert_snapshot!(is_on("on", &["off", "on"]), @"on");
        assert_snapshot!(is_on("on", &["on", "off"]), @"off");
        assert_snapshot!(is_on("on", &["on", "on"]), @"on");
    }

    #[test]
    fn rule_color_can_override_base_gradient() {
        let config = Config::parse_mem(
            r##"
            // Start with gradient set.
            layout {
                border {
                    active-gradient from="#101010" to="#202020"
                    inactive-gradient from="#111111" to="#212121"
                    urgent-gradient from="#121212" to="#222222"
                }
            }

            // Override with color.
            window-rule {
                border {
                    active-color "#abcdef"
                    inactive-color "#123456"
                    urgent-color "#fedcba"
                }
            }
            "##,
        )
        .unwrap();

        let mut border = config.layout.border;
        for rule in &config.window_rules {
            border.merge_with(&rule.border);
        }

        // Gradient should be None because it's overwritten.
        assert_debug_snapshot!(
            (
                border.active_gradient.is_some(),
                border.inactive_gradient.is_some(),
                border.urgent_gradient.is_some(),
            ),
            @r"
        (
            false,
            false,
            false,
        )
        "
        );
    }

    #[test]
    fn rule_color_can_override_rule_gradient() {
        let config = Config::parse_mem(
            r##"
            // Start with gradient set.
            layout {
                border {
                    active-gradient from="#101010" to="#202020"
                    inactive-gradient from="#111111" to="#212121"
                    urgent-gradient from="#121212" to="#222222"
                }
            }

            // Window rule with gradients set.
            window-rule {
                border {
                    active-gradient from="#303030" to="#404040"
                    inactive-gradient from="#313131" to="#414141"
                    urgent-gradient from="#323232" to="#424242"
                }

                tab-indicator {
                    active-gradient from="#505050" to="#606060"
                    inactive-gradient from="#515151" to="#616161"
                    urgent-gradient from="#525252" to="#626262"
                }
            }

            // Override with color.
            window-rule {
                border {
                    active-color "#abcdef"
                    inactive-color "#123456"
                    urgent-color "#fedcba"
                }

                tab-indicator {
                    active-color "#abcdef"
                    inactive-color "#123456"
                    urgent-color "#fedcba"
                }
            }
            "##,
        )
        .unwrap();

        let mut border = config.layout.border;
        let mut tab_indicator_rule = TabIndicatorRule::default();
        for rule in &config.window_rules {
            border.merge_with(&rule.border);
            tab_indicator_rule.merge_with(&rule.tab_indicator);
        }

        // Gradient should be None because it's overwritten.
        assert_debug_snapshot!(
            (
                border.active_gradient.is_some(),
                border.inactive_gradient.is_some(),
                border.urgent_gradient.is_some(),
                tab_indicator_rule.active_gradient.is_some(),
                tab_indicator_rule.inactive_gradient.is_some(),
                tab_indicator_rule.urgent_gradient.is_some(),
            ),
            @r"
        (
            false,
            false,
            false,
            false,
            false,
            false,
        )
        "
        );
    }
}
