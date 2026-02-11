### Overview

When building niri, check `Cargo.toml` for a list of build features.
For example, you can replace systemd integration with dinit integration using `cargo build --release --no-default-features --features dinit,dbus,xdp-gnome-screencast`.
The defaults however should work fine for most distributions.

> [!WARNING]
> Do NOT build with `--all-features`!
>
> Some features are meant only for development use.
> For example, one of the features enables collection of profiling data into a memory buffer that will grow indefinitely until you run out of memory.

The `niri-visual-tests` sub-crate/binary is development-only and should not be packaged.

The recommended way to package niri is so that it runs as a standalone desktop session.
To do that, put files into the correct directories according to this table.

| File | Destination |
| ---- | ----------- |
| `target/release/niri` | `/usr/bin/` |
| `resources/niri-session` | `/usr/bin/` |
| `resources/niri.desktop` | `/usr/share/wayland-sessions/` |
| `resources/niri-portals.conf` | `/usr/share/xdg-desktop-portal/` |
| `resources/niri.service` (systemd) | `/usr/lib/systemd/user/` |
| `resources/niri-shutdown.target` (systemd) | `/usr/lib/systemd/user/` |
| `resources/dinit/niri` (dinit) | `/usr/lib/dinit.d/user/` |
| `resources/dinit/niri.target` (dinit) | `/usr/lib/dinit.d/user/` |

Doing this will make niri appear in GDM and other display managers.

See the [Integrating niri](./Integrating-niri.md) page for further information on distribution integration.

### Recommended dependencies

First of all, make sure niri depends on `libwayland-server`.
This library is currently loaded dynamically, so it's not picked up as a dependency at niri build time.

Then, the following dependencies are optional, but strongly recommended.
Set them as automatically-installed optional dependencies, if possible.

- `xwayland-satellite`: required to run X11 applications (Steam, Discord, etc.).
- `xdg-desktop-portal-gnome`: required for screencasting.
- `xdg-desktop-portal-gtk`: configured as the fallback portal in `niri-portals.conf`.
(This is in general the standard fallback portal that you want installed.)
- `gnome-keyring`: configured as the Secret portal provider in `niri-portals.conf`.
- Your distro's GPU driver package, such as `mesa-dri-drivers` and `mesa-libEGL`.
Working hardware acceleration is required for running niri.
- Some notification daemon like `mako`, generally required for apps to work correctly.

Finally, you may want to auto-install some of the applications bound in niri's [default configuration file](https://github.com/niri-wm/niri/blob/main/resources/default-config.kdl) (search for `spawn`), such as `alacritty` and `fuzzel`.

### Running tests

A bulk of our tests spawn niri compositor instances and test Wayland clients.
This does not require a graphical session, however due to test parallelism, it can run into file descriptor limits on high core count systems.

If you run into this problem, you may need to limit not just the Rust test harness thread count, but also the Rayon thread count, since some niri tests use internal Rayon threading:

```
$ export RAYON_NUM_THREADS=2
...proceed to run cargo test, perhaps with --test-threads=2
```

Don't forget to exclude the development-only `niri-visual-tests` crate when running tests.

Some tests require surfaceless EGL to be available at test time.
If this is problematic, you can skip them like so:

```
$ cargo test -- --skip=::egl
```

You may also want to set the `RUN_SLOW_TESTS=1` environment variable to run the slower tests.

### Version string

The niri version string includes its version and commit hash:

```
$ niri --version
niri 25.01 (e35c630)
```

When building in a packaging system, there's usually no repository, so the commit hash is unavailable and the version will show "unknown commit".
In this case, please set the commit hash manually:

```
$ export NIRI_BUILD_COMMIT="e35c630"
...proceed to build niri
```

You can also override the version string entirely, in this case please make sure the corresponding niri version stays intact:

```
$ export NIRI_BUILD_VERSION_STRING="25.01-1 (e35c630)"
...proceed to build niri
```

Remember to set this variable for both `cargo build` and `cargo install` since the latter will rebuild niri if the environment changes.

### Panics

Good panic backtraces are required for diagnosing niri crashes.
Please use the `niri panic` command to test that your package produces good backtraces.

```
$ niri panic
thread 'main' panicked at /builddir/build/BUILD/rust-1.83.0-build/rustc-1.83.0-src/library/core/src/time.rs:1142:31:
overflow when subtracting durations
stack backtrace:
   0: rust_begin_unwind
             at /builddir/build/BUILD/rust-1.83.0-build/rustc-1.83.0-src/library/std/src/panicking.rs:665:5
   1: core::panicking::panic_fmt
             at /builddir/build/BUILD/rust-1.83.0-build/rustc-1.83.0-src/library/core/src/panicking.rs:74:14
   2: core::panicking::panic_display
             at /builddir/build/BUILD/rust-1.83.0-build/rustc-1.83.0-src/library/core/src/panicking.rs:264:5
   3: core::option::expect_failed
             at /builddir/build/BUILD/rust-1.83.0-build/rustc-1.83.0-src/library/core/src/option.rs:2021:5
   4: expect<core::time::Duration>
             at /builddir/build/BUILD/rust-1.83.0-build/rustc-1.83.0-src/library/core/src/option.rs:933:21
   5: sub
             at /builddir/build/BUILD/rust-1.83.0-build/rustc-1.83.0-src/library/core/src/time.rs:1142:31
   6: cause_panic
             at /builddir/build/BUILD/niri-0.0.git.1699.279c8b6a-build/niri/src/utils/mod.rs:382:13
   7: main
             at /builddir/build/BUILD/niri-0.0.git.1699.279c8b6a-build/niri/src/main.rs:107:27
   8: call_once<fn() -> core::result::Result<(), alloc::boxed::Box<dyn core::error::Error, alloc::alloc::Global>>, ()>
             at /builddir/build/BUILD/rust-1.83.0-build/rustc-1.83.0-src/library/core/src/ops/function.rs:250:5
note: Some details are omitted, run with `RUST_BACKTRACE=full` for a verbose backtrace.
```

Important things to look for:

- The panic message is there: "overflow when subtracting durations".
- The backtrace goes all the way up to `main` and includes `cause_panic`.
- The backtrace includes the file and line number for `cause_panic`: `at /.../src/utils/mod.rs:382:13`.

If possible, please ensure that your niri package on its own has good panics, i.e. *without* installing debuginfo or other packages.
The user likely won't have debuginfo installed when their compositor first crashes, and we really want to be able to diagnose and fix all crashes right away.

### Rust dependencies

Every niri release comes with a vendored dependencies archive from `cargo vendor`.
You can use it to build the corresponding niri release completely offline.

If you don't want to use vendored dependencies, consider following the niri release's `Cargo.lock`.
It contains the exact dependency versions that I used when testing the release.

If you need to change the versions of some dependencies, pay extra attention to `smithay` and `smithay-drm-extras` commit hash.
These crates don't currently have regular stable releases, so niri uses git snapshots.
Upstream frequently has breaking changes (API and behavior), so you're strongly advised to use the exact commit hash from the niri release's `Cargo.lock`.

### Shell completions

You can generate shell completions for several shells via `niri completions <SHELL>`, i.e. `niri completions bash`.
See `niri completions -h` for a full list.
