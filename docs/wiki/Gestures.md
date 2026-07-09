### Overview

There are several gestures in niri.

Also see the [gestures configuration](./Configuration:-Gestures.md) wiki page.

### Mouse

#### Interactive Move

<sup>Since: 0.1.10</sup>

You can move windows by holding <kbd>Mod</kbd> and the left mouse button.

You can customize the look of the window insertion preview in the [`insert-hint` layout config](./Configuration:-Layout.md#insert-hint).

<sup>Since: 25.01</sup> Right click while moving to toggle between floating and tiling layout to put the window into.

#### Interactive Resize

<sup>Since: 0.1.6</sup>

You can resize windows by holding <kbd>Mod</kbd> and the right mouse button.

#### Reset Window Height

<sup>Since: 0.1.6</sup>

If you double-click on a top or bottom tiled window resize edge, the window height will reset to automatic.

This works with both window-initiated resizes (when using client-side decorations), and niri-initiated <kbd>Mod</kbd> + right click resizes.

#### Toggle Full Width

<sup>Since: 0.1.6</sup>

If you double-click on a left or right tiled window resize edge, the column will expand to the full workspace width.

This works with both window-initiated resizes (when using client-side decorations), and niri-initiated <kbd>Mod</kbd> + right click resizes.

#### Horizontal View Movement

<sup>Since: 0.1.6</sup>

Move the view horizontally by holding <kbd>Mod</kbd> and the middle mouse button (or the wheel) and dragging the mouse horizontally.

#### Workspace Switch

<sup>Since: 0.1.7</sup>

Switch workspaces by holding <kbd>Mod</kbd> and the middle mouse button (or the wheel) and dragging the mouse vertically.

### Touchpad

#### Workspace Switch

Switch workspaces with three-finger vertical swipes.

#### Horizontal View Movement

Move the view horizontally with three-finger horizontal swipes.

#### Open and Close the Overview

<sup>Since: 25.05</sup>

Open and close the overview with a four-finger vertical swipe.

### All Pointing Devices

#### Drag-and-Drop Edge View Scroll

<sup>Since: 25.02</sup>

Scroll the tiling view when moving the mouse cursor against a monitor edge during drag-and-drop (DnD).
Also works on a touchscreen.

#### Drag-and-Drop Edge Workspace Switch

<sup>Since: 25.05</sup>

Scroll the workspaces up/down when moving the mouse cursor against a monitor edge during drag-and-drop (DnD) while in the overview.
Also works on a touchscreen.

#### Drag-and-Drop Hold to Activate

<sup>Since: 25.05</sup>

While drag-and-dropping, hold your mouse over a window to activate it.
This will bring a floating window to the top for example.

In the overview, you can also hold the mouse over a workspace to switch to it.

#### Hot Corner to Toggle the Overview

<sup>Since: 25.05</sup>

Put your mouse at the very top-left corner of a monitor to toggle the overview.
Also works during drag-and-dropping something.
