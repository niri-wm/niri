use std::ffi::OsStr;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::{io, thread};

use anyhow::Context;
use atomic::Atomic;
use futures_util::AsyncWriteExt;
use libc::{getrlimit, rlim_t, rlimit, setrlimit, RLIMIT_NOFILE};
use niri_config::Environment;
use smithay::wayland::xdg_activation::XdgActivationToken;

use crate::utils::expand_home;

pub static REMOVE_ENV_RUST_BACKTRACE: AtomicBool = AtomicBool::new(false);
pub static REMOVE_ENV_RUST_LIB_BACKTRACE: AtomicBool = AtomicBool::new(false);
pub static CHILD_ENV: RwLock<Environment> = RwLock::new(Environment(Vec::new()));
pub static CHILD_DISPLAY: RwLock<Option<String>> = RwLock::new(None);

static ORIGINAL_NOFILE_RLIMIT_CUR: Atomic<rlim_t> = Atomic::new(0);
static ORIGINAL_NOFILE_RLIMIT_MAX: Atomic<rlim_t> = Atomic::new(0);

type Message = (Vec<String>, Option<XdgActivationToken>);
pub struct Spawner(async_channel::Sender<Message>);

impl Spawner {
    pub fn new() -> Spawner {
        let (sender, receiver) = async_channel::unbounded::<Message>();

        thread::Builder::new()
            .name("Command Spawner".to_owned())
            .spawn(move || {
                async_io::block_on(async {
                    let mut spawnserver = prepare_spawnserver();
                    while let Ok((command, token)) = receiver.recv().await {
                        let Some((command, args)) = command.split_first() else {
                            continue;
                        };
                        let spawnee = prepare_command(command, args, token);

                        let server = spawnserver.take();
                        if let Some(server) = server {
                            spawn_in_spawnserver(server, spawnee).await
                        } else {
                            just_spawn(spawnee)
                        };
                        spawnserver = prepare_spawnserver();
                    }
                });
            })
            .unwrap();

        Self(sender)
    }

    /// Spawns the command to run independently of the compositor.
    pub fn spawn(&self, command: Vec<String>, token: Option<XdgActivationToken>) {
        let _span = tracy_client::span!();

        if command.is_empty() {
            return;
        }

        if let Err(err) = self.0.send_blocking((command, token)) {
            warn!("error sending command to the spawning thread: {err:?}");
        }
    }

    /// Spawns the command through the shell.
    ///
    /// We hardcode `sh -c`, consistent with other compositors:
    ///
    /// - https://github.com/swaywm/sway/blob/b3dcde8d69c3f1304b076968a7a64f54d0c958be/sway/commands/exec_always.c#L64
    /// - https://github.com/hyprwm/Hyprland/blob/1ac1ff457ab8ef1ae6a8f2ab17ee7965adfa729f/src/managers/KeybindManager.cpp#L987
    pub fn spawn_sh(&self, command: String, token: Option<XdgActivationToken>) {
        self.spawn(vec![String::from("sh"), String::from("-c"), command], token);
    }
}

/// Increases the nofile rlimit to the maximum and stores the original value.
pub fn store_and_increase_nofile_rlimit() {
    let mut rlim = rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { getrlimit(RLIMIT_NOFILE, &mut rlim) } != 0 {
        let err = io::Error::last_os_error();
        warn!("error getting nofile rlimit: {err:?}");
        return;
    }

    ORIGINAL_NOFILE_RLIMIT_CUR.store(rlim.rlim_cur, Ordering::SeqCst);
    ORIGINAL_NOFILE_RLIMIT_MAX.store(rlim.rlim_max, Ordering::SeqCst);

    trace!(
        "changing nofile rlimit from {} to {}",
        rlim.rlim_cur,
        rlim.rlim_max
    );
    rlim.rlim_cur = rlim.rlim_max;

    if unsafe { setrlimit(RLIMIT_NOFILE, &rlim) } != 0 {
        let err = io::Error::last_os_error();
        warn!("error setting nofile rlimit: {err:?}");
    }
}

/// Restores the original nofile rlimit.
pub fn restore_nofile_rlimit() {
    let rlim_cur = ORIGINAL_NOFILE_RLIMIT_CUR.load(Ordering::SeqCst);
    let rlim_max = ORIGINAL_NOFILE_RLIMIT_MAX.load(Ordering::SeqCst);

    if rlim_cur == 0 {
        return;
    }

    let rlim = rlimit { rlim_cur, rlim_max };
    unsafe { setrlimit(RLIMIT_NOFILE, &rlim) };
}

fn just_spawn(mut command: async_process::Command) -> Option<async_process::Child> {
    match command.spawn() {
        Ok(child) => Some(child),
        Err(err) => {
            warn!("error spawning {command:?}: {err:?}");
            None
        }
    }
}

