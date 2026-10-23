use std::ffi::OsStr;
use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::thread;

use anyhow::Context;
use atomic::Atomic;
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
                let ex = async_executor::LocalExecutor::new();
                async_io::block_on(ex.run(async {
                    #[cfg(feature = "systemd")]
                    let mut spawnserver = systemd::prepare_spawnserver();
                    while let Ok((command, token)) = receiver.recv().await {
                        let Some((command, args)) = command.split_first() else {
                            continue;
                        };
                        let spawnee = spawnee(command, args, token);
                        #[cfg(feature = "systemd")]
                        {
                            let server = spawnserver.take().or_else(systemd::prepare_spawnserver);
                            if let Some(server) = server {
                                ex.spawn(systemd::spawn_in_spawnserver(server, spawnee))
                                    .detach();
                                spawnserver = systemd::prepare_spawnserver();
                                continue;
                            }
                        }
                        spawn_cmd(spawnee);
                    }
                }));
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
        };
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

fn spawn_cmd(mut command: async_process::Command) {
    if let Err(err) = command.spawn() {
        warn!("error spawning {command:?}: {err:?}");
    }
}

fn spawnee(
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

    spawnee_restore_state(&mut process);

    let mut process = async_process::Command::from(process);

    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    process
}

fn spawnee_restore_state(cmd: &mut std::process::Command) {
    unsafe {
        cmd.pre_exec(|| {
            restore_nofile_rlimit();
            crate::utils::signals::unblock_all()
        });
    }
}

pub fn spawnserver() -> Result<(), anyhow::Error> {
    #[cfg(feature = "systemd")]
    let could_prepare_scope = match systemd::scope_self(OsStr::new("spawnserver")) {
        Ok(()) => true,
        Err(err) => {
            eprintln!("spawnserver could not put itself into scope: {err:?}");
            false
        }
    };

    let mut buf = Vec::new();
    io::stdin()
        .lock()
        .read_to_end(&mut buf)
        .context("could not read the command to spawn")?;
    let Some((args, buf)) = buf.split_first_chunk() else {
        std::process::exit(0)
    };
    let argc = usize::from_ne_bytes(*args);
    let mut bufi = buf
        .split(|b| *b == 0)
        .map(|b| unsafe { OsStr::from_encoded_bytes_unchecked(b) });
    let Some(program) = bufi.next() else {
        return Ok(());
    };
    let mut command = Command::new(program);
    for _ in 0..argc {
        let arg = bufi
            .next()
            .ok_or_else(|| anyhow::anyhow!("malformed command"))?;
        command.arg(arg);
    }
    command.env_clear();
    while let (Some(k), Some(v)) = (bufi.next(), bufi.next()) {
        command.env(k, v);
    }

    #[cfg(feature = "systemd")]
    if !could_prepare_scope {
        if let Err(err) = systemd::scope_self(program) {
            eprintln!("spawnserver could not put itself into scope on 2nd attempt either: {err:?}");
        }
    }

    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    // SUBTLE: The following will close `stderr` descriptor before the exec, so we make a CLOEXEC
    // copy of it and will restore stderr if exec fails so we can print out the error.
    command.stderr(Stdio::null());
    let stderr = io::stderr().as_fd().try_clone_to_owned()?;
    let exec_err = Err(command.exec()).context(format!("could not spawn {program:?}"));
    unsafe { libc::dup2(stderr.as_raw_fd(), io::stderr().as_raw_fd()) };
    exec_err
}

#[cfg(feature = "systemd")]
mod systemd {
    use std::fmt::Write as _;
    use std::os::unix::ffi::OsStrExt;

    use anyhow::Context;
    use futures_util::{AsyncReadExt, AsyncWriteExt, StreamExt};

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

    pub(super) fn prepare_spawnserver() -> Option<async_process::Child> {
        if !crate::utils::IS_SYSTEMD_SERVICE.load(Ordering::Relaxed) {
            return None;
        }
        let mut spawnserver = if std::fs::exists("/proc/self/exe").map_err(|_| ()) == Ok(true) {
            std::process::Command::new("/proc/self/exe")
        } else {
            std::process::Command::new(std::env::current_exe().ok()?)
        };
        spawnee_restore_state(&mut spawnserver);
        spawnserver.arg("spawnserver");
        let mut spawnserver = async_process::Command::from(spawnserver);
        spawnserver.stdin(Stdio::piped());
        spawnserver.stdout(Stdio::piped());
        // exec errors output by spawnserver.
        spawnserver.stderr(Stdio::inherit());
        match spawnserver.spawn() {
            Ok(server) => Some(server),
            Err(e) => {
                info!("cannot spawn a spawnserver, will fall back to direct spawning: {e:?}");
                None
            }
        }
    }

