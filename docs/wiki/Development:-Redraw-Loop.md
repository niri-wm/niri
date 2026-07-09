On a TTY, only one frame can be submitted to an output at a time, and the compositor must wait until the output repaints (indicated by a VBlank) to be able to submit the next frame.
In niri we keep track of this via the `RedrawState` enum that you can find in an `OutputState`.

Here's a diagram of state transitions for the `RedrawState` state machine:

<picture>
    <source media="(prefers-color-scheme: dark)" srcset="./img/RedrawState-dark.drawio.png">
    <img alt="RedrawState state transition diagram" src="./img/RedrawState-light.drawio.png">
</picture>

`Idle` is the default state, when the output does not need to be repainted.
Any operation that may cause the screen to update calls `queue_redraw()`, which moves the output to a `Queued` state.
Then, at the end of an event loop dispatch, niri calls `redraw()` for every `Queued` output.

If the redraw causes damage (i.e. something on the output changed), we move into the `WaitingForVBlank` state, since we cannot redraw until we receive a VBlank event.
However, if there's no damage, we do not return to `Idle` right away.
Instead, we set a timer to fire roughly at when the next VBlank would occur, and transition to a `WaitingForEstimatedVBlank` state.

This is necessary in order to throttle frame callbacks sent to applications to at most once per output refresh cycle.
Without this throttling, applications can start continuously redrawing without damage (for instance, if the application window is partially off-screen, and it is only the off-screen part that changes), and eating a lot of CPU in the process.

Then, either the estimated VBlank timer completes, and we go back to `Idle`, or maybe we call `queue_redraw()` once more and try to redraw again.
