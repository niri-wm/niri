use crate::utils::MergeWith;
use crate::FloatOrInt;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Gestures {
    pub dnd_edge_view_scroll: DndEdgeViewScroll,
    pub dnd_edge_workspace_switch: DndEdgeWorkspaceSwitch,
    pub hot_corners: HotCorners,
}

#[derive(knus::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct GesturesPart {
    #[knus(child)]
    pub dnd_edge_view_scroll: Option<DndEdgeViewScrollPart>,
    #[knus(child)]
    pub dnd_edge_workspace_switch: Option<DndEdgeWorkspaceSwitchPart>,
    #[knus(child)]
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

#[derive(knus::Decode, Debug, Clone, Copy, PartialEq)]
pub struct DndEdgeViewScrollPart {
    #[knus(child, unwrap(argument))]
    pub trigger_width: Option<FloatOrInt<0, 65535>>,
    #[knus(child, unwrap(argument))]
    pub delay_ms: Option<u16>,
    #[knus(child, unwrap(argument))]
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

#[derive(knus::Decode, Debug, Clone, Copy, PartialEq)]
pub struct DndEdgeWorkspaceSwitchPart {
    #[knus(child, unwrap(argument))]
    pub trigger_height: Option<FloatOrInt<0, 65535>>,
    #[knus(child, unwrap(argument))]
    pub delay_ms: Option<u16>,
    #[knus(child, unwrap(argument))]
    pub max_speed: Option<FloatOrInt<0, 1_000_000>>,
}

impl MergeWith<DndEdgeWorkspaceSwitchPart> for DndEdgeWorkspaceSwitch {
    fn merge_with(&mut self, part: &DndEdgeWorkspaceSwitchPart) {
        merge!((self, part), trigger_height, max_speed);
        merge_clone!((self, part), delay_ms);
    }
}

#[derive(knus::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct HotCorners {
    #[knus(child)]
    pub off: bool,
    #[knus(child)]
    pub top_left: bool,
    #[knus(child)]
    pub top_right: bool,
    #[knus(child)]
    pub bottom_left: bool,
    #[knus(child)]
    pub bottom_right: bool,
}
