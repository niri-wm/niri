use std::path::PathBuf;

use crate::utils::{Flag, MergeWith};

#[derive(Debug, Default, PartialEq)]
pub struct Debug {
    pub preview_render: Option<PreviewRender>,
    pub dbus_interfaces_in_non_session_instances: bool,
    pub wait_for_frame_completion_before_queueing: bool,
    pub enable_overlay_planes: bool,
    pub disable_cursor_plane: bool,
    pub disable_direct_scanout: bool,
    pub keep_max_bpc_unchanged: bool,
    pub restrict_primary_scanout_to_matching_format: bool,
    pub force_disable_connectors_on_resume: bool,
    pub render_drm_device: Option<PathBuf>,
    pub ignored_drm_devices: Vec<PathBuf>,
    pub force_pipewire_invalid_modifier: bool,
    pub emulate_zero_presentation_time: bool,
    pub disable_resize_throttling: bool,
    pub disable_transactions: bool,
    pub keep_laptop_panel_on_when_lid_is_closed: bool,
    pub disable_monitor_names: bool,
    pub strict_new_window_focus_policy: bool,
    pub honor_xdg_activation_with_invalid_serial: bool,
    pub deactivate_unfocused_windows: bool,
    pub skip_cursor_only_updates_during_vrr: bool,
}

#[derive(knuffel::Decode, Debug, Default, PartialEq)]
pub struct DebugPart {
    #[knuffel(child, unwrap(argument))]
    pub preview_render: Option<PreviewRender>,
    #[knuffel(child)]
    pub dbus_interfaces_in_non_session_instances: Option<Flag>,
    #[knuffel(child)]
    pub wait_for_frame_completion_before_queueing: Option<Flag>,
    #[knuffel(child)]
    pub enable_overlay_planes: Option<Flag>,
    #[knuffel(child)]
    pub disable_cursor_plane: Option<Flag>,
    #[knuffel(child)]
    pub disable_direct_scanout: Option<Flag>,
    #[knuffel(child)]
    pub keep_max_bpc_unchanged: Option<Flag>,
    #[knuffel(child)]
    pub restrict_primary_scanout_to_matching_format: Option<Flag>,
    #[knuffel(child)]
    pub force_disable_connectors_on_resume: Option<Flag>,
    #[knuffel(child, unwrap(argument))]
    pub render_drm_device: Option<PathBuf>,
    #[knuffel(children(name = "ignore-drm-device"), unwrap(argument))]
    pub ignored_drm_devices: Vec<PathBuf>,
    #[knuffel(child)]
    pub force_pipewire_invalid_modifier: Option<Flag>,
    #[knuffel(child)]
    pub emulate_zero_presentation_time: Option<Flag>,
    #[knuffel(child)]
    pub disable_resize_throttling: Option<Flag>,
    #[knuffel(child)]
    pub disable_transactions: Option<Flag>,
    #[knuffel(child)]
    pub keep_laptop_panel_on_when_lid_is_closed: Option<Flag>,
    #[knuffel(child)]
    pub disable_monitor_names: Option<Flag>,
    #[knuffel(child)]
    pub strict_new_window_focus_policy: Option<Flag>,
    #[knuffel(child)]
    pub honor_xdg_activation_with_invalid_serial: Option<Flag>,
    #[knuffel(child)]
    pub deactivate_unfocused_windows: Option<Flag>,
    #[knuffel(child)]
    pub skip_cursor_only_updates_during_vrr: Option<Flag>,
}

impl MergeWith<DebugPart> for Debug {
    fn merge_with(&mut self, part: &DebugPart) {
        merge!(
            (self, part),
            dbus_interfaces_in_non_session_instances,
            wait_for_frame_completion_before_queueing,
            enable_overlay_planes,
            disable_cursor_plane,
            disable_direct_scanout,
            keep_max_bpc_unchanged,
            restrict_primary_scanout_to_matching_format,
            force_disable_connectors_on_resume,
            force_pipewire_invalid_modifier,
            emulate_zero_presentation_time,
            disable_resize_throttling,
            disable_transactions,
            keep_laptop_panel_on_when_lid_is_closed,
            disable_monitor_names,
            strict_new_window_focus_policy,
            honor_xdg_activation_with_invalid_serial,
            deactivate_unfocused_windows,
            skip_cursor_only_updates_during_vrr,
        );

        merge_clone_opt!((self, part), preview_render, render_drm_device);

        self.ignored_drm_devices
            .extend(part.ignored_drm_devices.iter().cloned());
    }
}

#[derive(knuffel::DecodeScalar, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewRender {
    Screencast,
    ScreenCapture,
}
