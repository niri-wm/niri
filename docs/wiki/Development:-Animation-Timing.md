> *Time, Dr. Freeman? Is it really that... time again?*

A compositor deals with one or more monitors on mostly fixed refresh cycles.
For example, a 170 Hz monitor can draw a frame every ~5.88 ms.

Most of the time, the compositor doesn't actually redraw the monitor: when nothing changes on screen (e.g. you're reading a document and aren't moving your cursor), it would be wasteful to wake up the GPU to composite the same image.
During an animation however, screen contents do change every frame.
Niri will generally start drawing the next frame as soon as the previous one shows up on screen.

Since the monitor refresh cycle is fixed in most cases (even with VRR, there's a maximum refresh rate), the compositor can predict when the next frame will show up on the monitor, and render ongoing animations for that exact moment in time.
This way, all animation frames are perfectly timed with no jitter, regardless of when exactly the rendering code had a chance to run.
For example, even if the compositor has to process new window events, delaying the rendering by a few ms, the animation timing will remain exactly aligned to the monitor refresh cycle.

There are hence several properties that a compositor wants from its timing system.

1. It should be possible to get the state of the animations at a specific time in the near future, for rendering a frame exactly timed to when the monitor will show it.
    - This time override ability should be usable in tests to advance the time in a fully controlled fashion.
1. Animations in response to user actions should begin at the moment when the action happens.
   For example, pressing a workspace switch key should start the animation at the instant when the user pressed the key (rather than, say, slightly in the future where we predicted the next monitor frame, which we had already rendered by now).
1. During the processing of a single action, querying the current time should return the exact same value.
   Even if the processing finishes a few microseconds after it started, querying the time in the end should return the same thing.
   This generally makes writing code much more sane; otherwise you'd need to for example avoid reading the position of some element twice in a row, since it could have moved by one pixel in-between, screwing with the logic.
   Also, fetching the current system time [can be quite expensive](https://mastodon.online/@YaLTeR/109934977035721850) in terms of overhead.
1. It should be reasonably easy to implement an animation slow-down preference, so all animations can be slowed down or sped up by the same factor.

The solution in niri is a `LazyClock`, a clock that remembers one timestamp.
Initially, the timestamp is empty, so when you ask `LazyClock` for the current time, it will fetch and return the system time, and also remember it.
Subsequently, it will keep returning the same timestamp that it had remembered.

You can also clear the timestamp, then `LazyClock` will fetch the system time anew when it's needed.
In niri, the timestamp is cleared at the end of every event loop iteration, right before going to sleep waiting for new events.
This way, anything that happens next (like a user key press) will fetch and use the most up-to-date timestamp as soon as one is needed, but then the processing code will keep getting the exact same timestamp, since `LazyClock` stores it.

You can also just manually set the timestamp to a specific value.
This is how we render a frame for the predicted time of when the monitor will show it.
Also, this is used by tests: they simply always set the timestamp and never use the system time.

Finally, there's an `AdjustableClock` wrapper on top that provides the ability to control the slow-down rate by modifying the timestamps returned by the clock.

An important detail is that with rate changes, timestamps from the `AdjustableClock` will drift away and become unrelated to the system time.
However, our target timestamp (for rendering) comes from the system time, so the override works directly on the underlying `LazyClock`.
That is, overriding the timestamp and then querying the `AdjustableClock` will return a *different* timestamp that is correct and consistent with the adjustments made by `AdjustableClock`.
This is reflected in the API by naming the function `Clock::set_unadjusted()` (and there's also `Clock::now_unadjusted()` to get the raw timestamp).

The clock is shared among all animations in niri through passing around and storing a reference-counted pointer.
This way, overriding the time automatically applies to everything, whereas in tests we can use a separate clock per test so that they don't interfere with each other.
