#[macro_use]
extern crate tracing;

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::FromRawFd;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::{env, mem};

use anyhow::{bail, Context as _};
use calloop::EventLoop;
use clap::{CommandFactory, Parser};
use clap_complete::Shell;
use clap_complete_nushell::Nushell;
use directories::ProjectDirs;
use niri::cli::{Cli, CompletionShell, Sub};
#[cfg(feature = "dbus")]
use niri::dbus;
use niri::ipc::client::handle_msg;
use niri::niri::State;
use niri::utils::spawning::{
    spawn, spawn_sh, store_and_increase_nofile_rlimit, CHILD_DISPLAY, CHILD_ENV,
    REMOVE_ENV_RUST_BACKTRACE, REMOVE_ENV_RUST_LIB_BACKTRACE,
};
use niri::utils::{cause_panic, expand_home, version, watcher, xwayland, IS_SYSTEMD_SERVICE};
use niri_config::{Config, ConfigPath, Xkb};
use niri_ipc::socket::SOCKET_PATH_ENV;
use sd_notify::NotifyState;
use smithay::input::keyboard::xkb;
use smithay::reexports::wayland_server::Display;
use tracing_subscriber::EnvFilter;

const DEFAULT_LOG_FILTER: &str = "niri=debug,smithay::backend::renderer::gles=error";

