fn main() {
    println!("cargo:rustc-check-cfg=cfg(have_libinput_plugin_system)");
    println!("cargo:rustc-check-cfg=cfg(have_libinput_3fg_drag)");

    if pkg_config::Config::new()
        .atleast_version("1.28.0")
        .probe("libinput")
        .is_ok()
    {
        println!("cargo:rustc-cfg=have_libinput_3fg_drag")
    }

    if pkg_config::Config::new()
        .atleast_version("1.30.0")
        .probe("libinput")
        .is_ok()
    {
        println!("cargo:rustc-cfg=have_libinput_plugin_system")
    }
}
