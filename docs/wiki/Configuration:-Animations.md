### Overview

Niri has several animations which you can configure in the same way.
Additionally, you can disable or slow down all animations at once.

Here's a quick glance at the available animations with their default values.

```kdl
animations {
    // Uncomment to turn off all animations.
    // You can also put "off" into each individual animation to disable it.
    // off

    // Slow down all animations by this factor. Values below 1 speed them up instead.
    // slowdown 3.0

    // Individual animations.

    workspace-switch {
        spring damping-ratio=1.0 stiffness=1000 epsilon=0.0001
    }

    window-open {
        duration-ms 150
        curve "ease-out-expo"
    }

    window-close {
        duration-ms 150
        curve "ease-out-quad"
    }

    horizontal-view-movement {
        spring damping-ratio=1.0 stiffness=800 epsilon=0.0001
    }

    window-movement {
        spring damping-ratio=1.0 stiffness=800 epsilon=0.0001
    }

    window-resize {
        spring damping-ratio=1.0 stiffness=800 epsilon=0.0001
    }

    config-notification-open-close {
        spring damping-ratio=0.6 stiffness=1000 epsilon=0.001
    }

    exit-confirmation-open-close {
        spring damping-ratio=0.6 stiffness=500 epsilon=0.01
    }

    screenshot-ui-open {
        duration-ms 200
        curve "ease-out-quad"
    }

    overview-open-close {
        spring damping-ratio=1.0 stiffness=800 epsilon=0.0001
    }

    recent-windows-close {
        spring damping-ratio=1.0 stiffness=800 epsilon=0.001
    }
}
```

### Animation Types

There are two animation types: easing and spring.
Each animation can be either an easing or a spring.

#### Easing

This is a relatively common animation type that changes the value over a set duration using an interpolation curve.

To use this animation, set the following parameters:

- `duration-ms`: duration of the animation in milliseconds.
- `curve`: the easing curve to use.

```kdl
animations {
    window-open {
        duration-ms 150
        curve "ease-out-expo"
    }
}
```

