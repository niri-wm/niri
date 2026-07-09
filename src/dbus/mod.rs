use zbus::blocking::Connection;
use zbus::object_server::Interface;

use crate::niri::State;

pub mod freedesktop_a11y;
pub mod freedesktop_locale1;
pub mod freedesktop_login1;
pub mod freedesktop_screensaver;
pub mod gnome_shell_introspect;
pub mod gnome_shell_screenshot;
pub mod mutter_display_config;
pub mod mutter_service_channel;

#[cfg(feature = "xdp-gnome-screencast")]
pub mod mutter_screen_cast;
#[cfg(feature = "xdp-gnome-screencast")]
use mutter_screen_cast::ScreenCast;

use self::freedesktop_a11y::KeyboardMonitor;
use self::freedesktop_screensaver::ScreenSaver;
use self::gnome_shell_introspect::Introspect;
use self::mutter_display_config::DisplayConfig;
use self::mutter_service_channel::ServiceChannel;

trait Start: Interface {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection>;
}

#[derive(Default)]
pub struct DBusServers {
    pub conn_service_channel: Option<Connection>,
    pub conn_display_config: Option<Connection>,
    pub conn_screen_saver: Option<Connection>,
    pub conn_screen_shot: Option<Connection>,
    pub conn_introspect: Option<Connection>,
    #[cfg(feature = "xdp-gnome-screencast")]
    pub conn_screen_cast: Option<Connection>,
    pub conn_login1: Option<Connection>,
    pub conn_locale1: Option<Connection>,
    pub conn_keyboard_monitor: Option<Connection>,
}

impl DBusServers {
    pub fn start(state: &mut State, is_session_instance: bool) {
        let _span = tracy_client::span!("DBusServers::start");

        let backend = &state.backend;
        let niri = &mut state.niri;
        let config = niri.config.borrow();

        let mut dbus = Self::default();

        if is_session_instance {
            let (to_niri, from_service_channel) = calloop::channel::channel();
            let service_channel = ServiceChannel::new(to_niri);
            niri.event_loop
                .insert_source(from_service_channel, move |event, _, state| match event {
                    calloop::channel::Event::Msg(new_client) => {
                        state.niri.insert_client(new_client);
                    }
                    calloop::channel::Event::Closed => (),
                })
                .unwrap();
            dbus.conn_service_channel = try_start(service_channel);
        }

        if is_session_instance || config.debug.dbus_interfaces_in_non_session_instances {
            let (to_niri, from_display_config) = calloop::channel::channel();
            let display_config = DisplayConfig::new(to_niri, backend.ipc_outputs());
            niri.event_loop
                .insert_source(from_display_config, move |event, _, state| match event {
                    calloop::channel::Event::Msg(new_conf) => {
                        for (name, conf) in new_conf {
                            state.modify_output_config(&name, move |output| {
                                if let Some(new_output) = conf {
                                    *output = new_output;
                                } else {
                                    output.off = true;
                                }
                            });
                        }
                        state.reload_output_config();
                    }
                    calloop::channel::Event::Closed => (),
                })
                .unwrap();
            dbus.conn_display_config = try_start(display_config);

            let screen_saver = ScreenSaver::new(niri.is_fdo_idle_inhibited.clone());
            dbus.conn_screen_saver = try_start(screen_saver);

            let (to_niri, from_screenshot) = calloop::channel::channel();
            let (to_screenshot, from_niri) = async_channel::unbounded();
            niri.event_loop
                .insert_source(from_screenshot, move |event, _, state| match event {
                    calloop::channel::Event::Msg(msg) => {
                        state.on_screen_shot_msg(&to_screenshot, msg)
                    }
                    calloop::channel::Event::Closed => (),
                })
                .unwrap();
            let screenshot = gnome_shell_screenshot::Screenshot::new(to_niri, from_niri);
            dbus.conn_screen_shot = try_start(screenshot);

            let (to_niri, from_introspect) = calloop::channel::channel();
            let (to_introspect, from_niri) = async_channel::unbounded();
            niri.event_loop
                .insert_source(from_introspect, move |event, _, state| match event {
                    calloop::channel::Event::Msg(msg) => {
                        state.on_introspect_msg(&to_introspect, msg)
                    }
                    calloop::channel::Event::Closed => (),
                })
                .unwrap();
            let introspect = Introspect::new(to_niri, from_niri);
            dbus.conn_introspect = try_start(introspect);

            #[cfg(feature = "xdp-gnome-screencast")]
            {
                let (to_niri, from_screen_cast) = calloop::channel::channel();
                niri.event_loop
                    .insert_source(from_screen_cast, {
                        move |event, _, state| match event {
                            calloop::channel::Event::Msg(msg) => state.on_screen_cast_msg(msg),
                            calloop::channel::Event::Closed => (),
                        }
                    })
                    .unwrap();
                let screen_cast = ScreenCast::new(backend.ipc_outputs(), to_niri);
                dbus.conn_screen_cast = try_start(screen_cast);
            }

            let keyboard_monitor = KeyboardMonitor::new();
            if let Some(x) = try_start(keyboard_monitor.clone()) {
                dbus.conn_keyboard_monitor = Some(x);
                niri.a11y_keyboard_monitor = Some(keyboard_monitor);
            }
        }

        let (to_niri, from_login1) = calloop::channel::channel();
        niri.event_loop
            .insert_source(from_login1, move |event, _, state| match event {
                calloop::channel::Event::Msg(msg) => state.on_login1_msg(msg),
                calloop::channel::Event::Closed => (),
            })
            .unwrap();
        match freedesktop_login1::start(to_niri) {
            Ok(conn) => {
                dbus.conn_login1 = Some(conn);
            }
            Err(err) => {
                warn!("error starting login1 watcher: {err:?}");
            }
        }

        let (to_niri, from_locale1) = calloop::channel::channel();
        niri.event_loop
            .insert_source(from_locale1, move |event, _, state| match event {
                calloop::channel::Event::Msg(msg) => state.on_locale1_msg(msg),
                calloop::channel::Event::Closed => (),
            })
            .unwrap();
        match freedesktop_locale1::start(to_niri) {
            Ok(conn) => {
                dbus.conn_locale1 = Some(conn);
            }
            Err(err) => {
                warn!("error starting locale1 watcher: {err:?}");
            }
        }

        niri.dbus = Some(dbus);
    }
}

fn try_start<I: Start>(iface: I) -> Option<Connection> {
    match iface.start() {
        Ok(conn) => Some(conn),
        Err(err) => {
            warn!("error starting {}: {err:?}", I::name());
            None
        }
    }
}
