use std::collections::HashSet;
use std::str::FromStr;

use knuffel::errors::DecodeError;
use miette::miette;

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
    EdgeSwipeLeft,
    EdgeSwipeRight,
    EdgeSwipeTop,
    EdgeSwipeBottom,
}

/// Composite key for a touch gesture bind: gesture type + finger count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TouchGestureKey {
    pub gesture: TouchGestureType,
    /// Finger count for this bind. 0 means "any" (used for edge swipes).
    pub finger_count: u8,
}

/// A single touch gesture binding: trigger -> action.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchGestureBind {
    pub key: TouchGestureKey,
    pub action: Action,
    pub sensitivity: Option<f64>,
    pub natural_scroll: bool,
}

/// Which continuous gesture animation to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuousGestureKind {
    WorkspaceSwitch,
    ViewScroll,
    OverviewToggle,
}

impl TouchGestureBind {
    /// Returns the continuous gesture kind if this bind's action drives an
    /// animation, or None if the action is discrete (fire-once).
    pub fn continuous_kind(&self) -> Option<ContinuousGestureKind> {
        match &self.action {
            Action::FocusWorkspaceUp | Action::FocusWorkspaceDown => {
                Some(ContinuousGestureKind::WorkspaceSwitch)
            }
            Action::FocusColumnLeft | Action::FocusColumnRight => {
                Some(ContinuousGestureKind::ViewScroll)
            }
            Action::ToggleOverview => Some(ContinuousGestureKind::OverviewToggle),
            _ => None,
        }
    }
}

/// Collection of touch gesture binds.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TouchBinds(pub Vec<TouchGestureBind>);

impl TouchBinds {
    /// Find the best-matching bind for a gesture + finger count.
    ///
    /// Priority:
    /// 1. Exact finger count match
    /// 2. Highest finger_count that is <= actual count
    /// 3. Wildcard (finger_count 0)
    pub fn find_bind(
        &self,
        gesture: TouchGestureType,
        finger_count: u8,
    ) -> Option<&TouchGestureBind> {
        let mut best: Option<&TouchGestureBind> = None;

        for bind in &self.0 {
            if bind.key.gesture != gesture {
                continue;
            }

            // Exact match always wins immediately.
            if bind.key.finger_count == finger_count {
                return Some(bind);
            }

            // Wildcard (0) or lower finger count can match.
            if bind.key.finger_count == 0 || bind.key.finger_count <= finger_count {
                match best {
                    None => best = Some(bind),
                    Some(prev) if bind.key.finger_count > prev.key.finger_count => {
                        best = Some(bind);
                    }
                    _ => {}
                }
            }
        }

        best
    }
}

/// Parse a touch gesture key from the node name.
///
/// Formats:
///   Touch{N}Swipe{Up|Down|Left|Right}
///   Touch{N}Pinch{In|Out}
///   TouchEdge{Left|Right|Top|Bottom}
impl FromStr for TouchGestureKey {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s_lower = s.to_ascii_lowercase();

        // Edge swipes: TouchEdge{Direction}
        if let Some(edge) = s_lower.strip_prefix("touchedge") {
            let gesture = match edge {
                "left" => TouchGestureType::EdgeSwipeLeft,
                "right" => TouchGestureType::EdgeSwipeRight,
                "top" => TouchGestureType::EdgeSwipeTop,
                "bottom" => TouchGestureType::EdgeSwipeBottom,
                _ => {
                    return Err(miette!(
                        "invalid edge direction `{edge}`, expected left/right/top/bottom"
                    ))
                }
            };
            return Ok(TouchGestureKey {
                gesture,
                finger_count: 0,
            });
        }

        // Finger gestures: Touch{N}{Type}{Direction}
        let rest = s_lower
            .strip_prefix("touch")
            .ok_or_else(|| miette!("touch gesture bind must start with `Touch`"))?;

