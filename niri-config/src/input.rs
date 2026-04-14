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
    /// Swipe commit gate in libinput delta units (from
    /// `swipe-trigger-distance`). Default 16.
    pub fn swipe_trigger_distance(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.swipe_trigger_distance)
            .unwrap_or(16.0)
    }

    /// Libinput delta units of swipe motion that map to IPC
    /// `GestureProgress = 1.0`. Default 40.
    pub fn swipe_progress_distance(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.swipe_progress_distance)
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
    /// Swipe commit gate: centroid must travel this many pixels before a
    /// swipe can latch. Default 100.
    pub fn swipe_trigger_distance(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.swipe_trigger_distance)
            .unwrap_or(100.0)
    }

    /// Width (in pixels) of the screen-edge start zone within which a
    /// touch must begin to count as a `TouchEdge`. Default 12.
    pub fn edge_start_distance(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.edge_start_distance)
            .unwrap_or(12.0)
    }

    /// Pinch commit gate: `|spread_change|` must exceed this many pixels
    /// before a pinch can latch. Default 100.
    pub fn pinch_trigger_distance(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.pinch_trigger_distance)
            .unwrap_or(100.0)
    }

    /// Pinch dominance ratio: `|spread_change|` must exceed
    /// `swipe_distance × this` for pinch to win the race against swipe.
    /// Higher = stricter pinch. Default 1.0.
    pub fn pinch_dominance_ratio(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.pinch_dominance_ratio)
            .unwrap_or(1.0)
    }

    pub fn pinch_sensitivity(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.pinch_sensitivity)
            .unwrap_or(1.0)
    }

    /// Multi-finger scaling applied to `swipe_trigger_distance` for
    /// gestures with more than 3 fingers. Default 1.2 — gives a small
    /// pinch-priority bias at high finger counts (4/5-finger swipes need
    /// slightly more commitment, so ambiguous pinches usually win).
    pub fn swipe_multi_finger_scale(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.swipe_multi_finger_scale)
            .unwrap_or(1.2)
    }

    /// Pixels of swipe distance that map to IPC `GestureProgress = 1.0`.
    /// Default 200.
    pub fn swipe_progress_distance(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.swipe_progress_distance)
            .unwrap_or(200.0)
    }

    /// Pixels of spread change that map to IPC `GestureProgress = ±1.0`.
    /// Default 100.
    pub fn pinch_progress_distance(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.pinch_progress_distance)
            .unwrap_or(100.0)
    }

    /// Rotation commit gate: cumulative rotation must exceed this many
    /// **degrees** (in the KDL config — converted to radians internally)
    /// before a rotation can latch. Default 20°.
    pub fn rotation_trigger_angle(&self) -> f64 {
        let deg = self
            .gestures
            .as_ref()
            .and_then(|g| g.rotation_trigger_angle)
            .unwrap_or(20.0);
        deg.to_radians()
    }

    /// Rotation dominance ratio: `rotation_arc` must exceed both
    /// `swipe_distance × this` and `|spread_change| × this` for rotation
    /// to win the race. Higher = stricter rotation. **Matches
    /// `pinch_dominance_ratio` semantics** — both knobs read as
    /// "higher = stricter".
    ///
    /// Default 0.5 (arc must be ≥ 0.5 × competing motion). This is
    /// deliberately lenient because rotating a finger cluster almost
    /// always produces some incidental translation; requiring arc to
    /// *exceed* the translation (ratio ≥ 1.0) would reject most
    /// real-world rotations.
    pub fn rotation_dominance_ratio(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.rotation_dominance_ratio)
            .unwrap_or(0.5)
    }

    /// Degrees of rotation (in the KDL config — converted to radians
    /// internally) that map to IPC `GestureProgress = ±1.0` for rotation
    /// gestures. Default 90°.
    pub fn rotation_progress_angle(&self) -> f64 {
        let deg = self
            .gestures
            .as_ref()
            .and_then(|g| g.rotation_progress_angle)
            .unwrap_or(90.0);
        deg.to_radians()
    }

    /// Returns the swipe trigger distance scaled for a given finger
    /// count. Extra fingers above 3 increase the distance by the
    /// `swipe_multi_finger_scale` factor.
    pub fn scaled_swipe_trigger_distance(&self, finger_count: usize) -> f64 {
        let base = self.swipe_trigger_distance();
        let scale = self.swipe_multi_finger_scale();
        let extra = finger_count.saturating_sub(3) as f64;
        base * (1.0 + extra * (scale - 1.0))
    }

    /// Maximum per-finger displacement (px) before a tap candidate is
    /// killed. Default 15.
    pub fn tap_wobble_threshold(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.tap_wobble_threshold)
            .unwrap_or(15.0)
    }

    /// Maximum tap duration in milliseconds. Default 500.
    pub fn tap_timeout_ms(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.tap_timeout_ms)
            .unwrap_or(500.0)
    }

    /// Minimum hold duration (ms) before a wobble-kill can activate a
    /// TouchTapHoldDrag bind. Prevents fast swipes from accidentally
    /// triggering hold-drag. Default 200.
    pub fn tap_hold_trigger_delay_ms(&self) -> f64 {
        self.gestures
            .as_ref()
            .and_then(|g| g.tap_hold_trigger_delay_ms)
            .unwrap_or(200.0)
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
    /// Swipe commit gate: pixels of centroid movement required before a
    /// swipe gesture latches. Lower values feel more responsive but risk
    /// triggering on incidental finger drift. Default: 100.0.
    #[knuffel(child, unwrap(argument))]
    pub swipe_trigger_distance: Option<f64>,
    /// Width (in pixels) of the screen-edge start zone. A touch must
    /// *begin* within this distance from an edge for it to count as a
    /// `TouchEdge edge="..."` gesture; touches starting farther in are
    /// treated as regular swipes. Default: 12.0.
    #[knuffel(child, unwrap(argument))]
    pub edge_start_distance: Option<f64>,
    /// Pinch commit gate: pixels of `|spread_change|` required before a
    /// pinch gesture latches. Default: 100.0.
    #[knuffel(child, unwrap(argument))]
    pub pinch_trigger_distance: Option<f64>,
    /// Pinch dominance ratio: `|spread_change|` must exceed
    /// `swipe_distance × this` for pinch to beat swipe in the race.
    /// Higher values make pinch stricter — the fingers really have to
    /// move apart/together rather than glide across the screen.
    /// Default: 1.0.
    #[knuffel(child, unwrap(argument))]
    pub pinch_dominance_ratio: Option<f64>,
    /// Multiplier mapping finger spread change (in screen pixels) to
    /// continuous pinch animation delta. Applies to all pinch-bound
    /// continuous actions — the bind's own `sensitivity` property is
    /// ignored for pinch, since raw spread-delta pixels need very
    /// different scaling from linear swipe distances. At 1.0, one pixel
    /// of spread change contributes one pixel to the underlying gesture
    /// accumulator (same scale swipes use). Default: 1.0.
    #[knuffel(child, unwrap(argument))]
    pub pinch_sensitivity: Option<f64>,
    /// Scaling applied to `swipe_trigger_distance` for gestures with
    /// more than 3 fingers. The formula is
    /// `base * (1 + (fingers − 3) * (scale − 1))`, so with a base of 100
    /// and scale 1.2 a 4-finger swipe needs 120 px and a 5-finger swipe
    /// needs 140 px. Default 1.2 — gives a small pinch-priority bias at
    /// high finger counts so ambiguous 4/5-finger motions resolve as
    /// pinch rather than swipe. Set 1.0 to disable the bias entirely.
    #[knuffel(child, unwrap(argument))]
    pub swipe_multi_finger_scale: Option<f64>,
    /// Pixels of swipe distance that map to IPC `GestureProgress = 1.0`.
    /// IPC-only output knob — doesn't affect classification. Tune this
    /// to make tagged external-app gestures (sidebar drawers etc.) feel
    /// right on your display. Default: 200.0.
    #[knuffel(child, unwrap(argument))]
    pub swipe_progress_distance: Option<f64>,
    /// Pixels of spread change that map to IPC
    /// `GestureProgress = ±1.0` for pinch gestures. Signed: positive for
    /// pinch-out (spread growing), negative for pinch-in (spread
    /// shrinking). Default: 100.0.
    #[knuffel(child, unwrap(argument))]
    pub pinch_progress_distance: Option<f64>,
    /// Rotation commit gate: cumulative rotation (in **degrees**)
    /// required before a rotation gesture latches. Converted to radians
    /// internally. Default: 20°.
    #[knuffel(child, unwrap(argument))]
    pub rotation_trigger_angle: Option<f64>,
    /// Rotation dominance ratio: `rotation_arc` must exceed
    /// `swipe_distance × this` AND `|spread_change| × this` for rotation
    /// to beat swipe and pinch in the race. Higher = stricter, matching
    /// `pinch_dominance_ratio` semantics. Default: 0.5 (deliberately
    /// lenient — rotation almost always includes incidental translation,
    /// so requiring arc to strictly exceed translation would reject
    /// nearly all real-world rotations).
    #[knuffel(child, unwrap(argument))]
    pub rotation_dominance_ratio: Option<f64>,
    /// Degrees of cumulative rotation that map to IPC
    /// `GestureProgress = ±1.0` for rotation gestures. Signed: positive
    /// for counter-clockwise, negative for clockwise. Default: 90°.
    #[knuffel(child, unwrap(argument))]
    pub rotation_progress_angle: Option<f64>,
    /// Maximum per-finger displacement (in pixels) allowed during a tap
    /// gesture. If any single finger moves more than this distance from
    /// its initial landing position, the tap candidate is killed and the
    /// gesture can only resolve as swipe/pinch/rotate. Default: 15.0.
    #[knuffel(child, unwrap(argument))]
    pub tap_wobble_threshold: Option<f64>,
    /// Maximum duration (in milliseconds) from the third finger landing
    /// to all fingers lifting for a tap to fire. Taps slower than this
    /// are discarded — acts as a tap-vs-hold safety cap. Default: 500.
    #[knuffel(child, unwrap(argument))]
    pub tap_timeout_ms: Option<f64>,
    /// Minimum hold duration (in milliseconds) before a wobble-kill can
    /// activate a `TouchTapHoldDrag` bind. If fingers move before this
    /// delay elapses, normal swipe/pinch/rotate recognition continues
    /// instead. Prevents fast swipes from accidentally triggering
    /// hold-drag. Default: 200.
    #[knuffel(child, unwrap(argument))]
    pub tap_hold_trigger_delay_ms: Option<f64>,
}

/// Tuning parameters for touchpad gesture recognition.
///
/// The actual gesture binds (e.g. `TouchpadSwipe fingers=3 direction="up"`)
/// live in the main `binds {}` block — this struct only controls how
/// movement is classified and how IPC progress is reported.
#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct TouchpadGesturesConfig {
    /// Swipe commit gate: libinput delta units of centroid motion before
    /// a swipe gesture latches. These units are acceleration-adjusted
    /// and not directly comparable to touchscreen pixels. Default: 16.0.
    #[knuffel(child, unwrap(argument))]
    pub swipe_trigger_distance: Option<f64>,
    /// Libinput delta units of swipe movement that map to IPC
    /// `GestureProgress = 1.0`. Because libinput acceleration curves are
    /// nonlinear, the same physical swipe can produce different delta
    /// magnitudes depending on speed — this value is not directly
    /// comparable to the touchscreen `swipe-progress-distance`.
    /// Default: 40.0.
    #[knuffel(child, unwrap(argument))]
    pub swipe_progress_distance: Option<f64>,
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
