
# Niri shortcuts

**Running shortcuts** : "Mod" is a special modifier equal to `Super` when running on a TTY, and to `Alt` when running as a winit window.
Release binds allow to use a single modifier like `Mod` : press and release Mod by itself to trigger it ("Overview" is the only release bind in default).

**Defaults & customisation** : If you run niri without changing its configuration, default shortcuts will apply.

See [keybinds setting](./Configuration:-Key-Bindings.md) if you wish to amend these : you may change shortcuts keys and there are also *alternate commands* you may use, for example to move only a window rather than a full column which is default command. Some shortcuts may also be configured by using *percentage*, *hardcoded values*, ....

Note you may also use [gestures](./Gestures.md) to navigate and control windows.

# General shortcuts

If you forget some important shortcuts, niri might display these.

| Shortcut | Action |
| :--- | :--- |
| `Mod` + `Shift` + `/` | Shows a list of important hotkeys |
| `Mod` + `Alt` + `L` | Lock the Screen: swaylock |
| `Mod` + `Shift` + `P` | Power off monitors | 
| `Mod` + `Shift` + `E` | Quit niri |
| `Ctrl` + `Alt` + `Delete` | Quit niri |


## Running and closing applications

| Shortcut | Action |
| :--- | :--- |
| `Mod` + `T` | Open a Terminal: alacritty |
| `Mod` + `D` | Run an Application: fuzzel|


| Shortcut | Action |
| :--- | :--- |
| `Mod` + `Q` |  Close window |
| `Alt` + `F4` |  Close window |

# Basic Navigation & Window management

One of the methods to trigger the [overview](./Overview.md) is a default shortcut. It is a release mod. See the dedicated page for more details.

| Shortcut | Action |
| :--- | :--- |
| Release `Mod` | Open/close the Overview |


The usual vim inspired keys allows to move the focus.


| Shortcut | Action |
| :--- | :--- |
| `Mod` + `H` | Focus column left |
| `Mod` + `J` | Focus window down |
| `Mod` + `K` | Focus window up |
| `Mod` + `L` | Focus column right |
| `Mod` + `Home` | Focus first column |
| `Mod` + `End` | Focus last column |
| `Mod` + `C` | Center column |
| `Mod` + `Ctrl` + `C` | Center visible columns |

Arrows may also be used instead of *h j k l*...

| Shortcut | Action |
| :--- | :--- |
| `Mod` + `Left` | Focus column left |
| `Mod` + `Down` | Focus window down |
| `Mod` + `Up` | Focus window up |
| `Mod` + `Right` | Focus column right |


... or wheel scroll.

| Shortcut | Action |
| :--- | :--- |
| `Mod` + `Wheel Scroll Left` | Focus column left  |
| `Mod` + `Wheel Scroll Right` | Focus column right |

Adding `Ctrl` to navigation shortcuts will instead move the window. It also works with arrows or wheel scroll.

| Shortcut | Action |
| :--- | :--- |
| `Mod` + `Ctrl` + `H` | Move column left |
| `Mod` + `Ctrl` + `J` | Move window down |
| `Mod` + `Ctrl` + `K` | Move window up |
| `Mod` + `Ctrl` + `L` | Move column right |
| `Mod` + `Ctrl` + `Home` | Move column to the beginning |
| `Mod` + `Ctrl` + `End` | Move column to the end |
| `Mod` + `Ctrl` + `Wheel Scroll Left` | Move column left|
| `Mod` + `Ctrl` + `Wheel Scroll Right` | Move column right |


The window switcher allows to quickly reach a window. Once it is opened, some keys allow to filter windows.

| Shortcut | Action |
| :--- | :--- |
| `Alt` + `Tab` | Window Switcher |
| `S` | Window switcher scope: Cycle |
| `A` | Window switcher scope: All|
| `W` | Window switcher scope : Workspace |
| `O` | Window switcher scope : Output|
| Left | Window switcher : move left|
| Right | Window switcher : move right|
| Home | Window switcher : move to first|
| End | Window switcher : move to end|

# Screens

## Monitors

You may move the focus to another monitor holding the `Shift` key. Arrows also work.

| Shortcut | Action |
| :--- | :--- |
| `Mod` + `Shift` + `H` | Focus monitor left |
| `Mod` + `Shift` + `J` | Focus monitor down |
| `Mod` + `Shift` + `K` | Focus monitor up |
| `Mod` + `Shift` + `L` | Focus monitor right |

Similarly, holding `Shift` + `Control` allows to move a column to another screen. Arrows also work.

| Shortcut | Action |
| :--- | :--- |
| `Mod` + `Shift` + `Ctrl` + `H` | Move column to left monitor |
| `Mod` + `Shift` + `Ctrl` + `J` | Move column to down monitor |
| `Mod` + `Shift` + `Ctrl` + `K` | Move column to up monitor |
| `Mod` + `Shift` + `Ctrl` + `L` | Move column to right monitor |


## Workspaces

