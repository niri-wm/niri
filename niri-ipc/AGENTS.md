# IPC LIBRARY

**Scope:** niri-ipc/ - Client IPC types and socket communication

## OVERVIEW

Public IPC types library for niri-client communication. Defines Request/Reply/Event enums exchanged over Unix socket.

## STRUCTURE

```
niri-ipc/src/
├── lib.rs      # Main: Request, Reply, Event enums (2095 lines)
├── socket.rs   # Socket handling utilities
└── state.rs    # IPC state types
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| IPC protocol | `lib.rs` | Request/Reply/Event definitions |
| Socket comm | `socket.rs` | Unix socket handling |

## CONVENTIONS

- Serde derives for all IPC types
- Version-tagged IPC (backward compatibility)

## ANTI-PATTERNS

- IPC version drift between client/server causes silent failures
