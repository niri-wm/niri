use smithay::reexports::input::Libinput;

/// Initializes the libinput plugin system.
///
/// # Safety
///
/// This function must be called before libinput iterates through the devices, i.e. before
/// `libinput_udev_assign_seat()` or the first call to `libinput_path_add_device()`.
#[allow(unused_variables)]
pub(super) unsafe fn init_libinput_plugin_system(libinput: &Libinput) {
    #[cfg(have_libinput_plugin_system)]
    unsafe {
        use std::ffi::{c_char, c_int, CString};
        use std::os::unix::ffi::OsStringExt;

        use directories::BaseDirs;
        use input::ffi::libinput;
        use input::AsRaw as _;

        extern "C" {
            fn libinput_plugin_system_append_path(libinput: *const libinput, path: *const c_char);
            fn libinput_plugin_system_append_default_paths(libinput: *const libinput);
            fn libinput_plugin_system_load_plugins(
                libinput: *const libinput,
                flags: c_int,
            ) -> c_int;
        }
        const LIBINPUT_PLUGIN_SYSTEM_FLAG_NONE: c_int = 0;
        let libinput = libinput.as_raw();

        // Also load plugins from $XDG_CONFIG_HOME/libinput/plugins.
        if let Some(dirs) = BaseDirs::new() {
            let mut plugins_dir = dirs.config_dir().to_path_buf();
            plugins_dir.push("libinput");
            plugins_dir.push("plugins");
            if let Ok(plugins_dir) = CString::new(plugins_dir.into_os_string().into_vec()) {
                libinput_plugin_system_append_path(libinput, plugins_dir.as_ptr());
            }
        }

        libinput_plugin_system_append_default_paths(libinput);
        libinput_plugin_system_load_plugins(libinput, LIBINPUT_PLUGIN_SYSTEM_FLAG_NONE);
    }

    // When libinput's plugin system isn't available, this function is intentionally a no-op.
    // We keep an explicit reference so the intent is clear under that cfg.
    #[cfg(not(have_libinput_plugin_system))]
    let _ = libinput;
}
