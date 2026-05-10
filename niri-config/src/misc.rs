use crate::appearance::{Color, WorkspaceShadow, WorkspaceShadowPart, DEFAULT_BACKDROP_COLOR};
use crate::utils::{Flag, MergeWith};
use crate::FloatOrInt;

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct SpawnAtStartup {
    #[knuffel(arguments)]
    pub command: Vec<String>,
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct SpawnShAtStartup {
    #[knuffel(argument)]
    pub command: String,
}

#[derive(Debug, PartialEq)]
pub struct Cursor {
    pub xcursor_theme: String,
    pub xcursor_size: u8,
    pub hide_when_typing: bool,
    pub hide_after_inactive_ms: Option<u32>,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            xcursor_theme: String::from("default"),
            xcursor_size: 24,
            hide_when_typing: false,
            hide_after_inactive_ms: None,
        }
    }
}

#[derive(knuffel::Decode, Debug, PartialEq)]
pub struct CursorPart {
    #[knuffel(child, unwrap(argument))]
    pub xcursor_theme: Option<String>,
    #[knuffel(child, unwrap(argument))]
    pub xcursor_size: Option<u8>,
    #[knuffel(child)]
    pub hide_when_typing: Option<Flag>,
    #[knuffel(child, unwrap(argument))]
    pub hide_after_inactive_ms: Option<u32>,
}

impl MergeWith<CursorPart> for Cursor {
    fn merge_with(&mut self, part: &CursorPart) {
        merge_clone!((self, part), xcursor_theme, xcursor_size);
        merge!((self, part), hide_when_typing);
        merge_clone_opt!((self, part), hide_after_inactive_ms);
    }
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub struct ScreenshotPath(#[knuffel(argument)] pub Option<String>);

impl Default for ScreenshotPath {
    fn default() -> Self {
        Self(Some(String::from(
            "~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png",
        )))
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyOverlay {
    pub skip_at_startup: bool,
    pub hide_not_bound: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyOverlayPart {
    #[knuffel(child)]
    pub skip_at_startup: Option<Flag>,
    #[knuffel(child)]
    pub hide_not_bound: Option<Flag>,
}

impl MergeWith<HotkeyOverlayPart> for HotkeyOverlay {
    fn merge_with(&mut self, part: &HotkeyOverlayPart) {
        merge!((self, part), skip_at_startup, hide_not_bound);
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigNotification {
    pub disable_failed: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigNotificationPart {
    #[knuffel(child)]
    pub disable_failed: Option<Flag>,
}

impl MergeWith<ConfigNotificationPart> for ConfigNotification {
    fn merge_with(&mut self, part: &ConfigNotificationPart) {
        merge!((self, part), disable_failed);
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Clipboard {
    pub disable_primary: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardPart {
    #[knuffel(child)]
    pub disable_primary: Option<Flag>,
}

impl MergeWith<ClipboardPart> for Clipboard {
    fn merge_with(&mut self, part: &ClipboardPart) {
        merge!((self, part), disable_primary);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overview {
    pub zoom: f64,
    pub backdrop_color: Color,
    pub workspace_shadow: WorkspaceShadow,
}

impl Default for Overview {
    fn default() -> Self {
        Self {
            zoom: 0.5,
            backdrop_color: DEFAULT_BACKDROP_COLOR,
            workspace_shadow: WorkspaceShadow::default(),
        }
    }
}

#[derive(knuffel::Decode, Debug, Clone, Copy, PartialEq)]
pub struct OverviewPart {
    #[knuffel(child, unwrap(argument))]
    pub zoom: Option<FloatOrInt<0, 1>>,
    #[knuffel(child)]
    pub backdrop_color: Option<Color>,
    #[knuffel(child)]
    pub workspace_shadow: Option<WorkspaceShadowPart>,
}

impl MergeWith<OverviewPart> for Overview {
    fn merge_with(&mut self, part: &OverviewPart) {
        merge!((self, part), zoom, workspace_shadow);
        merge_clone!((self, part), backdrop_color);
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq, Eq)]
pub struct Environment(#[knuffel(children)] pub Vec<EnvironmentVariable>);

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentVariable {
    #[knuffel(node_name)]
    pub name: String,
    #[knuffel(argument)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XwaylandSatellite {
    pub off: bool,
    pub path: String,
}

impl Default for XwaylandSatellite {
    fn default() -> Self {
        Self {
            off: false,
            path: String::from("xwayland-satellite"),
        }
    }
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct XwaylandSatellitePart {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child, unwrap(argument))]
    pub path: Option<String>,
}

impl MergeWith<XwaylandSatellitePart> for XwaylandSatellite {
    fn merge_with(&mut self, part: &XwaylandSatellitePart) {
        self.off |= part.off;
        if part.on {
            self.off = false;
        }

        merge_clone!((self, part), path);
    }
}
