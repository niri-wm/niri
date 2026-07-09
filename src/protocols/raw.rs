pub mod mutter_x11_interop {
    pub mod v1 {
        pub use self::generated::server;

        mod generated {
            pub mod server {
                #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
                #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
                #![allow(missing_docs, clippy::all)]

                use smithay::reexports::wayland_server;
                use wayland_server::protocol::*;

                pub mod __interfaces {
                    use smithay::reexports::wayland_server;
                    use wayland_server::protocol::__interfaces::*;
                    wayland_scanner::generate_interfaces!("resources/mutter-x11-interop.xml");
                }
                use self::__interfaces::*;

                wayland_scanner::generate_server_code!("resources/mutter-x11-interop.xml");
            }
        }
    }
}
