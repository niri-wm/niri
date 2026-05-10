### Overview

<sup>Since: 0.1.10</sup>

Switch event bindings are declared in the `switch-events {}` section of the config.

Here are all the events that you can bind at a glance:

```kdl
switch-events {
    lid-close { spawn "notify-send" "The laptop lid is closed!"; }
    lid-open { spawn "notify-send" "The laptop lid is open!"; }
    tablet-mode-on { spawn "bash" "-c" "gsettings set org.gnome.desktop.a11y.applications screen-keyboard-enabled true"; }
    tablet-mode-off { spawn "bash" "-c" "gsettings set org.gnome.desktop.a11y.applications screen-keyboard-enabled false"; }
}
```

The syntax is similar to key bindings.
Currently, only the [`spawn` action](./Configuration:-Key-Bindings.md#spawn) are supported.

> [!NOTE]
> In contrast to key bindings, switch event bindings are *always* executed, even when the session is locked.

### `lid-close`, `lid-open`

These events correspond to closing and opening of the laptop lid.

Note that niri will already automatically turn the internal laptop monitor on and off in accordance with the laptop lid.

```kdl
switch-events {
    lid-close { spawn "notify-send" "The laptop lid is closed!"; }
    lid-open { spawn "notify-send" "The laptop lid is open!"; }
}
```

### `tablet-mode-on`, `tablet-mode-off`

These events trigger when a convertible laptop goes into or out of tablet mode.
In tablet mode, the keyboard and mouse are usually inaccessible, so you can use these events to activate the on-screen keyboard.

> [!NOTE]
> The commands below are just examples, you will need to provide your own on-screen keyboard, such as [sysboard](https://github.com/System64fumo/sysboard) or [wvkbd](https://github.com/jjsullivan5196/wvkbd).

```kdl
switch-events {
    tablet-mode-on { spawn "bash" "-c" "gsettings set org.gnome.desktop.a11y.applications screen-keyboard-enabled true"; }
    tablet-mode-off { spawn "bash" "-c" "gsettings set org.gnome.desktop.a11y.applications screen-keyboard-enabled false"; }
}
```
