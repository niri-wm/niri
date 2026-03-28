use fixture::Fixture;

mod client;
mod fixture;
mod server;

mod animations;
#[cfg(feature = "dbus")]
mod appearance_rules;
mod floating;
mod fullscreen;
mod layer_shell;
mod transactions;
mod window_opening;
