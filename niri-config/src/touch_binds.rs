//! Touchscreen gesture types and continuous gesture detection.
//!
//! Gesture binds are configured in the main `binds {}` block using
//! parameterized trigger families (`TouchSwipe fingers=3 direction="up"`,
//! `TouchEdge edge="left"`, etc.). This module provides the gesture type
//! enum and continuous/discrete classification used by the touchscreen
//! dispatch code.

use crate::binds::Action;

/// Type of touchscreen gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TouchGestureType {
    SwipeUp,
    SwipeDown,
    SwipeLeft,
    SwipeRight,
    PinchIn,
    PinchOut,
    RotateCw,
    RotateCcw,
    EdgeSwipeLeft,
    EdgeSwipeRight,
    EdgeSwipeTop,
    EdgeSwipeBottom,
}

/// Which continuous gesture animation to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuousGestureKind {
    WorkspaceSwitch,
    ViewScroll,
    OverviewToggle,
    /// No compositor animation — only emits IPC progress events for external tools.
    Noop,
}

/// Returns the continuous gesture kind for an action, or None if discrete.
pub fn continuous_gesture_kind(action: &Action) -> Option<ContinuousGestureKind> {
    match action {
        Action::FocusWorkspaceUp | Action::FocusWorkspaceDown => {
            Some(ContinuousGestureKind::WorkspaceSwitch)
        }
        Action::FocusColumnLeft | Action::FocusColumnRight => {
            Some(ContinuousGestureKind::ViewScroll)
        }
        Action::ToggleOverview
        | Action::OpenOverview
        | Action::CloseOverview => Some(ContinuousGestureKind::OverviewToggle),
        Action::Noop => Some(ContinuousGestureKind::Noop),
        _ => None,
    }
}
