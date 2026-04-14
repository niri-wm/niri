use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

use bitflags::bitflags;
use knuffel::errors::DecodeError;
use miette::miette;
use niri_ipc::{
    ColumnDisplay, LayoutSwitchTarget, PositionChange, SizeChange, WorkspaceReferenceArg,
};
use smithay::input::keyboard::keysyms::KEY_NoSymbol;
use smithay::input::keyboard::xkb::{keysym_from_name, KEYSYM_CASE_INSENSITIVE, KEYSYM_NO_FLAGS};
use smithay::input::keyboard::Keysym;

use crate::input::{EdgeZone, ScreenEdge};
use crate::recent_windows::{MruDirection, MruFilter, MruScope};
use crate::utils::{expect_only_children, MergeWith};

/// Direction for a linear swipe gesture.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum SwipeDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Direction for a pinch gesture.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum PinchDirection {
    In,
    Out,
}

/// Direction for a rotation gesture (as seen on screen).
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum RotateDirection {
    /// Clockwise on screen.
    Cw,
    /// Counter-clockwise on screen.
    Ccw,
}

/// Inclusive bounds on the `fingers=` property for multi-finger gestures.
/// Parser rejects `fingers` values outside `[MIN_FINGERS, MAX_FINGERS]`.
/// `< 3` would collide with two-finger passthrough (scroll/zoom) and plain
/// single-finger touch handling; `> 10` exceeds any realistic hardware.
pub const MIN_FINGERS: u8 = 3;
pub const MAX_FINGERS: u8 = 10;

#[derive(Debug, Default, PartialEq)]
pub struct Binds(pub Vec<Bind>);

