## Running a Local Build

The main way of testing niri during development is running it as a nested window. The second step is usually switching to a different TTY and running niri there.

Once a feature or fix is reasonably complete, you generally want to run a local build as your main compositor for proper testing. The easiest way to do that is to install niri normally (from a distro package for example), then overwrite the binary with `sudo cp ./target/release/niri /usr/bin/niri`. Do make sure that you know how to revert to a working version in case everything breaks though.

If you use an RPM-based distro, you can generate an RPM package for a local build with `cargo generate-rpm`.

## Logging Levels

Niri uses [`tracing`](https://lib.rs/crates/tracing) for logging. This is how logging levels are used:

- `error!`: programming errors and bugs that are recoverable. Things you'd normally use `unwrap()` for. However, when a Wayland compositor crashes, it brings down the entire session, so it's better to recover and log an `error!` whenever reasonable. If you see an `ERROR` in the niri log, that always indicates a *bug*.
- `warn!`: something bad but still *possible* happened. Informing the user that they did something wrong, or that their hardware did something weird, falls into this category. For example, config parsing errors should be indicated with a `warn!`.
- `info!`: the most important messages related to normal operation. Running niri with `RUST_LOG=niri=info` should not make the user want to disable logging altogether.
- `debug!`: less important messages related to normal operation. Running niri with `debug!` messages hidden should not negatively impact the UX.
- `trace!`: everything that can be useful for debugging but is otherwise too spammy or performance intensive. `trace!` messages are *compiled out* of release builds.

## Tests

We have some unit tests, most prominently for the layout code and for config parsing.

When adding new operations to the layout, add them to the `Op` enum at the bottom of `src/layout/mod.rs` (this will automatically include it in the randomized tests), and if applicable to the `every_op` arrays below.

When adding new config options, include them in the config parsing test.

### Running Tests

Make sure to run `cargo test --all` to run tests from sub-crates too.

Some tests are a bit too slow to run normally, like the randomized tests of the layout code, so they are normally skipped. Set the `RUN_SLOW_TESTS` variable to run them:

```
env RUN_SLOW_TESTS=1 cargo test --all
```

It also usually helps to run the randomized tests for a longer period, so that they can explore more inputs. You can control this with environment variables. This is how I usually run tests before pushing:

```
env RUN_SLOW_TESTS=1 PROPTEST_CASES=200000 PROPTEST_MAX_GLOBAL_REJECTS=200000 RUST_BACKTRACE=1 cargo test --release --all
```

### Visual Tests

The `niri-visual-tests` sub-crate is a GTK application that runs hard-coded test cases so that you can visually check that they look right. It uses mock windows with the real layout and rendering code. It is especially helpful when working on animations.

## Profiling

We have integration with the [Tracy](https://github.com/wolfpld/tracy) profiler which you can enable by building niri with a feature flag:

```
cargo build --release --features=profile-with-tracy-ondemand
```

Then you can open Tracy (you will need the latest stable release) and attach to a running niri instance to collect profiling data. Profiling data is collected "on demand"—that is, only when Tracy is connected. You can run a niri build like this as your main compositor if you'd like.

> [!NOTE]
> If you need to profile niri startup or the niri CLI, you can opt for "always on" profiling instead, using this feature flag:
>
> ```
> cargo build --release --features=profile-with-tracy
> ```
>
> When compiled this way, niri will **always** collect profiling data, so you can't run a build like this as your main compositor.

To make a niri function show up in Tracy, instrument it like this:

```rust
pub fn some_function() {
    let _span = tracy_client::span!("some_function");

    // Code of the function.
}
```

You can also enable Rust memory allocation profiling with `--features=profile-with-tracy-allocations`.