fn prepare_command(
    command: impl AsRef<OsStr>,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    token: Option<XdgActivationToken>,
) -> async_process::Command {
    let _span = tracy_client::span!();

    let mut command = command.as_ref();

    // Expand `~` at the start.
    let expanded = expand_home(Path::new(command));
    match &expanded {
        Ok(Some(expanded)) => command = expanded.as_ref(),
        Ok(None) => (),
        Err(err) => {
            warn!("error expanding ~: {err:?}");
        }
    }

    let mut process = Command::new(command);
    process.args(args);

    // Remove RUST_BACKTRACE and RUST_LIB_BACKTRACE from the environment if needed.
    if REMOVE_ENV_RUST_BACKTRACE.load(Ordering::Relaxed) {
        process.env_remove("RUST_BACKTRACE");
    }
    if REMOVE_ENV_RUST_LIB_BACKTRACE.load(Ordering::Relaxed) {
        process.env_remove("RUST_LIB_BACKTRACE");
    }

    // Remove the systemd NOTIFY_SOCKET variable.
    process.env_remove("NOTIFY_SOCKET");

    // Set DISPLAY if needed.
    let display = CHILD_DISPLAY.read().unwrap();
    if let Some(display) = &*display {
        process.env("DISPLAY", display);
    } else {
        process.env_remove("DISPLAY");
    }

    // Set configured environment.
    let env = CHILD_ENV.read().unwrap();
    for var in &env.0 {
        if let Some(value) = &var.value {
            process.env(&var.name, value);
        } else {
            process.env_remove(&var.name);
        }
    }
    drop(env);

    if let Some(token) = token.as_ref() {
        process.env("XDG_ACTIVATION_TOKEN", token.as_str());
        process.env("DESKTOP_STARTUP_ID", token.as_str());
    }

    unsafe {
        process.pre_exec(|| {
            restore_nofile_rlimit();
            crate::utils::signals::unblock_all()
        });
    }

    let mut process = async_process::Command::from(process);
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    process
}

pub fn spawnserver() -> Result<(), anyhow::Error> {
    let mut limits = std::env::args().skip(1).map(|arg| arg.parse::<rlim_t>());
    let (Some(Ok(cur)), Some(Ok(max))) = (limits.next(), limits.next()) else {
        anyhow::bail!("usage: niri-spawnserver <nofile_rlim_cur> <nofile_rlim_max>");
    };
    ORIGINAL_NOFILE_RLIMIT_CUR.store(cur, Ordering::Relaxed);
    ORIGINAL_NOFILE_RLIMIT_MAX.store(max, Ordering::Relaxed);
    restore_nofile_rlimit();
    crate::utils::signals::unblock_all()?;

    async_io::block_on(async {
        let scope = async {
            #[cfg(feature = "systemd")]
            if super::IS_SYSTEMD_SERVICE.load(Ordering::Relaxed) {
                let timeout = std::pin::pin!(async {
                    async_io::Timer::after(std::time::Duration::from_secs(5)).await;
                    anyhow::bail!("timed out");
                });
                let scope = std::pin::pin!(systemd::scope_self(OsStr::new("spawn")));
                let future = futures_util::future::select(scope, timeout);
                if let Err(err) = future.await.factor_first().0 {
                    eprintln!("spawnserver could not put itself into scope: {err:?}")
                }
            }
        };
        let read = async {
            let mut stdin = async_io::Async::new(io::stdin())?;
            let mut buf = Vec::new();
            futures_util::AsyncReadExt::read_to_end(&mut stdin, &mut buf)
                .await
                .context("could not read stdin")?;
            Ok::<_, anyhow::Error>(buf)
        };
        let (_, read) = futures_util::future::join(scope, read).await;

        let buf = read?;
        let Some((argc, mut buf)) = buf.split_first_chunk() else {
            std::process::exit(1)
        };
        let argc = u32::from_ne_bytes(*argc);
        let mut command_args = std::iter::from_fn(move || {
            let (size_chunk, remainder) = buf.split_first_chunk()?;
            let len = u32::from_ne_bytes(*size_chunk);
            let (string, remainder) = remainder.split_at_checked(len as usize)?;
            buf = remainder;
            Some(unsafe { OsStr::from_encoded_bytes_unchecked(string) })
        });
        let arg0 = command_args.next().context("missing program")?;
        let mut command = Command::new(arg0);
        command.args(command_args.by_ref().take(argc as usize));
        command.env_clear();
        command.envs(std::iter::from_fn(|| {
            Some((command_args.next()?, command_args.next()?))
        }));
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        // SUBTLE: The following will close `stderr` descriptor before the exec, so we make a
        // CLOEXEC copy of it and will restore stderr if exec fails so we can print out the error.
        command.stderr(Stdio::null());
        let stderr = io::stderr().as_fd().try_clone_to_owned()?;
        let exec_err = Err(command.exec());
        unsafe { libc::dup2(stderr.as_raw_fd(), io::stderr().as_raw_fd()) };
        exec_err.context(format!("could not spawn {arg0:?}"))
    })
}

