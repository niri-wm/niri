use knuffel::ast::SpannedNode;
use knuffel::decode::Context;
use knuffel::errors::DecodeError;
use knuffel::traits::ErrorSpan;

use crate::binds::Action;
use crate::utils::MergeWith;
use crate::FloatOrInt;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Gestures {
    pub dnd_edge_view_scroll: DndEdgeViewScroll,
    pub dnd_edge_workspace_switch: DndEdgeWorkspaceSwitch,
    pub hot_corners: HotCorners,
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct GesturesPart {
    #[knuffel(child)]
    pub dnd_edge_view_scroll: Option<DndEdgeViewScrollPart>,
    #[knuffel(child)]
    pub dnd_edge_workspace_switch: Option<DndEdgeWorkspaceSwitchPart>,
    #[knuffel(child)]
    pub hot_corners: Option<HotCorners>,
}

impl MergeWith<GesturesPart> for Gestures {
    fn merge_with(&mut self, part: &GesturesPart) {
        merge!(
            (self, part),
            dnd_edge_view_scroll,
            dnd_edge_workspace_switch,
        );
        merge_clone!((self, part), hot_corners);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DndEdgeViewScroll {
    pub trigger_width: f64,
    pub delay_ms: u16,
    pub max_speed: f64,
}

impl Default for DndEdgeViewScroll {
    fn default() -> Self {
        Self {
            trigger_width: 30., // Taken from GTK 4.
            delay_ms: 100,
            max_speed: 1500.,
        }
    }
}

#[derive(knuffel::Decode, Debug, Clone, Copy, PartialEq)]
pub struct DndEdgeViewScrollPart {
    #[knuffel(child, unwrap(argument))]
    pub trigger_width: Option<FloatOrInt<0, 65535>>,
    #[knuffel(child, unwrap(argument))]
    pub delay_ms: Option<u16>,
    #[knuffel(child, unwrap(argument))]
    pub max_speed: Option<FloatOrInt<0, 1_000_000>>,
}

impl MergeWith<DndEdgeViewScrollPart> for DndEdgeViewScroll {
    fn merge_with(&mut self, part: &DndEdgeViewScrollPart) {
        merge!((self, part), trigger_width, max_speed);
        merge_clone!((self, part), delay_ms);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DndEdgeWorkspaceSwitch {
    pub trigger_height: f64,
    pub delay_ms: u16,
    pub max_speed: f64,
}

impl Default for DndEdgeWorkspaceSwitch {
    fn default() -> Self {
        Self {
            trigger_height: 50.,
            delay_ms: 100,
            max_speed: 1500.,
        }
    }
}

#[derive(knuffel::Decode, Debug, Clone, Copy, PartialEq)]
pub struct DndEdgeWorkspaceSwitchPart {
    #[knuffel(child, unwrap(argument))]
    pub trigger_height: Option<FloatOrInt<0, 65535>>,
    #[knuffel(child, unwrap(argument))]
    pub delay_ms: Option<u16>,
    #[knuffel(child, unwrap(argument))]
    pub max_speed: Option<FloatOrInt<0, 1_000_000>>,
}

impl MergeWith<DndEdgeWorkspaceSwitchPart> for DndEdgeWorkspaceSwitch {
    fn merge_with(&mut self, part: &DndEdgeWorkspaceSwitchPart) {
        merge!((self, part), trigger_height, max_speed);
        merge_clone!((self, part), delay_ms);
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq)]
pub struct HotCorners {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub top_left: Option<HotCorner>,
    #[knuffel(child)]
    pub top_right: Option<HotCorner>,
    #[knuffel(child)]
    pub bottom_left: Option<HotCorner>,
    #[knuffel(child)]
    pub bottom_right: Option<HotCorner>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct HotCorner {
    pub action: Option<Action>,
}

impl HotCorners {

    pub fn has_any_explicit_corner(&self) -> bool {
        self.top_left.is_some()
            || self.top_right.is_some()
            || self.bottom_left.is_some()
            || self.bottom_right.is_some()
    }

    pub fn action_top_left(&self) -> Option<Action> {
        if self.off {
            return None;
        }

        if let Some(corner) = &self.top_left {
            return Some(corner.action.clone().unwrap_or(Action::ToggleOverview));
        }

        if !self.has_any_explicit_corner() {
            return Some(Action::ToggleOverview);
        }

        None
    }

    pub fn action_top_right(&self) -> Option<Action> {
        if self.off {
            return None;
        }

        self.top_right
            .as_ref()
            .map(|corner| corner.action.clone().unwrap_or(Action::ToggleOverview))
    }

    pub fn action_bottom_left(&self) -> Option<Action> {
        if self.off {
            return None;
        }

        self.bottom_left
            .as_ref()
            .map(|corner| corner.action.clone().unwrap_or(Action::ToggleOverview))
    }

    pub fn action_bottom_right(&self) -> Option<Action> {
        if self.off {
            return None;
        }

        self.bottom_right
            .as_ref()
            .map(|corner| corner.action.clone().unwrap_or(Action::ToggleOverview))
    }
}

impl<S: ErrorSpan> knuffel::Decode<S> for HotCorner {
    fn decode_node(node: &SpannedNode<S>, ctx: &mut Context<S>) -> Result<Self, DecodeError<S>> {
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

        for (name, _) in &node.properties {
            ctx.emit_error(DecodeError::unexpected(
                name,
                "property",
                format!("unexpected property `{}`", name.escape_default()),
            ));
        }

        let mut children = node.children();
        let action = if let Some(child) = children.next() {
            for unwanted_child in children {
                ctx.emit_error(DecodeError::unexpected(
                    unwanted_child,
                    "node",
                    "only one action is allowed per hot corner",
                ));
            }
            match <Action as knuffel::Decode<S>>::decode_node(child, ctx) {
                Ok(action) => Some(action),
                Err(e) => {
                    ctx.emit_error(e);
                    None
                }
            }
        } else {
            None
        };

        Ok(Self { action })
    }
}