    /// Send a command to the spawnserver to spawn.
    pub(super) async fn spawn_in_spawnserver(
        mut spawnserver: async_process::Child,
        command: async_process::Command,
    ) {
        let mut buf = Vec::new();
        let args = command.get_args();
        buf.extend_from_slice(&args.len().to_ne_bytes());
        buf.extend_from_slice(command.get_program().as_encoded_bytes());
        buf.push(0);
        for arg in args {
            buf.extend_from_slice(arg.as_encoded_bytes());
            buf.push(0);
        }
        let mut environ = std::env::vars_os().collect::<std::collections::HashMap<_, _>>();
        for (k, v) in command.get_envs() {
            if let Some(v) = v {
                environ.insert(k.into(), v.into());
            } else {
                environ.remove(k);
            }
        }
        for (k, v) in environ {
            buf.extend_from_slice(k.as_encoded_bytes());
            buf.push(0);
            buf.extend_from_slice(v.as_encoded_bytes());
            buf.push(0);
        }

        {
            let Some(mut stdin) = spawnserver.stdin.take() else {
                // This should be unreachable, but we have a reasonable fallback to try.
                let _ = spawnserver.kill();
                return super::spawn_cmd(command);
            };
            let Ok(_) = stdin.write_all(&buf).await else {
                // Likely reason for a failure here is that spawnserver crashed or got killed.
                let _ = spawnserver.kill();
                return super::spawn_cmd(command);
            };
            drop(stdin); // spawnserver may proceed to exec
        }

        if let Some(mut stdout) = spawnserver.stdout.take() {
            // Wait for exec to happen, spawnserver indicates this by closing its stdout.
            let mut buf = [0];
            let result = stdout.read(&mut buf).await;
            debug_assert_eq!(result.map_err(|_| ()), Ok(0));
        }

        if let Ok(None) = spawnserver.try_status() {
            if let Err(e) = rescope_spawned(command.get_program(), spawnserver.id()).await {
                warn!("could not migrate spawned program to its named scope: {e:?}");
            };
        }
    }

    pub(super) fn scope_self(name: &OsStr) -> anyhow::Result<()> {
        if !crate::utils::IS_SYSTEMD_SERVICE.load(Ordering::Relaxed) {
            return Ok(());
        }
        async_io::block_on(async {
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
                .context("could not put spawnserver into an intermediate scope")?;
            Ok(())
        })
    }

    async fn rescope_spawned(command: &OsStr, pid: u32) -> anyhow::Result<()> {
        let conn = zbus::Connection::session()
            .await
            .context("could not connect to session bus")?;
        rescope(conn, command, pid)
            .await
            .context("error moving spawned process into its scope")?;
        Ok(())
    }

    async fn rescope(conn: zbus::Connection, command: &OsStr, pid: u32) -> anyhow::Result<()> {
        let from = scope_name(OsStr::new("spawnserver"), pid);
        let to = scope_name(command, pid);
        let proxy = zbus::Proxy::new(
            &conn,
            "org.freedesktop.systemd1",
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
        )
        .await
        .context("error creating a Proxy")?;
        start_systemd_scope(&proxy, command, pid).await?;

        for _ in 0..64 {
            let procs: Vec<(String, u32, String)> =
                match proxy.call("GetUnitProcesses", &from).await {
                    Ok(procs) => procs,
                    // spawnserver scope emptied and got garbage collected (program has exited).
                    Err(zbus::Error::MethodError(name, _, _))
                        if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit" =>
                    {
                        return Ok(())
                    }
                    Err(err) => return Err(err).context("error calling GetUnitProcesses"),
                };
            if procs.is_empty() {
                return Ok(());
            }
            let pids: Vec<u32> = procs.into_iter().map(|(_, pid, _)| pid).collect();
            trace!("moving {pids:?} from {from} to {to}");
            // Processes may spawn or exit in between iterations.
            if let Err(err) = proxy
                .call::<_, _, ()>("AttachProcessesToUnit", &(&to, "", &pids))
                .await
            {
                trace!("error attaching {pids:?} to {to}: {err:?}");
            }
        }
        anyhow::bail!("{from} still not empty after 64 attempts to rescope, giving up")
    }

    /// Puts a (newly spawned) pid into a transient systemd scope.
    ///
    /// This separates the pid from the compositor scope, which for example prevents the OOM killer
    /// from bringing down the compositor together with a misbehaving client.
    async fn start_systemd_scope(
        proxy: &zbus::Proxy<'_>,
        name: &OsStr,
        child_pid: u32,
    ) -> anyhow::Result<()> {
        use zbus::zvariant::{OwnedObjectPath, Value};

        let _span = tracy_client::span!();

        // Extract the basename.
        let name = Path::new(name).file_name().unwrap_or(name);
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
                // Our transient unit had started, we're good to proceed to exec.
                break;
            }
        }

        Ok(())
    }
}
