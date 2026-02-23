### Overview

Appearance rules let you apply parts of the config based on system appearance settings (provided through `xdg-desktop-portal`).

Like window rules, appearance rules are processed in order of appearance in the config file.
This means you can start with a base configuration, then override it for specific appearance modes later.

Each `appearance-rule` can contain `layout`, `overview`, and `animations` blocks (with the same contents as the top-level blocks).

```kdl
overview {
    backdrop-color "#112233"
}

appearance-rule {
    match color-scheme="dark"

    overview {
        backdrop-color "#445566"
    }
}
```

### Matching

Each appearance rule can have several `match` and `exclude` directives.
In order for the rule to apply, the current appearance settings need to match *any* of the `match` directives, and *none* of the `exclude` directives.

If you omit all `match` directives, the rule matches all appearance settings (subject to `exclude`).

Match and exclude directives have the same syntax.
There can be multiple matchers in one directive, then all of them need to match for the directive to apply.

```kdl
appearance-rule {
    match color-scheme="dark" contrast="high"
    exclude reduced-motion=true

    animations {
        // For example, slow down animations in high-contrast dark mode.
        slowdown 2.0
    }
}
```

### Matchers

- `color-scheme`: `"light"` or `"dark"`.
- `contrast`: `"high"`.
- `reduced-motion`: `true` or `false`.

For example, you can honor reduced motion by disabling animations when the setting is enabled:

```kdl
appearance-rule {
    match reduced-motion=true

    animations {
        off
    }
}
```
