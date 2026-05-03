#[macro_use]
extern crate tracing;

#[cfg(feature = "dbus")]
pub mod a11y;
pub mod animation;
pub mod backend;
pub mod cli;
pub mod cursor;
#[cfg(feature = "dbus")]
pub mod dbus;
pub mod frame_clock;
pub mod handlers;
pub mod input;
pub mod ipc;
pub mod layer;
pub mod layout;
pub mod niri;
pub mod protocols;
pub mod render_helpers;
pub mod rubber_band;
#[cfg(feature = "xdp-gnome-screencast")]
pub mod screencasting;
pub mod ui;
pub mod utils;
pub mod window;

#[cfg(test)]
mod tests;
