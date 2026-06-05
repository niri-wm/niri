use std::str::FromStr;

use miette::miette;
use smithay::input::keyboard::XkbConfig;
use smithay::reexports::input;

use crate::FloatOrInt;
use crate::binds::Modifiers;
use crate::utils::{Flag, MergeWith, Percent};

#[derive(Debug, Default, PartialEq)]
pub struct Input {
    pub keyboard: Keyboard,
    pub touchpad: Touchpad,
    pub mouse: Mouse,
    pub trackpoint: Trackpoint,
    pub trackball: Trackball,
    pub tablet: Tablet,
    pub touch: Touch,
    pub disable_power_key_handling: bool,
    pub warp_mouse_to_focus: Option<WarpMouseToFocus>,
    pub focus_follows_mouse: Option<FocusFollowsMouse>,
    pub workspace_auto_back_and_forth: bool,
    pub mod_key: Option<ModKey>,
    pub mod_key_nested: Option<ModKey>,
}

#[derive(knus::Decode, Debug, Default, PartialEq)]
pub struct InputPart {
    #[knus(child)]
    pub keyboard: Option<KeyboardPart>,
    #[knus(child)]
    pub touchpad: Option<Touchpad>,
    #[knus(child)]
    pub mouse: Option<Mouse>,
    #[knus(child)]
    pub trackpoint: Option<Trackpoint>,
    #[knus(child)]
    pub trackball: Option<Trackball>,
    #[knus(child)]
    pub tablet: Option<Tablet>,
    #[knus(child)]
    pub touch: Option<Touch>,
    #[knus(child)]
    pub disable_power_key_handling: Option<Flag>,
    #[knus(child)]
    pub warp_mouse_to_focus: Option<WarpMouseToFocus>,
    #[knus(child)]
    pub focus_follows_mouse: Option<FocusFollowsMouse>,
    #[knus(child)]
    pub workspace_auto_back_and_forth: Option<Flag>,
    #[knus(child, unwrap(argument, str))]
    pub mod_key: Option<ModKey>,
    #[knus(child, unwrap(argument, str))]
    pub mod_key_nested: Option<ModKey>,
}

impl MergeWith<InputPart> for Input {
    fn merge_with(&mut self, part: &InputPart) {
        merge!(
            (self, part),
            keyboard,
            disable_power_key_handling,
            workspace_auto_back_and_forth,
        );

        merge_clone!(
            (self, part),
            touchpad,
            mouse,
            trackpoint,
            trackball,
            tablet,
            touch,
        );

        merge_clone_opt!(
            (self, part),
            warp_mouse_to_focus,
            focus_follows_mouse,
            mod_key,
            mod_key_nested,
        );
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Keyboard {
    pub xkb: Xkb,
    pub repeat_delay: u16,
    pub repeat_rate: u8,
    pub track_layout: TrackLayout,
    pub numlock: bool,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self {
            xkb: Default::default(),
            // The defaults were chosen to match wlroots and sway.
            repeat_delay: 600,
            repeat_rate: 25,
            track_layout: Default::default(),
            numlock: Default::default(),
        }
    }
}

#[derive(knus::Decode, Debug, PartialEq, Eq)]
pub struct KeyboardPart {
    #[knus(child)]
    pub xkb: Option<Xkb>,
    #[knus(child, unwrap(argument))]
    pub repeat_delay: Option<u16>,
    #[knus(child, unwrap(argument))]
    pub repeat_rate: Option<u8>,
    #[knus(child, unwrap(argument))]
    pub track_layout: Option<TrackLayout>,
    #[knus(child)]
    pub numlock: Option<Flag>,
}

impl MergeWith<KeyboardPart> for Keyboard {
    fn merge_with(&mut self, part: &KeyboardPart) {
        merge_clone!((self, part), xkb, repeat_delay, repeat_rate, track_layout);
        merge!((self, part), numlock);
    }
}

#[derive(knus::Decode, Debug, Default, PartialEq, Eq, Clone)]
pub struct Xkb {
    #[knus(child, unwrap(argument), default)]
    pub rules: String,
    #[knus(child, unwrap(argument), default)]
    pub model: String,
    #[knus(child, unwrap(argument), default)]
    pub layout: String,
    #[knus(child, unwrap(argument), default)]
    pub variant: String,
    #[knus(child, unwrap(argument))]
    pub options: Option<String>,
    #[knus(child, unwrap(argument))]
    pub file: Option<String>,
}

impl Xkb {
    pub fn to_xkb_config(&self) -> XkbConfig<'_> {
        XkbConfig {
            rules: &self.rules,
            model: &self.model,
            layout: &self.layout,
            variant: &self.variant,
            options: self.options.clone(),
        }
    }
}

