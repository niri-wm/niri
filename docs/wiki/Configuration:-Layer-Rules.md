### Overview

<sup>Since: 25.01</sup>

Layer rules let you adjust behavior for individual layer-shell surfaces.
They have `match` and `exclude` directives that control which layer-shell surfaces the rule should apply to, and a number of properties that you can set.

Layer rules are processed and work very similarly to window rules, just with different matchers and properties.
Please read the [window rules wiki page](./Configuration:-Window-Rules.md) to learn how matching works.

Here are all matchers and properties that a layer rule could have:

```kdl
layer-rule {
    match namespace="waybar"
    match at-startup=true
    match layer="top"

    // Properties that apply continuously.
    opacity 0.5
    block-out-from "screencast"
    // block-out-from "screen-capture"

    shadow {
        on
        // off
        softness 40
        spread 5
        offset x=0 y=5
        draw-behind-window true
        color "#00000064"
        // inactive-color "#00000064"
    }

    geometry-corner-radius 12
    place-within-backdrop true
    baba-is-float true

    background-effect {
        xray true
        blur true
        noise 0.05
        saturation 3
    }
}
```

### Layer Surface Matching

Let's look at the matchers in more detail.

#### `namespace`

This is a regular expression that should match anywhere in the surface namespace.
You can read about the supported regular expression syntax [here](https://docs.rs/regex/latest/regex/#syntax).

```kdl
// Match surfaces with namespace containing "waybar",
layer-rule {
    match namespace="waybar"
}
```

You can find the namespaces of all open layer-shell surfaces by running `niri msg layers`.

#### `at-startup`

Can be `true` or `false`.
Matches during the first 60 seconds after starting niri.

```kdl
// Show layer-shell surfaces with 0.5 opacity at niri startup, but not afterwards.
layer-rule {
    match at-startup=true

    opacity 0.5
}
```

#### `layer`

<sup>Since: next release</sup>

Matches surfaces on this layer-shell layer.
Can be `"background"`, `"bottom"`, `"top"`, or `"overlay"`.

```kdl
// Make all overlay-layer surfaces FLOAT.
layer-rule {
    match layer="overlay"

    baba-is-float true
}
```

### Dynamic Properties

These properties apply continuously to open layer-shell surfaces.

#### `block-out-from`

You can block out surfaces from xdg-desktop-portal screencasts or all screen captures.
They will be replaced with solid black rectangles.

This can be useful for notifications.

The same caveats and instructions apply as for the [`block-out-from` window rule](./Configuration:-Window-Rules.md#block-out-from), so check the documentation there.

![Screenshot showing a notification visible normally, but blocked out on OBS.](./img/layer-block-out-from-screencast.png)

```kdl
// Block out mako notifications from screencasts.
layer-rule {
    match namespace="^notifications$"

    block-out-from "screencast"
}
```

#### `opacity`

Set the opacity of the surface.
`0.0` is fully transparent, `1.0` is fully opaque.
This is applied on top of the surface's own opacity, so semitransparent surfaces will become even more transparent.

Opacity is applied to every child of the layer-shell surface individually, so subsurfaces and pop-up menus will show window content behind them.

```kdl
// Make fuzzel semitransparent.
layer-rule {
    match namespace="^launcher$"

    opacity 0.95
}
```

#### `shadow`

<sup>Since: 25.02</sup>

Override the shadow options for the surface.

These rules have the same options as the normal [`shadow` config in the layout section](./Configuration:-Layout.md#shadow), so check the documentation there.

Unlike window shadows, layer surface shadows always need to be enabled with a layer rule.
That is, enabling shadows in the layout config section won't automatically enable them for layer surfaces.

> [!NOTE]
> Layer surfaces have no way to tell niri about their *visual geometry*.
> For example, if a layer surface includes some invisible margins (like mako), niri has no way of knowing that, and will draw the shadow behind the entire surface, including the invisible margins.
>
> So to use niri shadows, you'll need to configure layer-shell clients to remove their own margins or shadows.

```kdl
// Add a shadow for fuzzel.
layer-rule {
    match namespace="^launcher$"

    shadow {
        on
    }

    // Fuzzel defaults to 10 px rounded corners.
    geometry-corner-radius 10
}
```

#### `geometry-corner-radius`

<sup>Since: 25.02</sup>

Set the corner radius of the surface.

This setting will only affect the shadow—it will round its corners to match the geometry corner radius.

```kdl
layer-rule {
    match namespace="^launcher$"

    geometry-corner-radius 12
}
```

#### `place-within-backdrop`

<sup>Since: 25.05</sup>

Set to `true` to place the surface into the backdrop visible in the [Overview](./Overview.md) and between workspaces.

This will only work for *background* layer surfaces that ignore exclusive zones (typical for wallpaper tools).
Layers within the backdrop will ignore all input.

```kdl
// Put swaybg inside the overview backdrop.
layer-rule {
    match namespace="^wallpaper$"

    place-within-backdrop true
}
```

#### `baba-is-float`

<sup>Since: 25.05</sup>

Make your layer surfaces FLOAT up and down.

This is a natural extension of the [April Fools' 2025 feature](./Configuration:-Window-Rules.md#baba-is-float).

```kdl
// Make fuzzel FLOAT.
layer-rule {
    match namespace="^launcher$"

    baba-is-float true
}
```

#### `background-effect`

<sup>Since: next release</sup>

Override the background effect options for this surface.

- `xray`: set to `true` to enable the xray effect, or `false` to disable it.
- `blur`: set to `true` to enable blur behind this surface, or `false` to force-disable it.
- `noise`: amount of pixel noise added to the background (helps with color banding from blur).
- `saturation`: color saturation of the background (`0` is desaturated, `1` is normal, `2` is 200% saturation).

See the [window effects page](./Window-Effects.md) for an overview of background effects.

```kdl
// Make top and overlay layers use the regular blur (if enabled),
// while bottom and background layers keep using the efficient xray blur.
layer-rule {
    match layer="top"
    match layer="overlay"

    background-effect {
        xray false
    }
}
```
