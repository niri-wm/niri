# IPC SERVER

**Scope:** src/ipc/ - Server-side IPC implementation

## OVERVIEW

IPC server implementation. Handles client connections via Unix socket, dispatches commands.

## STRUCTURE

```
src/ipc/
├── mod.rs    # IPC types shared with niri-ipc
├── server.rs # IPC server (945 lines)
└── client.rs # Client connection handling (815 lines)
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| IPC server | `server.rs` | Socket server loop |
| Client handling | `client.rs` | Per-client state |

## CONVENTIONS

- Unix socket at `$XDG_RUNTIME_DIR/niri.sock`
- Async via calloop

## ANTI-PATTERNS

- Socket permissions issues
- Client disconnects during command