#[derive(knus::DecodeScalar, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TrackLayout {
    /// The layout change is global.
    #[default]
    Global,
    /// The layout change is window local.
    Window,
}

#[derive(knus::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct ScrollFactor {
    #[knus(argument)]
    pub base: Option<FloatOrInt<0, 100>>,
    #[knus(property)]
    pub horizontal: Option<FloatOrInt<-100, 100>>,
    #[knus(property)]
    pub vertical: Option<FloatOrInt<-100, 100>>,
}

impl ScrollFactor {
    pub fn h_v_factors(&self) -> (f64, f64) {
        let base_value = self.base.map(|f| f.0).unwrap_or(1.0);
        let h = self.horizontal.map(|f| f.0).unwrap_or(base_value);
        let v = self.vertical.map(|f| f.0).unwrap_or(base_value);
        (h, v)
    }
}

#[derive(knus::Decode, Debug, Default, Clone, PartialEq)]
pub struct Touchpad {
    #[knus(child)]
    pub off: bool,
    #[knus(child)]
    pub tap: bool,
    #[knus(child)]
    pub dwt: bool,
    #[knus(child)]
    pub dwtp: bool,
    #[knus(child, unwrap(argument))]
    pub drag: Option<bool>,
    #[knus(child)]
    pub drag_lock: bool,
    #[knus(child)]
    pub natural_scroll: bool,
    #[knus(child, unwrap(argument, str))]
    pub click_method: Option<ClickMethod>,
    #[knus(child, unwrap(argument), default)]
    pub accel_speed: FloatOrInt<-1, 1>,
    #[knus(child, unwrap(argument, str))]
    pub accel_profile: Option<AccelProfile>,
    #[knus(child, unwrap(argument, str))]
    pub scroll_method: Option<ScrollMethod>,
    #[knus(child, unwrap(argument))]
    pub scroll_button: Option<u32>,
    #[knus(child)]
    pub scroll_button_lock: bool,
    #[knus(child, unwrap(argument, str))]
    pub tap_button_map: Option<TapButtonMap>,
    #[knus(child)]
    pub left_handed: bool,
    #[knus(child)]
    pub disabled_on_external_mouse: bool,
    #[knus(child)]
    pub middle_emulation: bool,
    #[knus(child)]
    pub scroll_factor: Option<ScrollFactor>,
}

#[derive(knus::Decode, Debug, Default, Clone, PartialEq)]
pub struct Mouse {
    #[knus(child)]
    pub off: bool,
    #[knus(child)]
    pub natural_scroll: bool,
    #[knus(child, unwrap(argument), default)]
    pub accel_speed: FloatOrInt<-1, 1>,
    #[knus(child, unwrap(argument, str))]
    pub accel_profile: Option<AccelProfile>,
    #[knus(child, unwrap(argument, str))]
    pub scroll_method: Option<ScrollMethod>,
    #[knus(child, unwrap(argument))]
    pub scroll_button: Option<u32>,
    #[knus(child)]
    pub scroll_button_lock: bool,
    #[knus(child)]
    pub left_handed: bool,
    #[knus(child)]
    pub middle_emulation: bool,
    #[knus(child)]
    pub scroll_factor: Option<ScrollFactor>,
}

