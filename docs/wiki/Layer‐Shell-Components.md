Things to keep in mind with layer-shell components (bars, launchers, etc.):

1. When a [full-screen](./Fullscreen-and-Maximize.md) window is active and covers the entire screen, it will render above the top layer, and it will be prioritized for keyboard focus. If your launcher uses the top layer, and you try to run it while looking at a full-screen window, it won't show up. Only the overlay layer will show up on top of full-screen windows.
1. Components on the bottom and background layers will receive *on-demand* keyboard focus as expected. However, they will only receive *exclusive* keyboard focus when there are no windows on the workspace.
1. When opening the [Overview](./Overview.md), components on the bottom and background layers will zoom out and remain on the workspaces, while the top and overlay layers remain on top of the Overview. So, if you want the bar to remain on top, put it on the *top* layer.
