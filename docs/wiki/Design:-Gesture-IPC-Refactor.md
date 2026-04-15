# Gesture State via Environment Variables — Design Plan

> [!NOTE]
> This is an open design RFC tied to PR [niri-wm/niri#3771](https://github.com/niri-wm/niri/pull/3771).
> Feedback, counterproposals, and use-case testing from the niri community are welcome.

## Acknowledgments

The core architectural ideas in this document — env-var spawn context, stdin-pipe progress streaming, the public IPC event stream, `noop = consume` semantics, per-window `binds {}` in `window-rules` with an `unbound` sentinel for fingers=1/2 disambiguation, and the critique of the tag system as a layer violation — originated from **Atan-D-RP4** in PR review discussion on [niri-wm/niri#3771](https://github.com/niri-wm/niri/pull/3771). This document consolidates those proposals into an implementation plan and extends them in a few places (the internal-vs-IPC progress mismatch analysis in Part 11, and the earlier three-gate disambiguation sketch in Part 12 now superseded by Atan's window-rule `binds {}` proposal).

## Document Status and Reading Order

This document evolved in layers as design discussion progressed. Read it in order, but be aware that later parts **supersede** earlier ones in places:

- **Parts 1–9** — Initial spawn + env-var + stdin-pipe proposal (self-contained, covers the tag-replacement case)
- **Part 10** — Second-pass refinements: adds a public IPC event stream as a complementary channel, and proposes `noop = consume` semantics — this **supersedes Part 3d's claim that `noop` loses its purpose**
- **Part 11** — Cross-cutting concern: internal vs IPC progress mismatch (applies to all paths)
- **Part 12** — Disambiguation flow for `fingers=1`/`fingers=2` — this **supersedes Part 10c's "keep fingers=3..=10"** position. Originally a three-gate heuristic (passthrough rule + bind existence + threshold timing); **now superseded by Atan's per-window `binds {}` in `window-rules` proposal**, which collapses the three gates into one declarative mechanism. Scoped as a follow-up PR; the current PR stays at `fingers=3..=10`.

Net current thinking: three complementary user paths (spawn / IPC event stream / direct action), `noop` means "compositor claims this gesture," `unbound` in a window-rule `binds {}` block releases the claim per-app, and fingers=1/2 lands in a separate follow-up PR per Part 12.

## The Problem

The current tag system creates a **split-brain** between configuration and consumption:

1. The user writes a bind with `tag "workspace-nav"` in their niri config
2. A separate external app must independently know to connect to niri's IPC socket, subscribe to the event stream, and filter for events with tag `"workspace-nav"`
3. The bind config and the consuming app are coupled by a string convention that lives outside either one

This doesn't fit niri's design principle where **config declares intent and the compositor executes it**. Tags leak compositor-internal state into a global IPC namespace that external apps must subscribe to and parse.

## Proposal: Spawn with Gesture State in Environment Variables

When a gesture fires a `spawn` action, attach the gesture's state as environment variables to the spawned process. The script reads its own env, does its thing, and exits. No IPC socket, no tag matching, no event stream — fully self-contained.

## Detailed Design

### Part 1: Environment Variables (Static State at Spawn Time)

When `spawn` or `spawn-sh` fires from a gesture bind, set these env vars on the child process:

```sh
NIRI_GESTURE_TYPE=TouchSwipe         # TouchSwipe | TouchPinch | TouchRotate | TouchEdge | TouchpadSwipe
NIRI_GESTURE_FINGERS=3               # finger count
NIRI_GESTURE_DIRECTION=up            # up|down|left|right (swipe), in|out (pinch), cw|ccw (rotate)
NIRI_GESTURE_EDGE=left               # (edge only) top|bottom|left|right
NIRI_GESTURE_ZONE=start              # (edge only) full|start|center|end
NIRI_GESTURE_CONTINUOUS=true         # whether progress will stream on stdin
```

> [!NOTE]
> `sensitivity` and `natural_scroll` are **not** exposed as env vars. These are compositor-internal tuning — the compositor applies them when computing the `progress` value that streams on stdin. The spawned process receives already-adjusted progress and doesn't need to know or reapply these. This keeps the env vars focused on **what happened** (gesture identity) rather than **how it was configured** (tuning knobs).

This is the **easy part**. All this state is already available in `extract_bind_info()` and the `Trigger` enum at the point where `do_action` is called. The spawn functions (`spawn`, `spawn_sh`, `spawn_sync`) just need a new parameter for optional gesture context, and `spawn_sync` adds `.env()` calls before spawning.

**Config example (discrete gesture):**
```text
binds {
    TouchSwipe fingers=3 direction="up" {
        spawn "notify-send" "Swiped up with 3 fingers"
    }
}
```

The spawned `notify-send` sees `NIRI_GESTURE_TYPE=TouchSwipe`, `NIRI_GESTURE_FINGERS=3`, etc. in its env. For discrete gestures this is all you need — the script runs, reads env, done.

### Part 2: stdin Pipe (Dynamic State for Continuous Gestures)

This is the **hard part** and where the real architectural value is.

For continuous gestures (workspace-switch, overview, column-nav animations), the spawned process needs **live progress updates** as fingers move. Environment variables are write-once-at-spawn; they can't carry streaming state.

**Solution: pipe progress to the child's stdin.**

Currently, `spawn_sync` sets `stdin(Stdio::null())`. For continuous gesture spawns, change this to `stdin(Stdio::piped())` and keep the write-end of the pipe alive in the gesture's active state.

#### Data format on stdin

One JSON object per line (newline-delimited JSON / NDJSON):

```jsonl
{"event":"progress","progress":0.15,"dx":0.0,"dy":-8.3,"timestamp_ms":48201}
{"event":"progress","progress":0.42,"dx":0.0,"dy":-12.1,"timestamp_ms":48217}
{"event":"progress","progress":0.73,"dx":0.0,"dy":-9.7,"timestamp_ms":48233}
{"event":"end","completed":true}
```

- `progress`: normalized, non-monotonic (same semantics as current `GestureProgress.progress`)
- `dx`/`dy` or `d_spread` or `d_angle`: raw physical delta, typed by gesture kind
- `timestamp_ms`: frame timestamp
- Final `{"event":"end","completed":true/false}` then stdin closes

A bash script consuming this looks like:

```bash
#!/bin/bash
# NIRI_GESTURE_TYPE, NIRI_GESTURE_FINGERS, etc. are in our env
echo "Gesture started: $NIRI_GESTURE_TYPE with $NIRI_GESTURE_FINGERS fingers"

while IFS= read -r line; do
    progress=$(echo "$line" | jq -r '.progress // empty')
    event=$(echo "$line" | jq -r '.event')
    
    if [ "$event" = "end" ]; then
        completed=$(echo "$line" | jq -r '.completed')
        echo "Gesture ended, completed=$completed"
        break
    fi
    
    # Drive your animation with $progress
    echo "Progress: $progress"
done
```

A Rust/Python/Go consumer reads stdin line-by-line and deserializes JSON.

### Part 3: Architectural Changes Required

#### 3a. Spawn infrastructure (`src/utils/spawning.rs`)

Current signatures:
```rust
pub fn spawn<T>(command: Vec<T>, token: Option<XdgActivationToken>)
pub fn spawn_sh(command: String, token: Option<XdgActivationToken>)
fn spawn_sync(command, args, token)
```

The existing signatures gain an optional gesture context parameter. Internally, `spawn` checks whether it's in a gesture context and adjusts its behavior:

```rust
// Same function, now context-aware
pub fn spawn<T>(command: Vec<T>, token: Option<XdgActivationToken>, gesture: Option<GestureSpawnContext>)
    -> Option<std::fs::File>  // None for keyboard spawns; Some(pipe) for gesture spawns
```

When `gesture` is `Some`:
- Sets `NIRI_GESTURE_*` env vars on the child
- Uses `Stdio::piped()` for stdin and returns the write-end
- The process always gets the pipe — whether it reads stdin is the process's choice

When `gesture` is `None`: behaves exactly as today (no env vars, `Stdio::null()`), returns `None`.

This is the "spawn action has different behavior when called with these binds" pattern — the function itself is context-aware, not a new function.

**Key concern: the double-fork.** Currently:
1. Main thread → spawner thread → `Command::spawn()` → intermediate child → grandchild (actual process)
2. Intermediate child exits immediately, grandchild is orphaned to init/systemd

With stdin piping, the pipe's write-end must stay alive in the **compositor process** (not the spawner thread), because the gesture handler runs on the main thread and needs to write to it on every motion frame.

Implementation approach for continuous spawns:
- Do the piped spawn synchronously on the main thread — fork+exec is fast (<1ms), and gesture commit only happens once per gesture, so blocking briefly is fine
- The double-fork still happens for process isolation, but the pipe write-end stays in compositor space
- This avoids the complexity of threading pipe fds back from a spawner thread via channels

#### 3b. Action dispatch (`src/input/mod.rs` and `src/input/touch_gesture.rs`)

Currently, `do_action` handles `Action::Spawn` generically with no context about what triggered it. The gesture code should intercept spawn actions before they reach `do_action`:

```rust
// In touch_gesture.rs / mod.rs, at the point where a gesture bind fires:
if matches!(action, Action::Spawn(_) | Action::SpawnSh(_)) {
    let ctx = GestureSpawnContext::from_trigger(trigger, continuous);
    let pipe = match action {
        Action::Spawn(cmd) => spawn(cmd, Some(token), Some(ctx)),
        Action::SpawnSh(cmd) => spawn_sh(cmd, Some(token), Some(ctx)),
        _ => unreachable!(),
    };
    // If continuous, store pipe in ActiveTouchBind/ActiveSwipeBind
} else {
    self.do_action(action, false);
}
```

This keeps `do_action` untouched — it doesn't need to know about gestures. The gesture code is the one that knows it's in a gesture context, so it handles spawn specially. All other actions (workspace-switch, focus-column, etc.) flow through `do_action` as before.

#### 3c. Active gesture state (`src/niri.rs`)

For continuous gesture spawns, the pipe write-end needs to live in the active gesture state so progress updates can write to it. With tags removed, these structs simplify — `tag` and `ipc_progress` are gone, replaced by `spawn_pipe`:

```rust
pub struct ActiveSwipeBind {
    pub kind: ContinuousGestureKind,
    pub sensitivity: f64,
    pub spawn_pipe: Option<std::fs::File>,  // write-end for spawned process stdin
}

pub enum ActiveTouchBind {
    Swipe {
        kind: ContinuousGestureKind,
        sensitivity: f64,
        natural_scroll: bool,
        spawn_pipe: Option<std::fs::File>,
    },
    Pinch {
        kind: ContinuousGestureKind,
        spawn_pipe: Option<std::fs::File>,
        start_spread: f64,
        last_spread: f64,
    },
    Rotate {
        kind: ContinuousGestureKind,
        spawn_pipe: Option<std::fs::File>,
        start_rotation: f64,
    },
}
```

On each gesture progress frame, if `spawn_pipe` is `Some`, write a JSON progress line to it. On gesture end, write the end event and drop the pipe (closes stdin).

**EPIPE handling:** If the child process exits early, writing to the pipe will return EPIPE. This must be handled gracefully — just set `spawn_pipe = None` and continue (the gesture still drives compositor animations even if the external process died).

#### 3d. Tags are removed entirely

Since this is a private prototype, we don't need backwards compatibility. Tags are **replaced**, not supplemented.

**What gets removed:**
- `tag: Option<String>` from `Bind`, `TouchBindEntry`, `ActiveTouchBind`, `ActiveSwipeBind`
- `GestureBegin`, `GestureProgress`, `GestureEnd` IPC events (the tag-bearing ones)
- Tag field in the settings UI
- All `ipc_gesture_begin/progress/end` emission logic in `touch_gesture.rs` and `mod.rs`

**What replaces each use case:**

| Old (tags) | New |
|-----------|-----|
| Script reacts to a specific gesture | `spawn` + env vars |
| Animation driven by gesture progress | `spawn` + stdin pipe |
| Debug inspector sees all gestures | `RecognitionFrame` events (already exist, debug-only) — or we add a new lightweight `GestureEvent` on the IPC stream that carries the same env-var-level info but without requiring a tag in config |
| Long-running daemon monitors gestures | Same IPC stream, but tag-free: events identify gestures by type/fingers/direction, not user-assigned strings |

**The `noop` action loses its gesture-specific purpose** *(superseded — see Part 10b).* Originally this proposal drops `noop`'s special meaning along with tags, but second-pass refinements reintroduce `noop = consume` as the "compositor claims this gesture for IPC consumption" signal. The current position is: `noop` gains new meaning as the consume marker, not loses it. See Part 10b for details.

**What about `niri-gesture-inspector`?** It currently uses `GestureBegin`/`GestureEnd` events. Two options:
1. Keep a simplified, tag-free `GestureEvent` on the IPC stream (just type/fingers/direction/completed, no user tag)
2. The inspector already uses `RecognitionFrame` events — extend those slightly to cover the commit/end phase too

Option 1 is cleaner: a single `GestureEvent` that fires on every gesture commit, carrying the trigger description and completion status. No tag field, no user config needed — it's purely observational.

### Part 4: Implementation Phases

#### Phase 0: Rip out tags

**Scope:** Remove the entire tag system — config field, IPC events, emission logic, UI.

**Changes:**
1. Remove `tag: Option<String>` from `Bind` in `niri-config/src/binds.rs`
2. Remove `tag` from `ActiveTouchBind` variants and `ActiveSwipeBind` in `niri.rs`
3. Remove `GestureBegin`, `GestureProgress`, `GestureEnd` event variants from `niri-ipc/src/lib.rs`
4. Remove all `ipc_gesture_begin/progress/end` calls in `touch_gesture.rs` and `mod.rs`
5. Remove `extract_bind_info`'s tag extraction, simplify the tuple it returns
6. Remove `noop` action support from gesture binds (or keep `noop` as a general action but remove its special tag-emitting behavior)
7. Remove tag field from `TouchBindEntry` in the settings UI `config.rs`
8. Remove tag rows from touchscreen.rs and touchpad.rs add/edit forms
9. Add a simple, tag-free `GestureEvent` to the IPC stream for debug tools:
   ```rust
   GestureCommit {
       trigger: String,       // "TouchSwipe fingers=3 direction=\"up\""
       finger_count: u8,
       is_continuous: bool,
   }
   GestureFinish {
       trigger: String,
       completed: bool,
   }
   ```
   These fire for ALL gesture commits unconditionally — no config needed. Debug tools (gesture-inspector) observe the stream without any bind config.

**Complexity:** Medium (lots of deletion, but deletion is safe). The new `GestureCommit`/`GestureFinish` events are simpler than the old tagged trio because they have no user-defined fields.

#### Phase 1: Environment variables + stdin pipe

**Scope:** Make `spawn` context-aware when fired from gesture binds — env vars for identity, stdin pipe for progress.

These ship together because the pipe is what makes this a real replacement for tags. Env vars without the pipe only covers discrete gestures; with the pipe, it covers everything.

**Changes:**
1. Define `GestureSpawnContext` struct in `spawning.rs`:
   ```rust
   pub struct GestureSpawnContext {
       pub gesture_type: String,       // "TouchSwipe", "TouchPinch", etc.
       pub fingers: u8,
       pub direction: Option<String>,  // "up", "in", "cw", etc.
       pub edge: Option<String>,       // "left", "top", etc.  (edge gestures only)
       pub zone: Option<String>,       // "full", "start", etc. (edge gestures only)
   }
   ```
   Note: no `continuous` flag — the compositor determines this from the gesture type. All gesture spawns get the pipe; `NIRI_GESTURE_CONTINUOUS` env var tells the process whether to expect progress data on stdin.
2. Modify `spawn`/`spawn_sh`/`spawn_sync` to accept `Option<GestureSpawnContext>` — when present, set `NIRI_GESTURE_*` env vars and use `Stdio::piped()` for stdin
3. Return `Option<std::fs::File>` (pipe write-end) from spawn — `Some` for gesture spawns, `None` for keyboard spawns
4. Add `spawn_pipe: Option<std::fs::File>` to `ActiveTouchBind` variants and `ActiveSwipeBind`
5. In `touch_gesture.rs` and `mod.rs`, intercept `Action::Spawn`/`Action::SpawnSh` before `do_action` — build context from trigger, call gesture-aware spawn, store pipe in active state
6. On gesture progress, write NDJSON line to the pipe (`O_NONBLOCK`, skip frame on `EAGAIN`)
7. On gesture end, write `{"event":"end","completed":true/false}` and drop the pipe
8. Handle EPIPE: set `spawn_pipe = None`, continue gesture normally
9. **Refactor spawn for piped mode:** do the piped spawn synchronously on the main thread (fork+exec is fast, <1ms). Double-fork still happens for process isolation, but pipe write-end stays in compositor space

**Complexity:** Medium-high. The spawn architecture change for piped mode is the hardest part. Non-blocking pipe writes at 120 Hz need care.

**Value:** Full replacement for tags. Discrete scripts read env vars and ignore stdin. Continuous scripts read env vars and stdin. Same `spawn` action, compositor handles the rest.

#### Phase 2: Reserved

Originally skipped in the first-pass plan. **Filled in by Part 10d** as the `noop = consume` phase (replacing the `touchscreen-gesture-passthrough` window rule with bind-existence consumption semantics).

#### Phase 3: Settings UI updates

**Scope:** Update niri-touch-settings-UI to reflect the tag-free model.

**Changes:**
1. Remove all tag-related UI (already done in Phase 0)
2. Remove `noop` from the action list for gesture binds (or keep it for "gesture exists but does nothing visible")
3. For `spawn` actions, add a help label: "Spawned processes receive gesture state via NIRI_GESTURE_* environment variables"
4. Consider adding a "Test" button that spawns a built-in script showing the env vars (nice-to-have)

**Complexity:** Low.

### Part 5: Non-Trivial Concerns

#### 5a. Spawn latency on the main thread

Currently spawn runs in a separate thread to avoid blocking the compositor. For piped spawns, we need the pipe fd on the main thread. Options:
- Fork+exec is fast (~1ms) — doing it on the main thread for gesture spawns is probably fine, especially since gesture commit only happens once per gesture
- Or: spawn in thread, send pipe fd back via a one-shot channel

#### 5b. Pipe write blocking at 120 Hz

If the child doesn't read fast enough, the pipe buffer fills and `write()` blocks. Solutions:
- Use `O_NONBLOCK` on the write-end; if write returns `EAGAIN`, skip that frame (child will see the next one)
- Pipe buffer is typically 64KB on Linux — at ~100 bytes per JSON line, that's ~640 frames of buffer, more than enough

#### 5c. Child process lifecycle

The child is spawned at gesture begin. What if:
- **Child exits early:** EPIPE on write → set `spawn_pipe = None`, continue gesture normally
- **Gesture ends before child is done:** Write end event, drop pipe (stdin closes), child sees EOF and should exit. If it doesn't, it's the child's problem (orphaned to init, same as any spawn)
- **Multiple rapid gestures:** Each spawns a new process. Previous one gets EOF'd when the gesture ends. This is fine — same as spawning any command rapidly

#### 5d. Security / information leak surface

Current spawn already inherits the compositor's environment (minus RUST_BACKTRACE, plus DISPLAY and user-configured env). Adding gesture state doesn't meaningfully expand the attack surface — the spawned process already runs with the user's privileges. The gesture info (fingers, direction) isn't sensitive.

The stdin pipe is scoped to the child process's fd table — no other process can read it (unlike the IPC socket which any process can connect to). This is actually **better** isolation than tags.

### Part 6: Config Example — Before and After

**Before (tags + external daemon):**
```text
// niri config — user must invent a tag name
binds {
    TouchSwipe fingers=3 direction="up" tag="ws-up" { noop; }
}

// Separate daemon that must:
// 1. Be running before the gesture fires
// 2. Connect to niri's IPC socket
// 3. Subscribe to the event stream
// 4. Know the exact tag string "ws-up"
// 5. Filter events, handle begin/progress/end lifecycle

// daemon.py:
//   socket = connect_niri_ipc()
//   for event in socket.event_stream():
//     if event.type == "GestureBegin" and event.tag == "ws-up":
//       handle_begin(event)
//     elif event.type == "GestureProgress" and event.tag == "ws-up":
//       drive_animation(event.progress)
//     elif event.type == "GestureEnd" and event.tag == "ws-up":
//       finish(event.completed)
```

**After (spawn + env vars + stdin):**
```text
// niri config — no tag, no daemon coordination
binds {
    TouchSwipe fingers=3 direction="up" {
        spawn-sh "my-gesture-handler.sh"
    }
}
```

```bash
#!/bin/bash
# my-gesture-handler.sh — fully self-contained
# Everything we need is in our environment:
echo "Got $NIRI_GESTURE_TYPE $NIRI_GESTURE_DIRECTION with $NIRI_GESTURE_FINGERS fingers"

# For continuous gestures, progress streams on stdin:
if [ "$NIRI_GESTURE_CONTINUOUS" = "true" ]; then
    while IFS= read -r line; do
        progress=$(echo "$line" | jq -r '.progress // empty')
        event=$(echo "$line" | jq -r '.event')
        [ "$event" = "end" ] && break
        # Drive animation with $progress
    done
fi
```

No tag, no daemon, no IPC socket, no event stream, no string coordination between config and consumer. The process is born knowing everything about its gesture.

### Part 7: What This Means for the Architecture

**Tags were a layer-violation.** They made the compositor's IPC stream carry user-defined semantics (arbitrary tag strings) that only had meaning to external processes. The compositor itself never used the tag — it just forwarded it. This is the separation concern that motivated the rethink.

**Env vars + stdin is compositor-native.** The compositor already spawns processes with enriched environments (`XDG_ACTIVATION_TOKEN`, `DISPLAY`, user-configured env). Adding gesture state to the spawn environment is the same pattern — the compositor prepares the child's world, the child runs in it.

**The stdin pipe replaces the IPC event stream for the per-gesture case.** Instead of a global pub-sub channel (IPC event stream) where consumers filter by tag, each gesture gets a dedicated, private, typed channel (stdin pipe) that lives exactly as long as the gesture. This is:
- **Better isolated** — no cross-gesture interference, no global namespace
- **Simpler lifecycle** — pipe opens at gesture begin, closes at gesture end, no subscribe/unsubscribe
- **Self-cleaning** — when the gesture ends, the pipe closes, the process gets EOF, done

**The IPC event stream survives** but becomes simpler: tag-free `GestureCommit`/`GestureFinish` events for observability tools, not for driving application logic. This is the right separation — the IPC stream is for *watching*, spawn is for *doing*.

### Part 8: How `spawn` Becomes Context-Aware (Discrete vs Continuous)

There's a key design question: **how does the compositor know whether a gesture spawn is discrete or continuous?**

Currently, continuous vs discrete is inferred from the action type — `workspace-switch-gesture` is continuous because the compositor knows it drives an animation. `spawn` is always discrete (fire and forget). But with env-var spawn replacing tags, we need `spawn` to handle both modes.

The guiding principle: the **spawn action** has different behavior **when called from a gesture bind** — `spawn` itself becomes context-aware, not a new action type.

#### The approach: `spawn` always pipes on gesture binds

When `spawn` fires from a gesture bind, the compositor always sets up the stdin pipe for gestures that support continuous tracking (swipe, pinch, rotate, edge). The process gets `NIRI_GESTURE_CONTINUOUS=true` in its env and progress streams on stdin.

If the process doesn't care about continuous progress, it simply doesn't read stdin. The kernel pipe buffer (64KB on Linux, ~640 frames of JSON) absorbs the writes silently. When the gesture ends and the pipe closes, any unread data is discarded. No harm done.

```text
// Discrete use — script ignores stdin, reads env, does its thing
TouchSwipe fingers=3 direction="up" {
    spawn "notify-send" "Swiped up!"
}

// Continuous use — same spawn, script reads stdin for progress
TouchSwipe fingers=4 direction="up" {
    spawn-sh "my-animation-driver.sh"
}
```

Both are just `spawn`. The compositor doesn't need to know the script's intent. The pipe is always there; the script decides whether to use it.

- From a keyboard/switch bind: `spawn` fires as today — no env vars, no pipe, `Stdio::null()`
- From a gesture bind: `spawn` sets `NIRI_GESTURE_*` env vars and pipes progress on stdin

This is the most "niri" answer — the compositor figures out the right thing from context, the user just writes `spawn`. No new action types, no new config properties, no user-facing complexity.

#### Alternatives considered and rejected

- **`spawn-continuous` as a separate action:** Explicit, but doubles the action surface (spawn/spawn-sh/spawn-continuous/spawn-sh-continuous) for no real gain — the compositor already knows the context.
- **`spawn-gesture` as a new action:** Clean separation, but means `spawn` on a gesture bind would be "dumb" (no env vars), wasting the opportunity to enrich the existing action. Splits the action surface unnecessarily.
- **`continuous=true` bind property:** Unnecessary indirection — if the script doesn't want progress, it just doesn't read stdin.

### Part 9: Open Questions

1. **Progress format on stdin:** NDJSON (`{"progress":0.42,"dx":0.0,"dy":-12.1,"timestamp_ms":48217}`) is flexible and self-describing but requires a JSON parser. Alternative: tab-separated values (`0.42\t0.0\t-12.1\t48217`) — trivial to parse in bash with `read`, but harder to extend. JSON is probably the right default since niri's IPC already speaks JSON, and most languages parse it trivially. Feedback welcome on whether a simpler line format would better serve shell-script use cases.

2. **Does spawn fully replace the external-daemon pattern?** With tags, a long-running daemon could monitor *all* gestures from one process. With spawn, each gesture gets its own short-lived process. For most users this is simpler, but a power-user who wants a single daemon reacting to multiple gesture types would need either: (a) the tag-free `GestureCommit`/`GestureFinish` IPC events proposed in Phase 0, or (b) multiple spawn binds that all call the same script (which reads its env to know which gesture fired). Option (b) is probably fine — the "daemon" pattern was always over-engineered for most use cases, but real use cases that break under (b) would be worth hearing about.

3. **Does this fully address the layering concern?** The separation concern (config ↔ external app coupled by string convention) is eliminated: the process gets its context from the compositor directly via env vars, no string convention needed. But the process itself is still external — is `spawn` + env vars enough "niri-native", or would something more integrated (e.g., compositor-internal scripting for animation logic) be preferable? Opinions welcome.

---

## Part 10: Second-Pass Refinements (2026-04)

After the initial proposal (Parts 1–9), a second round of design discussion raised additional ideas. This section captures those refinements and how they interact with the original plan.

### 10a. Prefer IPC event stream over spawn-pipe for observers

A complementary channel was proposed:

> Extending the IPC with something like `niri msg watch-gestures` or `niri msg event-stream --filter gestures`. Emit events with associated data (which can be limited or expanded via config with some sane defaults) for all committed gestures and clients can subscribe to those events to do things.

**How this relates to the existing spawn+pipe proposal:**

- The spawn+pipe approach (Phase 1 above) is still valuable for **self-contained per-gesture scripts** — no IPC subscription required, the process is born knowing its context.
- But for **long-running observers** (quickshell panels, sidebar drawers, HUDs), a public IPC event stream is the right channel — a daemon subscribes once and reacts to all gestures.
- These are **complementary**, not competing. Both should exist.

**Refinement to Phase 0:** The "tag-free `GestureCommit`/`GestureFinish`" event from Phase 0 gets upgraded from a minor observability aside to a **first-class public API**:

```console
$ niri msg event-stream | grep -E "GestureBegin|GestureProgress|GestureEnd"
GestureBegin trigger="TouchEdge edge=\"left\"" fingers=1 continuous=true
GestureProgress trigger="TouchEdge edge=\"left\"" progress=0.23 dx=0.0 dy=-12.1 timestamp_ms=48217
GestureProgress trigger="TouchEdge edge=\"left\"" progress=0.47 dx=0.0 dy=-18.4 timestamp_ms=48233
GestureEnd trigger="TouchEdge edge=\"left\"" completed=true
```

Trigger field is the **same string the user writes in config** — no invented tags, direct pattern-match. A sidebar daemon filters by `trigger="TouchEdge edge=\"left\""`, not by a user-assigned tag.

Fields available on the stream (optionally filterable via config):
- `trigger` — e.g. `"TouchSwipe fingers=3 direction=\"up\""`
- `fingers` — finger count
- `continuous` — whether progress will stream
- `progress`, `delta`, `timestamp_ms` — on `GestureProgress` events
- `completed` — on `GestureEnd` events

### 10b. `noop = consume` semantics (replaces `touchscreen-gesture-passthrough`)

A proposed consumption model based on whether a gesture is bound:

> If the IPC is used to let some app handle a gesture, bind it to `noop` in `binds {}` with a comment, and niri knows not to forward it to clients. Without a noop bind, it gets forwarded. This removes the need for the `touchscreen-gesture-passthrough` window-rule as a simplification.

**The claim/forward decision:**

| Bind state | Compositor action | Client receives event |
|------------|-------------------|----------------------|
| No bind for this gesture | Nothing | Yes (forwarded) |
| Bound to concrete action | Executes action, emits IPC | No (consumed) |
| Bound to `noop` | Does nothing, emits IPC | No (consumed by IPC daemon) |

This replaces the current `touchscreen-gesture-passthrough` window rule semantics for the **gesture family level**. The window rule becomes unnecessary for the "block gesture passthrough for this trigger" use case — just bind it to `noop` or an action.

**Caveat:** The current `touchscreen-gesture-passthrough` is a **window rule** — scoped per-window. The proposed model is **global per-trigger**. These aren't fully equivalent:

- Today: window rule lets a browser handle its own 2-finger gestures while niri still handles them over other windows
- Proposed model: binding is global — you can't say "don't intercept 3-finger swipes over this specific window"

Whether that loss of per-window scoping matters in practice is a separate design question. For the 3+ finger compositor-gesture use case it probably doesn't (users want the same gestures everywhere). For the 2-finger scroll/zoom passthrough it still matters — but that's handled by finger-range restriction, not by `noop`.

**Conclusion on the window rule:** Originally considered for removal once `noop = consume` is in place. **However, Part 12 re-introduces it as Gate 1 of the fingers=1/2 disambiguation model** — the per-app escape hatch for apps like Firefox/PDF viewers that need native 1/2-finger touch. So the window rule stays if fingers=1/2 support ships; it can only be removed if the finger range stays at 3+.

### 10c. Open question: fingers=1/2 with noop-consume

A related suggestion:

> Expand the valid finger range down to 1 finger. Especially useful for edge gestures.

This interacts with `noop = consume` in a concerning way:

- Binding `TouchSwipe fingers=1 direction="up" { noop; }` would **globally claim** all 1-finger up-swipes
- Every app loses its primary scroll interaction
- Email lists, photo viewers, web pages — all broken

The `noop = consume` model works cleanly for 3+ finger gestures because they don't overlap with primary client input. Extending it to 1-2 fingers requires a spatial/temporal disambiguation mechanism — exactly what `TouchEdge` already provides via `edge-start-distance`.

**Suggested position at the time of 10c (superseded — see Part 12):** Keep `fingers=3..=10` as the range for the 5 non-edge families. `TouchEdge` remains the 1-finger option (spatially restricted to avoid client conflict). If someone wants middle-of-screen 1-finger gestures, they need to propose a spatial/temporal disambiguation mechanism — not just expand the range.

**Current position (Part 12):** That disambiguation mechanism exists as the three-gate model (window rule → bind-existence → threshold timing), making fingers=1/2 viable as an opt-in-per-pattern feature with zero cost for users who don't write such binds. See Part 12 for the full flow.

### 10d. Refined implementation sketch

Combining the original plan with the second-pass refinements:

**Phase 0 — Rip out tags, add public gesture event stream**
- Remove `tag: Option<String>` everywhere
- Add `GestureBegin`/`GestureProgress`/`GestureEnd` events to the public IPC stream, emitted for **all** committed gestures (no opt-in tag needed)
- Events carry `trigger` (config-matching string), `fingers`, `continuous`, `progress`, `delta`, `completed`
- This replaces the current tag-gated events with a universal stream

**Phase 1 — spawn + env vars + stdin pipe**
- Unchanged from the original proposal
- For per-gesture self-contained scripts that don't need IPC

**Phase 2 — `noop = consume` semantics (new)**
- Replace the current `touch_gesture_passthrough` check with: bound gesture (including `noop`) → don't forward to client
- Deprecate/remove `touchscreen-gesture-passthrough` window rule after verifying no real use case depends on per-window scoping
- Document clearly: `noop` bind = "niri claims this gesture for IPC consumption"

**Phase 3 — Settings UI updates** (unchanged)

### 10e. What this means for users

**Before (tags):**
```text
TouchSwipe fingers=3 direction="up" tag="ws-up" { noop; }
```
Plus a separate daemon that subscribes to IPC, filters by tag="ws-up", drives animation.

**After (event stream + noop):**
```text
TouchSwipe fingers=3 direction="up" { noop; }   // claims this gesture for IPC
```
Daemon subscribes to IPC, filters by trigger pattern-match. No invented tag names.

**Or even simpler (spawn + env):**
```text
TouchSwipe fingers=3 direction="up" {
    spawn-sh "my-handler.sh"
}
```
Script reads `NIRI_GESTURE_*` env vars, reads stdin for progress.

Three clean paths — user picks whichever fits their use case:
1. **I want a self-contained script** → `spawn`
2. **I want a long-running daemon watching multiple gestures** → `noop` + IPC event stream
3. **I just want niri to do the thing** → bind to an action directly

---

## Part 11: Cross-cutting Concern — Internal vs IPC Progress Mismatch

This concern applies to **all three paths** above (spawn, event stream, noop-consume) because it's about the fundamental design of how niri's internal gesture math relates to what external consumers see. Previously documented separately in `TAG_GESTURE_PROGRESS_MISMATCH.md` — consolidated here since it's a subproblem of the tag-replacement architecture.

### The two threshold systems

Niri gesture handling has two independent progress/threshold systems that are not synchronized.

**1. Internal compositor animations:**
Niri's layout code decides when to commit actions (workspace switch, column scroll, overview toggle) based on its own internal gesture math:

- `workspace_switch_gesture_end()` — uses internal distance + velocity to decide whether to switch or snap back
- `view_offset_gesture_end()` — same for column scrolling
- `overview_gesture_update()` / `overview_gesture_end()` — own threshold for toggle commit

These thresholds are **not configurable** and **not exposed** via IPC. External tools cannot know when niri will commit an action.

**2. IPC progress events:**
External tools receive `GestureProgress` events with an accumulated `progress` value:

```text
progress = accumulated_delta * sensitivity / gesture-progress-distance
```

Where:
- `sensitivity` — per-bind config (touchscreen default: 0.4, touchpad default: 1.0)
- `gesture-progress-distance` — configurable per-input-type (touchscreen: 200 px, touchpad: 40 libinput units)

### The mismatch

These two systems operate independently:

- A touchscreen swipe might reach `progress = 0.8` in IPC, but niri's internal threshold commits the workspace switch at a completely different point
- Conversely, `progress` could hit `1.0` before niri commits, or niri could commit when `progress` is only `0.3`
- The IPC `GestureEnd { completed }` field distinguishes normal end (`true` when all fingers lift without interruption) from cancellation (`false` when a new finger arrives mid-gesture or cleanup fires on interruption). It does **not** indicate whether niri's internal threshold caused the compositor to actually commit the bound action — a touch workspace swipe that ends with all fingers lifted emits `completed: true` regardless of whether the compositor snapped forward to the new workspace or snapped back to the original.

### Where this matters across the three paths

- **`spawn` path:** The script's stdin stream has the same mismatch — progress values don't tell the script whether niri committed
- **`noop` + IPC event stream path:** Same mismatch — daemons watching the event stream can't predict niri's commits
- **`noop` with no compositor animation:** No mismatch — progress IS the sole output (the clean case)

**Conclusion:** The mismatch is inherent to "gesture drives both a compositor animation AND external consumers." It's not fixed by changing the IPC channel.

### Touchscreen vs touchpad scale difference

The delta units are fundamentally different between input types:

| Input | Delta Units | Default `gesture-progress-distance` | Default `sensitivity` |
|-------|------------|--------------------------------------|----------------------|
| Touchscreen | Screen pixels (large numbers, e.g., 500px per swipe) | 200 | 0.4 |
| Touchpad | Libinput acceleration-adjusted units (small numbers, e.g., 30 per swipe) | 40 | 1.0 |

Both aim for roughly equivalent physical gesture sizes, but the underlying units are incomparable. A third-party app receiving progress events from both input types gets consistent 0-1 progress values, but the raw `delta_x`/`delta_y` values will differ dramatically in scale.

### Touchscreen tracks closer to internal state than touchpad

In practice, touchscreen IPC progress aligns more closely with niri's internal animation state than touchpad does. This is because:

- **Touchscreen** deltas are in **screen pixels** — the same unit niri's layout code uses to track scroll offset and animation position. So accumulated `progress = pixels * sensitivity / distance` naturally correlates with niri's internal `scroll_offset / output_height`.
- **Touchpad** deltas pass through **libinput's acceleration curves** first, making the relationship between physical finger movement and layout displacement nonlinear. The same physical swipe distance can produce different delta magnitudes depending on speed, making it harder to tune IPC progress to match niri's commit point.

This means an external app showing visual feedback alongside a compositor-animated gesture (e.g., a progress bar for workspace switching) will feel more in sync on touchscreen than touchpad. The mismatch on touchpad is more noticeable — niri may snap back while the external progress indicator shows 80%.

### Potential fixes (independent of the IPC channel choice)

- **Expose whether niri actually committed the action** in `GestureEnd` — add a `triggered` or `action_committed` field. Probably the simplest fix with highest value.
- **Expose niri's internal gesture completion percentage** alongside IPC progress, so consumers can drive their UI from the compositor's view of commit instead of raw finger motion.
- **Unify the two systems** so IPC progress matches the compositor's internal state. Biggest change, probably not worth it — the raw progress value is useful for apps that want to drive their own independent animations.

These fixes should be tackled as part of Phase 0 (tag removal + public event stream) since they affect the event API shape.

---

## Part 12: Disambiguation Flow for fingers=1 / fingers=2

The PR currently restricts touch gesture families (Swipe/Pinch/Rotate/Tap/TapHoldDrag) to `fingers=3..=10`. TouchEdge is hardcoded 1-finger and spatially scoped to the edge pixel range, so it doesn't conflict with general 1/2-finger app input. Opening up fingers=1 and fingers=2 on the other five families raises a disambiguation question: how do we distinguish "compositor wants this gesture" from "client wants this touch"?

### Scope: deferred to a follow-up PR

Per Atan's suggestion (2026-04-16), the current PR (niri-wm/niri#3771) stays scoped to `fingers=3..=10`, which is already larger than the Blur and Zoom PRs combined. fingers=1/2 support lands as a separate, focused follow-up PR built on the `window-rules` mechanism described below.

### The conflict space

- **fingers=1** — every tap, scroll, drag, and text selection in every app is a 1-finger touch
- **fingers=2** — every pinch-zoom, every two-finger scroll in browsers/PDF viewers/image viewers

At fingers=3+ the contract is easy because virtually no native Wayland client uses 3+ finger gestures. At 1/2 we have to arbitrate.

### Current direction: per-window `binds {}` in window-rules (Atan's proposal)

Rather than heuristically arbitrating at runtime (the earlier three-gate model), expose a `binds {}` block inside `window-rule {}` so apps can declaratively release gestures the compositor claimed globally. This collapses "does this app want the gesture?" from three gates into one config lookup.

```text
binds {
    // Compositor claims 1-finger swipe up globally for IPC / bound action
    TouchSwipe fingers=1 direction="up" { noop; }
}

window-rule {
    match app-id="firefox"
    binds {
        // Release the claim for firefox — gesture forwards to client so
        // native scroll keeps working. `unbound` is a sentinel because an
        // empty action block is invalid KDL.
        TouchSwipe fingers=1 direction="up" { unbound; }
    }
}
```

**Semantics:**

- Global `binds {}` — compositor's default claim on a gesture pattern. `noop` = claim with no action (IPC/event consumer), a real action = claim + execute.
- Window-rule `binds {}` — per-app override. `unbound` releases the claim and forwards touch events to the client (when that window is focused / under the touch centroid).
- **Precedence:** window-rule `unbound` > window-rule action > global `noop` > global action > no bind (default passthrough).

**Why this is cleaner than the old three-gate model:**

- **Gate 1 (passthrough rule) + Gate 2 (bind existence) collapse into one** — the `binds {}` block in the window-rule *is* the passthrough decision, and `unbound` is its explicit keyword.
- **Gate 3 (threshold timing) becomes simpler, not disappeared** — if the matched rule says `unbound`, the compositor can skip the grab entirely for that window (no event buffering, no latency). If the rule claims the gesture, buffering kicks in normally.
- **Declarative, not heuristic** — intent lives in config, not in timing windows.
- **Reuses existing infrastructure** — niri already matches window rules on `app-id` / `title` / etc.

### Properties that fall out for free

- **Sentinel keyword is `unbound`.** Atan used `unbound` in the original proposal — going with that. (KDL can't have empty action blocks, so a sentinel is required.)
- **Partial direction overrides work naturally.** Each `fingers=N direction=D` combination is a separate bind entry, so a window rule can release `direction="up"` for native scroll while leaving `direction="left"` claimed by the global bind. No extra syntax needed.

### Decided behavior — touch resolves like the mouse cursor (spatial, not focus-following)

Window-rule matching for gestures works **exactly like mouse-cursor semantics**: the rule matches against the window the fingers are physically on, not the keyboard-focused window.

**The mental model:** you can hover the mouse cursor over an unfocused firefox window and scroll-wheel — firefox scrolls without stealing focus from your terminal. Touch should behave identically: if you're typing in a terminal and you touch firefox to scroll it, firefox's window-rule applies (so its native scroll passthrough kicks in), even though the terminal still has keyboard focus. Your touch acts where your finger is, not where your keyboard cursor is blinking.

**Concretely:**
- The window-rule lookup uses the window under the touch centroid at touch-down (with first-finger-position as the multi-finger tiebreaker).
- Keyboard focus is irrelevant to this decision — exactly as it is for mouse pointer events.
- This means an unfocused app's window-rule `binds { ... unbound; }` works without requiring the user to focus the app first — touching it is enough.

**Edge cases:**
- **Touch on empty desktop / layer-shell surface** (no app window underneath) — no window-rule match; global `binds {}` applies as the default.
- **Touch crossing windows mid-gesture** — already decided: claim is locked at touch-down, doesn't re-evaluate (see "Decided behavior — claim resolves at touch-down" below).
- **Multi-finger gestures with fingers on different windows at touch-down** — centroid picks one window deterministically; first-finger-position is the tiebreaker if centroid lands on a gap.
- **IPC-claimed gestures (`noop` with no action)** — same rule applies. The window under the touch determines whether its rule releases the claim, even when the eventual consumer is an external IPC listener.


### Decided behavior — claim resolves at touch-down, stable for gesture lifetime

The claim (compositor-grab vs client-passthrough) is decided **once**, at the moment the first finger lands, based on the window under the centroid at touch-down. It stays stable for the entire gesture lifetime — until all fingers lift — even if focus changes, the window moves, the cursor would be over a different window now, or additional fingers land later.

**Why this matters in practice:**

1. **The recognizer is stateful.** Once the compositor decides "this is mine," it begins accumulating `cumulative_dx`/`cumulative_dy`, computing pinch spread from initial finger spread, tracking rotation from initial angles. Flipping mid-gesture to "actually, give it to the client" would mean tearing down that state with no clean exit — the recognizer has no concept of "abort and rewind."

2. **The client-event stream is also stateful.** Wayland clients expect a `touch_down → touch_motion* → touch_up` lifecycle per slot. If the compositor consumed the early events and then mid-gesture decides to forward, the client sees a `touch_motion` with no preceding `touch_down` — which is a protocol violation. The reverse (forwarding then consuming) leaves the client with a `touch_down` that never gets a `touch_up`, so it sits with a dangling slot until the next gesture cleans it up.

3. **Continuous animations would visibly glitch.** A workspace-switch animation tracking finger position would reach 60% progress, then suddenly stop receiving updates because the claim flipped. The animation either snaps back, freezes, or completes phantom-style — all bad.

4. **Focus-change-during-gesture is normal, not exceptional.** An interactive-move drag *deliberately* crosses windows — the whole point is moving a window across other windows. If the claim re-evaluated based on "what's under the centroid right now," every move-grab would be hijacked the moment it crossed another app's window. Touch-down resolution makes the claim about *intent at gesture start*, not *current spatial position*.

5. **Late-landing fingers don't change the claim.** If the gesture started as 1-finger (compositor-claimed) and a second finger lands mid-gesture, the existing claim sticks. The new finger participates in the existing recognizer's state. Whether this triggers an unlock-to-higher-finger-count (the existing `unlock-on-new-finger` mechanism in `touch_gesture.rs`) is orthogonal — that's about gesture *type* (e.g. 3-finger swipe → 4-finger swipe), not about *who owns* the gesture.

**Implementation:** the claim resolution lives in the `TouchDown` handler. The result (claimed-by-compositor vs forward-to-client + which client) gets stored on the active gesture state struct and read by every subsequent `TouchMotion`/`TouchUp`/`TouchFrame` in this gesture. No re-lookup, no re-matching of window rules.

---

### Superseded: three-gate disambiguation

The following was the earlier design before the window-rule `binds {}` proposal. Kept for reference; no longer the active plan.

#### Three-gate disambiguation (composes bind-existence + window rule + threshold timing)

**Gate 1 — Window rule passthrough (app opts out):**

`touchscreen-gesture-passthrough` already exists as a window rule. This is the per-app override when a user has global fingers=1/2 binds but wants specific apps to feel native:

```text
window-rule {
    match app-id="firefox"
    touchscreen-gesture-passthrough "always"   // never defer, never consume
}
```

When matched → forward immediately, no recognizer involvement.

**Gate 2 — Bind existence = consume signal (`noop=consume` model from Part 10b):**

Today `noop` is just "no action." The proposal: the *presence of any bind* (including `noop`) at a given `fingers=N direction=D` slot is the claim "compositor wants this pattern, don't forward."

- No bind at `TouchSwipe fingers=1 direction="up"` → pass through (current default, unchanged)
- `TouchSwipe fingers=1 direction="up" { noop; }` → compositor watches, claims if matched, never reaches client
- `TouchSwipe fingers=1 direction="up" { focus-workspace-up; }` → same, plus the action runs

This keeps the opt-in per-pattern rather than requiring a global "enable fingers=1/2" switch. Users who don't write fingers=1 binds experience zero behavior change.

**Gate 3 — Threshold timing (recognizer decides):**

When a bind exists and the window is not in passthrough, the compositor must buffer the first ~100px/200ms before deciding "bound swipe or client drag?" This is the latency cost of opting in.

### Disambiguation flow

```text
TouchDown (1 or 2 fingers)
  ↓
Window under finger has touchscreen-gesture-passthrough="always"?
  → yes: forward immediately, no recognizer (Gate 1)
  ↓ no
Any TouchSwipe/Pinch/Rotate/Tap/TapHoldDrag fingers=N bind exists for this N?
  → no: forward immediately (current behavior preserved) (Gate 2)
  ↓ yes
Buffer events, run recognizer
  ↓
Threshold crossed, matches a bound pattern?
  → yes: consume, drop buffered events, fire bind (Gate 3 commit)
  ↓ no
Timeout expired or motion stopped?
  → yes: flush buffered events to client, resume passthrough (Gate 3 release)
```

### Cost analysis

- **Zero-cost for users who don't bind fingers=1/2** — if no such bind exists, Gate 2 short-circuits to immediate passthrough. No latency, no regression.
- **Per-pattern cost for users who do bind** — fingers=1/2 taps/drags in non-passthrough apps get buffered for threshold duration. Users accepted this cost by writing the bind.
- **Escape hatch for power users** — window rule passthrough lets them keep global fingers=1 binds while exempting specific apps.

### Why `noop=consume` is the right primitive

Without it, we'd need a separate syntax to say "claim this gesture but do nothing" — either a new keyword (`TouchSwipe fingers=1 consume;`) or a separate block. Treating bind presence as the claim signal means:

- No new syntax
- `noop` gets a meaningful use (it's currently a no-op action with no purpose)
- Composable with real actions — binding to `focus-workspace-up` implies consume, same as binding to `noop`
- Matches the intuition that "if you told niri what to do with this gesture, niri should grab it"

### Interaction with existing 2-finger scroll/touchpad semantics

Touchpad 2-finger scroll is libinput-native and pre-classified — it arrives as `PointerAxis` events, not `GestureSwipe`. So `TouchpadSwipe fingers=2` would be an impossible bind (libinput never delivers 2-finger swipe events for touchpads). Touchpad fingers=1/2 disambiguation isn't really in scope — this applies to **touchscreen** fingers=1/2 only.

### Open question

Should fingers=1 and fingers=2 be *opt-in behind a config flag* (e.g., `allow-low-finger-gestures`) as a safety measure against users accidentally breaking their text selection? Arguments both ways:

- **Opt-in flag:** explicit consent, easier to document "fingers=1 has latency cost"
- **No flag:** writing the bind is already opt-in per Gate 2; extra flag is redundant

Leaning toward no flag — the bind existence is already the opt-in signal, and Gate 2 makes the cost zero for users who don't write the binds.
