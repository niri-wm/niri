Niri assumes that programs running unsandboxed on the host are **trusted**.

This is a reasonable assumption because programs running on the host have a wide variety of ways to get all access they need, even without niri.
For instance:

- They can set `$LD_PRELOAD` in `.bashrc` or similar files to load an arbitrary library into all processes.
- They can replace binaries in `$PATH` with malicious code.
- They can interpose any socket in `$XDG_RUNTIME_DIR`, like Wayland, and do keylogging or record window contents.
- They can scan the filesystem for secrets: SSH keys, password stores, etc.
- They can connect to an unlocked keyring and steal credentials.
- And so on and so forth.

## Unsandboxed clients

Anything with access to niri's Wayland socket can, among other things:

- Record the user's screen via [wlr-screencopy](https://wayland.app/protocols/wlr-screencopy-unstable-v1).
- Emulate input via [wlr-virtual-pointer](https://wayland.app/protocols/wlr-virtual-pointer-unstable-v1) and [virtual-keyboard](https://wayland.app/protocols/virtual-keyboard-unstable-v1).
- Get the user's clipboard contents via [wlr-data-control](https://wayland.app/protocols/ext-data-control-v1).
- Create arbitrary fullscreen surfaces through [wlr-layer-shell](https://wayland.app/protocols/wlr-layer-shell-unstable-v1) that can steal the user's input, pretend to be a password entry, or lock the user out of their session.
- Kill a running lockscreen, create a new lock surface, and tell niri to unlock a locked session.

Anything with access to niri's [IPC](./IPC.md) socket can, among other things:

- Spawn a Wayland client which can do everything in the list above.

Anything with access to niri's D-Bus interfaces can, among other things:

- Record the user's screen via the screencast interface.
- Fully listen to and emulate input from the user's keyboard via the accessibility interface.

Also, while niri doesn't directly integrate Xwayland, it's worth reminding that anything with access to the X11 `$DISPLAY` (which comes both as a socket file on disk **and** as an abstract socket in the network namespace) can intercept and emulate all input and record the contents of any X11 windows on the same `$DISPLAY` (but not Wayland windows).

## Running untrusted clients

Considering all of the above, for running untrusted clients, you need a proper sandbox that:

- Removes niri's IPC socket.
- Prevents D-Bus access to host services.
- Uses a filtered Wayland socket.

For creating a filtered Wayland socket, you can use the [security-context](https://wayland.app/protocols/security-context-v1) protocol which niri implements.
All unsafe protocols are made inaccessible through this filtered Wayland socket.

One sandbox that satisfies all of these criteria is the [Flatpak](https://flatpak.org/) sandbox.

Importantly, filtering just the Wayland socket (and leaving, for example, unrestricted D-Bus access) is **not enough** to prevent untrusted clients from doing bad things.

## Lock screen

When the session is locked via [ext-session-lock](https://wayland.app/protocols/ext-session-lock-v1), most actions (keybindings) are automatically disabled.
Only a very small set of safe actions is allowed.
In particular, spawning will not work, with the exception of binds explicitly configured with `allow-when-locked=true`.

Importantly, the **quit** action is allowed—you can always quit niri, even when on a lock screen.
Therefore, you must ensure that quitting niri does not drop you into an unprotected TTY commandline.
Usually, a display manager, like GDM, will do this for you: when niri exits (via the quit bind or if it crashes), it'll put you back into a safe password prompt.

Other than quitting, the only way to exit a lock screen is for the lock screen client to tell niri to unlock the session.
If the lock screen client crashes, the session remains locked with a solid red background.
In this case, another lock screen client can take over (so you can start a fresh lock screen if it crashes, and still unlock your session).
