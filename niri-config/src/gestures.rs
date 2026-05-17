use crate::utils::MergeWith;
use crate::FloatOrInt;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Gestures {
    pub dnd_edge_view_scroll: DndEdgeViewScroll,
    pub dnd_edge_workspace_switch: DndEdgeWorkspaceSwitch,
    pub edge_overscroll: EdgeOverscroll,
    pub hot_corners: HotCorners,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct GesturesPart {
    #[knuffel(child)]
    pub dnd_edge_view_scroll: Option<DndEdgeViewScrollPart>,
    #[knuffel(child)]
    pub dnd_edge_workspace_switch: Option<DndEdgeWorkspaceSwitchPart>,
    #[knuffel(child)]
    pub edge_overscroll: Option<EdgeOverscrollPart>,
    #[knuffel(child)]
    pub hot_corners: Option<HotCorners>,
}

impl MergeWith<GesturesPart> for Gestures {
    fn merge_with(&mut self, part: &GesturesPart) {
        merge!(
            (self, part),
            dnd_edge_view_scroll,
            dnd_edge_workspace_switch,
            edge_overscroll,
        );
        merge_clone!((self, part), hot_corners);
    }
}

/// Push the pointer past a true screen edge (a hard edge with no adjacent
/// output) to pan focus to the adjacent column (left/right) or workspace
/// (up/down). Unlike `dnd-edge-*`, this works outside drag-and-drop, requires
/// the pointer to reach the actual screen boundary (not a proximity band),
/// has no timer, and performs a single discrete navigation step.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct EdgeOverscroll {
    /// Accumulated overscroll past the edge (logical px) required to trigger.
    /// `0` (the default) disables the gesture entirely.
    pub resistance: f64,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct EdgeOverscrollPart {
    #[knuffel(child, unwrap(argument))]
    pub resistance: Option<FloatOrInt<0, 65535>>,
}

impl MergeWith<EdgeOverscrollPart> for EdgeOverscroll {
    fn merge_with(&mut self, part: &EdgeOverscrollPart) {
        merge!((self, part), resistance);
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

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct HotCorners {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub top_left: bool,
    #[knuffel(child)]
    pub top_right: bool,
    #[knuffel(child)]
    pub bottom_left: bool,
    #[knuffel(child)]
    pub bottom_right: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn parse(text: &str) -> Gestures {
        let part: GesturesPart = knuffel::parse("test.kdl", text)
            .map_err(miette::Report::new)
            .unwrap();
        let mut gestures = Gestures::default();
        gestures.merge_with(&part);
        gestures
    }

    #[test]
    fn parse_edge_overscroll() {
        let gestures = parse("edge-overscroll {\n    resistance 80\n}\n");
        assert_eq!(gestures.edge_overscroll.resistance, 80.0);

        // Omitted → 0.0 (disabled), the default — no behavior change.
        let gestures = parse("");
        assert_eq!(gestures.edge_overscroll.resistance, 0.0);
    }
}
