# PROJECT KNOWLEDGE BASE

**Tests Directory**

## OVERVIEW

Integration tests using a headless Wayland client-server architecture to test window management, protocols, and compositor behavior.

## STRUCTURE

```
src/tests/
├── mod.rs              # Test module entry point
├── fixture.rs          # Test harness (Fixture struct)
├── server.rs           # Headless niri server for tests
├── client.rs           # Wayland client abstraction
├── snapshots/          # 5280+ insta snapshot files
├── window_opening.rs   # Window lifecycle tests
├── animations.rs       # Animation timing/curve tests
├── floating.rs         # Floating window tests
├── fullscreen.rs       # Fullscreen/maximize tests
├── layer_shell.rs      # Layer-shell protocol tests
└── transactions.rs     # Transaction ordering tests
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Test harness | `fixture.rs` | Fixture struct - creates server + clients |
| New integration test | `window_opening.rs` | Pattern for client-server tests |
| Animation tests | `animations.rs` | Time manipulation, curve verification |
| Snapshots | `snapshots/*.snap` | Auto-generated, review with `cargo insta` |
| Property tests | `layout/tests.rs` | proptest for randomized layout actions |

## CONVENTIONS

- **Fixture API**: `f.add_output(1, (1920, 1080))`, `f.add_client()`, `f.roundtrip(id)`
- **Snapshots**: Use `assert_snapshot!()` for protocol state, window positions, sizes
- **Client ops**: `f.client(id).create_window()`, `window.ack_last_and_commit()`
- **Animations**: `f.niri_complete_animations()` to skip wait times
- **Property tests**: Use `proptest!` macro with `ProptestConfig` for cases

## ANTI-PATTERNS

- **Manual time advancement**: Use `niri_complete_animations()` instead of sleep
- **Hardcoded IDs**: Window IDs are non-deterministic (global counter)
- **Snapshot drift**: Always run `cargo insta review` after changes
- **Missing roundtrip**: Always call `f.roundtrip(id)` after client commits
