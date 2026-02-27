use std::str::FromStr;

use miette::miette;
use smithay::input::keyboard::XkbConfig;
use smithay::reexports::input;

use crate::binds::Modifiers;
use crate::utils::{Flag, MergeWith, Percent};
use crate::FloatOrInt;

#[derive(Debug, Default, PartialEq)]
pub struct Input {
    pub keyboards: Keyboards,
    pub touchpad: Touchpad,
    pub mice: Mice,
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

#[derive(knuffel::Decode, Debug, Default, PartialEq)]
pub struct InputPart {
    #[knuffel(children(name = "keyboard"))]
    pub keyboards: Keyboards,
    #[knuffel(child)]
    pub touchpad: Option<Touchpad>,
    #[knuffel(children(name = "mouse"))]
    pub mice: Mice,
    #[knuffel(child)]
    pub trackpoint: Option<Trackpoint>,
    #[knuffel(child)]
    pub trackball: Option<Trackball>,
    #[knuffel(child)]
    pub tablet: Option<Tablet>,
    #[knuffel(child)]
    pub touch: Option<Touch>,
    #[knuffel(child)]
    pub disable_power_key_handling: Option<Flag>,
    #[knuffel(child)]
    pub warp_mouse_to_focus: Option<WarpMouseToFocus>,
    #[knuffel(child)]
    pub focus_follows_mouse: Option<FocusFollowsMouse>,
    #[knuffel(child)]
    pub workspace_auto_back_and_forth: Option<Flag>,
    #[knuffel(child, unwrap(argument, str))]
    pub mod_key: Option<ModKey>,
    #[knuffel(child, unwrap(argument, str))]
    pub mod_key_nested: Option<ModKey>,
}

impl MergeWith<InputPart> for Input {
    fn merge_with(&mut self, part: &InputPart) {
        merge!(
            (self, part),
            disable_power_key_handling,
            workspace_auto_back_and_forth,
        );

        merge_clone!((self, part), touchpad, trackpoint, trackball, tablet, touch,);

        merge_clone_opt!(
            (self, part),
            warp_mouse_to_focus,
            focus_follows_mouse,
            mod_key,
            mod_key_nested,
        );

        self.keyboards.0.extend(part.keyboards.0.iter().cloned());
        self.mice.0.extend(part.mice.0.iter().cloned());
    }
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct Keyboard {
    #[knuffel(argument)]
    pub name: Option<String>,
    #[knuffel(child)]
    pub xkb: Option<Xkb>,
    #[knuffel(child, unwrap(argument))]
    pub repeat_delay: Option<u16>,
    #[knuffel(child, unwrap(argument))]
    pub repeat_rate: Option<u8>,
    #[knuffel(child, unwrap(argument))]
    pub track_layout: Option<TrackLayout>,
    #[knuffel(child)]
    pub numlock: Option<Flag>,
}

const DEFAULT_KEYBOARD: Keyboard = Keyboard {
    name: None,
    xkb: None,
    repeat_delay: None,
    repeat_rate: None,
    track_layout: None,
    numlock: None,
};

impl Default for Keyboard {
    fn default() -> Self {
        DEFAULT_KEYBOARD
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Keyboards(pub Vec<Keyboard>);

impl FromIterator<Keyboard> for Keyboards {
    fn from_iter<T: IntoIterator<Item = Keyboard>>(iter: T) -> Self {
        Self(Vec::from_iter(iter))
    }
}

impl Keyboards {
    pub fn find<'a>(&'a self, name: Option<&str>) -> &'a Keyboard {
        name.and_then(|name| {
            self.0.iter().find(|k| {
                k.name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
        })
        .or_else(|| self.0.iter().find(|k| k.name.is_none()))
        .unwrap_or(&DEFAULT_KEYBOARD)
    }

    /// Get the XKB configuration, defaulting to an empty config if not set.
    pub fn xkb(&self) -> Xkb {
        self.find(None).xkb.clone().unwrap_or_default()
    }

    /// Get the repeat delay, defaulting to 600ms if not set.
    pub fn repeat_delay(&self) -> u16 {
        self.find(None).repeat_delay.unwrap_or(600)
    }

    /// Get the repeat rate, defaulting to 25 if not set.
    pub fn repeat_rate(&self) -> u8 {
        self.find(None).repeat_rate.unwrap_or(25)
    }

    /// Get the track layout setting, defaulting to Global if not set.
    pub fn track_layout(&self) -> TrackLayout {
        self.find(None).track_layout.unwrap_or_default()
    }

    /// Get the numlock setting, defaulting to false if not set.
    pub fn numlock(&self) -> bool {
        self.find(None).numlock.map(|f| f.0).unwrap_or(false)
    }
}

#[derive(knuffel::Decode, Debug, Default, PartialEq, Eq, Clone)]
pub struct Xkb {
    #[knuffel(child, unwrap(argument), default)]
    pub rules: String,
    #[knuffel(child, unwrap(argument), default)]
    pub model: String,
    #[knuffel(child, unwrap(argument), default)]
    pub layout: String,
    #[knuffel(child, unwrap(argument), default)]
    pub variant: String,
    #[knuffel(child, unwrap(argument))]
    pub options: Option<String>,
    #[knuffel(child, unwrap(argument))]
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

#[derive(knuffel::DecodeScalar, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TrackLayout {
    /// The layout change is global.
    #[default]
    Global,
    /// The layout change is window local.
    Window,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct ScrollFactor {
    #[knuffel(argument)]
    pub base: Option<FloatOrInt<0, 100>>,
    #[knuffel(property)]
    pub horizontal: Option<FloatOrInt<-100, 100>>,
    #[knuffel(property)]
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

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct Touchpad {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub tap: bool,
    #[knuffel(child)]
    pub dwt: bool,
    #[knuffel(child)]
    pub dwtp: bool,
    #[knuffel(child, unwrap(argument))]
    pub drag: Option<bool>,
    #[knuffel(child)]
    pub drag_lock: bool,
    #[knuffel(child)]
    pub natural_scroll: bool,
    #[knuffel(child, unwrap(argument, str))]
    pub click_method: Option<ClickMethod>,
    #[knuffel(child, unwrap(argument), default)]
    pub accel_speed: FloatOrInt<-1, 1>,
    #[knuffel(child, unwrap(argument, str))]
    pub accel_profile: Option<AccelProfile>,
    #[knuffel(child, unwrap(argument, str))]
    pub scroll_method: Option<ScrollMethod>,
    #[knuffel(child, unwrap(argument))]
    pub scroll_button: Option<u32>,
    #[knuffel(child)]
    pub scroll_button_lock: bool,
    #[knuffel(child, unwrap(argument, str))]
    pub tap_button_map: Option<TapButtonMap>,
    #[knuffel(child)]
    pub left_handed: bool,
    #[knuffel(child)]
    pub disabled_on_external_mouse: bool,
    #[knuffel(child)]
    pub middle_emulation: bool,
    #[knuffel(child)]
    pub scroll_factor: Option<ScrollFactor>,
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub struct Mouse {
    #[knuffel(argument)]
    pub name: Option<String>,
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub natural_scroll: bool,
    #[knuffel(child, unwrap(argument), default)]
    pub accel_speed: FloatOrInt<-1, 1>,
    #[knuffel(child, unwrap(argument, str))]
    pub accel_profile: Option<AccelProfile>,
    #[knuffel(child, unwrap(argument, str))]
    pub scroll_method: Option<ScrollMethod>,
    #[knuffel(child, unwrap(argument))]
    pub scroll_button: Option<u32>,
    #[knuffel(child)]
    pub scroll_button_lock: bool,
    #[knuffel(child)]
    pub left_handed: bool,
    #[knuffel(child)]
    pub middle_emulation: bool,
    #[knuffel(child)]
    pub scroll_factor: Option<ScrollFactor>,
}

const DEFAULT_MOUSE: Mouse = Mouse {
    name: None,
    off: false,
    natural_scroll: false,
    accel_speed: FloatOrInt(0.0),
    accel_profile: None,
    scroll_method: None,
    scroll_button: None,
    scroll_button_lock: false,
    left_handed: false,
    middle_emulation: false,
    scroll_factor: None,
};

impl Default for Mouse {
    fn default() -> Self {
        DEFAULT_MOUSE
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Mice(pub Vec<Mouse>);

impl FromIterator<Mouse> for Mice {
    fn from_iter<T: IntoIterator<Item = Mouse>>(iter: T) -> Self {
        Self(Vec::from_iter(iter))
    }
}

impl Mice {
    pub fn find<'a>(&'a self, name: Option<&str>) -> &'a Mouse {
        name.and_then(|name| {
            self.0.iter().find(|m| {
                m.name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
        })
        .or_else(|| self.0.iter().find(|m| m.name.is_none()))
        .unwrap_or(&DEFAULT_MOUSE)
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct Trackpoint {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub natural_scroll: bool,
    #[knuffel(child, unwrap(argument), default)]
    pub accel_speed: FloatOrInt<-1, 1>,
    #[knuffel(child, unwrap(argument, str))]
    pub accel_profile: Option<AccelProfile>,
    #[knuffel(child, unwrap(argument, str))]
    pub scroll_method: Option<ScrollMethod>,
    #[knuffel(child, unwrap(argument))]
    pub scroll_button: Option<u32>,
    #[knuffel(child)]
    pub scroll_button_lock: bool,
    #[knuffel(child)]
    pub left_handed: bool,
    #[knuffel(child)]
    pub middle_emulation: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct Trackball {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub natural_scroll: bool,
    #[knuffel(child, unwrap(argument), default)]
    pub accel_speed: FloatOrInt<-1, 1>,
    #[knuffel(child, unwrap(argument, str))]
    pub accel_profile: Option<AccelProfile>,
    #[knuffel(child, unwrap(argument, str))]
    pub scroll_method: Option<ScrollMethod>,
    #[knuffel(child, unwrap(argument))]
    pub scroll_button: Option<u32>,
    #[knuffel(child)]
    pub scroll_button_lock: bool,
    #[knuffel(child)]
    pub left_handed: bool,
    #[knuffel(child)]
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

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct Tablet {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child, unwrap(arguments))]
    pub calibration_matrix: Option<Vec<f32>>,
    #[knuffel(child, unwrap(argument))]
    pub map_to_output: Option<String>,
    #[knuffel(child)]
    pub left_handed: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct Touch {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child, unwrap(arguments))]
    pub calibration_matrix: Option<Vec<f32>>,
    #[knuffel(child, unwrap(argument))]
    pub map_to_output: Option<String>,
}

#[derive(knuffel::Decode, Debug, Clone, Copy, PartialEq)]
pub struct FocusFollowsMouse {
    #[knuffel(property, str)]
    pub max_scroll_amount: Option<Percent>,
}

#[derive(knuffel::Decode, Debug, PartialEq, Eq, Clone, Copy)]
pub struct WarpMouseToFocus {
    #[knuffel(property, str)]
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
        let part = knuffel::parse("test.kdl", text)
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

        assert_debug_snapshot!(parsed.mice.find(None).scroll_factor, @r#"
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

        assert_debug_snapshot!(parsed.mice.find(None).scroll_factor, @r#"
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

        assert_debug_snapshot!(parsed.mice.find(None).scroll_factor, @r#"
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

        assert_debug_snapshot!(parsed.mice.find(None).scroll_factor, @r#"
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
    fn parse_keyboards_name() {
        // Test no name and name provided
        let parsed = do_parse(
            r#"
            keyboard {
                xkb {
                    layout "us"
                }
            }

            keyboard "AT Translated Set 2 keyboard" {
                xkb {
                    layout "us,ru"
                    options "grp:alt_shift_toggle"
                }
            }
            "#,
        );

        assert_debug_snapshot!(parsed.keyboards, @r#"
        Keyboards(
            [
                Keyboard {
                    name: None,
                    xkb: Some(
                        Xkb {
                            rules: "",
                            model: "",
                            layout: "us",
                            variant: "",
                            options: None,
                            file: None,
                        },
                    ),
                    repeat_delay: None,
                    repeat_rate: None,
                    track_layout: None,
                    numlock: None,
                },
                Keyboard {
                    name: Some(
                        "AT Translated Set 2 keyboard",
                    ),
                    xkb: Some(
                        Xkb {
                            rules: "",
                            model: "",
                            layout: "us,ru",
                            variant: "",
                            options: Some(
                                "grp:alt_shift_toggle",
                            ),
                            file: None,
                        },
                    ),
                    repeat_delay: None,
                    repeat_rate: None,
                    track_layout: None,
                    numlock: None,
                },
            ],
        )
        "#);
    }

    #[test]
    fn parse_mice_name() {
        // Test no name and name provided
        let parsed = do_parse(
            r#"
            mouse {
                natural-scroll
                accel-speed 0.5
            }

            mouse "Logitech G Pro" {
                accel-speed -0.3
                accel-profile "flat"
            }
            "#,
        );

        assert_debug_snapshot!(parsed.mice, @r#"
        Mice(
            [
                Mouse {
                    name: None,
                    off: false,
                    natural_scroll: true,
                    accel_speed: FloatOrInt(
                        0.5,
                    ),
                    accel_profile: None,
                    scroll_method: None,
                    scroll_button: None,
                    scroll_button_lock: false,
                    left_handed: false,
                    middle_emulation: false,
                    scroll_factor: None,
                },
                Mouse {
                    name: Some(
                        "Logitech G Pro",
                    ),
                    off: false,
                    natural_scroll: false,
                    accel_speed: FloatOrInt(
                        -0.3,
                    ),
                    accel_profile: Some(
                        Flat,
                    ),
                    scroll_method: None,
                    scroll_button: None,
                    scroll_button_lock: false,
                    left_handed: false,
                    middle_emulation: false,
                    scroll_factor: None,
                },
            ],
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
