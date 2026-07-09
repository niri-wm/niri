### Overview

<sup>Since: 25.01</sup>

Floating windows in niri always show on top of the tiled windows.
The floating layout does not scroll.
Each workspace/monitor has its own floating layout, just like each workspace/monitor has its own tiling layout.

New windows will automatically float if they have a parent (e.g. dialogs) or if they are fixed size (e.g. splash screens).
To change a window between floating and tiling, you can use the `toggle-window-floating` bind or right click while dragging/moving the window.
You can also use the `open-floating true/false` window rule to either force a window to open as floating, or to disable the automatic floating logic.

Use `switch-focus-between-floating-and-tiling` to switch the focus between the two layouts.
When focused on the floating layout, binds (like `focus-column-right`) will operate on the floating window.

You can precisely position a floating window with a command like `niri msg action move-floating-window -x 100 -y 200`.