You can refer to [workspaces](./Workspaces.md) by index. However, keep in mind that niri is a dynamic workspace system, so these commands are kind of *best effort*.

| Shortcut | Action |
| :--- | :--- |
| `Mod` + `1–9` | Focus workspace 1–9 |
| `Mod` + `I` | Focus workspace up |
| `Mod` + `Page Up` | Focus workspace up |
| `Mod` + `Wheel Scroll Up` | Focus workspace up |
| `Mod` + `U` | Focus workspace down |
| `Mod` + `Wheel Scroll Down` | Focus workspace down |
| `Mod` + `Page Down` | Focus workspace down |


Hold `Control` to move a column to a workspace.

| Shortcut | Action |
| :--- | :--- |
| `Mod` + `Ctrl` + `1–9` | Move column to workspace 1–9 |
| `Mod` + `Ctrl` + `U` | Move column to workspace up |
| `Mod` + `Ctrl` + `Page Up` | Move column to workspace up |
| `Mod` + `Ctrl` + `Wheel Scroll Up` | Move column to workspace up |
| `Mod` + `Ctrl` + `I` | Move column to workspace down |
| `Mod` + `Ctrl` + `Page Down` | Move column to workspace down |
| `Mod` + `Ctrl` + `Wheel Scroll Down` | Move column to workspace down |


You may finally move workspaces.

| Shortcut | Action |
| :--- | :--- |
|    `Mod` + `Shift` + `Page_Down` | Move workspace down |
|    `Mod` + `Shift` + `Page_Up`  | Move workspace up |
|    `Mod` + `Shift` + `U`        | Move workspace up |
|    `Mod` + `Shift` + `I`        | Move workspace down |


# Columns, Tabs, Size


## Layouts management

| Shortcut | Action |
| :--- | :--- |
| `Mod` + `V` | toggle-window-floating |
| `Mod` + `Shift` + `V` | switch-focus-between-floating-and-tiling |
|  `Mod` + `W` | Toggle tabbed column display |
| `Mod` + `[` |  Consume or expel window left |
|    `Mod` + `]` | Consume or expel window right |
| `Mod` + `,` | Consume one window from the right to the bottom of the focused column |
| `Mod` +  `.` |  Expel the bottom window from the focused column to the right |

    

## Resizing

| Shortcut | Action |
| :--- | :--- |
| `Mod` + `F` | Maximize the column |
| `Mod` + `Shift` + `F` | Make window fullscreen |
| `Mod` + `Ctrl` + `F` | Expand column to max available width |
| `Mod` + `R` | Cycle through widths set |
| `Mod` + `Shift` + `R` | Cycle through widths set backward|
| `Mod` + `Ctrl` + `Shift` + `R` | Cycle through height set |
| `Mod` + `Ctrl` + `R` | Reset height |
| `Mod` + `M` | Maximise window to edge |
| `Mod` + `-` | set-window-height −10% |
| `Mod` + `=` | set-window-height +10% |
| `Mod` + `Shift` + `-` | Decrease window height −10% |
| `Mod` + `Shift` + `=` | Increase window height +10% |



# Additional keys

Example screenshot key mappings.

| Shortcut | Action |
| :--------------------- | :---------------------------------------- |
| `Print`           | Screenshot : zone       |
| `Ctrl` + `Print`  | Screenshot : screen            |
| `Alt` + `Print`  | Screenshot : active window          |

While you perform a zone screenshot, note certain shortcuts allow to move & resize region. If you amend these shortcuts then new shortcuts will apply.

| Shortcut | Action
|:-----|:-----|
| `Mod` + `Ctrl` + h   | Move screenshot region left |
| `Mod` + `Ctrl` + j   | Move screenshot region down |
| `Mod` + `Ctrl` + k   | Move screenshot region up  |
| `Mod` + `Ctrl` + l   | Move screenshot region right  |
| `Mod` +  `-`  | Shrink region horizontally  |
| `Mod` +  `+`  | Enlarge region horizontally |
| `Mod` +  `Shit` + `-`  | Shrink region vertically |
| `Mod` +  `Shit` + `+`  | Enlarge region vertically  | 


Example brightness key mappings for brightnessctl.

| Shortcut | Action |
| :---------------------| :----|
| `Bright+` / `Bright-`  | Increase / Decrease brightness (10%) |


Example media keys mappings for PipeWire & WirePlumber.

| Shortcut | Action |
| :--------------------- | :---------------------------------------- |
| `RaiseVolume`       | Increase volume |
| `LowerVolume`       | Lower volume |
| `Mute`                 | Mute audio      |
| `MicMute`              | Mute micro        |
| `Play`                 | Lecture / Pause (`playerctl`)             |
| `Prev` / `Next`        | Next / Previous track             |
| `Stop`                 | Stop                        |


In some cases you may need active window to consume shortcut rather than niri.

| Shortcut | Action |
| :--- | :--- |
| `Mod` + `Escape` | Pass shortcuts to active window rather than activate niri shortcuts  |
