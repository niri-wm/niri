# PROJECT KNOWLEDGE BASE

**Generated:** 2026-02-15
**Commit:** 41b5de87
**Branch:** feat/layer-anims

## OVERVIEW

Niri - a scrollable-tiling Wayland compositor written in Rust. Uses Smithay for Wayland backend, KDL for config, supports animations, multi-monitor, and custom shaders.

## STRUCTURE

```
./
├── src/                    # Main compositor (100+ modules)
│   ├── niri.rs            # Core (6171 lines - largest file)
│   ├── layout/            # Window management (13 files, 25k+ lines)
│   ├── input/             # Input handling (5462 lines)
│   ├── render_helpers/    # Rendering utilities
│   ├── ui/                # UI elements
│   ├── handlers/          # Wayland protocol handlers
│   ├── protocols/         # Custom Wayland protocols
│   └── tests/             # Integration tests + snapshots
├── niri-config/           # KDL config parsing crate
├── niri-ipc/              # IPC types for client communication
├── niri-visual-tests/      # GTK visual testing app
├── resources/             # Cursors, default config, desktop files
└── docs/wiki/             # MkDocs documentation
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Window tiling/layout | `src/layout/` | scrolling.rs, tile.rs, workspace.rs |
| Input handling | `src/input/mod.rs` | 5462 lines - primary input event handling |
| Animations | `src/animation/` | Animation system |
| Layer-shell | `src/handlers/layer_shell.rs` | Waybar, dunst handling |
| Config parsing | `niri-config/src/lib.rs` | KDL via knuffel |
| IPC protocol | `niri-ipc/src/lib.rs` | Request/Reply/Event enums |
| Tests | `src/tests/` | Integration tests with fixtures |

## CONVENTIONS

- **Format**: `cargo +nightly fmt --all` (NOT stable rustfmt)
- **Lint**: `cargo clippy --all --all-targets`
- **Test**: `cargo test` + randomized with `PROPTEST_CASES=200000`
- **Config**: KDL format (NOT TOML/YAML) via knuffel
- **Import grouping**: Module-level, StdExternalCrate
- **Comment width**: 100 chars

## ANTI-PATTERNS (THIS PROJECT)

- **FIXME markers**: 237+ occurrences - known technical debt
- **HACK markers**: 10+ workarounds in code
- **Unsafe code**: ~10 instances (signals, shaders, PipeWire)
- **Interior mutability**: Explicitly allowed for smithay types

## UNIQUE STYLES

- **Monolithic core**: niri.rs is 6171 lines (unusually large)
- **Property-based testing**: Heavy proptest usage in layout/tests.rs
- **Snapshot testing**: insta crate, 5280+ snapshot files
- **Custom session runner**: resources/niri-session (not systemd-only)

## COMMANDS

```bash
cargo run --release              # Build and run compositor
cargo test                       # Run all tests
cargo +nightly fmt --all         # Format code
cargo clippy --all --all-targets # Lint
cargo test --test layout         # Property-based layout tests
```

## NOTES

- MSRV: 1.85.0 (minimum Rust version)
- Feature-gated deps: dbus, systemd, xdp-gnome-screencast
- 43 files >500 lines - complexity hotspots in layout/input
- Layer-shell animations feature in development (feat/layer-anims branch)
