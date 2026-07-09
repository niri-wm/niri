### Overview

Niri has dynamic workspaces that can move between monitors.

Each monitor contains an independent set of workspaces arranged vertically.
You can switch between workspaces on a monitor with `focus-workspace-down` and `focus-workspace-up`.
Empty workspaces "in the middle" automatically disappear when you switch away from them.

There's always one empty workspace at the end (at the bottom) of every monitor.
When you open a window on this empty workspace, a new empty workspace will immediately appear further below it.

You can move workspaces up and down on the monitor with `move-workspace-up/down`.
The way to put a window on a new workspace "in the middle" is to put it on the last (empty) workspace, then move the workspace up to where you need.

Here's a visual representation that shows two monitors and their workspaces.
The left monitor has three workspaces (two with windows, plus one empty), and the right monitor has two workspaces (one with windows, plus one empty).

<picture>
    <source media="(prefers-color-scheme: dark)" srcset="./img/workspaces-dark.png">
    <img alt="Two monitors. First with three workspaces, second with two workspaces." src="./img/workspaces-light.png">
</picture>

You can move a workspace to a different monitor using binds like `move-workspace-to-monitor-left/right/up/down` and `move-workspace-to-monitor-next/previous`.

When you disconnect a monitor, its workspaces will automatically move to a different monitor.
But, they will also "remember" their original monitor, so when you reconnect it, the workspaces will automatically move back to it.

> [!TIP]
> From other tiling WMs, you may be used to thinking about workspaces like this: "These are all of my workspaces. I can show workspace X on my first monitor, and workspace Y on my second monitor."
> In niri, instead, think like this: "My first monitor contains these workspaces, including X and Y, and my second monitor contains these other workspaces. I can switch my first monitor to workspace X or Y. I can move workspace Y to my second monitor to show it there."

### Addressing workspaces by index

Several actions in niri can address workspaces "by index": `focus-workspace 2`, `move-column-to-workspace 4`.
This index refers to whichever workspace *currently happens to be* at this position on the focused monitor.
So, `focus-workspace 2` will always put you on the second workspace of the monitor, whichever workspace that currently is.

This is an important distinction from WMs with static workspace systems.
In niri, workspaces *do not have indices on their own*.
If you take the first workspace and move it further down on the monitor, `focus-workspace 1` will now put you on a different workspace (the one that was below the first workspace before you moved it).

When you want to have a more permanent workspace in niri, you can create a [named workspace](./Configuration:-Named-Workspaces.md) in the config or via the `set-workspace-name` action.
You can refer to named workspaces by name, e.g. `focus-workspace "browser"`, and they won't disappear when they become empty.

> [!TIP]
> You can try to emulate static workspaces by creating workspaces named "one", "two", "three", ..., and binding keys to `focus-workspace "one"`, `focus-workspace "two"`, ...
> This can work to some extent, but it can become somewhat confusing, since you can still move these workspaces up and down and between monitors.
>
> If you're coming from a static workspace WM, I suggest *not* doing that, but instead trying the "niri way" with dynamic workspaces, focusing and moving up/down instead of by index.
> Thanks to scrollable tiling, you generally need fewer workspaces than on a traditional tiling WM.

### Example workflow

This is how I like to use workspaces.

I will usually have my browser on the topmost workspace, then one workspace per project (or a "thing") I'm working on.
On a single workspace I have 1–2 windows that fit inside a monitor that I switch between frequently, and maybe extra windows scrolled outside the view, usually either ones I need rarely, or temporary windows that I quickly close.
When I need another permanent window, I'll put it on a new workspace.

I actively move workspaces up and down as I'm working on things to make what I need accessible in one motion.
For example, I usually frequently switch between the browser and whatever I'm doing, so I always move whatever I'm currently doing to right below the browser, so a single `focus-workspace-up/down` gets me where I want.
