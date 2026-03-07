# UTILITIES

**Scope:** src/utils/ - Shared utility functions

## OVERVIEW

Utility modules: file watching, spawning, signals, transactions, xwayland integration.

## STRUCTURE

```
src/utils/
├── mod.rs            # Utils aggregator (622 lines)
├── watcher.rs        # Config file watcher (754 lines)
├── spawning.rs       # Process spawning
├── signals.rs        # Signal handling (unsafe)
├── transaction.rs    # Client transaction batching
├── scale.rs          # DPI scale helpers
├── id.rs             # ID generation
├── xwayland/         # Xwayland satellite
│   ├── mod.rs
│   └── satellite.rs
└── vblank_throttle.rs
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Config reloading | `watcher.rs` | File system events |
| Process spawning | `spawning.rs` | Child process management |
| Signal handling | `signals.rs` | Panic/quit signals |

## CONVENTIONS

- Stateless utilities
- Error propagation via `Result`

## ANTI-PATTERNS

- Unsafe in signals.rs
- Watcher may miss rapid changes
