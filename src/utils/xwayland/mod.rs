use std::os::fd::OwnedFd;
use std::os::unix::net::{SocketAddr, UnixListener};

use anyhow::{anyhow, ensure, Context as _};
use rustix::fs::{lstat, mkdir};
use rustix::io::Errno;
use rustix::process::getuid;
use smithay::reexports::rustix::fs::{unlink, OFlags};
use smithay::reexports::rustix::process::getpid;
use smithay::reexports::rustix::{self};

pub mod satellite;

const TMP_UNIX_DIR: &str = "/tmp";
const X11_TMP_UNIX_DIR: &str = "/tmp/.X11-unix";

struct X11Connection {
    display_name: String,
    // Optional because there are no abstract sockets on FreeBSD.
    abstract_fd: Option<OwnedFd>,
    unix_fd: OwnedFd,
    _unix_guard: Unlink,
    _lock_guard: Unlink,
}

struct Unlink(String);
impl Drop for Unlink {
    fn drop(&mut self) {
        let _ = unlink(&self.0);
    }
}

// Adapted from Mutter code:
// https://gitlab.gnome.org/GNOME/mutter/-/blob/48.3.1/src/wayland/meta-xwayland.c?ref_type=tags#L513
fn ensure_x11_unix_dir() -> anyhow::Result<()> {
    match mkdir(X11_TMP_UNIX_DIR, 0o1777.into()) {
        Ok(()) => Ok(()),
        Err(Errno::EXIST) => {
            ensure_x11_unix_perms().context("wrong X11 directory permissions")?;
            Ok(())
        }
        Err(err) => Err(err).context("error creating X11 directory"),
    }
}

fn ensure_x11_unix_perms() -> anyhow::Result<()> {
    let x11_tmp = lstat(X11_TMP_UNIX_DIR).context("error checking X11 directory permissions")?;
    let tmp = lstat(TMP_UNIX_DIR).context("error checking /tmp directory permissions")?;

    ensure!(
        x11_tmp.st_uid == tmp.st_uid || x11_tmp.st_uid == getuid().as_raw(),
        "wrong ownership for X11 directory"
    );
    ensure!(
        (x11_tmp.st_mode & 0o022) == 0o022,
        "X11 directory is not writable"
    );
    ensure!(
        (x11_tmp.st_mode & 0o1000) == 0o1000,
        "X11 directory is missing the sticky bit"
    );

    Ok(())
}

fn pick_x11_display(start: u32) -> anyhow::Result<(u32, OwnedFd, Unlink)> {
    for n in start..start + 50 {
        let lock_path = format!("/tmp/.X{n}-lock");
        let flags = OFlags::WRONLY | OFlags::CLOEXEC | OFlags::CREATE | OFlags::EXCL;
        let Ok(lock_fd) = rustix::fs::open(&lock_path, flags, 0o444.into()) else {
            // FIXME: check if the target process is dead and reuse the lock.
            continue;
        };
        return Ok((n, lock_fd, Unlink(lock_path)));
    }

    Err(anyhow!("no free X11 display found after 50 attempts"))
}

fn bind_to_socket(addr: &SocketAddr) -> anyhow::Result<UnixListener> {
    let listener = UnixListener::bind_addr(addr).context("error binding socket")?;
    Ok(listener)
}

#[cfg(target_os = "linux")]
fn bind_to_abstract_socket(display: u32) -> anyhow::Result<UnixListener> {
    use std::os::linux::net::SocketAddrExt;

    let name = format!("/tmp/.X11-unix/X{display}");
    let addr = SocketAddr::from_abstract_name(name).unwrap();
    bind_to_socket(&addr)
}

fn bind_to_unix_socket(display: u32) -> anyhow::Result<(UnixListener, Unlink)> {
    let name = format!("/tmp/.X11-unix/X{display}");
    let addr = SocketAddr::from_pathname(&name).unwrap();
    // Unlink old leftover socket if any.
    let _ = unlink(&name);
    let guard = Unlink(name);
    bind_to_socket(&addr).map(|listener| (listener, guard))
}

fn open_display_sockets(
    display: u32,
) -> anyhow::Result<(Option<UnixListener>, UnixListener, Unlink)> {
    #[cfg(target_os = "linux")]
    let a = Some(bind_to_abstract_socket(display).context("error binding to abstract socket")?);
    #[cfg(not(target_os = "linux"))]
    let a = None;

    let (u, g) = bind_to_unix_socket(display).context("error binding to unix socket")?;
    Ok((a, u, g))
}

fn setup_connection() -> anyhow::Result<X11Connection> {
    let _span = tracy_client::span!("open_x11_sockets");

    ensure_x11_unix_dir()?;

    let mut n = 0;
    let mut attempt = 0;
    let (display, lock_guard, a, u, unix_guard) = loop {
        let (display, lock_fd, lock_guard) = pick_x11_display(n)?;

        // Write our PID into the lock file.
        let pid_string = format!("{:>10}\n", getpid().as_raw_nonzero());
        if let Err(err) = rustix::io::write(&lock_fd, pid_string.as_bytes()) {
            return Err(err).context("error writing PID to X11 lock file");
        }
        drop(lock_fd);

        match open_display_sockets(display) {
            Ok((a, u, g)) => {
                break (display, lock_guard, a, u, g);
            }
            Err(err) => {
                if attempt == 50 {
                    return Err(err)
                        .context("error opening X11 sockets after creating a lock file");
                }

                n = display + 1;
                attempt += 1;
                continue;
            }
        }
    };

    let display_name = format!(":{display}");
    let abstract_fd = a.map(OwnedFd::from);
    let unix_fd = OwnedFd::from(u);

    Ok(X11Connection {
        display_name,
        abstract_fd,
        unix_fd,
        _unix_guard: unix_guard,
        _lock_guard: lock_guard,
    })
}
