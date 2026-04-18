# Design: Touchscreen Gestures

> [!IMPORTANT]
> **Status: proposal / working prototype — not upstream niri's canonical design.**
>
> This document is not niri's official design position. It is a write-up of the choices I (the PR author) made while building a working touchscreen gesture implementation on the `feat/configurable-touch-gestures` branch, shaped by feedback from reviewers on the associated PR. The goal was to land *something that works* so there's a concrete reference point to experiment with, gather real-world feedback on, and iterate on — not to prescribe how niri should handle touch gestures long-term.
>
> Everything below describes what exists on this branch and why it was chosen over the alternatives I considered. It is explicitly open to being rethought, rewritten, or replaced. If you disagree with any section — especially §5 (design choices) and §6 (alternatives rejected) — that disagreement is the whole point of putting the rationale in writing. See §10 for how to push back.
>
> This document explains what Wayland gives us, what it doesn't, how other ecosystems solve the same problems, and why this implementation makes the specific choices it does. It is meant for contributors and reviewers deciding whether the current direction is worth building on, and for users of this branch curious about why the configuration surface looks the way it does.

For how to **configure** gestures on this branch, see [Configuration: Window Rules](./Configuration:-Window-Rules.md) and the main niri config documentation. This doc is strictly about the *why*.

---

## 1. Scope

What this doc covers:

- The Wayland protocol landscape relevant to touch input
- Why touchpad and touchscreen gestures live in different layers
- How iOS, Android, and other Linux shells approach gesture ownership
- The specific design choices niri makes and the reasoning behind each
- Alternatives we considered and rejected, with rationale
- Open questions and directions for future work

What this doc does **not** cover:

- Configuration syntax (see the wiki pages)
- Specific gesture recognizer math (read `src/input/touch_gesture.rs`)
- Touchpad gesture internals beyond "niri uses libinput via smithay" (they aren't a niri-local problem)

---

## 2. The Wayland protocol landscape

Wayland touch input has a hard split between two worlds. Understanding the split is prerequisite to understanding why this doc exists.

### 2.1 `wl_touch` (core, stable)

Part of the core Wayland protocol. Exposes raw touch point lifecycle events:

- `down(slot, surface, x, y)` — a new finger landed
- `motion(slot, x, y)` — an existing finger moved
- `up(slot)` — a finger lifted
- `frame` — atomic batch boundary for multi-point updates
- `cancel` — compositor revokes the touch stream

That is the entire API. No semantics, no gesture recognition, no "swipe" or "pinch" primitives. The spec is explicit that gesture interpretation is the caller's responsibility. The caller here means *whoever is reading `wl_touch`* — usually the compositor, sometimes the client.

### 2.2 `wp_pointer_gestures_v1` (unstable, widely adopted)

A separate protocol that provides **touchpad** gestures only. Defines three semantic gesture types:

- **Swipe** (`zwp_pointer_gesture_swipe_v1`) — begin / update / end lifecycle with finger count and dx/dy deltas
- **Pinch** (`zwp_pointer_gesture_pinch_v1`) — begin / update / end with scale factor and rotation
- **Hold** (`zwp_pointer_gesture_hold_v1`) — begin / end, used for tap-and-hold style interactions

This protocol exists because libinput already does touchpad gesture recognition from the raw hardware events, and the Wayland layer just needed a standard way to expose libinput's output to clients and compositors. Niri uses this for touchpad gestures via smithay's libinput integration — no custom recognizer needed on that side.

The protocol is still marked unstable (`unstable-v1`) but is implemented by all major compositors and all major toolkits. It is effectively the standard.

### 2.3 The gap: no touchscreen gesture protocol

There is **no** Wayland protocol for touchscreen gestures. Not stable, not unstable, not staged as a proposal in `wayland-protocols`. The explicit design position from both the Wayland community and libinput is that touchscreen gesture recognition requires context (focus, window layout, app intent) that the input stack doesn't have.

There is also no protocol for **client cooperation** — no way for an app to tell the compositor "I handle 3-finger swipes in my content area, leave them alone." The closest analogue, `zwp_keyboard_shortcuts_inhibit_v1`, exists for keyboard shortcuts but has no touch equivalent.

This gap is the root cause of nearly every design compromise in this document.

---

## 3. Why touchscreen gestures don't live in libinput

libinput is the layer that turns raw kernel input device events into semantic events for compositors. It recognizes touchpad gestures (swipe, pinch, hold) and hands them up the stack cleanly. It explicitly refuses to do the same for touchscreens. The reason isn't laziness — it's a genuine architectural difference between the two input types.

### 3.1 Touchpad: indirect manipulation, unambiguous recipient

- Fingers move on a surface that **isn't** the thing they're affecting. The pointer is the proxy.
- The touchpad belongs to the focused window or the compositor. There's exactly one plausible recipient for any gesture event — whoever has pointer focus.
- Many modern touchpads (Apple Magic Trackpad, Microsoft Precision Touchpads) recognize gestures in **firmware**. The hardware says "3-finger swipe" directly. libinput forwards that, adds fallback recognition for dumber hardware, and exposes semantic events.
- State is clean: `n` fingers down means a gesture is active with that many fingers. Any finger lifting ends it. Palm rejection is well-understood.
- libinput can confidently say "this is a compositor/pointer-bound gesture event" because there's no other reasonable interpretation.

### 3.2 Touchscreen: direct manipulation, ambiguous recipient

- Fingers are **on** the thing they're affecting. The content under the finger is the target.
- The same 3-finger contact at the same coordinates could legitimately mean:
  - The user is drawing three strokes in a paint app
  - Two people on a shared tablet both tapping at once
  - A compositor workspace-switch swipe
  - A browser pinch-to-zoom on a webpage
  - Palm rest plus one intentional tap
- libinput has zero visibility into what's under those coordinates. It doesn't know about Wayland surfaces, window focus, or client intent. Only the compositor has that context.
- Hardware is also dumber: touchscreens report `(slot, x, y)` per contact point with no gesture semantics. There's nothing for libinput to forward.
- State is messy: new fingers can arrive at any time, bezel phantom touches, hand resting, palm rejection depends on geometry libinput can't see.

### 3.3 libinput's stated position

Paraphrasing the libinput maintainers' public position: *"We can recognize motion from raw touch points, but we cannot tell you whether the user meant that motion for the compositor or for the app under their finger. That's a compositor decision, not an input-stack decision."*

On touchpad that question has a trivial answer ("the compositor, always"). On touchscreen it doesn't, and libinput refuses to guess because a wrong guess means silently stealing input from an app. So touchscreen gesture recognition ends up **inside each compositor**, built from raw `wl_touch` events. Every major compositor has independently reinvented its own recognizer for exactly this reason.

---

## 4. How other ecosystems solve it

Worth understanding because the design lessons map directly onto what a Wayland solution could look like.

### 4.1 iOS (UIKit)

iOS has an explicit gesture recognizer arbitration system baked into UIKit:

- **Every view can attach gesture recognizers.** Both apps and the system.
- **Priority chain.** System-level recognizers (home bar, control center, notification shade) sit at the top of the hierarchy.
- **Failure requirements.** A recognizer can declare "I only activate if this other recognizer fails first." This is how UIKit handles "tap vs. long-press" and also how system edge-swipe defers to an app's swipe when appropriate.
- **Simultaneous recognition.** Two recognizers can explicitly opt into firing at the same time — pinch + pan on a photo viewer, for example.
- **Dedicated edge recognizers.** `UIScreenEdgePanGestureRecognizer` is a distinct type. Apps can attach their own and negotiate with the system's.

The negotiation isn't a runtime question per touch event — it's **declared up front** by the view hierarchy. When a finger lands, UIKit walks the view tree from deepest to shallowest, collects every recognizer that could match, then arbitrates based on declared priority and failure rules.

This works because Apple ships iOS + UIKit + the hardware as one vertically integrated stack. There is no protocol problem because there is no protocol — it's all one process model with a shared API.

### 4.2 Android

Android takes a different approach but lands in the same place:

- **`onInterceptTouchEvent` chain.** Touch events bubble up through `ViewGroup`s. Each parent can claim ownership by returning true, at which point children stop seeing the events. This is how scroll containers steal touches from buttons mid-gesture.
- **Standard framework classes.** `GestureDetector` and `ScaleGestureDetector` are built into the Android framework. Everyone uses the same ones, so gesture behavior is consistent across apps.
- **System gestures live above the app.** Back, home, and recents (since Android 10 gesture nav) are handled at the `WindowManager` layer, not inside the view hierarchy.
- **`systemGestureExclusionRects`** — this is the important one. An app can tell the system: *"in these rectangles, don't treat edge swipes as system gestures."* Games and drawing apps use this to claim screen edges when the user is actively using them for content. Apps can also read `WindowInsets.getSystemGestures()` to see where system gestures are active and lay out their UI accordingly.

Android 10's gesture-nav rollout was specifically driven by this problem. Google needed to steal more of the screen edges for system gestures and ran into exactly the conflict niri runs into. Their answer was **`systemGestureExclusionRects`**: a tiny, minimal opt-out API that doesn't try to solve everything, just the most common conflict case.

This is the closest real-world precedent for what a Wayland touchscreen gesture protocol could look like.

### 4.3 Linux phone shells

Linux mobile UX is the most instructive comparison because it's a touch-first world built on the same protocol stack we have. Every Linux phone shell has independently reinvented the same hacks:

- **Phosh** (PinePhone, Librem 5, GNOME-based) — gestures handled inside Phoc, a wlroots-fork compositor. Apps receive raw `wl_touch` events; Phoc reserves edges for the app drawer and notification shade. No negotiation protocol.
- **Plasma Mobile** — uses KWin's touch handling. Hardcoded system edges. Same story.
- **SXMO** (minimalist postmarketOS shell) — uses `lisgd` as a separate daemon reading libinput directly. System owns everything; apps are effectively gesture-blind.
- **Furios / Droidian** (Halium-based, Android drivers underneath) — inherits Android's gesture semantics from the hardware layer but runs regular Wayland compositors on top. Ends up with the worst of both worlds.

Every one of these shells ships with the same core limitation: **system gestures are hardcoded, app gestures are whatever the toolkit happens to support, there is no negotiation**. When Firefox on a PinePhone handles pinch-to-zoom, it works because GTK handles 2-finger touches directly via `wl_touch` — not because anyone negotiated anything.

### 4.4 Userspace gesture daemons

Several projects have tried the "external daemon recognizes gestures, compositor reads from it" architecture:

- **TouchEgg** — originally X11, adapted for Wayland. Reads libinput events directly, recognizes gestures, maps them to actions via XML config. Popular as a "make Linux feel like macOS" touchpad tool.
- **lisgd** (libinput simple gesture daemon) — smaller scope, shell-command-based, stateless. Popular in SXMO and bespoke postmarketOS setups.
- **InputActions** — newer, KDE-specific, funded work for Plasma 6 Wayland. Lives *inside* KWin rather than as a separate daemon.

The common issue: on Wayland, any external daemon architecture breaks on **device ownership**. libinput exposes a single reader interface per device — whichever process grabs it "owns" the stream. If TouchEgg grabs exclusively, the compositor gets nothing. If neither grabs exclusively, they both see every event and double-handle. There's no "daemon sits between kernel and compositor" slot in the Wayland stack; the compositor is the input router by design.

X11 had this slot because of its split server/client architecture with a routable event path. Wayland removed it deliberately for security and simplicity. This is why **compositor-agnostic gesture daemons don't work on Wayland** and why KDE moved InputActions *inside* KWin.

### 4.5 The unifying observation

Every ecosystem that has solved touchscreen gesture ownership has done so by **owning the whole stack** — iOS with UIKit + the OS, Android with the view system + WindowManager, KDE with InputActions + KWin. The problem isn't that the solution is hard to design. It's that the solution requires coordination between input, toolkit, and window manager, and Linux has that coordination problem stratified across dozens of unrelated projects.

---

## 5. Niri's design choices

This section is explicitly opinionated. Each choice is labeled with its reasoning so reviewers can argue with the rationale, not just the result.

### 5.1 Compositor-side recognizer from raw `wl_touch`

**What:** Niri reads raw `wl_touch` events in `src/input/touch_gesture.rs` and runs its own gesture recognizer (direction lock, finger count tracking, pinch detection, edge swipe detection).

**Why:** There is no alternative. libinput won't recognize touchscreen gestures. Clients receiving raw touches can't participate in compositor actions. Userspace daemons can't sit between the compositor and libinput. The compositor is the only layer that has both the input stream *and* the window context needed to make gesture routing decisions. This is the same conclusion KWin, Mutter, Phoc, and every other Wayland compositor has reached.

### 5.2 Unified `binds {}` block with parameterized gesture triggers

**What:** Touchscreen, touchpad, keyboard, and mouse gesture binds all live in the same `binds {}` block. Multi-finger gestures are parameterized via KDL properties: `TouchSwipe fingers=3 direction="up"` rather than hardcoded node names like `TouchSwipe3Up`. The five gesture families (`TouchSwipe`, `TouchpadSwipe`, `TouchPinch`, `TouchRotate`, `TouchEdge`) are the only first-class gesture node names; everything else is properties.

**Why:**
- **Modifier combos come for free.** `Mod+TouchSwipe fingers=3 direction="up"` reuses the existing key-bind parser with no new code paths — modifiers are stripped off the node name before property parsing begins.
- **One lookup path.** `find_configured_bind()` handles every input type identically. `Trigger::TouchSwipe { fingers, direction }` is a struct variant, so `Eq`/`Hash` still work; bind lookup is unchanged from the hardcoded design.
- **Consistency with niri's existing model.** Niri's keyboard and mouse binds already live in `binds {}`, and all other bind attributes (`tag=`, `natural-scroll=`, `sensitivity=`, `cooldown-ms=`) are KDL properties. Hardcoding finger count into the *node name* was the one place where touch gestures diverged from the rest of the config grammar; this closes that gap.
- **Arbitrary finger counts.** `fingers=N` accepts any integer in `3..=10`. Users with tablets and large multitouch displays that report 6–10 contacts can bind to them without an enum change on the compositor side. The `3..=10` range is enforced by the parser with a clear error on out-of-range values.
- **Per-family validation.** Each family has its own legal direction vocabulary (swipe takes `up/down/left/right`, pinch takes `in/out`, rotate takes `cw/ccw`, edge takes `left/right/top/bottom` with optional `zone=`). Invalid combinations are rejected at parse time, not at runtime.
- **Hard break from the old syntax.** The previous enum-per-combination design (`TouchSwipe3Up`, `TouchEdgeTop:Left`) is gone — no dual-parse, no deprecation aliasing. A cleaner config grammar is worth the one-time migration cost for a pre-1.0 feature with a small user base.

### 5.3 Tag property + IPC gesture events

**What:** Gesture binds can carry an optional `tag="name"` property. Tagged binds emit `GestureBegin` / `GestureProgress` / `GestureEnd` events on niri's existing IPC event stream, letting external tools observe gestures for custom animations or UI feedback.

**Why:**
- **External extensibility without a scripting runtime.** niri doesn't need to embed Lua or JavaScript; tools subscribe to IPC events and react.
- **Security-scoped.** Only tagged gesture binds emit IPC events. Keyboard input never appears in the event stream. This is a deliberate scoping decision — "we expose gestures because they're low-frequency, high-intent user actions, but we don't expose every keystroke."
- **Three distinct modes.** With tags + the `noop` action, niri supports:
  1. **Observe** — `tag="ws"` + real action: niri runs the action and emits IPC events for external UI feedback
  2. **IPC-only** — `tag="drawer"` + `noop`: niri captures the gesture purely for IPC, runs no compositor action
  3. **Plain** — no tag: niri runs the action, no IPC emission
- **Both discrete and continuous noop are supported.** A tagged `noop` bind on a swipe or pinch drives the full begin/update/end lifecycle, emitting continuous `GestureProgress` events for external animations — external tools can draw finger-tracked UI without the compositor performing any action of its own.
- **Enables `niri-tag-sidebar` and similar tools** to build gesture-driven UIs without having to reimplement touch recognition themselves.

### 5.4 `touchscreen-gesture-passthrough` window rule

**What:** A window-rule bool field. When set on a matching window, niri's recognizer stays out of the way for touches that start on that window — events forward raw to the client for the lifetime of the gesture.

**Why:**
- **Solves the 80% case with the simplest possible mechanism.** For apps that always want touch events (browsers, drawing apps, mapping tools), a per-app static rule is enough.
- **User-controlled, not auto-detected.** Niri makes zero attempts to guess which apps want passthrough. Heuristics like "this is Electron, probably a webapp" produce unpredictable behavior. Explicit rule or nothing.
- **Doesn't wait for a Wayland protocol that isn't coming.** The reviewer who raised this concern ([issue discussion]) explicitly acknowledged the "elaborate automatic" version feels bad; this ships the blunt-but-predictable alternative now.
- **Discoverability via `RUST_LOG=niri=debug`.** When niri captures a gesture, it logs the app-id of the window under the touch, letting users see exactly which app-id to add to their passthrough rules.

### 5.5 Escape hatches: Mod+touch and edge zones always bypass passthrough

**What:** Even on a window with `touchscreen-gesture-passthrough true`, holding the mod key or starting a touch in a screen-edge zone still triggers compositor gestures.

**Why:**
- **Discoverable fallbacks.** "Gestures don't work in this app? Try Mod+gesture, or swipe from the edge." Every passthrough window has a way to invoke compositor actions without removing the rule.
- **Edge detection runs before window lookup.** This isn't a special case — edge zones are already evaluated before the window is even checked, so passthrough is automatically excluded.
- **Mod+ is an explicit user intent signal.** If the user holds the mod key, they are unambiguously asking for a compositor action. Passthrough is for implicit gestures; Mod+ is explicit, so it wins.

### 5.6 Per-edge zoned triggers

**What:** Each screen edge is split into thirds along its perpendicular axis. `TouchEdge` accepts an optional `zone=` property — `edge="top" zone="left"`, etc. — giving 12 zoned triggers in addition to the 4 unzoned parents. Zoned triggers fall back to the parent if not configured. The zone vocabulary rotates per edge: `top`/`bottom` edges take `left|center|right`; `left`/`right` edges take `top|center|bottom`. Mismatched vocabularies are a parse error.

**Why:**
- **12 + 4 = 16 edge actions possible** without adding a new concept; power users can bind distinct actions per edge zone.
- **Parent fallback.** A bare `TouchEdge edge="top"` catches any top-edge swipe that doesn't land in a more specific zoned bind, so adding one zoned bind doesn't break the others.
- **Matches real-world UI patterns.** Status bars, notification shades, and app drawers all want *different* actions for different parts of the same edge.
- **Matching UI support in external tooling.** `niri-tag-sidebar` mirrors the zone model so tagged panels can anchor to specific zones.

### 5.7 Touchpad via `wp_pointer_gestures_v1` (libinput)

**What:** Touchpad gestures are read from libinput via smithay's existing plumbing, exposed through the same `binds {}` block with `TouchpadSwipe fingers=N direction="..."` triggers. No compositor-side recognition.

**Why:** Touchpad gesture recognition is a solved problem at the libinput layer. Writing our own recognizer for touchpad would duplicate work, produce inconsistent semantics vs. other compositors, and lose firmware-reported gesture quality from modern hardware. The right answer is "use the standard, expose it through niri's bind model."

---

## 6. Alternatives considered and not shipped

Every decision in section 5 had alternatives. This section records the ones we looked at and why they didn't ship, so the same conversations don't have to happen repeatedly.

### 6.1 Dynamic per-gesture client dialog ("does your app want this?")

**The idea:** Compositor detects a gesture starting, asks the client under the touch "want this one?", client responds yes/no, compositor routes accordingly.

**Why not:** Requires a Wayland protocol that doesn't exist. Also adds IPC round-trip latency on gesture start, which is noticeable for continuous gestures. Parked until a protocol emerges.

### 6.2 `allow-forwarding=true` per-bind property

**The idea:** Each gesture bind gets a flag saying "forward to client instead of consuming." The reviewer's original proposal.

**Why not:** The reviewer themselves acknowledged it "feels way too complicated." It puts the opt-out at the wrong layer — gesture policy should follow the *target app*, not the *bind*. A user wanting Firefox to handle gestures would have to annotate every single bind with `allow-forwarding` conditionally based on the focused window, which is exactly the complexity a window rule avoids.

### 6.3 Zone granularity on passthrough window rule

**The idea:** Instead of `touchscreen-gesture-passthrough true`, specify which gesture classes passthrough: `touchscreen-gesture-passthrough "swipe"`, or rectangles within the window where passthrough applies, or per-finger-count opt-outs.

**Why not:** Overengineering for v1. The simple bool handles the common cases (browsers, drawing apps). Zone granularity only matters when the answer is "depends on what part of the window the finger is on," which is the dynamic case that only a real protocol can solve well. Trying to approximate it with static rectangles requires the user to manually track layout changes, which is worse than nothing.

If a concrete use case appears that the bool can't handle, the field type can be widened (`Option<bool>` → `Option<PassthroughSpec>`) without a breaking config change. Keeping v1 minimal preserves that flexibility.

### 6.4 Auto-detection heuristics

**The idea:** Niri guesses which apps want passthrough based on app-id patterns, toolkit detection, window class hints, etc.

**Why not:** Unpredictable. "This is Electron, probably a webapp" is wrong for VSCode. "This is Chromium, probably wants gestures" is wrong for a kiosk app. Heuristics fail in ways users can't debug, and silently stealing or forwarding input based on guesses is the worst possible failure mode. Explicit rule or nothing.

### 6.5 External gesture daemon (TouchEgg-style)

**The idea:** Run a separate process that recognizes gestures and sends actions to niri via IPC.

**Why not:** Breaks on Wayland device ownership (see section 4.4). Any daemon reading libinput directly conflicts with the compositor reading the same device. A daemon reading from niri via some new "raw touch" IPC would duplicate gesture state between processes and add latency. KDE tried the external path and pulled it in-process for exactly these reasons.

### 6.6 Global "disable all gestures when this app focused"

**The idea:** One big toggle — when a passthrough app has focus, niri disables all touch gestures everywhere on screen.

**Why not:** Too blunt. Breaks the edge swipe and Mod+gesture escape hatches that make passthrough tolerable in the first place. A user couldn't invoke the app drawer or workspace switch without unfocusing the app first. The per-touch decision made in `on_touch_down` is strictly better — it respects escape hatches automatically.

---

## 7. Future directions

Where this could go if the ecosystem moves.

### 7.1 A minimal Wayland touchscreen gesture protocol

The realistic shape, modeled on Android's `systemGestureExclusionRects`:

1. Client advertises support via a new global interface (`wp_touch_gesture_exclusion_v1` or similar).
2. Client submits per-surface rectangles: "in these regions of my window, don't handle compositor gestures."
3. Compositor evaluates the rectangles when a touch starts; if the touch lands in an exclusion rect, forwards raw touches to the client.
4. Rectangles update on window resize / layout change via standard surface commit.

This is intentionally narrower than a full capability-negotiation protocol. It doesn't try to support "client handles swipe but not pinch" or "client wants first 100ms of the gesture to decide." Android has shipped the rect-based model for 6+ years and it covers the important cases. Getting 80% of the solution into the protocol layer beats waiting forever for 100%.

**What niri could do if such a protocol existed:**

- The `touchscreen-gesture-passthrough` window rule becomes a **fallback** for apps that don't participate in the protocol.
- Apps that do participate (Firefox via GTK, Krita via Qt, etc.) get dynamic per-region control without any user configuration.
- The discoverability debug log becomes less important because correct behavior is automatic for participating apps.
- Niri would be one of the first compositors to support such a protocol if one is drafted.

### 7.2 Unify IPC progress with niri's internal commit threshold

IPC `GestureProgress` events already carry a normalized `progress: f64` (computed as `accumulated_delta * sensitivity / gesture-progress-distance`), so external consumers *do* get a 0→1 value. The unresolved problem is that niri has **two independent threshold systems** that are not synchronized:

1. **IPC progress** — the value external tools see, driven by configured `gesture-progress-distance`
2. **Internal compositor commit** — niri's layout code decides whether to snap to the next workspace / column / overview state based on its own distance and velocity math

These two can disagree. A swipe can reach IPC `progress = 0.8` while niri decides to snap back, or commit when IPC `progress = 0.3`. For external UIs driven by tagged gestures, this mismatch is visible — a progress bar showing 80% while niri snaps back feels broken.

The improvement is to either (a) expose niri's internal progress alongside or instead of the IPC progress, or (b) make the IPC progress drive the commit decision so the two always agree. See `GESTURE_PROGRESS_MISMATCH.md` for the full write-up.

In practice the touchscreen case tracks closer than touchpad because screen pixels match niri's internal units, while libinput's acceleration-curved touchpad deltas make the touchpad mismatch more noticeable.

### 7.3 Touchpad gesture passthrough (sibling rule)

For completeness, a `touchpad-gesture-passthrough` window rule could be added. The shape is different — touchpad has no "window under finger," so the rule would match the focused window instead — but the config surface would look analogous. Punted from v1 because the pain is smaller (2-finger touchpad gestures already forward by default via libinput) and the semantics need more thought.

### 7.4 Have `GestureEnd { completed }` reflect internal commit, not just cancellation

The `completed` field on `GestureEnd` currently distinguishes two cases:

- `completed: true` — gesture ended normally (all fingers lifted without external interruption)
- `completed: false` — gesture was cancelled (a new finger arrived and restarted recognition, or cleanup fired)

What it does **not** distinguish: whether niri's internal threshold actually committed the bound action. A touch workspace swipe that ends with all fingers lifted emits `completed: true` regardless of whether the compositor snapped forward to the new workspace or snapped back to the original. For tagged gestures driving external UIs, this is the same mismatch as §7.2 — the IPC event doesn't know what niri actually did.

The fix is the same as §7.2: either unify the threshold systems so the answer is always knowable, or add a separate `action_committed: bool` field that propagates niri's internal snap decision. Either way, external tools should be able to answer "did the swipe actually do the thing?" from the `GestureEnd` event alone.

---

## 8. Open questions

Explicitly inviting pushback. None of these have right answers yet.

### 8.1 Should passthrough be a simple bool or support zones?

Currently a simple bool. If someone comes up with a concrete use case the bool can't handle — for example, a browser where users want pinch forwarded but edge swipes intercepted — the field type would need to widen. The field name (`touchscreen-gesture-passthrough`) is generic enough that this extension is a non-breaking change (the bool becomes one arm of a widened sum type).

### 8.2 Should layer-shell windows support passthrough?

Currently no — `touchscreen-gesture-passthrough` is a `WindowRule` field and layer-shell surfaces don't go through window rules. A sidebar panel that wanted to claim gestures on itself has no way to do so today. Adding layer-shell passthrough is probably the right call but requires deciding where the config lives (a new `layer-rule {}` block? Matching criteria reused?) and is punted for v1.

### 8.3 Should Mod+gesture always bypass passthrough?

Currently yes, hard-coded. A case could be made for a `touchscreen-gesture-passthrough-respect-mod false` subfield to let passthrough *also* forward mod-combo gestures. Nobody has asked for this yet, and the hard-coded behavior preserves a discoverable escape hatch, so keeping it hard-coded feels right.

### 8.4 What about gestures that start on a passthrough window and drift onto the desktop?

Current behavior: once the first finger decides passthrough on touch-down, the entire gesture stays in passthrough mode until all fingers lift, even if fingers move off the window. This avoids confusing mid-gesture handoffs, but it means a user who accidentally starts a gesture on a passthrough window can't rescue it onto the compositor by dragging away. Reversing the policy (mid-gesture handoff based on current position) is probably worse, but this is the trade-off.

### 8.5 Continuous `noop` semantics

If we add continuous noop (section 7.2), should the delta stream be raw pixels, normalized progress, or both? Raw is more flexible but forces external tools to do their own normalization. Normalized is easier to consume but loses information. Both means more IPC traffic. No decision yet.

### 8.6 Should the debug log be promoted to `info` or stay at `debug`?

The `touch: captured N-finger gesture over app-id=X` log line is currently at `debug` level. That means it requires `RUST_LOG=niri=debug` to see. Promoting it to `info` would surface it by default, which helps discoverability but adds noise to logs during normal use. Leaning toward leaving it at `debug` and documenting the `RUST_LOG` requirement, but open to arguments.

---

## 9. Further reading

External references for the design space covered in this document.

### Wayland / libinput

- [Wayland Protocols: `wp_pointer_gestures_v1`](https://wayland.app/protocols/pointer-gestures-unstable-v1)
- [Wayland Book: Touch input](https://wayland-book.com/seat/touch.html)
- [libinput gestures documentation](https://wayland.freedesktop.org/libinput/doc/latest/gestures.html)
- [`zwp_keyboard_shortcuts_inhibit_v1`](https://wayland.app/protocols/keyboard-shortcuts-inhibit-unstable-v1) — the keyboard-side analogue of what touch gesture inhibit would need

### iOS / Android

- iOS: search Apple Developer docs for `UIGestureRecognizer`, `UIScreenEdgePanGestureRecognizer`
- Android: search AOSP docs for `View.setSystemGestureExclusionRects`, `WindowInsets.getSystemGestures`

### KDE / GNOME / Linux phone shells

- [Input handling in spring 2025 — KDE Blogs](https://blogs.kde.org/2025/05/14/input-handling-in-spring-2025/)
- KDE InputActions — search KDE Discuss for "InputActions mouse gestures Wayland"
- [GNOME Shell gesture extensions](https://extensions.gnome.org/extension/4245/gesture-improvements/)
- Phosh / Phoc source (GitLab) — how a wlroots-based mobile shell handles touch edges
- SXMO / `lisgd` — the external-daemon model on Wayland

### Niri internals

- `src/input/touch_gesture.rs` — touchscreen gesture recognizer
- `src/input/move_grab.rs` — touch-driven window move grab (interacts with gesture detection)
- `niri-config/src/window_rule.rs` — where `touchscreen_gesture_passthrough` is parsed
- `src/window/mod.rs` — `ResolvedWindowRules` and the rule compute path
- [Configuration: Window Rules](./Configuration:-Window-Rules.md) — user-facing docs for the passthrough rule