fn prepare_spawnserver() -> Option<async_process::Child> {
    let mut spawnserver = if std::fs::exists("/proc/self/exe").map_err(|_| ()) == Ok(true) {
        std::process::Command::new("/proc/self/exe")
    } else {
        std::process::Command::new(std::env::current_exe().ok()?)
    };

    spawnserver.arg0("niri-spawnserver");
    let limit_args = [&ORIGINAL_NOFILE_RLIMIT_CUR, &ORIGINAL_NOFILE_RLIMIT_MAX]
        .map(|limit| limit.load(Ordering::SeqCst).to_string());
    spawnserver.args(limit_args);

    let mut spawnserver = async_process::Command::from(spawnserver);
    spawnserver.stdin(Stdio::piped());
    spawnserver.stdout(Stdio::null());
    // exec errors output by spawnserver.
    spawnserver.stderr(Stdio::inherit());

    match spawnserver.spawn() {
        Ok(server) => Some(server),
        Err(e) => {
            debug!("could not spawn a spawnserver, will fall back to direct spawning: {e:?}");
            None
        }
    }
}

/// Send a command to the spawnserver to spawn.
async fn spawn_in_spawnserver(
    mut spawnserver: async_process::Child,
    command: async_process::Command,
) -> Option<async_process::Child> {
    let mut buf = Vec::with_capacity(1024);
    let args = command.get_args();
    buf.extend_from_slice(&(args.len() as u32).to_ne_bytes());
    let mut serialize = |str: &OsStr| {
        let len = str.len() as u32;
        buf.extend_from_slice(&len.to_ne_bytes());
        buf.extend_from_slice(str.as_encoded_bytes());
    };
    serialize(command.get_program());
    for arg in args {
        serialize(arg);
    }
    let overrides = command
        .get_envs()
        .collect::<std::collections::HashMap<_, _>>();
    for (k, v) in std::env::vars_os() {
        if !overrides.contains_key(&*k) {
            serialize(&k);
            serialize(&v);
        }
    }
    for (k, v) in overrides {
        if let Some(v) = v {
            serialize(k);
            serialize(v);
        }
    }

    {
        let Some(mut stdin) = spawnserver.stdin.take() else {
            // This should be unreachable, but we have a reasonable fallback to try.
            let _ = spawnserver.kill();
            return just_spawn(command);
        };
        let Ok(_) = stdin.write_all(&buf).await else {
            // Likely reason for a failure here is that spawnserver crashed or got killed.
            let _ = spawnserver.kill();
            return just_spawn(command);
        };
        drop(stdin); // spawnserver may proceed to exec
    }
    Some(spawnserver)
}

#[cfg(feature = "systemd")]
mod systemd {
    use std::fmt::Write as _;
    use std::os::unix::ffi::OsStrExt;

    use anyhow::Context;
    use futures_util::StreamExt;

    use super::*;

    fn scope_name(name: &OsStr, pid: u32) -> String {
        let name = Path::new(name).file_name().unwrap_or(name);
        let mut scope_name = String::from("app-niri-");
        for &c in name.as_bytes() {
            if c.is_ascii_alphanumeric() || matches!(c, b':' | b'_' | b'.') {
                scope_name.push(char::from(c));
            } else {
                let _ = write!(scope_name, "\\x{c:02x}");
            }
        }
        let _ = write!(scope_name, "-{pid}.scope");
        scope_name
    }

    /// Puts a pid into a transient systemd scope.
    ///
    /// This separates the pid from the compositor scope, which for example prevents the OOM killer
    /// from bringing down the compositor together with a misbehaving client.
    async fn start_systemd_scope(
        proxy: &zbus::Proxy<'_>,
        name: &OsStr,
        child_pid: u32,
    ) -> anyhow::Result<()> {
        use zbus::zvariant::{OwnedObjectPath, Value};
        let scope_name = scope_name(name, child_pid);
        let mut signals = proxy
            .receive_signal("JobRemoved")
            .await
            .context("error creating a signal iterator")?;

        let pids: &[_] = &[child_pid];
        let properties: &[_] = &[
            ("PIDs", Value::new(pids)),
            ("CollectMode", Value::new("inactive-or-failed")),
        ];
        let aux: &[(&str, &[(&str, Value)])] = &[];

        let job: OwnedObjectPath = proxy
            .call("StartTransientUnit", &(scope_name, "fail", properties, aux))
            .await
            .context("error calling StartTransientUnit")?;

        trace!("waiting for JobRemoved");
        while let Some(message) = signals.next().await {
            let body = message.body();
            let body: (u32, OwnedObjectPath, &str, &str) =
                body.deserialize().context("error parsing signal")?;
            if body.1 == job {
                // Our transient unit had started...
                break;
            }
        }
        Ok(())
    }

    pub(super) async fn scope_self(name: &OsStr) -> anyhow::Result<()> {
        let conn = zbus::Connection::session()
            .await
            .context("error connecting to session bus")?;
        let proxy = zbus::Proxy::new(
            &conn,
            "org.freedesktop.systemd1",
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
        )
        .await
        .context("error creating a Proxy")?;

        start_systemd_scope(&proxy, name, std::process::id())
            .await
            .context("could not put spawnserver into a scope")?;

        Ok(())
    }
}
