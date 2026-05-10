use std::str::FromStr;

use knuffel::ast::SpannedNode;
use knuffel::decode::Context;
use knuffel::errors::DecodeError;
use knuffel::traits::ErrorSpan;
use knuffel::Decode;
use niri_ipc::{ConfiguredMode, HSyncPolarity, Transform, VSyncPolarity};

use crate::gestures::HotCorners;
use crate::{Color, FloatOrInt, LayoutPart};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Outputs(pub Vec<Output>);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mode {
    pub custom: bool,
    pub mode: ConfiguredMode,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Modeline {
    /// The rate at which pixels are drawn in MHz.
    pub clock: f64,
    /// Horizontal active pixels.
    pub hdisplay: u16,
    /// Horizontal sync pulse start position in pixels.
    pub hsync_start: u16,
    /// Horizontal sync pulse end position in pixels.
    pub hsync_end: u16,
    /// Total horizontal number of pixels before resetting the horizontal drawing position to
    /// zero.
    pub htotal: u16,

    /// Vertical active pixels.
    pub vdisplay: u16,
    /// Vertical sync pulse start position in pixels.
    pub vsync_start: u16,
    /// Vertical sync pulse end position in pixels.
    pub vsync_end: u16,
    /// Total vertical number of pixels before resetting the vertical drawing position to zero.
    pub vtotal: u16,
    /// Horizontal sync polarity: "+hsync" or "-hsync".
    pub hsync_polarity: niri_ipc::HSyncPolarity,
    /// Vertical sync polarity: "+vsync" or "-vsync".
    pub vsync_polarity: niri_ipc::VSyncPolarity,
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub struct Output {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(argument)]
    pub name: String,
    #[knuffel(child, unwrap(argument))]
    pub scale: Option<FloatOrInt<0, 10>>,
    #[knuffel(child, unwrap(argument, str), default = Transform::Normal)]
    pub transform: Transform,
    #[knuffel(child)]
    pub position: Option<Position>,
    #[knuffel(child)]
    pub mode: Option<Mode>,
    #[knuffel(child)]
    pub modeline: Option<Modeline>,
    #[knuffel(child)]
    pub variable_refresh_rate: Option<Vrr>,
    #[knuffel(child)]
    pub focus_at_startup: bool,
    // Deprecated; use layout.background_color.
    #[knuffel(child)]
    pub background_color: Option<Color>,
    #[knuffel(child)]
    pub backdrop_color: Option<Color>,
    #[knuffel(child)]
    pub hot_corners: Option<HotCorners>,
    #[knuffel(child)]
    pub layout: Option<LayoutPart>,
}

impl Output {
    pub fn is_vrr_always_on(&self) -> bool {
        self.variable_refresh_rate == Some(Vrr { on_demand: false })
    }

    pub fn is_vrr_on_demand(&self) -> bool {
        self.variable_refresh_rate == Some(Vrr { on_demand: true })
    }

    pub fn is_vrr_always_off(&self) -> bool {
        self.variable_refresh_rate.is_none()
    }
}

impl Default for Output {
    fn default() -> Self {
        Self {
            off: false,
            focus_at_startup: false,
            name: String::new(),
            scale: None,
            transform: Transform::Normal,
            position: None,
            mode: None,
            modeline: None,
            variable_refresh_rate: None,
            background_color: None,
            backdrop_color: None,
            hot_corners: None,
            layout: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OutputName {
    pub connector: String,
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
}

#[derive(knuffel::Decode, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    #[knuffel(property)]
    pub x: i32,
    #[knuffel(property)]
    pub y: i32,
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Default)]
pub struct Vrr {
    #[knuffel(property, default = false)]
    pub on_demand: bool,
}

impl FromIterator<Output> for Outputs {
    fn from_iter<T: IntoIterator<Item = Output>>(iter: T) -> Self {
        Self(Vec::from_iter(iter))
    }
}

impl Outputs {
    pub fn find(&self, name: &OutputName) -> Option<&Output> {
        self.0.iter().find(|o| name.matches(&o.name))
    }

    pub fn find_mut(&mut self, name: &OutputName) -> Option<&mut Output> {
        self.0.iter_mut().find(|o| name.matches(&o.name))
    }
}

impl OutputName {
    pub fn from_ipc_output(output: &niri_ipc::Output) -> Self {
        Self {
            connector: output.name.clone(),
            make: (output.make != "Unknown").then(|| output.make.clone()),
            model: (output.model != "Unknown").then(|| output.model.clone()),
            serial: output.serial.clone(),
        }
    }

    /// Returns an output description matching what Smithay's `Output::new()` does.
    pub fn format_description(&self) -> String {
        format!(
            "{} - {} - {}",
            self.make.as_deref().unwrap_or("Unknown"),
            self.model.as_deref().unwrap_or("Unknown"),
            self.connector,
        )
    }

    /// Returns an output name that will match by make/model/serial or, if they are missing, by
    /// connector.
    pub fn format_make_model_serial_or_connector(&self) -> String {
        if self.make.is_none() && self.model.is_none() && self.serial.is_none() {
            self.connector.to_string()
        } else {
            self.format_make_model_serial()
        }
    }

    pub fn format_make_model_serial(&self) -> String {
        let make = self.make.as_deref().unwrap_or("Unknown");
        let model = self.model.as_deref().unwrap_or("Unknown");
        let serial = self.serial.as_deref().unwrap_or("Unknown");
        format!("{make} {model} {serial}")
    }

    pub fn matches(&self, target: &str) -> bool {
        // Match by connector.
        if target.eq_ignore_ascii_case(&self.connector) {
            return true;
        }

        // If no other fields are available, don't try to match by them.
        //
        // This is used by niri msg output.
        if self.make.is_none() && self.model.is_none() && self.serial.is_none() {
            return false;
        }

        // Match by "make model serial" with Unknown if something is missing.
        let make = self.make.as_deref().unwrap_or("Unknown");
        let model = self.model.as_deref().unwrap_or("Unknown");
        let serial = self.serial.as_deref().unwrap_or("Unknown");

        let Some(target_make) = target.get(..make.len()) else {
            return false;
        };
        let rest = &target[make.len()..];
        if !target_make.eq_ignore_ascii_case(make) {
            return false;
        }
        if !rest.starts_with(' ') {
            return false;
        }
        let rest = &rest[1..];

        let Some(target_model) = rest.get(..model.len()) else {
            return false;
        };
        let rest = &rest[model.len()..];
        if !target_model.eq_ignore_ascii_case(model) {
            return false;
        }
        if !rest.starts_with(' ') {
            return false;
        }

        let rest = &rest[1..];
        if !rest.eq_ignore_ascii_case(serial) {
            return false;
        }

        true
    }

    // Similar in spirit to Ord, but I don't want to derive Eq to avoid mistakes (you should use
    // `Self::match`, not Eq).
    pub fn compare(&self, other: &Self) -> std::cmp::Ordering {
        let self_missing_mms = self.make.is_none() && self.model.is_none() && self.serial.is_none();
        let other_missing_mms =
            other.make.is_none() && other.model.is_none() && other.serial.is_none();

        match (self_missing_mms, other_missing_mms) {
            (true, true) => self.connector.cmp(&other.connector),
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => self
                .make
                .cmp(&other.make)
                .then_with(|| self.model.cmp(&other.model))
                .then_with(|| self.serial.cmp(&other.serial))
                .then_with(|| self.connector.cmp(&other.connector)),
        }
    }
}

impl<S: ErrorSpan> knuffel::Decode<S> for Mode {
    fn decode_node(node: &SpannedNode<S>, ctx: &mut Context<S>) -> Result<Self, DecodeError<S>> {
        if let Some(type_name) = &node.type_name {
            ctx.emit_error(DecodeError::unexpected(
                type_name,
                "type name",
                "no type name expected for this node",
            ));
        }

        for child in node.children() {
            ctx.emit_error(DecodeError::unexpected(
                child,
                "node",
                format!("unexpected node `{}`", child.node_name.escape_default()),
            ));
        }

        let mut custom: Option<bool> = None;
        for (name, val) in &node.properties {
            match &***name {
                "custom" => {
                    if custom.is_some() {
                        ctx.emit_error(DecodeError::unexpected(
                            name,
                            "property",
                            "unexpected duplicate property `custom`",
                        ))
                    }
                    custom = Some(knuffel::traits::DecodeScalar::decode(val, ctx)?)
                }
                name_str => ctx.emit_error(DecodeError::unexpected(
                    node,
                    "property",
                    format!("unexpected property `{}`", name_str.escape_default()),
                )),
            }
        }
        let custom = custom.unwrap_or(false);

        let mut arguments = node.arguments.iter();
        let mode = if let Some(mode_str) = arguments.next() {
            let temp_mode: String = knuffel::traits::DecodeScalar::decode(mode_str, ctx)?;

            let res = ConfiguredMode::from_str(temp_mode.as_str()).and_then(|mode| {
                if custom {
                    if mode.refresh.is_none() {
                        return Err("no refresh rate found; required for custom mode");
                    } else if let Some(refresh) = mode.refresh {
                        if refresh <= 0. {
                            return Err("custom mode refresh rate must be > 0");
                        }
                    }
                }
                Ok(mode)
            });
            res.map_err(|err_msg| DecodeError::conversion(&mode_str.literal, err_msg))?
        } else {
            return Err(DecodeError::missing(node, "argument `mode` is required"));
        };

        if let Some(surplus) = arguments.next() {
            ctx.emit_error(DecodeError::unexpected(
                &surplus.literal,
                "argument",
                "unexpected argument",
            ))
        }

        Ok(Mode { custom, mode })
    }
}

macro_rules! ensure {
    ($cond:expr, $ctx:expr, $span:expr, $fmt:literal $($arg:tt)* ) => {
        if !$cond {
            $ctx.emit_error(DecodeError::Conversion {
                source: format!($fmt $($arg)*).into(),
                span: $span.literal.span().clone()
            });
        }
    };
}

impl<S: ErrorSpan> Decode<S> for Modeline {
    fn decode_node(node: &SpannedNode<S>, ctx: &mut Context<S>) -> Result<Self, DecodeError<S>> {
        if let Some(type_name) = &node.type_name {
            ctx.emit_error(DecodeError::unexpected(
                type_name,
                "type name",
                "no type name expected for this node",
            ));
        }

        for child in node.children() {
            ctx.emit_error(DecodeError::unexpected(
                child,
                "node",
                format!("unexpected node `{}`", child.node_name.escape_default()),
            ));
        }

        for span in node.properties.keys() {
            ctx.emit_error(DecodeError::unexpected(
                span,
                "node",
                format!("unexpected node `{}`", span.escape_default()),
            ));
        }

        let mut arguments = node.arguments.iter();

        macro_rules! m_required {
            // This could be one identifier if macro_metavar_expr_concat stabilizes
            ($field:ident, $value_field:ident) => {
                let $value_field = arguments.next().ok_or_else(|| {
                    DecodeError::missing(node, format!("missing {} argument", stringify!($value)))
                })?;
                let $field = knuffel::traits::DecodeScalar::decode($value_field, ctx)?;
            };
        }

        m_required!(clock, clock_value);
        m_required!(hdisplay, hdisplay_value);
        m_required!(hsync_start, hsync_start_value);
        m_required!(hsync_end, hsync_end_value);
        m_required!(htotal, htotal_value);
        m_required!(vdisplay, vdisplay_value);
        m_required!(vsync_start, vsync_start_value);
        m_required!(vsync_end, vsync_end_value);
        m_required!(vtotal, vtotal_value);
        m_required!(hsync_polarity, hsync_polarity_value);
        let hsync_polarity =
            HSyncPolarity::from_str(String::as_str(&hsync_polarity)).map_err(|msg| {
                DecodeError::Conversion {
                    span: hsync_polarity_value.literal.span().clone(),
                    source: msg.into(),
                }
            })?;

        m_required!(vsync_polarity, vsync_polarity_value);
        let vsync_polarity =
            VSyncPolarity::from_str(String::as_str(&vsync_polarity)).map_err(|msg| {
                DecodeError::Conversion {
                    span: vsync_polarity_value.literal.span().clone(),
                    source: msg.into(),
                }
            })?;

        ensure!(
            hdisplay < hsync_start,
            ctx,
            hdisplay_value,
            "hdisplay {} must be < hsync_start {}",
            hdisplay,
            hsync_start
        );
        ensure!(
            hsync_start < hsync_end,
            ctx,
            hsync_start_value,
            "hsync_start {} must be < hsync_end {}",
            hsync_start,
            hsync_end,
        );
        ensure!(
            hsync_end < htotal,
            ctx,
            hsync_end_value,
            "hsync_end {} must be < htotal {}",
            hsync_end,
            htotal,
        );
        ensure!(
            0u16 < htotal,
            ctx,
            htotal_value,
            "htotal {} must be > 0",
            htotal
        );
        ensure!(
            vdisplay < vsync_start,
            ctx,
            vdisplay_value,
            "vdisplay {} must be < vsync_start {}",
            vdisplay,
            vsync_start,
        );
        ensure!(
            vsync_start < vsync_end,
            ctx,
            vsync_start_value,
            "vsync_start {} must be < vsync_end {}",
            vsync_start,
            vsync_end,
        );
        ensure!(
            vsync_end < vtotal,
            ctx,
            vsync_end_value,
            "vsync_end {} must be < vtotal {}",
            vsync_end,
            vtotal,
        );
        ensure!(
            0u16 < vtotal,
            ctx,
            vtotal_value,
            "vtotal {} must be > 0",
            vtotal
        );

        if let Some(extra) = arguments.next() {
            ctx.emit_error(DecodeError::unexpected(
                &extra.literal,
                "argument",
                "unexpected argument, all possible arguments were already provided",
            ))
        }

        Ok(Modeline {
            clock,
            hdisplay,
            hsync_start,
            hsync_end,
            htotal,
            vdisplay,
            vsync_start,
            vsync_end,
            vtotal,
            hsync_polarity,
            vsync_polarity,
        })
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_debug_snapshot;

    use super::*;

    #[test]
    fn parse_mode() {
        assert_eq!(
            "2560x1600@165.004".parse::<ConfiguredMode>().unwrap(),
            ConfiguredMode {
                width: 2560,
                height: 1600,
                refresh: Some(165.004),
            },
        );

        assert_eq!(
            "1920x1080".parse::<ConfiguredMode>().unwrap(),
            ConfiguredMode {
                width: 1920,
                height: 1080,
                refresh: None,
            },
        );

        assert!("1920".parse::<ConfiguredMode>().is_err());
        assert!("1920x".parse::<ConfiguredMode>().is_err());
        assert!("1920x1080@".parse::<ConfiguredMode>().is_err());
        assert!("1920x1080@60Hz".parse::<ConfiguredMode>().is_err());
    }

    fn make_output_name(
        connector: &str,
        make: Option<&str>,
        model: Option<&str>,
        serial: Option<&str>,
    ) -> OutputName {
        OutputName {
            connector: connector.to_string(),
            make: make.map(|x| x.to_string()),
            model: model.map(|x| x.to_string()),
            serial: serial.map(|x| x.to_string()),
        }
    }

    #[test]
    fn test_output_name_match() {
        fn check(
            target: &str,
            connector: &str,
            make: Option<&str>,
            model: Option<&str>,
            serial: Option<&str>,
        ) -> bool {
            let name = make_output_name(connector, make, model, serial);
            name.matches(target)
        }

        assert!(check("dp-2", "DP-2", None, None, None));
        assert!(!check("dp-1", "DP-2", None, None, None));
        assert!(check("dp-2", "DP-2", Some("a"), Some("b"), Some("c")));
        assert!(check(
            "some company some monitor 1234",
            "DP-2",
            Some("Some Company"),
            Some("Some Monitor"),
            Some("1234")
        ));
        assert!(!check(
            "some other company some monitor 1234",
            "DP-2",
            Some("Some Company"),
            Some("Some Monitor"),
            Some("1234")
        ));
        assert!(!check(
            "make model serial ",
            "DP-2",
            Some("make"),
            Some("model"),
            Some("serial")
        ));
        assert!(check(
            "make  serial",
            "DP-2",
            Some("make"),
            Some(""),
            Some("serial")
        ));
        assert!(check(
            "make model unknown",
            "DP-2",
            Some("Make"),
            Some("Model"),
            None
        ));
        assert!(check(
            "unknown unknown serial",
            "DP-2",
            None,
            None,
            Some("Serial")
        ));
        assert!(!check("unknown unknown unknown", "DP-2", None, None, None));
    }

    #[test]
    fn test_output_name_sorting() {
        let mut names = vec![
            make_output_name("DP-2", None, None, None),
            make_output_name("DP-1", None, None, None),
            make_output_name("DP-3", Some("B"), Some("A"), Some("A")),
            make_output_name("DP-3", Some("A"), Some("B"), Some("A")),
            make_output_name("DP-3", Some("A"), Some("A"), Some("B")),
            make_output_name("DP-3", None, Some("A"), Some("A")),
            make_output_name("DP-3", Some("A"), None, Some("A")),
            make_output_name("DP-3", Some("A"), Some("A"), None),
            make_output_name("DP-5", Some("A"), Some("A"), Some("A")),
            make_output_name("DP-4", Some("A"), Some("A"), Some("A")),
        ];
        names.sort_by(|a, b| a.compare(b));
        let names = names
            .into_iter()
            .map(|name| {
                format!(
                    "{} | {}",
                    name.format_make_model_serial_or_connector(),
                    name.connector,
                )
            })
            .collect::<Vec<_>>();
        assert_debug_snapshot!(
            names,
            @r#"
        [
            "Unknown A A | DP-3",
            "A Unknown A | DP-3",
            "A A Unknown | DP-3",
            "A A A | DP-4",
            "A A A | DP-5",
            "A A B | DP-3",
            "A B A | DP-3",
            "B A A | DP-3",
            "DP-1 | DP-1",
            "DP-2 | DP-2",
        ]
        "#
        );
    }
}