#[cfg(feature = "profile-with-tracy-allocations")]
#[global_allocator]
static GLOBAL: tracy_client::ProfiledAllocator<std::alloc::System> =
    tracy_client::ProfiledAllocator::new(std::alloc::System, 100);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set backtrace defaults if not set.
    if env::var_os("RUST_BACKTRACE").is_none() {
        env::set_var("RUST_BACKTRACE", "1");
        REMOVE_ENV_RUST_BACKTRACE.store(true, Ordering::Relaxed);
    }
    if env::var_os("RUST_LIB_BACKTRACE").is_none() {
        env::set_var("RUST_LIB_BACKTRACE", "0");
        REMOVE_ENV_RUST_LIB_BACKTRACE.store(true, Ordering::Relaxed);
    }

    let directives = env::var("RUST_LOG").unwrap_or_else(|_| DEFAULT_LOG_FILTER.to_owned());
    let env_filter = EnvFilter::builder().parse_lossy(directives);
    tracing_subscriber::fmt()
        .compact()
        .with_writer(io::stderr)
        .with_env_filter(env_filter)
        .with_ansi_sanitization(false)
        .init();

    if env::var_os("NOTIFY_SOCKET").is_some() {
        IS_SYSTEMD_SERVICE.store(true, Ordering::Relaxed);

        #[cfg(not(feature = "systemd"))]
        warn!(
            "running as a systemd service, but systemd support is compiled out. \
             Are you sure you did not forget to set `--features systemd`?"
        );
    }

    let cli = Cli::parse();

    if cli.session {
        // If we're starting as a session, assume that the intention is to start on a TTY unless
        // this is a WSL environment. Remove DISPLAY, WAYLAND_DISPLAY or WAYLAND_SOCKET from our
        // environment if they are set, since they will cause the winit backend to be selected
        // instead.
        if env::var_os("WSL_DISTRO_NAME").is_none() {
            if env::var_os("DISPLAY").is_some() {
                warn!("running as a session but DISPLAY is set, removing it");
                env::remove_var("DISPLAY");
            }
            if env::var_os("WAYLAND_DISPLAY").is_some() {
                warn!("running as a session but WAYLAND_DISPLAY is set, removing it");
                env::remove_var("WAYLAND_DISPLAY");
            }
            if env::var_os("WAYLAND_SOCKET").is_some() {
                warn!("running as a session but WAYLAND_SOCKET is set, removing it");
                env::remove_var("WAYLAND_SOCKET");
            }
        }

        // Set the current desktop for xdg-desktop-portal.
        env::set_var("XDG_CURRENT_DESKTOP", "niri");
        // Ensure the session type is set to Wayland for xdg-autostart and Qt apps.
        env::set_var("XDG_SESSION_TYPE", "wayland");
    }

    // Handle subcommands.
    if let Some(subcommand) = cli.subcommand {
        match subcommand {
            Sub::Validate { config } => {
                tracy_client::Client::start();

                let config = config_path(config).load().config?;
                validate_config(&config)?;
                info!("config is valid");
                return Ok(());
            }
            Sub::Msg { msg, json } => {
                handle_msg(msg, json)?;
                return Ok(());
            }
            Sub::Panic => cause_panic(),
            Sub::Completions { shell } => {
                match shell {
                    CompletionShell::Nushell => {
                        clap_complete::generate(
                            Nushell,
                            &mut Cli::command(),
                            "niri",
                            &mut io::stdout(),
                        );
                    }
                    other => {
                        let generator = Shell::try_from(other).unwrap();
                        clap_complete::generate(
                            generator,
                            &mut Cli::command(),
                            "niri",
                            &mut io::stdout(),
                        );
                    }
                }
                return Ok(());
            }
        }
    }

    // Needs to be done before starting Tracy, so that it applies to Tracy's threads.
    niri::utils::signals::block_early().unwrap();

    // Avoid starting Tracy for the `niri msg` code path since starting/stopping Tracy is a bit
    // slow.
    tracy_client::Client::start();

    info!("starting version {}", &version());

    // Load the config.
    let config_path = config_path(cli.config);
    env::remove_var("NIRI_CONFIG");
    let (config_created_at, config_load_result) = config_path.load_or_create();
    let config_errored = config_load_result.config.is_err();
    let mut config = config_load_result.config.unwrap_or_else(|err| {
        warn!("{err:?}");
        Config::load_default()
    });
    let config_includes = config_load_result.includes;

    let spawn_at_startup = mem::take(&mut config.spawn_at_startup);
    let spawn_sh_at_startup = mem::take(&mut config.spawn_sh_at_startup);
    *CHILD_ENV.write().unwrap() = mem::take(&mut config.environment);

    store_and_increase_nofile_rlimit();

    // Create the main event loop.
    let mut event_loop = EventLoop::<State>::try_new().unwrap();

    // Handle Ctrl+C and other signals.
    niri::utils::signals::listen(&event_loop.handle());

    // Create the compositor.
    let display = Display::new().unwrap();

    // Increase the buffer size so that it's harder to crash a frozen client with a 1000 Hz mouse.
    set_default_max_buffer_size(&display, 1024 * 1024);

    let mut state = State::new(
        config,
        event_loop.handle(),
        event_loop.get_signal(),
        display,
        false,
        true,
        cli.session,
    )
    .unwrap();

    // Set WAYLAND_DISPLAY for children.
    let socket_name = state.niri.socket_name.as_deref().unwrap();
    env::set_var("WAYLAND_DISPLAY", socket_name);
    info!(
        "listening on Wayland socket: {}",
        socket_name.to_string_lossy()
    );

    // Set NIRI_SOCKET for children.
    if let Some(ipc) = &state.niri.ipc_server {
        let socket_path = ipc.socket_path.as_deref().unwrap();
        env::set_var(SOCKET_PATH_ENV, socket_path);
        info!("IPC listening on: {}", socket_path.to_string_lossy());
    }

    // Setup xwayland-satellite integration.
    xwayland::satellite::setup(&mut state);
    if let Some(satellite) = &state.niri.satellite {
        let name = satellite.display_name();
        *CHILD_DISPLAY.write().unwrap() = Some(name.to_owned());
        env::set_var("DISPLAY", name);
        info!("listening on X11 socket: {name}");
    } else {
        // Avoid spawning children in the host X11.
        env::remove_var("DISPLAY");
    }

    if cli.session {
        // We're starting as a session. Import our variables.
        import_environment();

        // Inhibit power key handling so we can suspend on it.
        #[cfg(feature = "dbus")]
        if !state.niri.config.borrow().input.disable_power_key_handling {
            if let Err(err) = state.niri.inhibit_power_key() {
                warn!("error inhibiting power key: {err:?}");
            }
        }
    }

    #[cfg(feature = "dbus")]
    dbus::DBusServers::start(&mut state, cli.session);

    #[cfg(feature = "dbus")]
    if cli.session {
        state.niri.a11y.start();
    }

    if env::var_os("NIRI_DISABLE_SYSTEM_MANAGER_NOTIFY").is_none_or(|x| x != "1") {
        // Notify systemd we're ready.
        if let Err(err) = sd_notify::notify(&[NotifyState::Ready]) {
            warn!("error notifying systemd: {err:?}");
        };

        // Send ready notification to the NOTIFY_FD file descriptor.
        if let Err(err) = notify_fd() {
            warn!("error notifying fd: {err:?}");
        }
    }

    watcher::setup(&mut state, &config_path, config_includes);

    // Spawn commands from cli and auto-start.
    spawn(cli.command, None);

    for elem in spawn_at_startup {
        spawn(elem.command, None);
    }
    for elem in spawn_sh_at_startup {
        spawn_sh(elem.command, None);
    }

    // Show the config error notification right away if needed.
    if config_errored {
        state.niri.config_error_notification.show();
        state.ipc_config_loaded(true);
    } else if let Some(path) = config_created_at {
        state.niri.config_error_notification.show_created(path);
    }

    // Run the compositor.
    event_loop
        .run(None, &mut state, |state| state.refresh_and_flush_clients())
        .unwrap();

    Ok(())
}

