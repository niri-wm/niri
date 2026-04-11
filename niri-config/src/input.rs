use std::str::FromStr;

use miette::miette;
use smithay::input::keyboard::XkbConfig;
use smithay::reexports::input;

use crate::binds::Modifiers;
use crate::utils::{Flag, MergeWith, Percent};
use crate::FloatOrInt;

#[derive(Debug, Default, PartialEq)]
pub struct Input {
    pub keyboard: Keyboard,
    pub touchpad: Touchpad,
    pub mouse: Mouse,
    pub trackpoint: Trackpoint,
    pub trackball: Trackball,
    pub tablet: Tablet,
    pub touchscreen: Touchscreen,
    pub disable_power_key_handling: bool,
    pub warp_mouse_to_focus: Option<WarpMouseToFocus>,
    pub focus_follows_mouse: Option<FocusFollowsMouse>,
    pub workspace_auto_back_and_forth: bool,
    pub mod_key: Option<ModKey>,
    pub mod_key_nested: Option<ModKey>,
}

#[derive(knuffel::Decode, Debug, Default, PartialEq)]
pub struct InputPart {
    #[knuffel(child)]
    pub keyboard: Option<KeyboardPart>,
    #[knuffel(child)]
    pub touchpad: Option<Touchpad>,
    #[knuffel(child)]
    pub mouse: Option<Mouse>,
    #[knuffel(child)]
    pub trackpoint: Option<Trackpoint>,
    #[knuffel(child)]
    pub trackball: Option<Trackball>,
    #[knuffel(child)]
    pub tablet: Option<Tablet>,
    #[knuffel(child)]
    pub touchscreen: Option<Touchscreen>,
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
            touchscreen,
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

#[derive(knuffel::Decode, Debug, PartialEq, Eq)]
pub struct KeyboardPart {
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

impl MergeWith<KeyboardPart> for Keyboard {
    fn merge_with(&mut self, part: &KeyboardPart) {
        merge_clone!((self, part), xkb, repeat_delay, repeat_rate, track_layout);
        merge!((self, part), numlock);
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
        let base_value = self.base.map(|f| f.0).unwrap_or(0.4);
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
    #[knuffel(child)]
    pub gestures: Option<TouchpadGesturesConfig>,
}

impl Touchpad {
    pub fn gesture_threshold(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.recognition_threshold)
            .unwrap_or(16.0)
    }

    pub fn gesture_progress_distance(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.gesture_progress_distance)
            .unwrap_or(40.0)
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct Mouse {
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
    pub map_to_focused_output: bool,
    #[knuffel(child)]
    pub left_handed: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct Touchscreen {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub natural_scroll: bool,
    #[knuffel(child, unwrap(arguments))]
    pub calibration_matrix: Option<Vec<f32>>,
    #[knuffel(child, unwrap(argument))]
    pub map_to_output: Option<String>,
    #[knuffel(child)]
    pub gestures: Option<TouchscreenGesturesConfig>,
}

impl Touchscreen {
    pub fn gesture_threshold(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.recognition_threshold)
            .unwrap_or(16.0)
    }

    pub fn edge_threshold(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.edge_threshold)
            .unwrap_or(20.0)
    }

    pub fn pinch_threshold(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.pinch_threshold)
            .unwrap_or(30.0)
    }

    pub fn pinch_ratio(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.pinch_ratio)
            .unwrap_or(2.0)
    }

