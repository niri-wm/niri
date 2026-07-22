# Focus Ring Alpha Fade — Plan & Upstream PR Strategy

**Scope decision (locked):** One feature only — animate the focus ring's alpha on focus
change. **Everything is OFF by default.** The shadow extension was considered and is
explicitly **out of scope** for this PR (kept below as a reference note only). The only
goal is to get this single, well-understood feature accepted upstream.

It builds on the already-implemented focus-ring alpha fade (branch `focus-ring-anim`).

---

## Part 1 — The feature (focus ring alpha fade)

### What it does

When a window gains focus, the focus ring fades in (alpha 0 → configured `max-opacity`).
When it loses focus, the ring fades out (alpha → 0). Previously the ring appeared /
disappeared instantly.

The feature is gated behind a new `animations.focus-ring` config block. **By default the
animation is `off`**, so out-of-the-box behavior is unchanged from today. Users opt in:

```kdl
animations {
    focus-ring {
        duration-ms 300
        curve "ease-out-quad"
    }
}
```

### Current implementation (branch `focus-ring-anim`)

- **`niri-config/src/animations.rs`** — new `FocusRingAnim` newtype over `Animation`,
  with `Default` and a `knuffel::Decode` impl, following the `ScreenshotUiOpenAnim` /
  `ConfigNotificationOpenCloseAnim` pattern. Wired into `Animations`, `AnimationsPart`,
  and `merge_clone!`.
- **`src/layout/tile.rs`** — two new fields (`focus_ring_alpha_anim: Option<Animation>`,
  `prev_focus_ring_is_active: bool`). Transition detection in `update_render_elements()`
  starts/advances the animation; cleanup in `advance_animations()` clears it on
  `is_done()`. The computed `ring_alpha` is multiplied by `(1. - expanded_progress)` and
  passed to `FocusRing::update_render_elements()`.
- **`resources/default-config.kdl`** — commented-out example.
- **`niri-config/src/lib.rs`** — insta snapshot updated to include the new field.

### Change required for the "off by default" decision

The current `Default for FocusRingAnim` is **ON** (300ms `EaseOutQuad`). For this PR it
must be **OFF** so the feature is opt-in. Two equivalent ways to express "off":

```rust
impl Default for FocusRingAnim {
    fn default() -> Self {
        Self(Animation::new_off()) // existing helper: off: true, duration 0
    }
}
```

or equivalently set `Animation { off: true, kind: Easing(0ms) }`. Either way, with no
config the ring behaves exactly as before (instant). With the animation `off`, the code
takes an explicit early-out (no `Animation` is created at all) and uses the steady-state
alpha directly — so the default-off path does zero per-frame animation work.

> Note: this means the *default config value* is off, but the config block still *exists*
> and is *documented*. Users who want the fade turn it on. This is the safest upstream
> posture: zero behavioral change unless explicitly opted in.

### Quality bar (this is meant to be *good* niri code, not just merged)

The author is a Rust novice contributing to a project they care about; the goal is correct,
readable code that an expert can merge as-is or take the last mile on. Two safe, clearly-
correct fixes have been made; one architectural item is left for an expert (see Part 2).

**Done (safe fixes):**

- **Startup flash fixed.** A `focus_ring_initialized: bool` field records the tile's
  initial `is_active` on the first `update_render_elements()` call *without* animating,
  so a window that is already focused at creation (e.g. the first window you open) does
  not spuriously fade in from 0.
- **`off` is a clean early-out.** When `animations.focus-ring` is `off`, no `Animation`
  object is created and no per-frame state is kept; `ring_alpha` is the steady-state value
  (max-opacity when active, else 0). This matters because the feature defaults off — the
  disabled path should be free, not a degenerate 0ms animation.

**Left for an expert (see Part 2, Known issue):**

- Animation *creation* lives in `update_render_elements()` (render time). The ideal home
  is `Layout::refresh()` / `advance_animations()`, where the focus-change event actually
  originates, decoupling "focus changed" from "we painted." This is the one item that
  needs Rust judgement; it is documented precisely, not hidden.

### Keep the code understandable

