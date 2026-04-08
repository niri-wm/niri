<sup>Since: 25.11</sup>

You can include other files at the top level of the config.

```kdl,must-fail
// Some settings...

include "colors.kdl"

// Some more settings...
```

Included files have the same structure as the main config file.
Settings from included files will be merged with the settings from the main config file.

Included config files can in turn include more files.
All included files are watched for changes, and the config live-reloads when any of them change.

Includes work only at the top level of the config:

```kdl,must-fail
// All good: include at the top level.
include "something.kdl"

layout {
    // NOT allowed: include inside some other section.
    include "other.kdl"
}
```

### Positionality

Includes are *positional*.
They will override options set *prior* to them.
Window rules from included files will be inserted at the position of the `include` line.
For example:

```kdl
// colors.kdl
layout {
    border {
        active-color "green"
    }
}

overview {
    backdrop-color "green"
}
```

```kdl,must-fail
// config.kdl
layout {
    border {
        active-color "red"
    }
}

// This overrides the border color and the backdrop color to green.
include "colors.kdl"

// This sets the overview backdrop color to red again.
overview {
    backdrop-color "red"
}
```

The end result:

- the border color is green (from `colors.kdl`),
- the overview backdrop color is red (it was set *after* `colors.kdl`).

Another example:

```kdl
// rules.kdl
window-rule {
    match app-id="Alacritty"
    open-maximized false
}
```

```kdl,must-fail
// config.kdl
window-rule {
    open-maximized true
}

// Window rules get inserted at this position.
include "rules.kdl"

window-rule {
    match app-id="firefox$"
    open-maximized true
}
```

This is equivalent to the following config file:

```kdl
window-rule {
    open-maximized true
}

// Included from rules.kdl.
window-rule {
    match app-id="Alacritty"
    open-maximized false
}

window-rule {
    match app-id="firefox$"
    open-maximized true
}
```

### Optional includes

<sup>Since: next release</sup>

By default, including a nonexistent file will cause an error.
You can allow nonexistent includes by setting `optional=true`:

```kdl,must-fail
// Won't fail if this file doesn't exist.
include optional=true "optional-config.kdl"

// Regular include, will fail if the file doesn't exist.
include "required-config.kdl"
```

When an optional include file is missing, niri will emit a warning in the logs on every config reload.
This reminds you that the file is missing while still loading the config successfully.

The optional file is still watched for changes, so if you create it later, the config will automatically reload and apply the new settings.

Note that `optional` only affects whether a missing file causes an error.
If the file exists but contains invalid syntax or other errors, those errors will still cause a parsing failure.


### Merging

Most config sections are merged between includes, meaning that you can set only a few properties, and only those properties will change.

```kdl
// colors.kdl
layout {
    // Does not affect gaps, border width, etc.
    // Only changes colors as written.
    focus-ring {
        active-color "blue"
    }

    border {
        active-color "green"
    }
}
```

```kdl,must-fail
// config.kdl
include "colors.kdl"

layout {
    // Does not set border and focus-ring colors,
    // so colors from colors.kdl are used.
    gaps 8

    border {
        width 8
    }
}
```

#### Multipart sections

Multipart sections like `window-rule`, `output`, or `workspace` are inserted as is without merging:

```kdl
// laptop.kdl
output "eDP-1" {
    // ...
}
```

```kdl,must-fail
// config.kdl
output "DP-2" {
    // ...
}

include "laptop.kdl"

// End result: both DP-2 and eDP-1 settings.
```

#### Binds

`binds` will override previously-defined conflicting keys:

```kdl
// binds.kdl
binds {
    Mod+T { spawn "alacritty"; }
}
```

```kdl,must-fail
// config.kdl
include "binds.kdl"

binds {
    // Overrides Mod+T from binds.kdl.
    Mod+T { spawn "foot"; }
}
```

#### Flags

Most flags can be disabled with `false`:

```kdl
// csd.kdl

// Write "false" to explicitly disable.
prefer-no-csd false
```

```kdl,must-fail
// config.kdl

// Enable prefer-no-csd in the main config.
prefer-no-csd

// Including csd.kdl will disable it again.
include "csd.kdl"
```

#### Non-merging sections

Some sections where the contents represent a combined structure are not merged.
Examples are `struts`, `preset-column-widths`, individual subsections in `animations`, pointing device sections in `input`.

```kdl
// struts.kdl
layout {
    struts {
        left 64
        right 64
    }
}
```

```kdl,must-fail
// config.kdl
layout {
    struts {
        top 64
        bottom 64
    }
}

include "struts.kdl"

// Struts are not merged.
// End result is only left and right struts.
```

### Border special case

There's one special case that differs between the main config and included configs.

Writing `layout { border {} }` in an included config does nothing (since no properties are changed).
However, writing the same in the main config will *enable* the border, i.e. it's equivalent to `layout { border { on; } }`.

So, if you want to move your layout configuration from the main config to a separate file, remember to add `on` to the border section, for example:

```kdl
// separate.kdl
layout {
    border {
        // Add this line:
        on

        width 4
        active-color "#ffc87f"
        inactive-color "#505050"
    }
}
```

The reason for this special case is that this is how it historically worked: back when I added borders, we didn't have any `on` flags, so I made writing the `border {}` section enable the border, with an explicit `off` to disable it.
It wouldn't be too problematic to change it, however the default config always had a pre-filled `layout { border { off; } }` section with a note saying that commenting out the `off` is enough to enable the border.
Many people likely have this part of the default config embedded in their configs now, so changing how it works would just cause a lot of confusion.
