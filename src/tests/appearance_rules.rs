use niri_config::{AppearanceSettings, ColorScheme, Config};
use smithay::backend::renderer::Color32F;

use super::fixture::Fixture;
use crate::dbus::freedesktop_portal_settings::PortalSettingsToNiri;

fn parse_config(text: &str) -> Config {
    Config::parse_mem(text).unwrap()
}

#[test]
fn portal_update_applies_and_reverts_config() {
    let config = parse_config(
        r##"
        overview {
            backdrop-color "#112233"
        }

        appearance-rule {
            match color-scheme="dark"

            overview {
                backdrop-color "#445566"
            }
        }
        "##,
    );

    let mut fixture = Fixture::with_config(config);

    let dark = AppearanceSettings {
        color_scheme: Some(ColorScheme::Dark),
        ..Default::default()
    };
    fixture
        .niri_state()
        .on_portal_settings_msg(PortalSettingsToNiri::AppearanceChanged(dark));
    let config = fixture.niri().config.borrow();
    assert_eq!(config.overview.backdrop_color, "#445566".parse().unwrap());
    drop(config);

    let light = AppearanceSettings {
        color_scheme: Some(ColorScheme::Light),
        ..Default::default()
    };
    fixture
        .niri_state()
        .on_portal_settings_msg(PortalSettingsToNiri::AppearanceChanged(light));
    let config = fixture.niri().config.borrow();
    assert_eq!(config.overview.backdrop_color, "#112233".parse().unwrap());
}

#[test]
fn reload_config_uses_current_portal_state() {
    let mut fixture = Fixture::new();

    fixture.niri().portal_appearance = AppearanceSettings {
        color_scheme: Some(ColorScheme::Dark),
        ..Default::default()
    };

    let config = parse_config(
        r##"
        overview {
            backdrop-color "#112233"
        }

        appearance-rule {
            match color-scheme="dark"

            overview {
                backdrop-color "#445566"
            }
        }
        "##,
    );

    fixture.niri_state().reload_config(Ok(config));
    let config = fixture.niri().config.borrow();
    assert_eq!(config.overview.backdrop_color, "#445566".parse().unwrap());
    drop(config);

    fixture.niri().portal_appearance = AppearanceSettings {
        color_scheme: Some(ColorScheme::Light),
        ..Default::default()
    };

    let config = parse_config(
        r##"
        overview {
            backdrop-color "#112233"
        }

        appearance-rule {
            match color-scheme="dark"

            overview {
                backdrop-color "#445566"
            }
        }
        "##,
    );

    fixture.niri_state().reload_config(Ok(config));
    let config = fixture.niri().config.borrow();
    assert_eq!(config.overview.backdrop_color, "#112233".parse().unwrap());
}

#[test]
fn backdrop_buffer_updates_on_portal_change() {
    let config = parse_config(
        r##"
        overview {
            backdrop-color "#112233"
        }

        appearance-rule {
            match color-scheme="dark"

            overview {
                backdrop-color "#445566"
            }
        }
        "##,
    );

    let mut fixture = Fixture::with_config(config);
    fixture.add_output(1, (800, 600));

    let output = fixture.niri_output(1);

    let base = "#112233".parse::<niri_config::Color>().unwrap();
    let mut base_arr = base.to_array_unpremul();
    base_arr[3] = 1.0;
    let base = Color32F::from(base_arr);

    let state = fixture.niri().output_state.get(&output).unwrap();
    assert_eq!(state.backdrop_buffer.color(), base);

    fixture
        .niri_state()
        .on_portal_settings_msg(PortalSettingsToNiri::AppearanceChanged(
            AppearanceSettings {
                color_scheme: Some(ColorScheme::Dark),
                ..Default::default()
            },
        ));

    let dark = "#445566".parse::<niri_config::Color>().unwrap();
    let mut dark_arr = dark.to_array_unpremul();
    dark_arr[3] = 1.0;
    let dark = Color32F::from(dark_arr);

    let state = fixture.niri().output_state.get(&output).unwrap();
    assert_eq!(state.backdrop_buffer.color(), dark);

    fixture
        .niri_state()
        .on_portal_settings_msg(PortalSettingsToNiri::AppearanceChanged(
            AppearanceSettings {
                color_scheme: Some(ColorScheme::Light),
                ..Default::default()
            },
        ));
    let state = fixture.niri().output_state.get(&output).unwrap();
    assert_eq!(state.backdrop_buffer.color(), base);

    // Ensure per-output override wins after a portal update.
    fixture
        .niri_state()
        .modify_output_config("headless-1", |output| {
            output.backdrop_color = Some("#010203".parse().unwrap());
        });
    fixture
        .niri_state()
        .on_portal_settings_msg(PortalSettingsToNiri::AppearanceChanged(
            AppearanceSettings {
                color_scheme: Some(ColorScheme::Dark),
                ..Default::default()
            },
        ));

    let override_color = "#010203".parse::<niri_config::Color>().unwrap();
    let mut override_arr = override_color.to_array_unpremul();
    override_arr[3] = 1.0;
    let override_color = Color32F::from(override_arr);

    let state = fixture.niri().output_state.get(&output).unwrap();
    assert_eq!(state.backdrop_buffer.color(), override_color);
}
