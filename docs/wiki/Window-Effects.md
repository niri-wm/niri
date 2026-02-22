### Overview

<sup>Since: next release</sup>

You can apply background effects to windows and layer-shell surfaces.
These include blur, xray, saturation, and noise.
They can be enabled in the `background-effect {}` section of [window](./Configuration:-Window-Rules.md#background-effect) or [layer](./Configuration:-Layer-Rules.md#background-effect) rules.

The window needs to be semitransparent for you to see the background effect (otherwise it's fully covered by the opaque window).
Focus ring and border can also cover the background effect, see [this FAQ entry](./FAQ.md#why-are-transparent-windows-tinted-why-is-the-borderfocus-ring-showing-up-through-semitransparent-windows) for how to change this.

### Blur

Windows and layer surfaces can request their background to be blurred via the [`ext-background-effect` protocol](https://wayland.app/protocols/ext-background-effect-v1).
In this case, the application will usually offer some "background blur" setting that you'll need to enable in its configuration.

You can also enable blur on the niri side with the `blur true` background effect window rule:

```kdl
// Enable blur behind the foot terminal.
window-rule {
    match app-id="^foot$"
 
    background-effect {
        blur true
    }
}

// Enable blur behind the fuzzel launcher.
layer-rule {
    match namespace="^launcher$"

    background-effect {
        blur true
    }
}
```

Blur enabled via the window rule will follow the window corner radius set via [`geometry-corner-radius`](./Configuration:-Window-Rules.md#geometry-corner-radius).
On the other hand, blur enabled through `ext-background-effect` will exactly follow the shape requested by the window.
If the window or layer has clientside rounded corners or other complex shape, it should set a corresponding blur shape through `ext-background-effect`, then it will get correctly shaped background blur without any manual niri configuration.

Global blur settings are configured in the [`blur {}` config section](./Configuration:-Miscellaneous.md#blur) and apply to all background blur.

### Xray

Xray makes the window background "see through" to your wallpaper, ignoring any other windows below.
You can enable it with `xray true` background effect [window](./Configuration:-Window-Rules.md#background-effect) or [layer](./Configuration:-Layer-Rules.md#background-effect) rule.

Xray is automatically enabled by default if any other background effect (like blur) is active.
This is because it's much more efficient: with xray active, niri only needs to blur the background once, and then can reuse this blurred version with no extra work (since the wallpaper changes very rarely).

#### Non-xray effects (experimental)

You can disable xray with `xray false` background effect window rule.
This gives you the normal kind of blur where everything below a window is blurred.
Keep in mind that non-xray blur and other non-xray effects are more expensive as niri has to recompute them any time you move the window, or the contents underneath change.

Non-xray effects are currently experimental because they have some known limitations.

- They disappear during window open/close animations and while dragging a tiled window.
Fixing this requries subframe support in the Smithay rendering code.

- Multiple clones of a non-xray background effect will interfere with each other and cause visual glitches.
You can see this if you enable non-xray effects on a bottom or background layer surface, then open the [Overview](./Overview.md).
Bottom and background layer surfaces are cloned on all workspaces that you can see in the Overview, causing interference.
Fixing this requires support for framebuffer effect clones in the Smithay rendering code.