    pub fn pinch_sensitivity(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.pinch_sensitivity)
            .unwrap_or(1.0)
    }

    pub fn finger_threshold_scale(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.finger_threshold_scale)
            .unwrap_or(1.5)
    }

    pub fn gesture_progress_distance(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.gesture_progress_distance)
            .unwrap_or(200.0)
    }

    pub fn pinch_progress_distance(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.pinch_progress_distance)
            .unwrap_or(100.0)
    }

    /// Minimum rotation (in **degrees** in the KDL config — converted to
    /// radians internally) that must be accumulated before a multi-finger
    /// gesture is classified as a rotation rather than a swipe or pinch.
    /// Default: 15°.
    pub fn rotation_threshold(&self) -> f64 {
        let deg = self
            .gestures
            .as_ref()
            .and_then(|g| g.rotation_threshold)
            .unwrap_or(15.0);
        deg.to_radians()
    }

    /// Leniency ratio for rotation dominance. Higher = rotation wins more
    /// easily. The effective gate is `rotation_arc >= swipe_distance *
    /// (1.0 / rotation_ratio)`, so `rotation_ratio=2.0` means rotation
    /// arc only needs to be half the swipe distance to count as a
    /// rotation — deliberately lenient, because a user rotating with
    /// incidental hand drift will often produce more translation than
    /// arc length. Default: 2.0 (arc must be ≥ 0.5 × swipe). Dropping
    /// this back to 0.5 reproduces the original strict behavior
    /// (arc must be ≥ 2 × swipe) which made drift-while-rotating
    /// essentially unrecognizable.
    pub fn rotation_ratio(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.rotation_ratio)
            .unwrap_or(2.0)
    }

    /// Degrees of rotation (in the KDL config — converted to radians
    /// internally) for IPC `GestureProgress` events on rotation gestures
    /// to reach `progress = ±1.0`. Default: 90°.
    pub fn rotation_progress_distance(&self) -> f64 {
        let deg = self
            .gestures
            .as_ref()
            .and_then(|g| g.rotation_progress_distance)
            .unwrap_or(90.0);
        deg.to_radians()
    }

    /// Returns the scaled recognition threshold for a given finger count.
    /// Extra fingers above 3 increase the threshold by the scale factor.
    pub fn scaled_threshold(&self, finger_count: usize) -> f64 {
        let base = self.gesture_threshold();
        let scale = self.finger_threshold_scale();
        let extra = finger_count.saturating_sub(3) as f64;
        base * (1.0 + extra * (scale - 1.0))
    }

}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenEdge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Which third of an edge a touch landed in.
///
/// The perpendicular axis of the edge is split into thirds: for Top/Bottom
/// that's the x axis (Start=leftmost third, End=rightmost third); for
/// Left/Right that's the y axis (Start=topmost third, End=bottommost third).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeZone {
    Start,
    Center,
    End,
}

impl ScreenEdge {
    /// Lower-cased name used in KDL config and IPC events (`"left"`,
    /// `"right"`, `"top"`, `"bottom"`).
    pub fn as_kdl_name(self) -> &'static str {
        match self {
            ScreenEdge::Left => "left",
            ScreenEdge::Right => "right",
            ScreenEdge::Top => "top",
            ScreenEdge::Bottom => "bottom",
        }
    }
}

/// Lower-cased zone name used in KDL config and IPC events. The
/// vocabulary rotates per edge axis: top/bottom edges take
/// `left|center|right`; left/right edges take `top|center|bottom`. This
/// is the single source of truth for that mapping — parsers, IPC
/// emitters, and display helpers all share it.
pub fn zone_kdl_name(edge: ScreenEdge, zone: EdgeZone) -> &'static str {
    match (edge, zone) {
        (ScreenEdge::Top | ScreenEdge::Bottom, EdgeZone::Start) => "left",
        (ScreenEdge::Top | ScreenEdge::Bottom, EdgeZone::Center) => "center",
        (ScreenEdge::Top | ScreenEdge::Bottom, EdgeZone::End) => "right",
        (ScreenEdge::Left | ScreenEdge::Right, EdgeZone::Start) => "top",
        (ScreenEdge::Left | ScreenEdge::Right, EdgeZone::Center) => "center",
        (ScreenEdge::Left | ScreenEdge::Right, EdgeZone::End) => "bottom",
    }
}