fn import_environment() {
    let variables = import_environment_variables().join(" ");
    let shell_command = import_environment_shell_command(&variables);

    let rv = Command::new("/bin/sh").args(["-c", &shell_command]).spawn();
    // Wait for the import process to complete, otherwise services will start too fast without
    // environment variables available.
    match rv {
        Ok(mut child) => match child.wait() {
            Ok(status) => {
                if !status.success() {
                    warn!("import environment shell exited with {status}");
                }
            }
            Err(err) => {
                warn!("error waiting for import environment shell: {err:?}");
            }
        },
        Err(err) => {
            warn!("error spawning shell to import environment: {err:?}");
        }
    }
}

fn import_environment_variables() -> [&'static str; 5] {
    [
        "WAYLAND_DISPLAY",
        "DISPLAY",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_TYPE",
        SOCKET_PATH_ENV,
    ]
}

fn import_environment_shell_command(variables: &str) -> String {
    let mut commands = Vec::new();
    if cfg!(feature = "systemd") {
        commands.push(format!(
            "if hash systemctl 2>/dev/null; then systemctl --user import-environment {variables}; fi"
        ));
    }
    if cfg!(feature = "dinit") {
        commands.push(format!(
            "if hash dinitctl 2>/dev/null; then dinitctl setenv {variables}; fi"
        ));
    }
    commands.push(format!(
        "if hash dbus-update-activation-environment 2>/dev/null; then dbus-update-activation-environment {variables}; fi"
    ));
    commands.join("; ")
}

fn env_config_path() -> Option<PathBuf> {
    env::var_os("NIRI_CONFIG")
        .filter(|x| !x.is_empty())
        .map(PathBuf::from)
}

fn default_config_path() -> Option<PathBuf> {
    let Some(dirs) = ProjectDirs::from("", "", "niri") else {
        warn!("error retrieving home directory");
        return None;
    };

    let mut path = dirs.config_dir().to_owned();
    path.push("config.kdl");
    Some(path)
}

fn system_config_path() -> PathBuf {
    PathBuf::from("/etc/niri/config.kdl")
}

fn config_path(cli_path: Option<PathBuf>) -> ConfigPath {
    if let Some(explicit) = cli_path.or_else(env_config_path) {
        return ConfigPath::Explicit(explicit);
    }

    let system_path = system_config_path();

    if let Some(user_path) = default_config_path() {
        ConfigPath::Regular {
            user_path,
            system_path,
        }
    } else {
        // Couldn't find the home directory, or whatever.
        ConfigPath::Explicit(system_path)
    }
}

fn validate_config(config: &Config) -> anyhow::Result<()> {
    validate_xkb_config(&config.input.keyboard.xkb)
}

fn validate_xkb_config(xkb_config: &Xkb) -> anyhow::Result<()> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);

    if let Some(file) = &xkb_config.file {
        validate_xkb_file(file, &context)?;
        return Ok(());
    }

    validate_xkb_name("rules", &xkb_config.rules)?;
    validate_xkb_name("model", &xkb_config.model)?;
    validate_xkb_name("layout", &xkb_config.layout)?;
    validate_xkb_name("variant", &xkb_config.variant)?;
    if let Some(options) = &xkb_config.options {
        validate_xkb_name("options", options)?;
    }

    let keymap = xkb::Keymap::new_from_names(
        &context,
        &xkb_config.rules,
        &xkb_config.model,
        &xkb_config.layout,
        &xkb_config.variant,
        xkb_config.options.clone(),
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    );

    if keymap.is_none() {
        bail!(
            "invalid xkb config: rules {:?}, model {:?}, layout {:?}, variant {:?}, options {:?}",
            xkb_config.rules,
            xkb_config.model,
            xkb_config.layout,
            xkb_config.variant,
            xkb_config.options
        );
    }

    Ok(())
}

fn validate_xkb_name(name: &str, value: &str) -> anyhow::Result<()> {
    if value.contains('\0') {
        bail!("invalid xkb config: {name} contains a NUL byte");
    }

    Ok(())
}