        // Parse finger count (one or two digits).
        let (finger_count, rest) = if rest.len() >= 2 && rest.as_bytes()[0].is_ascii_digit() {
            let digit_end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            let count: u8 = rest[..digit_end]
                .parse()
                .map_err(|_| miette!("invalid finger count in `{s}`"))?;
            (count, &rest[digit_end..])
        } else {
            return Err(miette!(
                "expected finger count after `Touch` in `{s}`, e.g. Touch3SwipeUp"
            ));
        };

        // Parse gesture type.
        let gesture = if let Some(dir) = rest.strip_prefix("swipe") {
            match dir {
                "up" => TouchGestureType::SwipeUp,
                "down" => TouchGestureType::SwipeDown,
                "left" => TouchGestureType::SwipeLeft,
                "right" => TouchGestureType::SwipeRight,
                _ => {
                    return Err(miette!(
                        "invalid swipe direction `{dir}`, expected up/down/left/right"
                    ))
                }
            }
        } else if let Some(dir) = rest.strip_prefix("pinch") {
            match dir {
                "in" => TouchGestureType::PinchIn,
                "out" => TouchGestureType::PinchOut,
                _ => {
                    return Err(miette!(
                        "invalid pinch direction `{dir}`, expected in/out"
                    ))
                }
            }
        } else {
            return Err(miette!(
                "unknown gesture type in `{s}`, expected Swipe{{Up/Down/Left/Right}} or Pinch{{In/Out}}"
            ));
        };

        Ok(TouchGestureKey {
            gesture,
            finger_count,
        })
    }
}

// -- knuffel Decode impls --

impl<S> knuffel::Decode<S> for TouchBinds
where
    S: knuffel::traits::ErrorSpan,
{
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        crate::utils::expect_only_children(node, ctx);

        let mut seen_keys = HashSet::new();
        let mut binds = Vec::new();

        for child in node.children() {
            match TouchGestureBind::decode_node(child, ctx) {
                Err(e) => {
                    ctx.emit_error(e);
                }
                Ok(bind) => {
                    if seen_keys.insert(bind.key) {
                        binds.push(bind);
                    } else {
                        ctx.emit_error(DecodeError::unexpected(
                            &child.node_name,
                            "touch gesture bind",
                            "duplicate touch gesture bind",
                        ));
                    }
                }
            }
        }

        Ok(Self(binds))
    }
}

impl<S> knuffel::Decode<S> for TouchGestureBind
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

        // Parse the key from the node name.
        let key = node
            .node_name
            .parse::<TouchGestureKey>()
            .map_err(|e| DecodeError::conversion(&node.node_name, e))?;

        // Parse properties: sensitivity, natural-scroll.
        let mut sensitivity = None;
        let mut natural_scroll = false;
        for (name, val) in &node.properties {
            match &***name {
                "sensitivity" => {
                    sensitivity = Some(knuffel::traits::DecodeScalar::decode(val, ctx)?);
                }
                "natural-scroll" => {
                    natural_scroll = knuffel::traits::DecodeScalar::decode(val, ctx)?;
                }
                name_str => {
                    ctx.emit_error(DecodeError::unexpected(
                        name,
                        "property",
                        format!(
                            "unexpected property `{}`, expected sensitivity or natural-scroll",
                            name_str.escape_default()
                        ),
                    ));
                }
            }
        }

        // Parse action from the single child node.
        let dummy = Self {
            key,
            action: Action::Spawn(vec![]),
            sensitivity: None,
            natural_scroll: false,
        };

        let mut children = node.children();
        if let Some(child) = children.next() {
            for unwanted_child in children {
                ctx.emit_error(DecodeError::unexpected(
                    unwanted_child,
                    "node",
                    "only one action is allowed per touch gesture bind",
                ));
            }
            match Action::decode_node(child, ctx) {
                Ok(action) => Ok(Self {
                    key,
                    action,
                    sensitivity,
                    natural_scroll,
                }),
                Err(e) => {
                    ctx.emit_error(e);
                    Ok(dummy)
                }
            }
        } else {
            ctx.emit_error(DecodeError::missing(
                node,
                "expected an action for this touch gesture bind",
            ));
            Ok(dummy)
        }
    }
}
