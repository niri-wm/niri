# Proposal: add an `is-xwayland` window-rule matcher

Niri currently exposes matchers for app ID, title, focus, urgency, floating
state, and related window state. It does not expose whether a window is an
XWayland client or a native Wayland client.

An explicit matcher would allow users to write compositor policies without
depending on restart-specific window IDs, titles, or application names:

```kdl
window-rule {
    match is-xwayland=true
    open-floating true
}
```

## Proposed semantics

- `is-xwayland=true` matches XWayland clients.
- `is-xwayland=false` matches native Wayland clients.
- The matcher is evaluated from compositor window/backend identity, not
  inferred from app ID or title.
- Existing configurations and rule ordering remain unchanged.
- Creation-only properties retain their current semantics; reloading a rule
  does not retroactively move an existing window.

## Acceptance criteria

1. A native Wayland test client does not match `is-xwayland=true`.
2. An XWayland test client matches `is-xwayland=true`.
3. Both boolean values parse and work with `open-floating`.
4. `exclude` and rule ordering continue to work.
5. Steam launched through XWayland can be floated without matching by title or
   application-specific ID.

This document is a design proposal for discussion; it intentionally does not
claim that the matcher is implemented yet.
