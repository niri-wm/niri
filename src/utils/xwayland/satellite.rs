use std::os::fd::{AsRawFd as _, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use calloop::channel::Sender;
use calloop::generic::Generic;
use calloop::{Interest, Mode, PostAction, RegistrationToken};
use smithay::reexports::rustix::io::{fcntl_setfd, FdFlags};

use crate::niri::State;
use crate::utils::expand_home;
use crate::utils::xwayland::X11Connection;

pub struct Satellite {
    x11: X11Connection,
    abstract_token: Option<RegistrationToken>,
    unix_token: Option<RegistrationToken>,
    to_main: Sender<ToMain>,
}

enum ToMain {
    SetupWatch,
}

impl Satellite {
    pub fn display_name(&self) -> &str {
        &self.x11.display_name
    }
}

pub fn setup(state: &mut State) {
    if state.niri.satellite.is_some() {
        return;
    }

    let config = state.niri.config.borrow();
    let xwls_config = &config.xwayland_satellite;
    if xwls_config.off {
        return;
    }

    if !test_ondemand(&xwls_config.path) {
        return;
    }
    drop(config);

    let x11 = match super::setup_connection() {
        Ok(x11) => x11,
        Err(err) => {
            warn!("error opening X11 sockets, disabling xwayland-satellite integration: {err:?}");
            return;
        }
    };

    let event_loop = &state.niri.event_loop;
    let (to_main, rx) = calloop::channel::channel();
    event_loop
        .insert_source(rx, move |event, _, state| match event {
            calloop::channel::Event::Msg(msg) => match msg {
                ToMain::SetupWatch => setup_watch(state),
            },
            calloop::channel::Event::Closed => (),
        })
        .unwrap();

    state.niri.satellite = Some(Satellite {
        x11,
        abstract_token: None,
        unix_token: None,
        to_main,
    });

    setup_watch(state);
}

fn test_ondemand(path: &str) -> bool {
    let _span = tracy_client::span!("satellite::test_ondemand");

    // Expand `~` at the start.
    let mut path = Path::new(path);
    let expanded = expand_home(path);
    match &expanded {
        Ok(Some(expanded)) => path = expanded.as_ref(),
        Ok(None) => (),
        Err(err) => {
            warn!("error expanding ~: {err:?}");
        }
    }

    let mut process = Command::new(path);
    process
        .args([":0", "--test-listenfd-support"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_remove("DISPLAY")
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE");

    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(err) => {
            warn!("error spawning xwayland-satellite at {path:?}, disabling integration: {err}");
            return false;
        }
    };

    let status = match child.wait() {
        Ok(status) => status,
        Err(err) => {
            warn!("error waiting for xwayland-satellite, disabling integration: {err}");
            return false;
        }
    };

    if !status.success() {
        warn!("xwayland-satellite doesn't support on-demand activation, disabling integration");
        return false;
    }

    true
}

// When xwayland-satellite fails to start and accept a connection on the socket, the socket will
// keep triggering our event source, even after the X11 client quits, resulting in a busyloop of
// trying to start xwayland-satellite. This function will clear out (accept and drop) all pending
// connections on the socket before registering a new event source, working around this problem.
// When the problem happens, it's very likely that xwayland-satellite won't be able to accept the
// pending client (since it had just failed to do so), so it's fine to drop the connections.
fn clear_out_pending_connections(fd: OwnedFd) -> OwnedFd {
    let listener = UnixListener::from(fd);

    if let Err(err) = listener.set_nonblocking(true) {
        warn!("error setting X11 socket to nonblocking: {err:?}");
        return OwnedFd::from(listener);
    }

    while listener.accept().is_ok() {}

    if let Err(err) = listener.set_nonblocking(false) {
        warn!("error setting X11 socket to blocking: {err:?}");
    }

    OwnedFd::from(listener)
}

fn setup_watch(state: &mut State) {
    let Some(satellite) = state.niri.satellite.as_mut() else {
        return;
    };

    let event_loop = &state.niri.event_loop;

    if let Some(token) = satellite.abstract_token.take() {
        error!("abstract_token must be None in setup_watch()");
        event_loop.remove(token);
    }
    if let Some(token) = satellite.unix_token.take() {
        error!("unix_token must be None in setup_watch()");
        event_loop.remove(token);
    }

    if let Some(fd) = &satellite.x11.abstract_fd {
        let fd = fd.try_clone().unwrap();
        let fd = clear_out_pending_connections(fd);
        let source = Generic::new(fd, Interest::READ, Mode::Level);
        let token = event_loop
            .insert_source(source, move |_, _, state| {
                if let Some(satellite) = &mut state.niri.satellite {
                    // Remove the other source.
                    if let Some(token) = satellite.unix_token.take() {
                        state.niri.event_loop.remove(token);
                    }
                    // Clear this source.
                    satellite.abstract_token = None;

                    debug!("connection to X11 abstract socket; spawning xwayland-satellite");
                    let path = state.niri.config.borrow().xwayland_satellite.path.clone();
                    spawn(path, satellite);
                }
                Ok(PostAction::Remove)
            })
            .unwrap();
        satellite.abstract_token = Some(token);
    }

    let fd = satellite.x11.unix_fd.try_clone().unwrap();
    let fd = clear_out_pending_connections(fd);
    let source = Generic::new(fd, Interest::READ, Mode::Level);
    let token = event_loop
        .insert_source(source, move |_, _, state| {
            if let Some(satellite) = &mut state.niri.satellite {
                // Remove the other source.
                if let Some(token) = satellite.abstract_token.take() {
                    state.niri.event_loop.remove(token);
                }
                // Clear this source.
                satellite.unix_token = None;

                debug!("connection to X11 unix socket; spawning xwayland-satellite");
                let path = state.niri.config.borrow().xwayland_satellite.path.clone();
                spawn(path, satellite);
            }
            Ok(PostAction::Remove)
        })
        .unwrap();
    satellite.unix_token = Some(token);
}

fn spawn(path: String, xwl: &Satellite) {
    let _span = tracy_client::span!("satellite::spawn");

    let abstract_fd = xwl
        .x11
        .abstract_fd
        .as_ref()
        .map(|fd| fd.try_clone().unwrap());
    let unix_fd = xwl.x11.unix_fd.try_clone().unwrap();
    let to_main = xwl.to_main.clone();

    // Expand `~` at the start.
    let mut path = PathBuf::from(path);
    let expanded = expand_home(&path);
    match expanded {
        Ok(Some(expanded)) => path = expanded,
        Ok(None) => (),
        Err(err) => {
            warn!("error expanding ~: {err:?}");
        }
    }

    let mut process = Command::new(&path);
    process.arg(&xwl.x11.display_name).env_remove("DISPLAY");

    // We don't want it spamming the niri output.
    process
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE");
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    unsafe { process.pre_exec(crate::utils::signals::unblock_all) };

    // Spawning and waiting takes some milliseconds, so do it in a thread.
    let res = thread::Builder::new()
        .name("Xwl-s Spawner".to_owned())
        .spawn(move || {
            spawn_and_wait(&path, process, abstract_fd, unix_fd);

            // Once xwayland-satellite crashes or fails to spawn, re-establish our X11 socket watch
            // to try again next time.
            let _ = to_main.send(ToMain::SetupWatch);
        });

    if let Err(err) = res {
        warn!("error spawning a thread to spawn xwayland-satellite: {err:?}");
        let _ = xwl.to_main.send(ToMain::SetupWatch);
    }
}

fn spawn_and_wait(
    path: &Path,
    mut process: Command,
    abstract_fd: Option<OwnedFd>,
    unix_fd: OwnedFd,
) {
    let abstract_raw = abstract_fd.as_ref().map(|fd| fd.as_raw_fd());
    let unix_raw = unix_fd.as_raw_fd();

    process.arg("-listenfd").arg(unix_raw.to_string());

    if let Some(abstract_raw) = abstract_raw {
        process.arg("-listenfd").arg(abstract_raw.to_string());
    }

    unsafe {
        process.pre_exec(move || {
            // We're about to exec xwl-s; perfect time to clear CLOEXEC on the file descriptors
            // that we want to pass it.

            // We're not dropping these until after spawn().
            let unix_fd = BorrowedFd::borrow_raw(unix_raw);
            fcntl_setfd(unix_fd, FdFlags::empty())?;

            if let Some(abstract_raw) = abstract_raw {
                let abstract_fd = BorrowedFd::borrow_raw(abstract_raw);
                fcntl_setfd(abstract_fd, FdFlags::empty())?;
            }

            Ok(())
        })
    };

    let mut child = {
        let _span = tracy_client::span!();
        match process.spawn() {
            Ok(child) => child,
            Err(err) => {
                warn!("error spawning {path:?}: {err:?}");
                return;
            }
        }
    };

    // The process spawned, we can drop our fds.
    drop(abstract_fd);
    drop(unix_fd);

    let status = match child.wait() {
        Ok(status) => status,
        Err(err) => {
            warn!("error waiting for xwayland-satellite: {err:?}");
            return;
        }
    };

    // This is most likely a crash, hence warn!().
    warn!("xwayland-satellite exited with: {status}");
}