#[derive(knus::Decode, Debug, Default, Clone, PartialEq)]
pub struct Trackpoint {
    #[knus(child)]
    pub off: bool,
    #[knus(child)]
    pub natural_scroll: bool,
    #[knus(child, unwrap(argument), default)]
    pub accel_speed: FloatOrInt<-1, 1>,
    #[knus(child, unwrap(argument, str))]
    pub accel_profile: Option<AccelProfile>,
    #[knus(child, unwrap(argument, str))]
    pub scroll_method: Option<ScrollMethod>,
    #[knus(child, unwrap(argument))]
    pub scroll_button: Option<u32>,
    #[knus(child)]
    pub scroll_button_lock: bool,
    #[knus(child)]
    pub left_handed: bool,
    #[knus(child)]
    pub middle_emulation: bool,
}

#[derive(knus::Decode, Debug, Default, Clone, PartialEq)]
pub struct Trackball {
    #[knus(child)]
    pub off: bool,
    #[knus(child)]
    pub natural_scroll: bool,
    #[knus(child, unwrap(argument), default)]
    pub accel_speed: FloatOrInt<-1, 1>,
    #[knus(child, unwrap(argument, str))]
    pub accel_profile: Option<AccelProfile>,
    #[knus(child, unwrap(argument, str))]
    pub scroll_method: Option<ScrollMethod>,
    #[knus(child, unwrap(argument))]
    pub scroll_button: Option<u32>,
    #[knus(child)]
    pub scroll_button_lock: bool,
    #[knus(child)]
    pub left_handed: bool,
    #[knus(child)]
    pub middle_emulation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickMethod {
    Clickfinger,
    ButtonAreas,
}

impl From<ClickMethod> for input::ClickMethod {
    fn from(value: ClickMethod) -> Self {
        match value {
            ClickMethod::Clickfinger => Self::Clickfinger,
            ClickMethod::ButtonAreas => Self::ButtonAreas,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccelProfile {
    Adaptive,
    Flat,
}

impl From<AccelProfile> for input::AccelProfile {
    fn from(value: AccelProfile) -> Self {
        match value {
            AccelProfile::Adaptive => Self::Adaptive,
            AccelProfile::Flat => Self::Flat,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollMethod {
    NoScroll,
    TwoFinger,
    Edge,
    OnButtonDown,
}

impl From<ScrollMethod> for input::ScrollMethod {
    fn from(value: ScrollMethod) -> Self {
        match value {
            ScrollMethod::NoScroll => Self::NoScroll,
            ScrollMethod::TwoFinger => Self::TwoFinger,
            ScrollMethod::Edge => Self::Edge,
            ScrollMethod::OnButtonDown => Self::OnButtonDown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapButtonMap {
    LeftRightMiddle,
    LeftMiddleRight,
}

impl From<TapButtonMap> for input::TapButtonMap {
    fn from(value: TapButtonMap) -> Self {
        match value {
            TapButtonMap::LeftRightMiddle => Self::LeftRightMiddle,
            TapButtonMap::LeftMiddleRight => Self::LeftMiddleRight,
        }
    }
}

#[derive(knus::Decode, Debug, Default, Clone, PartialEq)]
pub struct Tablet {
    #[knus(child)]
    pub off: bool,
    #[knus(child, unwrap(arguments))]
    pub calibration_matrix: Option<Vec<f32>>,
    #[knus(child, unwrap(argument))]
    pub map_to_output: Option<String>,
    #[knus(child)]
    pub map_to_focused_output: bool,
    #[knus(child)]
    pub map_to_focused_window: bool,
    #[knus(child)]
    pub left_handed: bool,
}

#[derive(knus::Decode, Debug, Default, Clone, PartialEq)]
pub struct Touch {
    #[knus(child)]
    pub off: bool,
    #[knus(child, unwrap(arguments))]
    pub calibration_matrix: Option<Vec<f32>>,
    #[knus(child, unwrap(argument))]
    pub map_to_output: Option<String>,
}

#[derive(knus::Decode, Debug, Clone, Copy, PartialEq)]
pub struct FocusFollowsMouse {
    #[knus(property, str)]
    pub max_scroll_amount: Option<Percent>,
}

#[derive(knus::Decode, Debug, PartialEq, Eq, Clone, Copy)]
pub struct WarpMouseToFocus {
    #[knus(property, str)]
    pub mode: Option<WarpMouseToFocusMode>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum WarpMouseToFocusMode {
    CenterXy,
    CenterXyAlways,
}

impl FromStr for WarpMouseToFocusMode {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "center-xy" => Ok(Self::CenterXy),
            "center-xy-always" => Ok(Self::CenterXyAlways),
            _ => Err(miette!(
                r#"invalid mode for warp-mouse-to-focus, can be "center-xy" or "center-xy-always" (or leave unset for separate centering)"#
            )),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ModKey {
    Ctrl,
    Shift,
    Alt,
    Super,
    IsoLevel3Shift,
    IsoLevel5Shift,
}

impl ModKey {
    pub fn to_modifiers(&self) -> Modifiers {
        match self {
            ModKey::Ctrl => Modifiers::CTRL,
            ModKey::Shift => Modifiers::SHIFT,
            ModKey::Alt => Modifiers::ALT,
            ModKey::Super => Modifiers::SUPER,
            ModKey::IsoLevel3Shift => Modifiers::ISO_LEVEL3_SHIFT,
            ModKey::IsoLevel5Shift => Modifiers::ISO_LEVEL5_SHIFT,
        }
    }
}

impl FromStr for ModKey {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match &*s.to_ascii_lowercase() {
            "ctrl" | "control" => Ok(Self::Ctrl),
            "shift" => Ok(Self::Shift),
            "alt" => Ok(Self::Alt),
            "super" | "win" => Ok(Self::Super),
            "iso_level3_shift" | "mod5" => Ok(Self::IsoLevel3Shift),
            "iso_level5_shift" | "mod3" => Ok(Self::IsoLevel5Shift),
            _ => Err(miette!("invalid Mod key: {s}")),
        }
    }
}

impl FromStr for ClickMethod {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "clickfinger" => Ok(Self::Clickfinger),
            "button-areas" => Ok(Self::ButtonAreas),
            _ => Err(miette!(
                r#"invalid click method, can be "button-areas" or "clickfinger""#
            )),
        }
    }
}

impl FromStr for AccelProfile {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "adaptive" => Ok(Self::Adaptive),
            "flat" => Ok(Self::Flat),
            _ => Err(miette!(
                r#"invalid accel profile, can be "adaptive" or "flat""#
            )),
        }
    }
}

impl FromStr for ScrollMethod {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "no-scroll" => Ok(Self::NoScroll),
            "two-finger" => Ok(Self::TwoFinger),
            "edge" => Ok(Self::Edge),
            "on-button-down" => Ok(Self::OnButtonDown),
            _ => Err(miette!(
                r#"invalid scroll method, can be "no-scroll", "two-finger", "edge", or "on-button-down""#
            )),
        }
    }
}

impl FromStr for TapButtonMap {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "left-right-middle" => Ok(Self::LeftRightMiddle),
            "left-middle-right" => Ok(Self::LeftMiddleRight),
            _ => Err(miette!(
                r#"invalid tap button map, can be "left-right-middle" or "left-middle-right""#
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_debug_snapshot;

    use super::*;

    #[track_caller]
    fn do_parse(text: &str) -> Input {
        let part = knus::parse("test.kdl", text)
            .map_err(miette::Report::new)
            .unwrap();
        Input::from_part(&part)
    }

    #[test]
    fn parse_scroll_factor_combined() {
        // Test combined scroll-factor syntax
        let parsed = do_parse(
            r#"
            mouse {
                scroll-factor 2.0
            }
            touchpad {
                scroll-factor 1.5
            }
            "#,
        );

        assert_debug_snapshot!(parsed.mouse.scroll_factor, @r#"
        Some(
            ScrollFactor {
                base: Some(
                    FloatOrInt(
                        2.0,
                    ),
                ),
                horizontal: None,
                vertical: None,
            },
        )
        "#);
        assert_debug_snapshot!(parsed.touchpad.scroll_factor, @r#"
        Some(
            ScrollFactor {
                base: Some(
                    FloatOrInt(
                        1.5,
                    ),
                ),
                horizontal: None,
                vertical: None,
            },
        )
        "#);
    }

    #[test]
    fn parse_scroll_factor_split() {
        // Test split horizontal/vertical syntax
        let parsed = do_parse(
            r#"
            mouse {
                scroll-factor horizontal=2.0 vertical=-1.0
            }
            touchpad {
                scroll-factor horizontal=-1.5 vertical=0.5
            }
            "#,
        );

        assert_debug_snapshot!(parsed.mouse.scroll_factor, @r#"
        Some(
            ScrollFactor {
                base: None,
                horizontal: Some(
                    FloatOrInt(
                        2.0,
                    ),
                ),
                vertical: Some(
                    FloatOrInt(
                        -1.0,
                    ),
                ),
            },
        )
        "#);
        assert_debug_snapshot!(parsed.touchpad.scroll_factor, @r#"
        Some(
            ScrollFactor {
                base: None,
                horizontal: Some(
                    FloatOrInt(
                        -1.5,
                    ),
                ),
                vertical: Some(
                    FloatOrInt(
                        0.5,
                    ),
                ),
            },
        )
        "#);
    }

    #[test]
    fn parse_scroll_factor_partial() {
        // Test partial specification (only one axis)
        let parsed = do_parse(
            r#"
            mouse {
                scroll-factor horizontal=2.0
            }
            touchpad {
                scroll-factor vertical=-1.5
            }
            "#,
        );

        assert_debug_snapshot!(parsed.mouse.scroll_factor, @r#"
        Some(
            ScrollFactor {
                base: None,
                horizontal: Some(
                    FloatOrInt(
                        2.0,
                    ),
                ),
                vertical: None,
            },
        )
        "#);
        assert_debug_snapshot!(parsed.touchpad.scroll_factor, @r#"
        Some(
            ScrollFactor {
                base: None,
                horizontal: None,
                vertical: Some(
                    FloatOrInt(
                        -1.5,
                    ),
                ),
            },
        )
        "#);
    }

    #[test]
    fn parse_scroll_factor_mixed() {
        // Test mixed base + override syntax
        let parsed = do_parse(
            r#"
            mouse {
                scroll-factor 2 vertical=-1
            }
            touchpad {
                scroll-factor 1.5 horizontal=3
            }
            "#,
        );

        assert_debug_snapshot!(parsed.mouse.scroll_factor, @r#"
        Some(
            ScrollFactor {
                base: Some(
                    FloatOrInt(
                        2.0,
                    ),
                ),
                horizontal: None,
                vertical: Some(
                    FloatOrInt(
                        -1.0,
                    ),
                ),
            },
        )
        "#);
        assert_debug_snapshot!(parsed.touchpad.scroll_factor, @r#"
        Some(
            ScrollFactor {
                base: Some(
                    FloatOrInt(
                        1.5,
                    ),
                ),
                horizontal: Some(
                    FloatOrInt(
                        3.0,
                    ),
                ),
                vertical: None,
            },
        )
        "#);
    }

    #[test]
    fn scroll_factor_h_v_factors() {
        let sf = ScrollFactor {
            base: Some(FloatOrInt(2.0)),
            horizontal: None,
            vertical: None,
        };
        assert_debug_snapshot!(sf.h_v_factors(), @r#"
        (
            2.0,
            2.0,
        )
        "#);

        let sf = ScrollFactor {
            base: None,
            horizontal: Some(FloatOrInt(3.0)),
            vertical: Some(FloatOrInt(-1.0)),
        };
        assert_debug_snapshot!(sf.h_v_factors(), @r#"
        (
            3.0,
            -1.0,
        )
        "#);

        let sf = ScrollFactor {
            base: Some(FloatOrInt(2.0)),
            horizontal: Some(FloatOrInt(1.0)),
            vertical: None,
        };
        assert_debug_snapshot!(sf.h_v_factors(), @r"
        (
            1.0,
            2.0,
        )
        ");
    }
}