#[derive(Debug, Clone, PartialEq)]
pub struct Bind {
    pub key: Key,
    pub action: Action,
    pub repeat: bool,
    pub cooldown: Option<Duration>,
    pub allow_when_locked: bool,
    pub allow_inhibiting: bool,
    pub hotkey_overlay_title: Option<Option<String>>,
    /// Sensitivity multiplier for touch gesture binds.
    pub sensitivity: Option<f64>,
    /// Natural scroll for touchscreen gesture binds.
    pub natural_scroll: bool,
    /// Optional tag for IPC gesture events.
    /// When set, gesture begin/progress/end events are emitted on the IPC
    /// event stream with this tag, allowing external tools to react.
    /// Restricted to gesture triggers only (Touch*/Touchpad*) — rejected
    /// on keyboard/mouse binds to prevent IPC event stream keylogging.
    pub tag: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct Key {
    pub trigger: Trigger,
    pub modifiers: Modifiers,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Trigger {
    Keysym(Keysym),
    MouseLeft,
    MouseRight,
    MouseMiddle,
    MouseBack,
    MouseForward,
    WheelScrollDown,
    WheelScrollUp,
    WheelScrollLeft,
    WheelScrollRight,
    TouchpadScrollDown,
    TouchpadScrollUp,
    TouchpadScrollLeft,
    TouchpadScrollRight,
    /// Multi-finger touchpad swipe.
    ///
    /// KDL syntax: `TouchpadSwipe fingers=3 direction="up"`. `fingers` must
    /// be in `MIN_FINGERS..=MAX_FINGERS`.
    TouchpadSwipe {
        fingers: u8,
        direction: SwipeDirection,
    },
    /// Multi-finger touchpad tap-hold (fingers land, hold stationary,
    /// then lift). libinput handles motion discrimination via its hold
    /// gesture API — `cancelled=false` on `GestureHoldEnd` means the
    /// fingers never moved. Fires on release. Fast taps that lift before
    /// libinput's hold threshold are not intercepted and pass through to
    /// clients. Always discrete (fire-and-forget).
    ///
    /// KDL syntax: `TouchpadTapHold fingers=3`.
    TouchpadTapHold {
        fingers: u8,
    },
    /// Multi-finger touchpad tap-hold-drag (fingers land, hold stationary,
    /// then start moving). Fires when the held fingers begin moving —
    /// libinput transitions from `GestureHold` to `GestureSwipe`.
    /// Can drive continuous actions (workspace switch, overview, window
    /// move) or fire a discrete action once on activation.
    ///
    /// KDL syntax: `TouchpadTapHoldDrag fingers=3`.
    TouchpadTapHoldDrag {
        fingers: u8,
    },
    /// Multi-finger touchscreen swipe.
    ///
    /// KDL syntax: `TouchSwipe fingers=3 direction="up"`.
    TouchSwipe {
        fingers: u8,
        direction: SwipeDirection,
    },
    /// Multi-finger touchscreen pinch (fingers converging / diverging
    /// around the cluster centroid).
    ///
    /// KDL syntax: `TouchPinch fingers=3 direction="in"`.
    TouchPinch {
        fingers: u8,
        direction: PinchDirection,
    },
    /// Multi-finger touchscreen rotation (fingers twisting as a group around
    /// the cluster centroid). Rotation starts at 3 fingers to preserve the
    /// 2-finger passthrough contract used by clients for scrolling/zooming.
    ///
    /// KDL syntax: `TouchRotate fingers=3 direction="cw"`.
    TouchRotate {
        fingers: u8,
        direction: RotateDirection,
    },
    /// Multi-finger touchscreen tap (all fingers land and lift with minimal
    /// motion). Runs in parallel with swipe/pinch/rotate recognition — if
    /// motion exceeds `tap-wobble-threshold` or the recognizer locks, the
    /// tap candidate is killed. Always discrete (fire-and-forget).
    ///
    /// KDL syntax: `TouchTap fingers=3`.
    TouchTap {
        fingers: u8,
    },
    /// Multi-finger touchscreen tap-hold-drag (fingers land, hold
    /// stationary within wobble threshold, then start moving). Fires at
    /// the wobble-kill moment — the transition from "was a tap candidate"
    /// to "started moving." Optional `direction` restricts to a specific
    /// initial movement direction; `None` = omnidirectional (fires
    /// regardless of direction). Can drive continuous actions.
    ///
    /// KDL syntax:
    /// - `TouchTapHoldDrag fingers=3` (omnidirectional)
    /// - `TouchTapHoldDrag fingers=3 direction="left"` (directional)
    TouchTapHoldDrag {
        fingers: u8,
        direction: Option<SwipeDirection>,
    },
    /// Single-finger touchscreen edge swipe.
    ///
    /// `zone` picks one of the three zones along the edge's perpendicular
    /// axis; `None` is the parent/any-zone fallback. At bind lookup time a
    /// zoned trigger is preferred, with `zone: None` as a fallback.
    ///
    /// KDL syntax:
    /// - `TouchEdge edge="left"` (parent)
    /// - `TouchEdge edge="left" zone="top"` (zoned)
    ///
    /// Top/Bottom edges accept `zone="left"|"center"|"right"`; Left/Right
    /// edges accept `zone="top"|"center"|"bottom"`.
    TouchEdge {
        edge: ScreenEdge,
        zone: Option<EdgeZone>,
    },
}

impl Trigger {
    /// Returns true if this trigger is a gesture (touchscreen or touchpad).
    /// Only gesture triggers support IPC tag events.
    pub fn is_gesture(&self) -> bool {
        matches!(
            self,
            Trigger::TouchpadSwipe { .. }
                | Trigger::TouchpadTapHold { .. }
                | Trigger::TouchpadTapHoldDrag { .. }
                | Trigger::TouchSwipe { .. }
                | Trigger::TouchPinch { .. }
                | Trigger::TouchRotate { .. }
                | Trigger::TouchTap { .. }
                | Trigger::TouchTapHoldDrag { .. }
                | Trigger::TouchEdge { .. }
        )
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Modifiers : u8 {
        const CTRL = 1;
        const SHIFT = 1 << 1;
        const ALT = 1 << 2;
        const SUPER = 1 << 3;
        const ISO_LEVEL3_SHIFT = 1 << 4;
        const ISO_LEVEL5_SHIFT = 1 << 5;
        const COMPOSITOR = 1 << 6;
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct SwitchBinds {
    #[knuffel(child)]
    pub lid_open: Option<SwitchAction>,
    #[knuffel(child)]
    pub lid_close: Option<SwitchAction>,
    #[knuffel(child)]
    pub tablet_mode_on: Option<SwitchAction>,
    #[knuffel(child)]
    pub tablet_mode_off: Option<SwitchAction>,
}

impl MergeWith<SwitchBinds> for SwitchBinds {
    fn merge_with(&mut self, part: &SwitchBinds) {
        merge_clone_opt!(
            (self, part),
            lid_open,
            lid_close,
            tablet_mode_on,
            tablet_mode_off,
        );
    }
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub struct SwitchAction {
    #[knuffel(child, unwrap(arguments))]
    pub spawn: Vec<String>,
}

// Remember to add new actions to the CLI enum too.
#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub enum Action {
    Quit(#[knuffel(property(name = "skip-confirmation"), default)] bool),
    #[knuffel(skip)]
    ChangeVt(i32),
    Suspend,
    PowerOffMonitors,
    PowerOnMonitors,
    ToggleDebugTint,
    DebugToggleOpaqueRegions,
    DebugToggleDamage,
    Spawn(#[knuffel(arguments)] Vec<String>),
    SpawnSh(#[knuffel(argument)] String),
    DoScreenTransition(#[knuffel(property(name = "delay-ms"))] Option<u16>),
    #[knuffel(skip)]
    ConfirmScreenshot {
        write_to_disk: bool,
    },
    #[knuffel(skip)]
    CancelScreenshot,
    #[knuffel(skip)]
    ScreenshotTogglePointer,
    Screenshot(
        #[knuffel(property(name = "show-pointer"), default = true)] bool,
        // Path; not settable from knuffel
        Option<String>,
    ),
    ScreenshotScreen(
        #[knuffel(property(name = "write-to-disk"), default = true)] bool,
        #[knuffel(property(name = "show-pointer"), default = true)] bool,
        // Path; not settable from knuffel
        Option<String>,
    ),
    ScreenshotWindow(
        #[knuffel(property(name = "write-to-disk"), default = true)] bool,
        #[knuffel(property(name = "show-pointer"), default = false)] bool,
        // Path; not settable from knuffel
        Option<String>,
    ),
    #[knuffel(skip)]
    ScreenshotWindowById {
        id: u64,
        write_to_disk: bool,
        show_pointer: bool,
        path: Option<String>,
    },
    ToggleKeyboardShortcutsInhibit,
    CloseWindow,
    #[knuffel(skip)]
    CloseWindowById(u64),
    FullscreenWindow,
    #[knuffel(skip)]
    FullscreenWindowById(u64),
    ToggleWindowedFullscreen,
    #[knuffel(skip)]
    ToggleWindowedFullscreenById(u64),
    #[knuffel(skip)]
    FocusWindow(u64),
    FocusWindowInColumn(#[knuffel(argument)] u8),
    FocusWindowPrevious,
    FocusColumnLeft,
    #[knuffel(skip)]
    FocusColumnLeftUnderMouse,
    FocusColumnRight,
    #[knuffel(skip)]
    FocusColumnRightUnderMouse,
    FocusColumnFirst,
    FocusColumnLast,
    FocusColumnRightOrFirst,
    FocusColumnLeftOrLast,
    FocusColumn(#[knuffel(argument)] usize),
    FocusWindowOrMonitorUp,
    FocusWindowOrMonitorDown,
    FocusColumnOrMonitorLeft,
    FocusColumnOrMonitorRight,
    FocusWindowDown,
    FocusWindowUp,
    FocusWindowDownOrColumnLeft,
    FocusWindowDownOrColumnRight,
    FocusWindowUpOrColumnLeft,
    FocusWindowUpOrColumnRight,
    FocusWindowOrWorkspaceDown,
    FocusWindowOrWorkspaceUp,
    FocusWindowTop,
    FocusWindowBottom,
    FocusWindowDownOrTop,
    FocusWindowUpOrBottom,
    MoveColumnLeft,
    MoveColumnRight,
    MoveColumnToFirst,
    MoveColumnToLast,
    MoveColumnLeftOrToMonitorLeft,
    MoveColumnRightOrToMonitorRight,
    MoveColumnToIndex(#[knuffel(argument)] usize),
    MoveWindowDown,
    MoveWindowUp,
    MoveWindowDownOrToWorkspaceDown,
    MoveWindowUpOrToWorkspaceUp,
    ConsumeOrExpelWindowLeft,
    #[knuffel(skip)]
    ConsumeOrExpelWindowLeftById(u64),
    ConsumeOrExpelWindowRight,
    #[knuffel(skip)]
    ConsumeOrExpelWindowRightById(u64),
    ConsumeWindowIntoColumn,
    ExpelWindowFromColumn,
    SwapWindowLeft,
    SwapWindowRight,
    ToggleColumnTabbedDisplay,
    SetColumnDisplay(#[knuffel(argument, str)] ColumnDisplay),
    CenterColumn,
    CenterWindow,
    #[knuffel(skip)]
    CenterWindowById(u64),
    CenterVisibleColumns,
    FocusWorkspaceDown,
    #[knuffel(skip)]
    FocusWorkspaceDownUnderMouse,
    FocusWorkspaceUp,
    #[knuffel(skip)]
    FocusWorkspaceUpUnderMouse,
    FocusWorkspace(#[knuffel(argument)] WorkspaceReference),
    FocusWorkspacePrevious,
    MoveWindowToWorkspaceDown(#[knuffel(property(name = "focus"), default = true)] bool),
    MoveWindowToWorkspaceUp(#[knuffel(property(name = "focus"), default = true)] bool),
    MoveWindowToWorkspace(
        #[knuffel(argument)] WorkspaceReference,
        #[knuffel(property(name = "focus"), default = true)] bool,
    ),
    #[knuffel(skip)]
    MoveWindowToWorkspaceById {
        window_id: u64,
        reference: WorkspaceReference,
        focus: bool,
    },
    MoveColumnToWorkspaceDown(#[knuffel(property(name = "focus"), default = true)] bool),
    MoveColumnToWorkspaceUp(#[knuffel(property(name = "focus"), default = true)] bool),
    MoveColumnToWorkspace(
        #[knuffel(argument)] WorkspaceReference,
        #[knuffel(property(name = "focus"), default = true)] bool,
    ),
    MoveWorkspaceDown,
    MoveWorkspaceUp,
    MoveWorkspaceToIndex(#[knuffel(argument)] usize),
    #[knuffel(skip)]
    MoveWorkspaceToIndexByRef {
        new_idx: usize,
        reference: WorkspaceReference,
    },
    #[knuffel(skip)]
    MoveWorkspaceToMonitorByRef {
        output_name: String,
        reference: WorkspaceReference,
    },
    MoveWorkspaceToMonitor(#[knuffel(argument)] String),
    SetWorkspaceName(#[knuffel(argument)] String),
    #[knuffel(skip)]
    SetWorkspaceNameByRef {
        name: String,
        reference: WorkspaceReference,
    },
    UnsetWorkspaceName,
    #[knuffel(skip)]
    UnsetWorkSpaceNameByRef(#[knuffel(argument)] WorkspaceReference),
    FocusMonitorLeft,
    FocusMonitorRight,
    FocusMonitorDown,
    FocusMonitorUp,
    FocusMonitorPrevious,
    FocusMonitorNext,
    FocusMonitor(#[knuffel(argument)] String),
    MoveWindowToMonitorLeft,
    MoveWindowToMonitorRight,
    MoveWindowToMonitorDown,
    MoveWindowToMonitorUp,
    MoveWindowToMonitorPrevious,
    MoveWindowToMonitorNext,
    MoveWindowToMonitor(#[knuffel(argument)] String),
    #[knuffel(skip)]
    MoveWindowToMonitorById {
        id: u64,
        output: String,
    },
    MoveColumnToMonitorLeft,
    MoveColumnToMonitorRight,
    MoveColumnToMonitorDown,
    MoveColumnToMonitorUp,
    MoveColumnToMonitorPrevious,
    MoveColumnToMonitorNext,
    MoveColumnToMonitor(#[knuffel(argument)] String),
    SetWindowWidth(#[knuffel(argument, str)] SizeChange),
    #[knuffel(skip)]
    SetWindowWidthById {
        id: u64,
        change: SizeChange,
    },
    SetWindowHeight(#[knuffel(argument, str)] SizeChange),
    #[knuffel(skip)]
    SetWindowHeightById {
        id: u64,
        change: SizeChange,
    },
    ResetWindowHeight,
    #[knuffel(skip)]
    ResetWindowHeightById(u64),
    SwitchPresetColumnWidth,
    SwitchPresetColumnWidthBack,
    SwitchPresetWindowWidth,
    SwitchPresetWindowWidthBack,
    #[knuffel(skip)]
    SwitchPresetWindowWidthById(u64),
    #[knuffel(skip)]
    SwitchPresetWindowWidthBackById(u64),
    SwitchPresetWindowHeight,
    SwitchPresetWindowHeightBack,
    #[knuffel(skip)]
    SwitchPresetWindowHeightById(u64),
    #[knuffel(skip)]
    SwitchPresetWindowHeightBackById(u64),
    MaximizeColumn,
    MaximizeWindowToEdges,
    #[knuffel(skip)]
    MaximizeWindowToEdgesById(u64),
    SetColumnWidth(#[knuffel(argument, str)] SizeChange),
    ExpandColumnToAvailableWidth,
    SwitchLayout(#[knuffel(argument, str)] LayoutSwitchTarget),
    ShowHotkeyOverlay,
    MoveWorkspaceToMonitorLeft,
    MoveWorkspaceToMonitorRight,
    MoveWorkspaceToMonitorDown,
    MoveWorkspaceToMonitorUp,
    MoveWorkspaceToMonitorPrevious,
    MoveWorkspaceToMonitorNext,
    ToggleWindowFloating,
    #[knuffel(skip)]
    ToggleWindowFloatingById(u64),
    MoveWindowToFloating,
    #[knuffel(skip)]
    MoveWindowToFloatingById(u64),
    MoveWindowToTiling,
    #[knuffel(skip)]
    MoveWindowToTilingById(u64),
    FocusFloating,
    FocusTiling,
    SwitchFocusBetweenFloatingAndTiling,
    #[knuffel(skip)]
    MoveFloatingWindowById {
        id: Option<u64>,
        x: PositionChange,
        y: PositionChange,
    },
    ToggleWindowRuleOpacity,
    #[knuffel(skip)]
    ToggleWindowRuleOpacityById(u64),
    SetDynamicCastWindow,
    #[knuffel(skip)]
    SetDynamicCastWindowById(u64),
    SetDynamicCastMonitor(#[knuffel(argument)] Option<String>),
    ClearDynamicCastTarget,
    #[knuffel(skip)]
    StopCast(u64),
    ToggleOverview,
    OpenOverview,
    CloseOverview,
    #[knuffel(skip)]
    ToggleWindowUrgent(u64),
    #[knuffel(skip)]
    SetWindowUrgent(u64),
    #[knuffel(skip)]
    UnsetWindowUrgent(u64),
    #[knuffel(skip)]
    LoadConfigFile(#[knuffel(argument)] Option<String>),
    #[knuffel(skip)]
    MruAdvance {
        direction: MruDirection,
        scope: Option<MruScope>,
        filter: Option<MruFilter>,
    },
    #[knuffel(skip)]
    MruConfirm,
    #[knuffel(skip)]
    MruCancel,
    #[knuffel(skip)]
    MruCloseCurrentWindow,
    #[knuffel(skip)]
    MruFirst,
    #[knuffel(skip)]
    MruLast,
    #[knuffel(skip)]
    MruSetScope(MruScope),
    #[knuffel(skip)]
    MruCycleScope,
    /// No-op action: the bind matches and consumes the gesture but does
    /// nothing inside the compositor. Useful with `tag` to pipe gesture
    /// events to external tools via IPC without triggering any niri action.
    Noop,
}

impl From<niri_ipc::Action> for Action {
    fn from(value: niri_ipc::Action) -> Self {
        match value {
            niri_ipc::Action::Quit { skip_confirmation } => Self::Quit(skip_confirmation),
            niri_ipc::Action::PowerOffMonitors {} => Self::PowerOffMonitors,
            niri_ipc::Action::PowerOnMonitors {} => Self::PowerOnMonitors,
            niri_ipc::Action::Spawn { command } => Self::Spawn(command),
            niri_ipc::Action::SpawnSh { command } => Self::SpawnSh(command),
            niri_ipc::Action::DoScreenTransition { delay_ms } => Self::DoScreenTransition(delay_ms),
            niri_ipc::Action::Screenshot { show_pointer, path } => {
                Self::Screenshot(show_pointer, path)
            }
            niri_ipc::Action::ScreenshotScreen {
                write_to_disk,
                show_pointer,
                path,
            } => Self::ScreenshotScreen(write_to_disk, show_pointer, path),
            niri_ipc::Action::ScreenshotWindow {
                id: None,
                write_to_disk,
                show_pointer,
                path,
            } => Self::ScreenshotWindow(write_to_disk, show_pointer, path),
            niri_ipc::Action::ScreenshotWindow {
                id: Some(id),
                write_to_disk,
                show_pointer,
                path,
            } => Self::ScreenshotWindowById {
                id,
                write_to_disk,
                show_pointer,
                path,
            },
            niri_ipc::Action::ToggleKeyboardShortcutsInhibit {} => {
                Self::ToggleKeyboardShortcutsInhibit
            }
            niri_ipc::Action::CloseWindow { id: None } => Self::CloseWindow,
            niri_ipc::Action::CloseWindow { id: Some(id) } => Self::CloseWindowById(id),
            niri_ipc::Action::FullscreenWindow { id: None } => Self::FullscreenWindow,
            niri_ipc::Action::FullscreenWindow { id: Some(id) } => Self::FullscreenWindowById(id),
            niri_ipc::Action::ToggleWindowedFullscreen { id: None } => {
                Self::ToggleWindowedFullscreen
            }
            niri_ipc::Action::ToggleWindowedFullscreen { id: Some(id) } => {
                Self::ToggleWindowedFullscreenById(id)
            }
            niri_ipc::Action::FocusWindow { id } => Self::FocusWindow(id),
            niri_ipc::Action::FocusWindowInColumn { index } => Self::FocusWindowInColumn(index),
            niri_ipc::Action::FocusWindowPrevious {} => Self::FocusWindowPrevious,
            niri_ipc::Action::FocusColumnLeft {} => Self::FocusColumnLeft,
            niri_ipc::Action::FocusColumnRight {} => Self::FocusColumnRight,
            niri_ipc::Action::FocusColumnFirst {} => Self::FocusColumnFirst,
            niri_ipc::Action::FocusColumnLast {} => Self::FocusColumnLast,
            niri_ipc::Action::FocusColumnRightOrFirst {} => Self::FocusColumnRightOrFirst,
            niri_ipc::Action::FocusColumnLeftOrLast {} => Self::FocusColumnLeftOrLast,
            niri_ipc::Action::FocusColumn { index } => Self::FocusColumn(index),
            niri_ipc::Action::FocusWindowOrMonitorUp {} => Self::FocusWindowOrMonitorUp,
            niri_ipc::Action::FocusWindowOrMonitorDown {} => Self::FocusWindowOrMonitorDown,
            niri_ipc::Action::FocusColumnOrMonitorLeft {} => Self::FocusColumnOrMonitorLeft,
            niri_ipc::Action::FocusColumnOrMonitorRight {} => Self::FocusColumnOrMonitorRight,
            niri_ipc::Action::FocusWindowDown {} => Self::FocusWindowDown,
            niri_ipc::Action::FocusWindowUp {} => Self::FocusWindowUp,
            niri_ipc::Action::FocusWindowDownOrColumnLeft {} => Self::FocusWindowDownOrColumnLeft,
            niri_ipc::Action::FocusWindowDownOrColumnRight {} => Self::FocusWindowDownOrColumnRight,
            niri_ipc::Action::FocusWindowUpOrColumnLeft {} => Self::FocusWindowUpOrColumnLeft,
            niri_ipc::Action::FocusWindowUpOrColumnRight {} => Self::FocusWindowUpOrColumnRight,
            niri_ipc::Action::FocusWindowOrWorkspaceDown {} => Self::FocusWindowOrWorkspaceDown,
            niri_ipc::Action::FocusWindowOrWorkspaceUp {} => Self::FocusWindowOrWorkspaceUp,
            niri_ipc::Action::FocusWindowTop {} => Self::FocusWindowTop,
            niri_ipc::Action::FocusWindowBottom {} => Self::FocusWindowBottom,
            niri_ipc::Action::FocusWindowDownOrTop {} => Self::FocusWindowDownOrTop,
            niri_ipc::Action::FocusWindowUpOrBottom {} => Self::FocusWindowUpOrBottom,
            niri_ipc::Action::MoveColumnLeft {} => Self::MoveColumnLeft,
            niri_ipc::Action::MoveColumnRight {} => Self::MoveColumnRight,
            niri_ipc::Action::MoveColumnToFirst {} => Self::MoveColumnToFirst,
            niri_ipc::Action::MoveColumnToLast {} => Self::MoveColumnToLast,
            niri_ipc::Action::MoveColumnToIndex { index } => Self::MoveColumnToIndex(index),
            niri_ipc::Action::MoveColumnLeftOrToMonitorLeft {} => {
                Self::MoveColumnLeftOrToMonitorLeft
            }
            niri_ipc::Action::MoveColumnRightOrToMonitorRight {} => {
                Self::MoveColumnRightOrToMonitorRight
            }
            niri_ipc::Action::MoveWindowDown {} => Self::MoveWindowDown,
            niri_ipc::Action::MoveWindowUp {} => Self::MoveWindowUp,
            niri_ipc::Action::MoveWindowDownOrToWorkspaceDown {} => {
                Self::MoveWindowDownOrToWorkspaceDown
            }
            niri_ipc::Action::MoveWindowUpOrToWorkspaceUp {} => Self::MoveWindowUpOrToWorkspaceUp,
            niri_ipc::Action::ConsumeOrExpelWindowLeft { id: None } => {
                Self::ConsumeOrExpelWindowLeft
            }
            niri_ipc::Action::ConsumeOrExpelWindowLeft { id: Some(id) } => {
                Self::ConsumeOrExpelWindowLeftById(id)
            }
            niri_ipc::Action::ConsumeOrExpelWindowRight { id: None } => {
                Self::ConsumeOrExpelWindowRight
            }
            niri_ipc::Action::ConsumeOrExpelWindowRight { id: Some(id) } => {
                Self::ConsumeOrExpelWindowRightById(id)
            }
            niri_ipc::Action::ConsumeWindowIntoColumn {} => Self::ConsumeWindowIntoColumn,
            niri_ipc::Action::ExpelWindowFromColumn {} => Self::ExpelWindowFromColumn,
            niri_ipc::Action::SwapWindowRight {} => Self::SwapWindowRight,
            niri_ipc::Action::SwapWindowLeft {} => Self::SwapWindowLeft,
            niri_ipc::Action::ToggleColumnTabbedDisplay {} => Self::ToggleColumnTabbedDisplay,
            niri_ipc::Action::SetColumnDisplay { display } => Self::SetColumnDisplay(display),
            niri_ipc::Action::CenterColumn {} => Self::CenterColumn,
            niri_ipc::Action::CenterWindow { id: None } => Self::CenterWindow,
            niri_ipc::Action::CenterWindow { id: Some(id) } => Self::CenterWindowById(id),
            niri_ipc::Action::CenterVisibleColumns {} => Self::CenterVisibleColumns,
            niri_ipc::Action::FocusWorkspaceDown {} => Self::FocusWorkspaceDown,
            niri_ipc::Action::FocusWorkspaceUp {} => Self::FocusWorkspaceUp,
            niri_ipc::Action::FocusWorkspace { reference } => {
                Self::FocusWorkspace(WorkspaceReference::from(reference))
            }
            niri_ipc::Action::FocusWorkspacePrevious {} => Self::FocusWorkspacePrevious,
            niri_ipc::Action::MoveWindowToWorkspaceDown { focus } => {
                Self::MoveWindowToWorkspaceDown(focus)
            }
            niri_ipc::Action::MoveWindowToWorkspaceUp { focus } => {
                Self::MoveWindowToWorkspaceUp(focus)
            }
            niri_ipc::Action::MoveWindowToWorkspace {
                window_id: None,
                reference,
                focus,
            } => Self::MoveWindowToWorkspace(WorkspaceReference::from(reference), focus),
            niri_ipc::Action::MoveWindowToWorkspace {
                window_id: Some(window_id),
                reference,
                focus,
            } => Self::MoveWindowToWorkspaceById {
                window_id,
                reference: WorkspaceReference::from(reference),
                focus,
            },
            niri_ipc::Action::MoveColumnToWorkspaceDown { focus } => {
                Self::MoveColumnToWorkspaceDown(focus)
            }
            niri_ipc::Action::MoveColumnToWorkspaceUp { focus } => {
                Self::MoveColumnToWorkspaceUp(focus)
            }
            niri_ipc::Action::MoveColumnToWorkspace { reference, focus } => {
                Self::MoveColumnToWorkspace(WorkspaceReference::from(reference), focus)
            }
            niri_ipc::Action::MoveWorkspaceDown {} => Self::MoveWorkspaceDown,
            niri_ipc::Action::MoveWorkspaceUp {} => Self::MoveWorkspaceUp,
            niri_ipc::Action::SetWorkspaceName {
                name,
                workspace: None,
            } => Self::SetWorkspaceName(name),
            niri_ipc::Action::SetWorkspaceName {
                name,
                workspace: Some(reference),
            } => Self::SetWorkspaceNameByRef {
                name,
                reference: WorkspaceReference::from(reference),
            },
            niri_ipc::Action::UnsetWorkspaceName { reference: None } => Self::UnsetWorkspaceName,
            niri_ipc::Action::UnsetWorkspaceName {
                reference: Some(reference),
            } => Self::UnsetWorkSpaceNameByRef(WorkspaceReference::from(reference)),
            niri_ipc::Action::FocusMonitorLeft {} => Self::FocusMonitorLeft,
            niri_ipc::Action::FocusMonitorRight {} => Self::FocusMonitorRight,
            niri_ipc::Action::FocusMonitorDown {} => Self::FocusMonitorDown,
            niri_ipc::Action::FocusMonitorUp {} => Self::FocusMonitorUp,
            niri_ipc::Action::FocusMonitorPrevious {} => Self::FocusMonitorPrevious,
            niri_ipc::Action::FocusMonitorNext {} => Self::FocusMonitorNext,
            niri_ipc::Action::FocusMonitor { output } => Self::FocusMonitor(output),
            niri_ipc::Action::MoveWindowToMonitorLeft {} => Self::MoveWindowToMonitorLeft,
            niri_ipc::Action::MoveWindowToMonitorRight {} => Self::MoveWindowToMonitorRight,
            niri_ipc::Action::MoveWindowToMonitorDown {} => Self::MoveWindowToMonitorDown,
            niri_ipc::Action::MoveWindowToMonitorUp {} => Self::MoveWindowToMonitorUp,
            niri_ipc::Action::MoveWindowToMonitorPrevious {} => Self::MoveWindowToMonitorPrevious,
            niri_ipc::Action::MoveWindowToMonitorNext {} => Self::MoveWindowToMonitorNext,
            niri_ipc::Action::MoveWindowToMonitor { id: None, output } => {
                Self::MoveWindowToMonitor(output)
            }
            niri_ipc::Action::MoveWindowToMonitor {
                id: Some(id),
                output,
            } => Self::MoveWindowToMonitorById { id, output },
            niri_ipc::Action::MoveColumnToMonitorLeft {} => Self::MoveColumnToMonitorLeft,
            niri_ipc::Action::MoveColumnToMonitorRight {} => Self::MoveColumnToMonitorRight,
            niri_ipc::Action::MoveColumnToMonitorDown {} => Self::MoveColumnToMonitorDown,
            niri_ipc::Action::MoveColumnToMonitorUp {} => Self::MoveColumnToMonitorUp,
            niri_ipc::Action::MoveColumnToMonitorPrevious {} => Self::MoveColumnToMonitorPrevious,
            niri_ipc::Action::MoveColumnToMonitorNext {} => Self::MoveColumnToMonitorNext,
            niri_ipc::Action::MoveColumnToMonitor { output } => Self::MoveColumnToMonitor(output),
            niri_ipc::Action::SetWindowWidth { id: None, change } => Self::SetWindowWidth(change),
            niri_ipc::Action::SetWindowWidth {
                id: Some(id),
                change,
            } => Self::SetWindowWidthById { id, change },
            niri_ipc::Action::SetWindowHeight { id: None, change } => Self::SetWindowHeight(change),
            niri_ipc::Action::SetWindowHeight {
                id: Some(id),
                change,
            } => Self::SetWindowHeightById { id, change },
            niri_ipc::Action::ResetWindowHeight { id: None } => Self::ResetWindowHeight,
            niri_ipc::Action::ResetWindowHeight { id: Some(id) } => Self::ResetWindowHeightById(id),
            niri_ipc::Action::SwitchPresetColumnWidth {} => Self::SwitchPresetColumnWidth,
            niri_ipc::Action::SwitchPresetColumnWidthBack {} => Self::SwitchPresetColumnWidthBack,
            niri_ipc::Action::SwitchPresetWindowWidth { id: None } => Self::SwitchPresetWindowWidth,
            niri_ipc::Action::SwitchPresetWindowWidthBack { id: None } => {
                Self::SwitchPresetWindowWidthBack
            }
            niri_ipc::Action::SwitchPresetWindowWidth { id: Some(id) } => {
                Self::SwitchPresetWindowWidthById(id)
            }
            niri_ipc::Action::SwitchPresetWindowWidthBack { id: Some(id) } => {
                Self::SwitchPresetWindowWidthBackById(id)
            }
            niri_ipc::Action::SwitchPresetWindowHeight { id: None } => {
                Self::SwitchPresetWindowHeight
            }
            niri_ipc::Action::SwitchPresetWindowHeightBack { id: None } => {
                Self::SwitchPresetWindowHeightBack
            }
            niri_ipc::Action::SwitchPresetWindowHeight { id: Some(id) } => {
                Self::SwitchPresetWindowHeightById(id)
            }
            niri_ipc::Action::SwitchPresetWindowHeightBack { id: Some(id) } => {
                Self::SwitchPresetWindowHeightBackById(id)
            }
            niri_ipc::Action::MaximizeColumn {} => Self::MaximizeColumn,
            niri_ipc::Action::MaximizeWindowToEdges { id: None } => Self::MaximizeWindowToEdges,
            niri_ipc::Action::MaximizeWindowToEdges { id: Some(id) } => {
                Self::MaximizeWindowToEdgesById(id)
            }
            niri_ipc::Action::SetColumnWidth { change } => Self::SetColumnWidth(change),
            niri_ipc::Action::ExpandColumnToAvailableWidth {} => Self::ExpandColumnToAvailableWidth,
            niri_ipc::Action::SwitchLayout { layout } => Self::SwitchLayout(layout),
            niri_ipc::Action::ShowHotkeyOverlay {} => Self::ShowHotkeyOverlay,
            niri_ipc::Action::MoveWorkspaceToMonitorLeft {} => Self::MoveWorkspaceToMonitorLeft,
            niri_ipc::Action::MoveWorkspaceToMonitorRight {} => Self::MoveWorkspaceToMonitorRight,
            niri_ipc::Action::MoveWorkspaceToMonitorDown {} => Self::MoveWorkspaceToMonitorDown,
            niri_ipc::Action::MoveWorkspaceToMonitorUp {} => Self::MoveWorkspaceToMonitorUp,
            niri_ipc::Action::MoveWorkspaceToMonitorPrevious {} => {
                Self::MoveWorkspaceToMonitorPrevious
            }
            niri_ipc::Action::MoveWorkspaceToIndex {
                index,
                reference: Some(reference),
            } => Self::MoveWorkspaceToIndexByRef {
                new_idx: index,
                reference: WorkspaceReference::from(reference),
            },
            niri_ipc::Action::MoveWorkspaceToIndex {
                index,
                reference: None,
            } => Self::MoveWorkspaceToIndex(index),
            niri_ipc::Action::MoveWorkspaceToMonitor {
                output,
                reference: Some(reference),
            } => Self::MoveWorkspaceToMonitorByRef {
                output_name: output,
                reference: WorkspaceReference::from(reference),
            },
            niri_ipc::Action::MoveWorkspaceToMonitor {
                output,
                reference: None,
            } => Self::MoveWorkspaceToMonitor(output),
            niri_ipc::Action::MoveWorkspaceToMonitorNext {} => Self::MoveWorkspaceToMonitorNext,
            niri_ipc::Action::ToggleDebugTint {} => Self::ToggleDebugTint,
            niri_ipc::Action::DebugToggleOpaqueRegions {} => Self::DebugToggleOpaqueRegions,
            niri_ipc::Action::DebugToggleDamage {} => Self::DebugToggleDamage,
            niri_ipc::Action::ToggleWindowFloating { id: None } => Self::ToggleWindowFloating,
            niri_ipc::Action::ToggleWindowFloating { id: Some(id) } => {
                Self::ToggleWindowFloatingById(id)
            }
            niri_ipc::Action::MoveWindowToFloating { id: None } => Self::MoveWindowToFloating,
            niri_ipc::Action::MoveWindowToFloating { id: Some(id) } => {
                Self::MoveWindowToFloatingById(id)
            }
            niri_ipc::Action::MoveWindowToTiling { id: None } => Self::MoveWindowToTiling,
            niri_ipc::Action::MoveWindowToTiling { id: Some(id) } => {
                Self::MoveWindowToTilingById(id)
            }
            niri_ipc::Action::FocusFloating {} => Self::FocusFloating,
            niri_ipc::Action::FocusTiling {} => Self::FocusTiling,
            niri_ipc::Action::SwitchFocusBetweenFloatingAndTiling {} => {
                Self::SwitchFocusBetweenFloatingAndTiling
            }
            niri_ipc::Action::MoveFloatingWindow { id, x, y } => {
                Self::MoveFloatingWindowById { id, x, y }
            }
            niri_ipc::Action::ToggleWindowRuleOpacity { id: None } => Self::ToggleWindowRuleOpacity,
            niri_ipc::Action::ToggleWindowRuleOpacity { id: Some(id) } => {
                Self::ToggleWindowRuleOpacityById(id)
            }
            niri_ipc::Action::SetDynamicCastWindow { id: None } => Self::SetDynamicCastWindow,
            niri_ipc::Action::SetDynamicCastWindow { id: Some(id) } => {
                Self::SetDynamicCastWindowById(id)
            }
            niri_ipc::Action::SetDynamicCastMonitor { output } => {
                Self::SetDynamicCastMonitor(output)
            }
            niri_ipc::Action::ClearDynamicCastTarget {} => Self::ClearDynamicCastTarget,
            niri_ipc::Action::StopCast { session_id } => Self::StopCast(session_id),
            niri_ipc::Action::ToggleOverview {} => Self::ToggleOverview,
            niri_ipc::Action::OpenOverview {} => Self::OpenOverview,
            niri_ipc::Action::CloseOverview {} => Self::CloseOverview,
            niri_ipc::Action::ToggleWindowUrgent { id } => Self::ToggleWindowUrgent(id),
            niri_ipc::Action::SetWindowUrgent { id } => Self::SetWindowUrgent(id),
            niri_ipc::Action::UnsetWindowUrgent { id } => Self::UnsetWindowUrgent(id),
            niri_ipc::Action::LoadConfigFile { path } => Self::LoadConfigFile(path),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum WorkspaceReference {
    Id(u64),
    Index(u8),
    Name(String),
}

impl From<WorkspaceReferenceArg> for WorkspaceReference {
    fn from(reference: WorkspaceReferenceArg) -> WorkspaceReference {
        match reference {
            WorkspaceReferenceArg::Id(id) => Self::Id(id),
            WorkspaceReferenceArg::Index(i) => Self::Index(i),
            WorkspaceReferenceArg::Name(n) => Self::Name(n),
        }
    }
}

impl<S: knuffel::traits::ErrorSpan> knuffel::DecodeScalar<S> for WorkspaceReference {
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
    ) -> Result<WorkspaceReference, DecodeError<S>> {
        match &**val {
            knuffel::ast::Literal::String(ref s) => Ok(WorkspaceReference::Name(s.clone().into())),
            knuffel::ast::Literal::Int(ref value) => match value.try_into() {
                Ok(v) => Ok(WorkspaceReference::Index(v)),
                Err(e) => {
                    ctx.emit_error(DecodeError::conversion(val, e));
                    Ok(WorkspaceReference::Index(0))
                }
            },
            _ => {
                ctx.emit_error(DecodeError::unsupported(
                    val,
                    "Unsupported value, only numbers and strings are recognized",
                ));
                Ok(WorkspaceReference::Index(0))
            }
        }
    }
}

impl<S> knuffel::Decode<S> for Binds
where
    S: knuffel::traits::ErrorSpan,
{
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        expect_only_children(node, ctx);

        let mut seen_keys: HashMap<Key, &knuffel::ast::SpannedNode<S>> = HashMap::new();

        let mut binds = Vec::new();

        for child in node.children() {
            match Bind::decode_node(child, ctx) {
                Err(e) => {
                    ctx.emit_error(e);
                }
                Ok(bind) => {
                    match seen_keys.entry(bind.key) {
                        Entry::Occupied(entry) => {
                            // Even though it's technically incorrect, we use
                            // `DecodeError::Missing` here because it labels the bind with
                            // "node starts here", which is the least bad option
                            ctx.emit_error(DecodeError::missing(
                                entry.get(),
                                "keybind first defined here",
                            ));

                            ctx.emit_error(DecodeError::unexpected(
                                &child.node_name,
                                "keybind",
                                "duplicate keybind later defined here",
                            ));
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(child);
                            binds.push(bind);
                        }
                    }
                }
            }
        }

        Ok(Self(binds))
    }
}

impl<S> knuffel::Decode<S> for Bind
where
    S: knuffel::traits::ErrorSpan,
{
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        if let Some(type_name) = &node.type_name {
            ctx.emit_error(DecodeError::unexpected(
                type_name,
                "type name",
                "no type name expected for this node",
            ));
        }

        for val in node.arguments.iter() {
            ctx.emit_error(DecodeError::unexpected(
                &val.literal,
                "argument",
                "no arguments expected for this node",
            ));
        }

        // Split modifiers from the node name. `Ctrl+Shift+TouchSwipe` →
        // (Modifiers::CTRL|SHIFT, "TouchSwipe").
        let (modifiers, trigger_name) = match parse_modifiers(&node.node_name) {
            Ok(pair) => pair,
            Err(e) => {
                return Err(DecodeError::conversion(
                    &node.node_name,
                    e.wrap_err("invalid keybind"),
                ))
            }
        };
        let is_gesture_family = is_gesture_family_name(trigger_name);

        // For non-gesture triggers, parse the node name directly (keysyms,
        // mouse buttons, wheel, TouchpadScroll). For gesture families we
        // build the Trigger from properties below, because the node name
        // alone carries no finger count / direction / edge info.
        let key_from_name = if is_gesture_family {
            None
        } else {
            Some(node.node_name.parse::<Key>().map_err(|e| {
                DecodeError::conversion(&node.node_name, e.wrap_err("invalid keybind"))
            })?)
        };

        let mut repeat = true;
        let mut cooldown = None;
        let mut allow_when_locked = false;
        let mut allow_when_locked_node = None;
        let mut allow_inhibiting = true;
        let mut hotkey_overlay_title = None;
        let mut sensitivity = None;
        let mut natural_scroll = false;
        let mut tag = None;

        // Gesture-specific properties, only populated / legal when
        // `is_gesture_family` is true.
        let mut gesture_fingers: Option<u8> = None;
        let mut gesture_direction: Option<String> = None;
        let mut gesture_edge: Option<String> = None;
        let mut gesture_zone: Option<String> = None;

        for (name, val) in &node.properties {
            match &***name {
                "repeat" => {
                    repeat = knuffel::traits::DecodeScalar::decode(val, ctx)?;
                }
                "cooldown-ms" => {
                    cooldown = Some(Duration::from_millis(
                        knuffel::traits::DecodeScalar::decode(val, ctx)?,
                    ));
                }
                "allow-when-locked" => {
                    allow_when_locked = knuffel::traits::DecodeScalar::decode(val, ctx)?;
                    allow_when_locked_node = Some(name);
                }
                "allow-inhibiting" => {
                    allow_inhibiting = knuffel::traits::DecodeScalar::decode(val, ctx)?;
                }
                "hotkey-overlay-title" => {
                    hotkey_overlay_title = Some(knuffel::traits::DecodeScalar::decode(val, ctx)?);
                }
                "sensitivity" => {
                    sensitivity = Some(knuffel::traits::DecodeScalar::decode(val, ctx)?);
                }
                "natural-scroll" => {
                    natural_scroll = knuffel::traits::DecodeScalar::decode(val, ctx)?;
                }
                "tag" => {
                    tag = Some(knuffel::traits::DecodeScalar::decode(val, ctx)?);
                }
                // Gesture-specific properties. Note that knuffel stores
                // `node.properties` as a BTreeMap keyed on name, so a
                // KDL node written with `fingers=3 fingers=5 ...` is
                // silently collapsed to its last value at AST-build
                // time — this loop only ever sees one entry per name.
                // Duplicate detection therefore can't happen here; the
                // only way to reject duplicates would be to intercept
                // the raw KDL source before knuffel parses it, which
                // isn't worth it. Last-wins is KDL-level behavior,
                // and users who care get the same hazard on every
                // other bind property (`tag=`, `cooldown-ms=`, etc.).
                "fingers" if is_gesture_family => {
                    gesture_fingers = Some(knuffel::traits::DecodeScalar::decode(val, ctx)?);
                }
                "direction" if is_gesture_family => {
                    gesture_direction = Some(knuffel::traits::DecodeScalar::decode(val, ctx)?);
                }
                "edge" if is_gesture_family => {
                    gesture_edge = Some(knuffel::traits::DecodeScalar::decode(val, ctx)?);
                }
                "zone" if is_gesture_family => {
                    gesture_zone = Some(knuffel::traits::DecodeScalar::decode(val, ctx)?);
                }
                name_str => {
                    ctx.emit_error(DecodeError::unexpected(
                        name,
                        "property",
                        format!("unexpected property `{}`", name_str.escape_default()),
                    ));
                }
            }
        }

        // Build the Key. For gesture families, combine node name +
        // collected properties via build_gesture_trigger.
        let key = if is_gesture_family {
            let props = GestureTriggerProps {
                fingers: gesture_fingers,
                direction: gesture_direction.as_deref(),
                edge: gesture_edge.as_deref(),
                zone: gesture_zone.as_deref(),
            };
            match build_gesture_trigger(trigger_name, &props) {
                Ok(trigger) => Key { trigger, modifiers },
                Err(msg) => {
                    return Err(DecodeError::conversion(&node.node_name, miette!("{msg}")));
                }
            }
        } else {
            key_from_name.unwrap()
        };

        // Tags are only supported on gesture triggers (touchscreen/touchpad).
        // Allowing tags on keyboard/mouse binds would let the IPC event stream
        // be used as a keylogger — every tagged keypress would emit an event
        // with the key name to any process listening on the socket. Gestures
        // are safe because they don't carry text input (you can't type a
        // password with a 3-finger swipe).
        if tag.is_some() && !key.trigger.is_gesture() {
            ctx.emit_error(DecodeError::unexpected(
                &node.node_name,
                "property",
                "tag is only supported on gesture triggers (Touch*/Touchpad*)",
            ));
            tag = None;
        }

        let mut children = node.children();

        // If the action is invalid but the key is fine, we still want to return something.
        // That way, the parent can handle the existence of duplicate keybinds,
        // even if their contents are not valid.
        let dummy = Self {
            key,
            action: Action::Spawn(vec![]),
            repeat: true,
            cooldown: None,
            allow_when_locked: false,
            allow_inhibiting: true,
            hotkey_overlay_title: None,
            sensitivity: None,
            natural_scroll: false,
            tag: None,
        };

        if let Some(child) = children.next() {
            for unwanted_child in children {
                ctx.emit_error(DecodeError::unexpected(
                    unwanted_child,
                    "node",
                    "only one action is allowed per keybind",
                ));
            }
            match Action::decode_node(child, ctx) {
                Ok(action) => {
                    if !matches!(action, Action::Spawn(_) | Action::SpawnSh(_)) {
                        if let Some(node) = allow_when_locked_node {
                            ctx.emit_error(DecodeError::unexpected(
                                node,
                                "property",
                                "allow-when-locked can only be set on spawn binds",
                            ));
                        }
                    }

                    // The toggle-inhibit action must always be uninhibitable.
                    // Otherwise, it would be impossible to trigger it.
                    if matches!(action, Action::ToggleKeyboardShortcutsInhibit) {
                        allow_inhibiting = false;
                    }

                    Ok(Self {
                        key,
                        action,
                        repeat,
                        cooldown,
                        allow_when_locked,
                        allow_inhibiting,
                        hotkey_overlay_title,
                        sensitivity,
                        natural_scroll,
                        tag,
                    })
                }
                Err(e) => {
                    ctx.emit_error(e);
                    Ok(dummy)
                }
            }
        } else {
            ctx.emit_error(DecodeError::missing(
                node,
                "expected an action for this keybind",
            ));
            Ok(dummy)
        }
    }
}

/// Returns true if `s` names one of the five parameterized gesture
/// families. These are parsed via KDL properties in `Bind::decode_node`,
/// not via `FromStr for Key`.
pub(crate) fn is_gesture_family_name(s: &str) -> bool {
    s.eq_ignore_ascii_case("TouchpadSwipe")
        || s.eq_ignore_ascii_case("TouchpadTapHold")
        || s.eq_ignore_ascii_case("TouchpadTapHoldDrag")
        || s.eq_ignore_ascii_case("TouchSwipe")
        || s.eq_ignore_ascii_case("TouchPinch")
        || s.eq_ignore_ascii_case("TouchRotate")
        || s.eq_ignore_ascii_case("TouchTap")
        || s.eq_ignore_ascii_case("TouchTapHoldDrag")
        || s.eq_ignore_ascii_case("TouchEdge")
}

/// Splits `Ctrl+Shift+Foo` into `(modifiers, "Foo")`.
fn parse_modifiers(s: &str) -> Result<(Modifiers, &str), miette::Error> {
    let mut modifiers = Modifiers::empty();
    let mut split = s.split('+');
    let key = split.next_back().unwrap();
    for part in split {
        let part = part.trim();
        if part.eq_ignore_ascii_case("mod") {
            modifiers |= Modifiers::COMPOSITOR;
        } else if part.eq_ignore_ascii_case("ctrl") || part.eq_ignore_ascii_case("control") {
            modifiers |= Modifiers::CTRL;
        } else if part.eq_ignore_ascii_case("shift") {
            modifiers |= Modifiers::SHIFT;
        } else if part.eq_ignore_ascii_case("alt") {
            modifiers |= Modifiers::ALT;
        } else if part.eq_ignore_ascii_case("super") || part.eq_ignore_ascii_case("win") {
            modifiers |= Modifiers::SUPER;
        } else if part.eq_ignore_ascii_case("iso_level3_shift") || part.eq_ignore_ascii_case("mod5")
        {
            modifiers |= Modifiers::ISO_LEVEL3_SHIFT;
        } else if part.eq_ignore_ascii_case("iso_level5_shift") || part.eq_ignore_ascii_case("mod3")
        {
            modifiers |= Modifiers::ISO_LEVEL5_SHIFT;
        } else {
            return Err(miette!("invalid modifier: {part}"));
        }
    }
    Ok((modifiers, key))
}

/// Properties collected from a gesture bind node that feed into building
/// a parameterized `Trigger` variant.
#[derive(Debug, Default)]
pub(crate) struct GestureTriggerProps<'a> {
    pub fingers: Option<u8>,
    pub direction: Option<&'a str>,
    pub edge: Option<&'a str>,
    pub zone: Option<&'a str>,
}

/// Build a parameterized gesture `Trigger` from a family name and the
/// properties collected on the KDL node. Returns a human-readable error
/// string on any invalid combination (the caller wraps it in a knuffel
/// `DecodeError`).
pub(crate) fn build_gesture_trigger(
    family: &str,
    props: &GestureTriggerProps<'_>,
) -> Result<Trigger, String> {
    let expect_fingers = |props: &GestureTriggerProps<'_>| -> Result<u8, String> {
        let Some(n) = props.fingers else {
            return Err(format!(
                "{family} requires `fingers=N` (valid range {MIN_FINGERS}..={MAX_FINGERS})"
            ));
        };
        if !(MIN_FINGERS..=MAX_FINGERS).contains(&n) {
            return Err(format!(
                "fingers={n} out of range (valid range {MIN_FINGERS}..={MAX_FINGERS})"
            ));
        }
        Ok(n)
    };
    let reject_edge_zone = |props: &GestureTriggerProps<'_>| -> Result<(), String> {
        if props.edge.is_some() {
            return Err(format!("{family} does not accept an `edge=` property"));
        }
        if props.zone.is_some() {
            return Err(format!("{family} does not accept a `zone=` property"));
        }
        Ok(())
    };

    if family.eq_ignore_ascii_case("TouchSwipe") || family.eq_ignore_ascii_case("TouchpadSwipe") {
        reject_edge_zone(props)?;
        let fingers = expect_fingers(props)?;
        let direction = props
            .direction
            .ok_or_else(|| format!("{family} requires `direction=\"up|down|left|right\"`"))?;
        let direction = match direction.to_ascii_lowercase().as_str() {
            "up" => SwipeDirection::Up,
            "down" => SwipeDirection::Down,
            "left" => SwipeDirection::Left,
            "right" => SwipeDirection::Right,
            other => {
                return Err(format!(
                    "invalid direction=\"{other}\" for {family} (expected up|down|left|right)"
                ))
            }
        };
        return Ok(if family.eq_ignore_ascii_case("TouchSwipe") {
            Trigger::TouchSwipe { fingers, direction }
        } else {
            Trigger::TouchpadSwipe { fingers, direction }
        });
    }

    if family.eq_ignore_ascii_case("TouchPinch") {
        reject_edge_zone(props)?;
        let fingers = expect_fingers(props)?;
        let direction = props
            .direction
            .ok_or_else(|| "TouchPinch requires `direction=\"in|out\"`".to_string())?;
        let direction = match direction.to_ascii_lowercase().as_str() {
            "in" => PinchDirection::In,
            "out" => PinchDirection::Out,
            other => {
                return Err(format!(
                    "invalid direction=\"{other}\" for TouchPinch (expected in|out)"
                ))
            }
        };
        return Ok(Trigger::TouchPinch { fingers, direction });
    }

    if family.eq_ignore_ascii_case("TouchRotate") {
        reject_edge_zone(props)?;
        let fingers = expect_fingers(props)?;
        let direction = props
            .direction
            .ok_or_else(|| "TouchRotate requires `direction=\"cw|ccw\"`".to_string())?;
        let direction = match direction.to_ascii_lowercase().as_str() {
            "cw" => RotateDirection::Cw,
            "ccw" => RotateDirection::Ccw,
            other => {
                return Err(format!(
                    "invalid direction=\"{other}\" for TouchRotate (expected cw|ccw)"
                ))
            }
        };
        return Ok(Trigger::TouchRotate { fingers, direction });
    }

    if family.eq_ignore_ascii_case("TouchTap")
        || family.eq_ignore_ascii_case("TouchpadTapHold")
        || family.eq_ignore_ascii_case("TouchpadTapHoldDrag")
    {
        reject_edge_zone(props)?;
        let fingers = expect_fingers(props)?;
        if props.direction.is_some() {
            return Err(format!("{family} does not accept a `direction=` property"));
        }
        return Ok(if family.eq_ignore_ascii_case("TouchTap") {
            Trigger::TouchTap { fingers }
        } else if family.eq_ignore_ascii_case("TouchpadTapHold") {
            Trigger::TouchpadTapHold { fingers }
        } else {
            Trigger::TouchpadTapHoldDrag { fingers }
        });
    }

    if family.eq_ignore_ascii_case("TouchTapHoldDrag") {
        reject_edge_zone(props)?;
        let fingers = expect_fingers(props)?;
        // direction= is optional for TouchTapHoldDrag (unlike TouchSwipe
        // where it's required). None = omnidirectional.
        let direction = match props.direction {
            None => None,
            Some(d) => {
                let dir = match d.to_ascii_lowercase().as_str() {
                    "up" => SwipeDirection::Up,
                    "down" => SwipeDirection::Down,
                    "left" => SwipeDirection::Left,
                    "right" => SwipeDirection::Right,
                    other => {
                        return Err(format!(
                            "invalid direction=\"{other}\" for TouchTapHoldDrag \
                             (expected up|down|left|right)"
                        ))
                    }
                };
                Some(dir)
            }
        };
        return Ok(Trigger::TouchTapHoldDrag { fingers, direction });
    }

    if family.eq_ignore_ascii_case("TouchEdge") {
        if props.fingers.is_some() {
            return Err("TouchEdge does not accept a `fingers=` property".to_string());
        }
        if props.direction.is_some() {
            return Err(
                "TouchEdge uses `edge=` (not `direction=`) and an optional `zone=`".to_string(),
            );
        }
        let edge = props
            .edge
            .ok_or_else(|| "TouchEdge requires `edge=\"left|right|top|bottom\"`".to_string())?;
        let edge = match edge.to_ascii_lowercase().as_str() {
            "left" => ScreenEdge::Left,
            "right" => ScreenEdge::Right,
            "top" => ScreenEdge::Top,
            "bottom" => ScreenEdge::Bottom,
            other => {
                return Err(format!(
                    "invalid edge=\"{other}\" (expected left|right|top|bottom)"
                ))
            }
        };
        // Zone parsing uses `zone_kdl_name` as the single source of truth
        // for the axis-rotating vocabulary (top/bottom edges take
        // left|center|right; left/right edges take top|center|bottom).
        // We try each of the three legal EdgeZone values and see which
        // one's KDL name matches the user's input.
        let zone = match props.zone {
            None => None,
            Some(z) => {
                let z_lower = z.to_ascii_lowercase();
                let matched = [EdgeZone::Start, EdgeZone::Center, EdgeZone::End]
                    .into_iter()
                    .find(|&ez| crate::input::zone_kdl_name(edge, ez) == z_lower);
                match matched {
                    Some(ez) => Some(ez),
                    None => {
                        let valid = format!(
                            "{}|{}|{}",
                            crate::input::zone_kdl_name(edge, EdgeZone::Start),
                            crate::input::zone_kdl_name(edge, EdgeZone::Center),
                            crate::input::zone_kdl_name(edge, EdgeZone::End),
                        );
                        return Err(format!(
                            "invalid zone=\"{z}\" for edge=\"{}\" (expected {valid})",
                            edge.as_kdl_name()
                        ));
                    }
                }
            }
        };
        return Ok(Trigger::TouchEdge { edge, zone });
    }

    Err(format!("unknown gesture family `{family}`"))
}

impl FromStr for Key {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (modifiers, key) = parse_modifiers(s)?;

        let trigger = if key.eq_ignore_ascii_case("MouseLeft") {
            Trigger::MouseLeft
        } else if key.eq_ignore_ascii_case("MouseRight") {
            Trigger::MouseRight
        } else if key.eq_ignore_ascii_case("MouseMiddle") {
            Trigger::MouseMiddle
        } else if key.eq_ignore_ascii_case("MouseBack") {
            Trigger::MouseBack
        } else if key.eq_ignore_ascii_case("MouseForward") {
            Trigger::MouseForward
        } else if key.eq_ignore_ascii_case("WheelScrollDown") {
            Trigger::WheelScrollDown
        } else if key.eq_ignore_ascii_case("WheelScrollUp") {
            Trigger::WheelScrollUp
        } else if key.eq_ignore_ascii_case("WheelScrollLeft") {
            Trigger::WheelScrollLeft
        } else if key.eq_ignore_ascii_case("WheelScrollRight") {
            Trigger::WheelScrollRight
        } else if key.eq_ignore_ascii_case("TouchpadScrollDown") {
            Trigger::TouchpadScrollDown
        } else if key.eq_ignore_ascii_case("TouchpadScrollUp") {
            Trigger::TouchpadScrollUp
        } else if key.eq_ignore_ascii_case("TouchpadScrollLeft") {
            Trigger::TouchpadScrollLeft
        } else if key.eq_ignore_ascii_case("TouchpadScrollRight") {
            Trigger::TouchpadScrollRight
        } else if is_gesture_family_name(key) {
            // Gesture families (TouchpadSwipe, TouchSwipe, TouchPinch,
            // TouchRotate, TouchEdge) are parameterized by KDL properties
            // (`fingers=`, `direction=`, `edge=`, `zone=`), so the node
            // name alone isn't enough to construct a Trigger. They are
            // parsed in `Bind::decode_node` where `node.properties` is
            // reachable. Reject them here so a bare gesture-family name
            // without the expected property-parsing path produces a clear
            // error instead of being silently routed to keysym lookup.
            return Err(miette!(
                "{key} is a parameterized gesture family — use property form like \
                 `TouchSwipe fingers=3 direction=\"up\"`"
            ));
        } else {
            let mut keysym = keysym_from_name(key, KEYSYM_CASE_INSENSITIVE);
            // The keyboard event handling code can receive either
            // XF86ScreenSaver or XF86Screensaver, because there is no
            // case mapping defined between these keysyms. If we just
            // use the case-insensitive version of keysym_from_name it
            // is not possible to bind the uppercase version, because the
            // case-insensitive match prefers the lowercase version when
            // there is a choice.
            //
            // Therefore, when we match this key with the initial
            // case-insensitive match we try a further case-sensitive match
            // (so that either key can be bound). If that fails, we change
            // to the uppercase version because:
            //
            // - A comment in xkb_keysym_from_name (in libxkbcommon) tells us that the uppercase
            //   version is the "best" of the two. [0]
            // - The xkbcommon crate only has a constant for ScreenSaver. [1]
            //
            // [0]: https://github.com/xkbcommon/libxkbcommon/blob/45a118d5325b051343b4b174f60c1434196fa7d4/src/keysym.c#L276
            // [1]: https://docs.rs/xkbcommon/latest/xkbcommon/xkb/keysyms/index.html#:~:text=KEY%5FXF86ScreenSaver
            //
            // See https://github.com/niri-wm/niri/issues/1969
            if keysym == Keysym::XF86_Screensaver {
                keysym = keysym_from_name(key, KEYSYM_NO_FLAGS);
                if keysym.raw() == KEY_NoSymbol {
                    keysym = Keysym::XF86_ScreenSaver;
                }
            }
            if keysym.raw() == KEY_NoSymbol {
                return Err(miette!("invalid key: {key}"));
            }
            Trigger::Keysym(keysym)
        };

        Ok(Key { trigger, modifiers })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_xf86_screensaver() {
        assert_eq!(
            "XF86ScreenSaver".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::XF86_ScreenSaver),
                modifiers: Modifiers::empty(),
            },
        );
        assert_eq!(
            "XF86Screensaver".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::XF86_Screensaver),
                modifiers: Modifiers::empty(),
            }
        );
        assert_eq!(
            "xf86screensaver".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::XF86_ScreenSaver),
                modifiers: Modifiers::empty(),
            }
        );
    }

    #[test]
    fn parse_iso_level_shifts() {
        assert_eq!(
            "ISO_Level3_Shift+A".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::a),
                modifiers: Modifiers::ISO_LEVEL3_SHIFT
            },
        );
        assert_eq!(
            "Mod5+A".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::a),
                modifiers: Modifiers::ISO_LEVEL3_SHIFT
            },
        );

        assert_eq!(
            "ISO_Level5_Shift+A".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::a),
                modifiers: Modifiers::ISO_LEVEL5_SHIFT
            },
        );
        assert_eq!(
            "Mod3+A".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::a),
                modifiers: Modifiers::ISO_LEVEL5_SHIFT
            },
        );
    }

    #[test]
    fn bare_gesture_family_name_is_rejected_by_fromstr() {
        // FromStr for Key doesn't have property context, so a bare
        // `TouchSwipe` with no properties must fail (property parsing
        // happens in Bind::decode_node).
        assert!("TouchSwipe".parse::<Key>().is_err());
        assert!("TouchPinch".parse::<Key>().is_err());
        assert!("TouchRotate".parse::<Key>().is_err());
        assert!("TouchTap".parse::<Key>().is_err());
        assert!("TouchEdge".parse::<Key>().is_err());
        assert!("TouchpadSwipe".parse::<Key>().is_err());
        assert!("TouchpadTapHold".parse::<Key>().is_err());
        assert!("TouchpadTapHoldDrag".parse::<Key>().is_err());
        assert!("TouchTapHoldDrag".parse::<Key>().is_err());
    }

    #[test]
    fn old_hardcoded_touch_names_no_longer_parse() {
        // Hard break: the old TouchSwipe3Up / TouchEdgeLeft style is gone.
        // These should now be interpreted as unknown keysyms and fail.
        assert!("TouchSwipe3Up".parse::<Key>().is_err());
        assert!("TouchPinch3In".parse::<Key>().is_err());
        assert!("TouchRotate4Cw".parse::<Key>().is_err());
        assert!("TouchEdgeTop:Left".parse::<Key>().is_err());
    }

    #[test]
    fn build_touchswipe() {
        let props = GestureTriggerProps {
            fingers: Some(3),
            direction: Some("up"),
            edge: None,
            zone: None,
        };
        assert_eq!(
            build_gesture_trigger("TouchSwipe", &props).unwrap(),
            Trigger::TouchSwipe {
                fingers: 3,
                direction: SwipeDirection::Up
            }
        );
    }

    #[test]
    fn build_touchswipe_arbitrary_fingers() {
        for n in MIN_FINGERS..=MAX_FINGERS {
            let props = GestureTriggerProps {
                fingers: Some(n),
                direction: Some("right"),
                edge: None,
                zone: None,
            };
            let got = build_gesture_trigger("TouchSwipe", &props).unwrap();
            assert_eq!(
                got,
                Trigger::TouchSwipe {
                    fingers: n,
                    direction: SwipeDirection::Right
                }
            );
        }
    }

    #[test]
    fn fingers_out_of_range_rejected() {
        for bad in [0u8, 1, 2, 11, 20] {
            let props = GestureTriggerProps {
                fingers: Some(bad),
                direction: Some("up"),
                edge: None,
                zone: None,
            };
            assert!(
                build_gesture_trigger("TouchSwipe", &props).is_err(),
                "fingers={bad} should be rejected"
            );
        }
    }

    #[test]
    fn direction_validated_per_family() {
        // "up" is valid for swipe but not pinch/rotate.
        let swipe_up = GestureTriggerProps {
            fingers: Some(3),
            direction: Some("up"),
            edge: None,
            zone: None,
        };
        assert!(build_gesture_trigger("TouchSwipe", &swipe_up).is_ok());
        assert!(build_gesture_trigger("TouchPinch", &swipe_up).is_err());
        assert!(build_gesture_trigger("TouchRotate", &swipe_up).is_err());

        // "in" is valid for pinch but not swipe/rotate.
        let pinch_in = GestureTriggerProps {
            fingers: Some(3),
            direction: Some("in"),
            edge: None,
            zone: None,
        };
        assert!(build_gesture_trigger("TouchPinch", &pinch_in).is_ok());
        assert!(build_gesture_trigger("TouchSwipe", &pinch_in).is_err());
        assert!(build_gesture_trigger("TouchRotate", &pinch_in).is_err());

        // "cw" is valid for rotate but not swipe/pinch.
        let rotate_cw = GestureTriggerProps {
            fingers: Some(3),
            direction: Some("cw"),
            edge: None,
            zone: None,
        };
        assert!(build_gesture_trigger("TouchRotate", &rotate_cw).is_ok());
        assert!(build_gesture_trigger("TouchSwipe", &rotate_cw).is_err());
        assert!(build_gesture_trigger("TouchPinch", &rotate_cw).is_err());
    }

    #[test]
    fn touchedge_parent_no_zone() {
        let props = GestureTriggerProps {
            fingers: None,
            direction: None,
            edge: Some("left"),
            zone: None,
        };
        assert_eq!(
            build_gesture_trigger("TouchEdge", &props).unwrap(),
            Trigger::TouchEdge {
                edge: ScreenEdge::Left,
                zone: None
            }
        );
    }

    #[test]
    fn touchedge_zoned() {
        // Top edge + zone="right" → EdgeZone::End (thirds along x-axis).
        let props = GestureTriggerProps {
            fingers: None,
            direction: None,
            edge: Some("top"),
            zone: Some("right"),
        };
        assert_eq!(
            build_gesture_trigger("TouchEdge", &props).unwrap(),
            Trigger::TouchEdge {
                edge: ScreenEdge::Top,
                zone: Some(EdgeZone::End)
            }
        );
        // Left edge + zone="top" → EdgeZone::Start (thirds along y-axis).
        let props = GestureTriggerProps {
            fingers: None,
            direction: None,
            edge: Some("left"),
            zone: Some("top"),
        };
        assert_eq!(
            build_gesture_trigger("TouchEdge", &props).unwrap(),
            Trigger::TouchEdge {
                edge: ScreenEdge::Left,
                zone: Some(EdgeZone::Start)
            }
        );
    }

    #[test]
    fn touchedge_zone_vocab_mismatch_rejected() {
        // Left/Right edges need top/center/bottom zones, not left/right.
        let bad = GestureTriggerProps {
            fingers: None,
            direction: None,
            edge: Some("left"),
            zone: Some("left"),
        };
        assert!(build_gesture_trigger("TouchEdge", &bad).is_err());

        // Top/Bottom edges need left/center/right zones, not top/bottom.
        let bad = GestureTriggerProps {
            fingers: None,
            direction: None,
            edge: Some("top"),
            zone: Some("top"),
        };
        assert!(build_gesture_trigger("TouchEdge", &bad).is_err());
    }

    #[test]
    fn touchedge_rejects_fingers() {
        let props = GestureTriggerProps {
            fingers: Some(3),
            direction: None,
            edge: Some("left"),
            zone: None,
        };
        assert!(build_gesture_trigger("TouchEdge", &props).is_err());
    }

    #[test]
    fn is_gesture_family_name_case_insensitive() {
        assert!(is_gesture_family_name("TouchSwipe"));
        assert!(is_gesture_family_name("touchswipe"));
        assert!(is_gesture_family_name("TOUCHPINCH"));
        assert!(is_gesture_family_name("TouchpadSwipe"));
        assert!(is_gesture_family_name("TouchpadTapHold"));
        assert!(is_gesture_family_name("touchpadtaphold"));
        assert!(is_gesture_family_name("TouchpadTapHoldDrag"));
        assert!(is_gesture_family_name("touchpadtapholddrag"));
        assert!(is_gesture_family_name("TouchTap"));
        assert!(is_gesture_family_name("touchtap"));
        assert!(is_gesture_family_name("TouchTapHoldDrag"));
        assert!(is_gesture_family_name("touchtapholddrag"));
        assert!(!is_gesture_family_name("TouchSwipe3Up"));
        assert!(!is_gesture_family_name("TouchpadScrollUp"));
    }

    // Integration tests exercising the full Bind::decode_node two-phase
    // parse path (strip modifiers → check family → conditional property
    // loop → build trigger). These go through Config::parse_mem so the
    // whole knuffel pipeline is exercised.

    #[track_caller]
    fn parse_binds(binds_kdl: &str) -> crate::Config {
        crate::Config::parse_mem(&format!("binds {{\n{binds_kdl}\n}}"))
            .map_err(miette::Report::new)
            .unwrap()
    }

    #[track_caller]
    fn parse_binds_err(binds_kdl: &str) -> String {
        match crate::Config::parse_mem(&format!("binds {{\n{binds_kdl}\n}}")) {
            Ok(_) => panic!("expected parse error, got Ok"),
            Err(e) => format!("{:?}", miette::Report::new(e)),
        }
    }

    fn first_bind(config: &crate::Config) -> &Bind {
        config.binds.0.first().expect("no binds parsed")
    }

    #[test]
    fn decode_node_touchswipe_basic() {
        let cfg = parse_binds(r#"TouchSwipe fingers=3 direction="up" { focus-workspace-up; }"#);
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchSwipe {
                fingers: 3,
                direction: SwipeDirection::Up,
            }
        );
        assert!(bind.key.modifiers.is_empty());
    }

    #[test]
    fn decode_node_touchswipe_with_modifier() {
        // `Mod+TouchSwipe ...` should strip the modifier and still parse
        // the property form correctly.
        let cfg = parse_binds(
            r#"Mod+TouchSwipe fingers=4 direction="left" { focus-column-right; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchSwipe {
                fingers: 4,
                direction: SwipeDirection::Left,
            }
        );
        assert!(bind.key.modifiers.contains(Modifiers::COMPOSITOR));
    }

    #[test]
    fn decode_node_tag_on_gesture_allowed() {
        let cfg = parse_binds(
            r#"TouchSwipe fingers=3 direction="up" tag="ws-nav" { focus-workspace-up; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(bind.tag.as_deref(), Some("ws-nav"));
    }

    #[test]
    fn decode_node_tag_on_keyboard_bind_rejected() {
        // tag="..." is a keylogging risk on keyboard binds and should
        // fail parsing.
        let err = parse_binds_err(r#"Ctrl+A tag="keylog" { spawn "uname"; }"#);
        assert!(
            err.contains("tag is only supported on gesture triggers"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_node_gesture_property_on_keyboard_bind_rejected() {
        // `fingers=3` on a keyboard bind should fall through to the
        // "unexpected property" arm.
        let err = parse_binds_err(r#"Ctrl+A fingers=3 { spawn "uname"; }"#);
        assert!(
            err.contains("unexpected property"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_node_duplicate_fingers_last_wins() {
        // KDL/knuffel stores properties in a BTreeMap keyed on name, so
        // `fingers=3 fingers=5` silently keeps the last value. Document
        // that observed behavior — this is *not* something niri controls
        // and it applies to every bind property, not just gesture ones.
        let cfg = parse_binds(
            r#"TouchSwipe fingers=3 fingers=5 direction="up" { focus-workspace-up; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchSwipe {
                fingers: 5,
                direction: SwipeDirection::Up,
            }
        );
    }

    #[test]
    fn decode_node_unknown_property_rejected() {
        let err = parse_binds_err(
            r#"TouchSwipe fingers=3 direction="up" foo="bar" { focus-workspace-up; }"#,
        );
        assert!(
            err.contains("unexpected property"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_node_touchedge_with_zone() {
        let cfg =
            parse_binds(r#"TouchEdge edge="top" zone="right" { spawn "screenshot"; }"#);
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchEdge {
                edge: ScreenEdge::Top,
                zone: Some(EdgeZone::End),
            }
        );
    }

    #[test]
    fn decode_node_touchedge_missing_edge_rejected() {
        let err = parse_binds_err(r#"TouchEdge { focus-column-right; }"#);
        assert!(
            err.contains("requires `edge="),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_node_touchedge_zone_vocab_mismatch_rejected() {
        // edge="left" doesn't take zone="left".
        let err = parse_binds_err(r#"TouchEdge edge="left" zone="left" { noop; }"#);
        assert!(
            err.contains("invalid zone"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_node_mod_shift_touchedge_zoned() {
        // Multi-modifier + zoned edge, exercising the full modifier
        // stripping + property path.
        let cfg = parse_binds(
            r#"Mod+Shift+TouchEdge edge="right" zone="bottom" tag="zone-rb" { noop; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchEdge {
                edge: ScreenEdge::Right,
                zone: Some(EdgeZone::End),
            }
        );
        assert!(bind.key.modifiers.contains(Modifiers::COMPOSITOR));
        assert!(bind.key.modifiers.contains(Modifiers::SHIFT));
        assert_eq!(bind.tag.as_deref(), Some("zone-rb"));
    }

    #[test]
    fn decode_node_touchpad_swipe_parses() {
        let cfg = parse_binds(
            r#"TouchpadSwipe fingers=3 direction="right" { focus-column-left; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchpadSwipe {
                fingers: 3,
                direction: SwipeDirection::Right,
            }
        );
    }

    #[test]
    fn decode_node_touchpad_tap_parses() {
        let cfg = parse_binds(
            r#"TouchpadTapHold fingers=3 { screenshot; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchpadTapHold { fingers: 3 }
        );
    }

    #[test]
    fn decode_node_touchpad_tap_with_modifier() {
        let cfg = parse_binds(
            r#"Mod+TouchpadTapHold fingers=4 { close-window; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchpadTapHold { fingers: 4 }
        );
        assert!(bind.key.modifiers.contains(Modifiers::COMPOSITOR));
    }

    #[test]
    fn touchpad_tap_rejects_direction() {
        let props = GestureTriggerProps {
            fingers: Some(3),
            direction: Some("up"),
            edge: None,
            zone: None,
        };
        assert!(
            build_gesture_trigger("TouchpadTapHold", &props).is_err(),
            "TouchpadTapHold should reject direction="
        );
    }

    #[test]
    fn touchpad_tap_rejects_fingers_below_3() {
        for bad in [0u8, 1, 2] {
            let props = GestureTriggerProps {
                fingers: Some(bad),
                direction: None,
                edge: None,
                zone: None,
            };
            assert!(
                build_gesture_trigger("TouchpadTapHold", &props).is_err(),
                "TouchpadTapHold fingers={bad} should be rejected"
            );
        }
    }

    #[test]
    fn decode_node_touchpad_tap_hold_drag_parses() {
        let cfg = parse_binds(
            r#"TouchpadTapHoldDrag fingers=3 { focus-workspace-up; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchpadTapHoldDrag { fingers: 3 }
        );
    }

    #[test]
    fn decode_node_touchpad_tap_hold_drag_with_modifier() {
        let cfg = parse_binds(
            r#"Mod+TouchpadTapHoldDrag fingers=4 { move-window-down; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchpadTapHoldDrag { fingers: 4 }
        );
        assert!(bind.key.modifiers.contains(Modifiers::COMPOSITOR));
    }

    #[test]
    fn touchpad_tap_hold_drag_rejects_direction() {
        let props = GestureTriggerProps {
            fingers: Some(3),
            direction: Some("up"),
            edge: None,
            zone: None,
        };
        assert!(
            build_gesture_trigger("TouchpadTapHoldDrag", &props).is_err(),
            "TouchpadTapHoldDrag should reject direction="
        );
    }

    #[test]
    fn decode_node_touch_tap_hold_drag_omnidirectional() {
        let cfg = parse_binds(
            r#"TouchTapHoldDrag fingers=3 { screenshot; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchTapHoldDrag { fingers: 3, direction: None }
        );
    }

    #[test]
    fn decode_node_touch_tap_hold_drag_directional() {
        let cfg = parse_binds(
            r#"TouchTapHoldDrag fingers=3 direction="left" { spawn "wl-copy"; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchTapHoldDrag {
                fingers: 3,
                direction: Some(SwipeDirection::Left),
            }
        );
    }

    #[test]
    fn decode_node_touch_tap_hold_drag_with_modifier() {
        let cfg = parse_binds(
            r#"Mod+TouchTapHoldDrag fingers=4 direction="up" { toggle-overview; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchTapHoldDrag {
                fingers: 4,
                direction: Some(SwipeDirection::Up),
            }
        );
        assert!(bind.key.modifiers.contains(Modifiers::COMPOSITOR));
    }

    #[test]
    fn decode_node_rotation_parses() {
        let cfg = parse_binds(
            r#"TouchRotate fingers=4 direction="cw" { focus-column-right; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchRotate {
                fingers: 4,
                direction: RotateDirection::Cw,
            }
        );
    }

    #[test]
    fn decode_node_pinch_parses() {
        let cfg = parse_binds(r#"TouchPinch fingers=3 direction="in" { open-overview; }"#);
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchPinch {
                fingers: 3,
                direction: PinchDirection::In,
            }
        );
    }

    #[test]
    fn decode_node_fingers_out_of_range_rejected() {
        let err = parse_binds_err(
            r#"TouchSwipe fingers=2 direction="up" { focus-workspace-up; }"#,
        );
        assert!(err.contains("out of range"), "unexpected error: {err}");
    }

    #[test]
    fn decode_node_pinch_direction_out_parses() {
        let cfg =
            parse_binds(r#"TouchPinch fingers=4 direction="out" { close-overview; }"#);
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchPinch {
                fingers: 4,
                direction: PinchDirection::Out,
            }
        );
    }

    #[test]
    fn decode_node_swipe_with_rotate_direction_rejected() {
        // `direction="cw"` is valid for TouchRotate but not TouchSwipe.
        // Integration-layer coverage that per-family direction validation
        // actually reaches the user through the full parse path.
        let err = parse_binds_err(
            r#"TouchSwipe fingers=3 direction="cw" { focus-workspace-up; }"#,
        );
        assert!(
            err.contains("invalid direction"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_node_touchpad_swipe_with_modifier() {
        // Modifier-stripping on the touchpad family, mirroring the
        // touchscreen `decode_node_touchswipe_with_modifier` test.
        let cfg = parse_binds(
            r#"Mod+TouchpadSwipe fingers=4 direction="down" { toggle-overview; }"#,
        );
        let bind = first_bind(&cfg);
        assert_eq!(
            bind.key.trigger,
            Trigger::TouchpadSwipe {
                fingers: 4,
                direction: SwipeDirection::Down,
            }
        );
        assert!(bind.key.modifiers.contains(Modifiers::COMPOSITOR));
    }
}