Currently, niri only supports five curves.
You can get a feel for them on pages like [easings.net](https://easings.net/).

- `ease-out-quad` <sup>Since: 0.1.5</sup>
- `ease-out-cubic`
- `ease-out-expo`
- `linear` <sup>Since: 0.1.6</sup>
- `cubic-bezier` <sup>Since: 25.08</sup>
    A custom [cubic Bézier curve](https://www.w3.org/TR/css-easing-1/#cubic-bezier-easing-functions). You need to set 4 numbers defining the control points of the curve, for example:
    ```kdl
    animations {
        window-open {
            // Same as CSS cubic-bezier(0.05, 0.7, 0.1, 1)
            curve "cubic-bezier" 0.05 0.7 0.1 1
        }
    }
    ```
    You can tweak the cubic-bezier parameters on pages like [easings.co](https://easings.co?curve=0.05,0.7,0.1,1).

#### Spring

Spring animations use a model of a physical spring to animate the value.
They notably feel better with touchpad gestures, because they take into account the velocity of your fingers as you release the swipe.
Springs can also oscillate / bounce at the end with the right parameters if you like that sort of thing, but they don't have to (and by default they mostly don't).

Due to springs using a physical model, the animation parameters are less obvious and generally should be tuned with trial and error.
Notably, you cannot directly set the duration.
You can use the [Elastic](https://flathub.org/apps/app.drey.Elastic) app to help visualize how the spring parameters change the animation.

A spring animation is configured like this, with three mandatory parameters:

```kdl
animations {
    workspace-switch {
        spring damping-ratio=1.0 stiffness=1000 epsilon=0.0001
    }
}
```

The `damping-ratio` goes from 0.1 to 10.0 and has the following properties:

- below 1.0: underdamped spring, will oscillate in the end.
- above 1.0: overdamped spring, won't oscillate.
- 1.0: critically damped spring, comes to rest in minimum possible time without oscillations.

However, even with damping ratio = 1.0, the spring animation may oscillate if "launched" with enough velocity from a touchpad swipe.

> [!WARNING]
> Overdamped springs currently have some numerical stability issues and may cause graphical glitches.
> Therefore, setting `damping-ratio` above `1.0` is not recommended.

Lower `stiffness` will result in a slower animation more prone to oscillation.

Set `epsilon` to a lower value if the animation "jumps" at the end.

> [!TIP]
> The spring *mass* (which you can see in Elastic) is hardcoded to 1.0 and cannot be changed.
> Instead, change `stiffness` proportionally.
> E.g. increasing mass by 2× is the same as decreasing stiffness by 2×.

### Animations

Now let's go into more detail on the animations that you can configure.

#### `workspace-switch`

Animation when switching workspaces up and down, including after the vertical touchpad gesture (a spring is recommended).

```kdl
animations {
    workspace-switch {
        spring damping-ratio=1.0 stiffness=1000 epsilon=0.0001
    }
}
```

#### `window-open`

Window opening animation.

This one uses an easing type by default.

```kdl
animations {
    window-open {
        duration-ms 150
        curve "ease-out-expo"
    }
}
```

##### `custom-shader`

<sup>Since: 0.1.6</sup>

You can write a custom shader for drawing the window during an open animation.

See [this example shader](./examples/open_custom_shader.frag) for a full documentation with several animations to experiment with.

If a custom shader fails to compile, niri will print a warning and fall back to the default, or previous successfully compiled shader.
When running niri as a systemd service, you can see the warnings in the journal: `journalctl -ef /usr/bin/niri`

> [!WARNING]
>
> Custom shaders do not have a backwards compatibility guarantee.
> I may need to change their interface as I'm developing new features.

Example: open will fill the current geometry with a solid gradient that gradually fades in.

```kdl
animations {
    window-open {
        duration-ms 250
        curve "linear"

        custom-shader r"
            vec4 open_color(vec3 coords_geo, vec3 size_geo) {
                vec4 color = vec4(0.0);

                if (0.0 <= coords_geo.x && coords_geo.x <= 1.0
                        && 0.0 <= coords_geo.y && coords_geo.y <= 1.0)
                {
                    vec4 from = vec4(1.0, 0.0, 0.0, 1.0);
                    vec4 to = vec4(0.0, 1.0, 0.0, 1.0);
                    color = mix(from, to, coords_geo.y);
                }

                return color * niri_clamped_progress;
            }
        "
    }
}
```

#### `window-close`

<sup>Since: 0.1.5</sup>

Window closing animation.

This one uses an easing type by default.

```kdl
animations {
    window-close {
        duration-ms 150
        curve "ease-out-quad"
    }
}
```

##### `custom-shader`

<sup>Since: 0.1.6</sup>

You can write a custom shader for drawing the window during a close animation.

See [this example shader](./examples/close_custom_shader.frag) for a full documentation with several animations to experiment with.

If a custom shader fails to compile, niri will print a warning and fall back to the default, or previous successfully compiled shader.
When running niri as a systemd service, you can see the warnings in the journal: `journalctl -ef /usr/bin/niri`

> [!WARNING]
>
> Custom shaders do not have a backwards compatibility guarantee.
> I may need to change their interface as I'm developing new features.

Example: close will fill the current geometry with a solid gradient that gradually fades away.

```kdl
animations {
    window-close {
        custom-shader r"
            vec4 close_color(vec3 coords_geo, vec3 size_geo) {
                vec4 color = vec4(0.0);

                if (0.0 <= coords_geo.x && coords_geo.x <= 1.0
                        && 0.0 <= coords_geo.y && coords_geo.y <= 1.0)
                {
                    vec4 from = vec4(1.0, 0.0, 0.0, 1.0);
                    vec4 to = vec4(0.0, 1.0, 0.0, 1.0);
                    color = mix(from, to, coords_geo.y);
                }

                return color * (1.0 - niri_clamped_progress);
            }
        "
    }
}
```

#### `horizontal-view-movement`

All horizontal camera view movement animations, such as:

- When a window off-screen is focused and the camera scrolls to it.
- When a new window appears off-screen and the camera scrolls to it.
- After a horizontal touchpad gesture (a spring is recommended).

```kdl
animations {
    horizontal-view-movement {
        spring damping-ratio=1.0 stiffness=800 epsilon=0.0001
    }
}
```

#### `window-movement`

<sup>Since: 0.1.5</sup>

Movement of individual windows within a workspace.

Includes:

- Moving window columns with `move-column-left` and `move-column-right`.
- Moving windows inside a column with `move-window-up` and `move-window-down`.
- Moving windows out of the way upon window opening and closing.
- Window movement between columns when consuming/expelling.

This animation *does not* include the camera view movement, such as scrolling the workspace left and right.

```kdl
animations {
    window-movement {
        spring damping-ratio=1.0 stiffness=800 epsilon=0.0001
    }
}
```

#### `window-resize`

<sup>Since: 0.1.5</sup>

Window resize animation.

Only manual window resizes are animated, i.e. when you resize the window with `switch-preset-column-width` or `maximize-column`.
Also, very small resizes (up to 10 pixels) are not animated.

```kdl
animations {
    window-resize {
        spring damping-ratio=1.0 stiffness=800 epsilon=0.0001
    }
}
```

##### `custom-shader`

<sup>Since: 0.1.6</sup>

You can write a custom shader for drawing the window during a resize animation.

See [this example shader](./examples/resize_custom_shader.frag) for a full documentation with several animations to experiment with.

If a custom shader fails to compile, niri will print a warning and fall back to the default, or previous successfully compiled shader.
When running niri as a systemd service, you can see the warnings in the journal: `journalctl -ef /usr/bin/niri`

> [!WARNING]
>
> Custom shaders do not have a backwards compatibility guarantee.
> I may need to change their interface as I'm developing new features.

Example: resize will show the next (after resize) window texture right away, stretched to the current geometry.

```kdl
animations {
    window-resize {
        custom-shader r"
            vec4 resize_color(vec3 coords_curr_geo, vec3 size_curr_geo) {
                vec3 coords_tex_next = niri_geo_to_tex_next * coords_curr_geo;
                vec4 color = texture2D(niri_tex_next, coords_tex_next.st);
                return color;
            }
        "
    }
}
```

#### `config-notification-open-close`

The open/close animation of the config parse error and new default config notifications.

This one uses an underdamped spring by default (`damping-ratio=0.6`) which causes a slight oscillation in the end.

```kdl
animations {
    config-notification-open-close {
        spring damping-ratio=0.6 stiffness=1000 epsilon=0.001
    }
}
```

#### `exit-confirmation-open-close`

<sup>Since: 25.08</sup>

The open/close animation of the exit confirmation dialog.

This one uses an underdamped spring by default (`damping-ratio=0.6`) which causes a slight oscillation in the end.

```kdl
animations {
    exit-confirmation-open-close {
        spring damping-ratio=0.6 stiffness=500 epsilon=0.01
    }
}
```

#### `screenshot-ui-open`

<sup>Since: 0.1.8</sup>

The open (fade-in) animation of the screenshot UI.

```kdl
animations {
    screenshot-ui-open {
        duration-ms 200
        curve "ease-out-quad"
    }
}
```

#### `overview-open-close`

<sup>Since: 25.05</sup>

The open/close zoom animation of the [Overview](./Overview.md).

```kdl
animations {
    overview-open-close {
        spring damping-ratio=1.0 stiffness=800 epsilon=0.0001
    }
}
```

#### `recent-windows-close`

<sup>Since: 25.11</sup>

The close fade-out animation of the recent windows switcher.

```kdl
animations {
    recent-windows-close {
        spring damping-ratio=1.0 stiffness=800 epsilon=0.001
    }
}
```

### Synchronized Animations

<sup>Since: 0.1.5</sup>

Sometimes, when two animations are meant to play together synchronized, niri will drive them both with the same configuration.

For example, if a window resize causes the view to move, then that view movement animation will also use the `window-resize` configuration (rather than the `horizontal-view-movement` configuration).
This is especially important for animated resizes to look good when using `center-focused-column "always"`.

As another example, resizing a window in a column vertically causes other windows to move up or down into their new position.
This movement will use the `window-resize` configuration, rather than the `window-movement` configuration, to keep the animations synchronized.

A few actions are still missing this synchronization logic, since in some cases it is difficult to implement properly.
Therefore, for the best results, consider using the same parameters for related animations (they are all the same by default):

- `horizontal-view-movement`
- `window-movement`
- `window-resize`
