# Submaps

### Overview

Submaps let you group keybindings into isolated input states (modal keybinding contexts).
When you enter a submap, only the submap's binds are active, optionally along with universal binds from the root `binds {}` section.

Submaps are defined as top-level `submap` blocks in the config file, outside of `binds {}`:

```kdl
submap "resize" {
    overlay-title "Resize Mode"
    clear-global-binds true
    catch-all "reset"
    timeout-ms 5000
    reset-target "default"
    input-policy {
        allow-mouse true
        allow-touchpad false
    }
    on-enter { show-hotkey-overlay; }
    on-exit { show-hotkey-overlay; }

    Mod+Right { focus-column-right; }
    Mod+Left { focus-column-left; }
    Mod+Up { focus-window-up; }
    Mod+Down { focus-window-down; }
}
```

To enter a submap, use the `switch-submap` action:

```kdl
binds {
    Mod+R { switch-submap "resize"; }
}
```

To exit a submap and return to the root, use `reset-submap`:

```kdl
submap "resize" {
    Mod+Escape { reset-submap; }
}
```

You can also use `toggle-submap` to toggle a submap on or off:

```kdl
binds {
    Mod+R { toggle-submap "resize"; }
}
```

### Submap Properties

#### `overlay-title`

Set the text shown in the overlay when this submap is active.

If not set, no overlay is shown when entering the submap.

```kdl
submap "resize" {
    overlay-title "Resize Mode"
}
```

#### `clear-global-binds`

Can be `true` or `false`. Default: `true`.

When `true`, only binds marked with the `universal` attribute in the root `binds {}` section will be active inside this submap. All other root binds are ignored.

When `false`, all root binds remain active alongside the submap's own binds.

```kdl
submap "resize" {
    clear-global-binds true
}
```

#### `catch-all`

Controls what happens when a key is pressed that has no matching bind in the submap (or in root binds, depending on `clear-global-binds`).

Valid values:

- `"ignore"` (default): the key is consumed and does nothing.
- `"reset"`: the submap is exited and the key is consumed.
- `"passthrough"`: the key is forwarded to the focused window.

```kdl
submap "resize" {
    catch-all "reset"
}
```

#### `timeout-ms`

Set an auto-exit timeout in milliseconds. After this many milliseconds, the submap will automatically exit.

If not set, the submap stays active until explicitly exited.

```kdl
submap "resize" {
    timeout-ms 5000
}
```

#### `reset-target`

Controls where the submap returns to when exiting. Default: `"default"`.

Valid values:

- `"default"`: return to the root (no active submap).
- `"previous"`: return to the previous submap if there was one, or to root.
- A named submap: return to that specific submap (if it exists in the config).

The `previous` option supports a bounded chain of depth 2 (root -> submap A -> submap B -> root), preventing infinite loops.

```kdl
submap "resize" {
    reset-target "previous"
}
```

#### `input-policy`

Controls which input device types are active within the submap.

All device types are allowed by default. You can disable specific device types:

```kdl
submap "resize" {
    input-policy {
        allow-mouse true
        allow-touchpad false
        allow-trackpoint true
        allow-trackball true
        allow-tablet true
        allow-touch true
    }
}
```

#### `on-enter`

A list of actions to run when entering this submap.

```kdl
submap "resize" {
    on-enter { show-hotkey-overlay; }
}
```

> [!TIP]
> The `on-enter` hooks run after the submap is fully active.
> If a hook calls `switch-submap` or `reset-submap`, the call is ignored to prevent infinite loops.

#### `on-exit`

A list of actions to run when exiting this submap.

```kdl
submap "resize" {
    on-exit { show-hotkey-overlay; }
}
```

### Multi-Action Binds

Submap binds support multiple actions per key using braces:

```kdl
submap "resize" {
    Mod+Right { focus-column-right; set-column-width "+100"; }
}
```

### Universal Binds

The `universal` attribute on root binds makes them available in all submaps, even when `clear-global-binds` is `true`:

```kdl
binds {
    Mod+Shift+Slash universal=true { show-hotkey-overlay; }
}
```

### Nested Submaps

You can enter a submap from within another submap. When you do, the current submap is replaced by the new one.

With `reset-target "previous"`, exiting the nested submap returns to the parent submap:

```kdl
submap "parent" {
    Mod+P { switch-submap "child"; }
}
```

```kdl
submap "child" {
    reset-target "previous"
    Mod+Escape { reset-submap; }
}
```

The chain is bounded at depth 2 (root -> parent -> child -> root) to prevent infinite loops.

### Lifecycle

1. `switch-submap "name"` activates the submap and triggers `on-enter` hooks.
2. Bindings are evaluated against the submap's bind list (and optionally root binds).
3. `reset-submap` triggers `on-exit` hooks and restores the previous state (or root).

If `timeout` is set, the submap automatically exits after the specified duration.

### Screen Lock Safety

Submaps are automatically exited when the screen is locked.
The `switch-submap`, `reset-submap`, and `toggle-submap` actions are not available when the screen is locked.

### Config Reload

When the config is reloaded, if the currently active submap no longer exists in the new config, it is automatically exited.

### IPC

IPC commands for submaps:

- `niri msg submaps` lists all configured submaps.
- `niri msg active-submap` shows the currently active submap, if any.
- The event stream emits `SubmapActivated` and `SubmapDeactivated` events.