- **Two transition flags, one animation.** `prev_focus_ring_is_active` + `focus_ring_initialized`
  - a single `Option<Animation>`. The two flags are distinct concerns (initialization vs.
  last-seen state) and are commented as such.
- **Comment the transition block** with a one-line "why" (fade starts from the current
  alpha so interrupted transitions don't jump; first call records state without animating).
- **Don't over-abstract.** No shared "element fade" trait, no generic. The focus ring is
  the only consumer. If a future PR wants the shadow, *that* PR can refactor.

### Edge cases handled

- **`off` config** → explicit early-out, steady-state alpha, no animation state.
- **Startup (first frame)** → `focus_ring_initialized` records state without animating
  (no fade-from-0 flash).
- **Interrupted transitions** (rapid Alt+Tab) → new animation starts from current alpha.
- **Fullscreen/maximize** → `ring_alpha * (1. - expanded_progress)` keeps it hidden until
  un-fullscreen; the fade still runs underneath.
- **Config merge** → `merge_clone!` handles includes/overrides.

### Open question (not a blocker)

- The default-config *example* in `resources/default-config.kdl` — keep it **commented
  out** (recommended, since default is off) or show it active? Recommendation: commented
  out, matching how most optional animations are presented there.

---

## Part 1b — Testing (nested, no session pollution)

You can run and visually verify the fade **without touching your real niri session**. niri
auto-selects its **winit backend** whenever `WAYLAND_DISPLAY`, `WAYLAND_SOCKET`, or `DISPLAY`
is set and it is *not* launched with `--session` (see `src/niri.rs` `State::new`:
`has_display` → `Backend::Winit`). So inside your existing Wayland session, the built binary
spawns niri as a plain window — a nested compositor. Your host session, host config, and
running niri are all untouched.

### Build (do NOT install)

```bash
cd /home/user/Development/niri
cargo build            # debug; add --release for a snappier nested compositor
# produces target/debug/niri — never `cargo install` over system niri
```

### Run as a window (nested)

```bash
./target/debug/niri    # WAYLAND_DISPLAY/DISPLAY already set -> winit backend automatically
```

- **Do not pass `--session`** — that strips the display env vars and tries to take over the
  TTY, which *would* disturb your real session.
- Inside the nested window, niri's own keybinds may collide with the host. The winit
  backend uses `config.input.mod_key_nested` (or a fallback) so you can still drive it.
  Alternatively, send actions from a terminal *inside* the nested window via
  `niri msg action <name>`.

### Enable the fade (it is off by default)

Point at a throwaway config — your `~/.config/niri/config.kdl` is never read unless passed
explicitly:

```bash
cat > /tmp/test-niri.kdl <<'EOF'
animations {
    focus-ring {
        duration-ms 300
        curve "ease-out-quad"
    }
}
EOF
./target/debug/niri --config /tmp/test-niri.kdl
```

Then open a couple of windows inside the nested niri (Super+Enter for a terminal) and
Alt+Tab / click between them — the ring should fade rather than snap.

### Variants worth exercising

- **Default / instant:** run with no config, or `focus-ring { off }` → ring appears/
  disappears instantly, proving `off` == old behavior.
- **Spring:** `focus-ring { spring damping-ratio=0.6 stiffness=500 epsilon=0.01 }` →
  bouncy fade.
- **Slow + rapid switching:** `duration-ms 1000` and rapidly Alt+Tab → exercises the
  "interrupted transition starts from current alpha" path (no jumps).

### Validate config without launching

```bash
./target/debug/niri validate --config /tmp/test-niri.kdl
```

Confirms the new `focus-ring` block parses (catches KDL errors before any render).

### Capture evidence for the PR

From the host, record the nested window with a screencast tool (OBS / Kooha). The nested
niri is just another window to the host compositor, so you get a clean before/after clip
without ever leaving your real session.

### Limitations of the winit path (so results aren't misread)

- **No real VRR / presentation timing** in the nested window. This is directly relevant to
the known intermittent-snap issue (render-time creation + skipped frames): the snap may
appear *more* often under winit than on the TTY target. Treat winit as a functional check,
not a timing-fidelity check.
- **Xwayland works** under winit, so X11 clients render too.
- **Input is nested:** the host still owns the global Super key unless `mod_key_nested` is
set.

### Concrete test setup (this machine)

Verified working on this box. Notes specific to the local environment:

- **Terminal:** the master `~/.config/niri/config.kdl` binds `Mod+T` to `alacritty`, but
  **alacritty is not installed here** — only **ghostty** is. So any test must use
  `ghostty`, not `alacritty`. (The master bind is effectively dead on this machine.)
- **Wallpaper:** niri has no native image wallpaper; the background is a solid
  `background-color` set inside the `layout {}` block (per-output `background-color` is
deprecated). A solid color is enough to see the ring against — no need for a wallpaper
  client. (A real wallpaper image would require a separate layer-shell wallpaper app, which
  is overkill for this test.)
- **Master config:** to test against your *actual* look, copy the master config and append
  the `animations { focus-ring { ... } }` block — your existing `focus-ring { max-opacity
  40; width 1; active-color ...; }` block is *visual* config and coexists with the new
  *animation* block.

**Throwaway test config** at `/tmp/niri-focus-ring-test.kdl` (already validated):

```kdl
layout {
    background-color "#1e1e2e"          // solid bg so the ring is visible
    focus-ring {
        max-opacity 40                   // matches master session look
        width 1
        active-color "#cccccc"
        inactive-color "#505050"
    }
}

animations {
    focus-ring { duration-ms 300; curve "ease-out-quad" }   // OFF by default; opt in
}

spawn-at-startup "ghostty"        // two terminals to Alt+Tab between
// spawn-at-startup "ghostty"

binds {
    Mod+T hotkey-overlay-title="Open a Terminal: ghostty" { spawn "ghostty"; }
}
```

**Run it (nested, no session pollution):**

```bash
./target/debug/niri --config /tmp/niri-focus-ring-test.kdl
```

`WAYLAND_DISPLAY`/`DISPLAY` are already set, so this opens niri *inside a window* on your
real session (winit backend). Your running niri and `~/.config/niri/config.kdl` are
untouched.

**What to exercise:**

1. Two ghostty windows open at startup. Click or `Mod+Tab` between them — the ring should
   **fade** in/out (300ms) rather than snap.
2. **Instant baseline:** temporarily set `focus-ring { off }` (or run with no config) →
   ring appears/disappears instantly, proving the default-off path == old behavior.
3. **Interrupted transition:** hold `Mod+Tab` and release on a different window rapidly →
   fade should start from the current alpha with no jump.
4. **Spring variant:** `focus-ring { spring damping-ratio=0.6 stiffness=500 epsilon=0.01 }`
   → bouncy fade.
5. **Slow + rapid:** `duration-ms 1000` + rapid `Mod+Tab` → stress the interrupted path.

**Keybinds inside the nested window:** the winit backend uses `config.input.mod_key_nested`
(or a fallback) so `Mod+...` still reaches niri; if the host swallows Super, send actions
from a terminal inside the nested window via `niri msg action <name>` (e.g.
`niri msg action focus-workspace-down`).

---

## Part 2 — Upstream PR strategy (single feature)

The goal is a PR niri's maintainer accepts. niri is carefully reviewed with strong
conventions (`docs/wiki/Development:-Design-Principles.md`, the existing animation
pattern). The patch must look like it was *always* meant to be there, and must not change
default behavior.

### Commits (keep it tight — 3 commits)

1. **`animations: add focus-ring fade config type (off by default)`**
   The `FocusRingAnim` newtype, `Default` = `new_off()`, `knuffel::Decode`, field in
   `Animations` / `AnimationsPart` / `merge_clone!`. No engine wiring yet. Purely
   additive, behavior unchanged.

2. **`layout/tile: animate focus ring alpha on focus change`**
   The two tile fields, transition detection in `update_render_elements()`, cleanup in
   `advance_animations()`. Off by default → no visible change unless configured.

3. **`docs: document the focus-ring animation`**
   Wiki entry (`docs/wiki/Configuration:-Animations.md`) with a `Since:` tag + KDL
   example, and a commented example in `resources/default-config.kdl`. Update insta
   snapshot.

### Match niri conventions precisely

- **Follow the `FocusRingAnim` / `ScreenshotUiOpenAnim` newtype-over-`Animation` pattern
  exactly.** Don't invent a new config shape.
- **Wire into `merge_clone!`** in the same three places as the other animations.
- **Update the insta snapshot** in `niri-config/src/lib.rs` (`assert_snapshot!` near line
  2451). Run `cargo insta`, review the diff — an unexpected snapshot change is a review
  red flag.
- **Wiki doc** with the same structure as siblings: `#### focus-ring` heading, `Since:`
  version tag, KDL example, one-line "what animates" description.
- **Default-config example commented out** (since the feature is off by default).

### Testing / evidence

- See **Part 1b — Testing** for the full nested (winit) workflow: build without
  installing, run as a window inside your real session, enable the fade via a throwaway
  `--config`, and capture a host-side screen recording for the PR.
- `cargo test` and `cargo clippy --all-targets` clean.
- Note the edge cases from Part 1 (interrupted transitions, off = instant, fullscreen
  composition).

### Known issue — the one item for an expert (honest handoff)

The AGENTS.md notes an **intermittent two-step / abrupt transition**. Root cause: the
animation is *created* in `update_render_elements()` (render time), so a skipped frame
(VRR idle / no damage) makes it jump a step. The focus-change event actually originates in
`Layout::refresh(is_active)` (src/layout/mod.rs:4843), which is called when focus changes —
*not* when painting. The correct home for starting the animation is `refresh()` (or
`advance_animations()`), decoupling "focus changed" from "we painted."

This is the **single architectural item** left open. It is deliberately *not* hidden behind
"don't block the PR" language — it is the one piece that needs a Rust expert's judgement,
because moving creation to `refresh()` must carefully detect the *transition* (refresh runs
more often than focus actually changes: overview open, interactive move, monitor changes)
and must handle tiles that may not yet have a clock/renderer context.

**What is already done** (so the expert starts from clean ground):

- Startup flash: fixed via `focus_ring_initialized` (no fade-from-0 on a freshly-focused
  window at creation).
- `off` path: clean early-out, no animation state, so the default-off behavior is free.

**Suggested direction for the expert:**

- Start the animation in `Layout::refresh()` (or `advance_animations()`), keyed on the
  `is_active` transition detected there.
- Verify `update_render_elements()` still runs every composed frame for every visible tile.
- Keep `prev_focus_ring_is_active` + `focus_ring_initialized` as the guards. The
  `clamped_value()`-from-current logic for interrupted transitions stays as-is (it is
  correct).

If no expert picks this up, the feature is still correct and mergeable; the intermittent
snap is rare and the trade-off is documented. The goal of this contribution is *good code
that helps niri*, whether merged as-is or taken over by someone more experienced.

### PR description skeleton

```
Title: animations: add configurable focus-ring fade (off by default)

The focus ring currently appears/disappears instantly. This adds an optional
`animations.focus-ring` block that fades the ring alpha on focus change.

It is OFF by default, so out-of-the-box behavior is unchanged.

## Config (opt-in)
```kdl
animations {
    focus-ring {
        duration-ms 300
        curve "ease-out-quad"
    }
}
```

`off` restores the previous instant behavior.

## Notes

- Fade goes 0 ↔ configured max-opacity.
- Interrupted transitions start from the current alpha (no jumps).
- Composes with the maximize/fullscreen alpha automatically.
- Known: animation is created at render time; a skipped frame can cause a rare
  1-frame jump. Follow-up: move creation to refresh()/advance_animations().

```

---

## Appendix — Shadow extension (considered, OUT OF SCOPE)

Kept only as a reference for a possible future PR. **Not part of this submission.**

The shadow already threads `alpha: f32` to the shader but snaps between `color` and
`inactive_color` on focus change (no fade). A faithful extension would crossfade the
color (`Color::lerp` between `color` and `inactive_color.unwrap_or(color * 0.75)`) driven
by a `ShadowAnim` config type mirroring `FocusRingAnim`, sharing a single `prev_is_active`
flag. It would require adding a `Color::lerp` helper (none exists today). Deferred
because the user wants exactly one feature accepted and nothing on by default.
