# Handover: Focus Ring Alpha Fade Animation

## Session Context

- **Date:** 2026-07-04
- **Goal achieved:** Focus ring alpha fades on focus change, implemented and tested live
- **Next session:** Simplify/prepare code for upstream PR, investigate known issues

## Fork & Branch

- Fork: `https://github.com/cillianpower/niri`
- Branch: `focus-ring-anim` (from tag `v26.04`, matches Fedora 44 package)
- Upstream remote: `https://github.com/niri-wm/niri.git`
- Discussion: <https://github.com/niri-wm/niri/discussions> (search "Animate focus ring alpha on focus change")

## Status

Feature **implemented, tested, and confirmed working** with a 3-second linear fade (diagnostic) then dialed back to 300ms EaseOutQuad as final default.

## Installed Binaries

| Path | Description |
| --- | --- |
| `/usr/bin/niri` | Our build (release + line tables, ~154MB) |
| `/usr/sbin/niri` | Same as above |
| `/usr/bin/niri.backup` | Original Fedora v26.04 build (~22MB) |
| `/usr/sbin/niri.backup` | Same as above |

To revert: `sudo mv /usr/bin/niri.backup /usr/bin/niri` (both paths).

**Restart niri to pick up a new build:**

```bash
niri msg action quit
# logs out to display manager; log back in
```

Or from TTY: `systemctl --user restart niri.service`

## Files Changed

### 1. `niri-config/src/animations.rs`

- **Added `FocusRingAnim(pub Animation)`** — newtype wrapper, default 300ms EaseOutQuad
- **Added `knuffel::Decode` impl** — enables KDL config parsing
- **Added to `Animations` struct** — field, Default, AnimationsPart, merge_clone!

### 2. `src/layout/tile.rs`

- **Added `focus_ring_alpha_anim: Option<Animation>`** — runtime animation state
- **Added `prev_focus_ring_is_active: bool`** — detects is_active transitions
- **Modified `update_render_elements()`** — detects focus changes, creates/starts/reads animation, multiplies alpha with expanded progress
- **Modified `advance_animations()`** — clears completed focus ring animations

### 3. `resources/default-config.kdl`

- Added commented-out `focus-ring` block example

### 4. `niri-config/src/lib.rs`

- Updated inline `insta` test snapshot

## Architecture

The focus ring alpha animation is driven entirely in `Tile` (which has `Clock` and `Options`). On each frame during rendering:

1. `update_render_elements()` compares `is_active` with `prev_focus_ring_is_active`
2. On mismatch, creates `Animation::new(clock, current_alpha, target_alpha, 0, config)`
3. Reads `clamped_value()` as the current animated alpha
4. Multiplies with the expanded-progress alpha, passes to `FocusRing::update_render_elements()`
5. `advance_animations()` clears completed animations

No changes were needed to `FocusRing` or `BorderRenderElement` — they already supported an `alpha` parameter.

## Configuration

In `~/.config/niri/config.kdl`:

```kdl
animations {
    focus-ring {
        duration-ms 300
        curve "ease-out-quad"
    }
}
```

Curves: `linear`, `ease-out-quad`, `ease-out-cubic`, `ease-out-expo`, `cubic-bezier x1 y1 x2 y2`
Springs: `spring damping-ratio=0.6 stiffness=500 epsilon=0.01`
Disable: `focus-ring { off }`

## Testing

```bash
cargo test           # 195 pass, 0 fail
cargo build --release
# install & restart to test visually
```

## Known Issues (For Future Investigation)

### 1. Intermittent two-step transition

Sometimes the focus ring fades smoothly, sometimes it appears to do a 2-step snap (dim → brighter → full). Inconsistently reproducible. See `AGENTS.md` → Observed Issues for possible causes (frame timing, clock granularity, clamping behavior).

### 2. Startup flash

On the very first frame of a new session, the active window's focus ring briefly fades from 0 because `prev_focus_ring_is_active` starts as `false`, triggering a fade-in on first render.

## Full Technical Documentation

See `AGENTS.md` at project root for complete technical docs, architecture diagram, edge cases, and observed issues.

## Next Steps (Suggested)

1. **Fix startup flash:** Add a `focus_ring_initialized` or first-frame flag to skip animation on tile creation
2. **Investigate two-step transition:** Add tracing/logging to track frame timing vs animation advancement
3. **Pre-upstream cleanup:** Simplify code, add comments, ensure formatting, consider rebasing
4. **Rebase on newer upstream tag** when available