fn validate_xkb_file(xkb_file: &str, context: &xkb::Context) -> anyhow::Result<()> {
    let path = PathBuf::from(xkb_file);
    let path = expand_home(&path)
        .context("failed to expand ~ in xkb_file")?
        .unwrap_or(path);

    let keymap = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read xkb_file {:?}", path))?;

    if xkb::Keymap::new_from_string(
        context,
        keymap,
        xkb::KEYMAP_FORMAT_TEXT_V1,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .is_none()
    {
        bail!("invalid xkb_file {:?}", path);
    }

    Ok(())
}

fn notify_fd() -> anyhow::Result<()> {
    let fd = match env::var("NOTIFY_FD") {
        Ok(notify_fd) => notify_fd.parse()?,
        Err(env::VarError::NotPresent) => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    env::remove_var("NOTIFY_FD");
    let mut notif = unsafe { File::from_raw_fd(fd) };
    notif.write_all(b"READY=1\n")?;
    Ok(())
}

// The wayland-server crate has set_default_max_buffer_size() under a libwayland_1_23 feature, but
// this hard-requires libwayland-server >= 1.23 which is not present on e.g. Ubuntu 24.04. Since
// calling this is an optional enhancement, do it optionally at runtime.
fn set_default_max_buffer_size(display: &Display<State>, size: usize) {
    use std::ffi::c_void;

    unsafe {
        // RTLD_NOLOAD ensures we only get a handle to the libwayland-server that wayland-rs has
        // already loaded into this process, rather than potentially pulling in a different copy.
        let lib = libc::dlopen(
            c"libwayland-server.so.0".as_ptr(),
            libc::RTLD_LAZY | libc::RTLD_NOLOAD,
        );
        if lib.is_null() {
            // It's not really expected that this can happen, maybe if some distro changes the
            // library name?
            warn!("cannot set default max buffer size: libwayland-server.so.0 is not loaded");
            return;
        }

        let sym = libc::dlsym(lib, c"wl_display_set_default_max_buffer_size".as_ptr());
        if sym.is_null() {
            // Expected on libwayland-server < 1.23.
            trace!("wl_display_set_default_max_buffer_size is missing; skipping");
        } else {
            let func: unsafe extern "C" fn(*mut c_void, libc::size_t) = std::mem::transmute(sym);
            let display_ptr = display.handle().backend_handle().display_ptr();
            func(display_ptr.cast(), size);
        }

        libc::dlclose(lib);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_test_keymap() -> anyhow::Result<String> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let Some(keymap) = xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            "us",
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        ) else {
            bail!("failed to compile valid test keymap");
        };

        Ok(keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1))
    }

    #[test]
    fn import_environment_shell_guards_systemctl_when_systemd_feature_enabled() {
        let command = import_environment_shell_command("WAYLAND_DISPLAY NIRI_SOCKET");

        if cfg!(feature = "systemd") {
            assert!(command.contains("if hash systemctl 2>/dev/null; then"));
            assert!(command.contains("systemctl --user import-environment"));
        }
        assert!(command.contains("if hash dbus-update-activation-environment 2>/dev/null; then"));
    }

    #[test]
    fn import_environment_variables_include_ipc_socket_when_importing_session() {
        let variables = import_environment_variables();

        assert_eq!(variables[0], "WAYLAND_DISPLAY");
        assert!(variables.contains(&SOCKET_PATH_ENV));
    }

    #[test]
    fn validate_config_rejects_invalid_xkb_layout() {
        let mut config = Config::default();
        config.input.keyboard.xkb.layout = "en".to_owned();

        let Err(err) = validate_config(&config) else {
            panic!("invalid xkb layout should fail validation");
        };
        let err = err.to_string();

        assert!(err.contains("invalid xkb config"));
        assert!(err.contains("layout \"en\""));
    }

    #[test]
    fn validate_config_uses_xkb_file_instead_of_rules_config() -> anyhow::Result<()> {
        let path = env::temp_dir().join(format!(
            "niri-test-keymap-{}-{}.xkb",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        std::fs::write(&path, valid_test_keymap()?)?;

        let mut config = Config::default();
        config.input.keyboard.xkb.layout = "en".to_owned();
        config.input.keyboard.xkb.file = Some(path.to_string_lossy().into_owned());

        let result = validate_config(&config);
        std::fs::remove_file(&path)?;

        result
    }
}