/// Tuning parameters for touchscreen gesture recognition.
///
/// The actual gesture binds (e.g. `TouchSwipe fingers=3 direction="up"`,
/// `TouchEdge edge="left"`) live in the main `binds {}` block — this
/// struct only controls how movement is classified and how IPC progress
/// is reported.
#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct TouchscreenGesturesConfig {
    /// Distance in pixels fingers must move before a swipe gesture is
    /// recognized and starts firing events. Lower values feel more responsive
    /// but risk triggering on incidental finger drift. Default: 16.0.
    #[knuffel(child, unwrap(argument))]
    pub recognition_threshold: Option<f64>,
    /// Distance in pixels from a screen edge within which a touch must start
    /// for it to count as an edge swipe (`TouchEdge edge="left|right|top|bottom"`).
    /// Touches beginning farther from the edge are treated as regular swipes.
    /// Default: 20.0.
    #[knuffel(child, unwrap(argument))]
    pub edge_threshold: Option<f64>,
    /// How far fingers must move together or apart (as total spread change in
    /// pixels) before niri classifies the gesture as a pinch rather than a
    /// swipe. Default: 30.0.
    #[knuffel(child, unwrap(argument))]
    pub pinch_threshold: Option<f64>,
    /// Ratio by which spread change must exceed linear swipe distance for a
    /// gesture to count as a pinch. Higher values make pinch detection stricter
    /// — the fingers really have to move apart/together rather than glide
    /// across the screen. Default: 2.0.
    #[knuffel(child, unwrap(argument))]
    pub pinch_ratio: Option<f64>,
    /// Multiplier mapping finger spread change (in screen pixels) to
    /// continuous pinch animation delta. Applies to all pinch-bound
    /// continuous actions — the bind's own `sensitivity` property is
    /// ignored for pinch, since raw spread-delta pixels need very
    /// different scaling from linear swipe distances. At 1.0, one pixel
    /// of spread change contributes one pixel to the underlying gesture
    /// accumulator (same scale swipes use). Default: 1.0.
    #[knuffel(child, unwrap(argument))]
    pub pinch_sensitivity: Option<f64>,
    /// Scaling applied to `recognition_threshold` for gestures with more than
    /// 3 fingers. The formula is `base * (1 + (fingers - 3) * (scale - 1))`,
    /// so with a base threshold of 16 and scale 1.5, a 4-finger gesture needs
    /// 24 px and a 5-finger gesture needs 32 px. Compensates for the extra
    /// movement spread that wider finger grips produce. Default: 1.5.
    #[knuffel(child, unwrap(argument))]
    pub finger_threshold_scale: Option<f64>,
    /// Pixels of finger movement for IPC `GestureProgress` events to reach
    /// `progress = 1.0`. Units are screen pixels. Tune this to make tagged
    /// external-app gestures (like sidebar drawers) feel right on your
    /// display. Default: 200.0.
    #[knuffel(child, unwrap(argument))]
    pub gesture_progress_distance: Option<f64>,
    /// Pixels of finger spread change for IPC `GestureProgress` events on
    /// pinch gestures to reach `progress = ±1.0`. Signed: positive for
    /// pinch-out (spread growing), negative for pinch-in (spread shrinking).
    /// Pinch spread changes are usually smaller than linear swipe distances,
    /// so this defaults lower than `gesture_progress_distance`. Default: 100.0.
    #[knuffel(child, unwrap(argument))]
    pub pinch_progress_distance: Option<f64>,
    /// Minimum rotation (in **radians**) before a multi-finger gesture is
    /// classified as a rotation rather than a swipe or pinch. Configured in
    /// radians so the config is unit-explicit; divide by π and multiply by
    /// 180 to get degrees. Default: ~0.2618 rad (15°).
    #[knuffel(child, unwrap(argument))]
    pub rotation_threshold: Option<f64>,
    /// Ratio by which the rotation arc length (cumulative rotation times
    /// cluster radius) must exceed linear swipe distance and spread change
    /// for a gesture to count as a rotation. Higher values make rotation
    /// detection stricter. Default: 0.5.
    #[knuffel(child, unwrap(argument))]
    pub rotation_ratio: Option<f64>,
    /// Radians of cumulative rotation for IPC `GestureProgress` events on
    /// rotation gestures to reach `progress = ±1.0`. Signed: positive for
    /// counter-clockwise, negative for clockwise. Default: ~1.5708 rad (π/2).
    #[knuffel(child, unwrap(argument))]
    pub rotation_progress_distance: Option<f64>,
}

/// Tuning parameters for touchpad gesture recognition.
///
/// The actual gesture binds (e.g. `TouchpadSwipe fingers=3 direction="up"`)
/// live in the main `binds {}` block — this struct only controls how
/// movement is classified and how IPC progress is reported.
#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct TouchpadGesturesConfig {
    /// Distance in libinput delta units that fingers must move before a swipe
    /// gesture is recognized. These units are acceleration-adjusted and not
    /// directly comparable to touchscreen pixels. Default: 16.0.
    #[knuffel(child, unwrap(argument))]
    pub recognition_threshold: Option<f64>,
    /// Libinput delta units of finger movement for IPC `GestureProgress`
    /// events to reach `progress = 1.0`. Because libinput acceleration curves
    /// are nonlinear, the same physical swipe can produce different delta
    /// magnitudes depending on speed — this value is not directly comparable
    /// to the touchscreen `gesture-progress-distance`. Default: 40.0.
    #[knuffel(child, unwrap(argument))]
    pub gesture_progress_distance: Option<f64>,
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
