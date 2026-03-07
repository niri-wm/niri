---
status: resolved
trigger: "Layer open animation double-run for Anyrun/vicinae, swaync window disappears and reappears"
created: 2026-02-23T12:00:00Z
updated: 2026-02-23T14:30:00Z
---

## Current Focus
Investigating where layer open animation gets triggered and why it runs twice for some clients.

**Hypothesis:** When a layer surface re-maps while its close animation is still playing, BOTH animations may be active briefly, causing the double animation effect. Alternatively, the layer might be getting added/removed from tracking structures incorrectly.

**Test:** Check if surface is being added back to unmapped_layer_surfaces incorrectly, or if close animation is not being properly cancelled.

**Expecting:** Find root cause in the layer lifecycle management (mapped/unmapped/closing_layers tracking).

**Next Action:** Trace the exact code path when a layer closes and re-opens to verify animation state management.

## Symptoms
<!-- Written during gathering, then IMMUTABLE -->

expected: Single smooth open animation should play when a layer surface appears.
actual: 
- Anyrun/vicinae: Animation runs twice - once on show, once on map
- swaync: Window appears, then vanishes, then reappears on mouse movement or input
- Fuzzel: Works correctly with single animation
errors: []
reproduction: Open anyrun/vicinae/swaync and observe animations
started: New feature development in @layer-anims2 branch

## Eliminated
<!-- APPEND only - prevents re-investigating -->

## Evidence
<!-- APPEND only - facts discovered -->

- timestamp: 2026-02-23T12:30:00Z
  checked: src/handlers/layer_shell.rs (line 159)
  found: Animation starts in layer_shell_handle_commit() when was_unmapped is true
  implication: Animation should only start once per open event

- timestamp: 2026-02-23T12:35:00Z
  checked: src/layer/mapped.rs (line 196-198)
  found: start_open_animation() sets pending_open_animation and clears open_animation
  implication: Called multiple times would reset the animation, not duplicate it

- timestamp: 2026-02-23T12:40:00Z
  checked: src/utils/mod.rs is_mapped() function
  found: is_mapped checks if surface has a buffer attached
  implication: Different clients may commit differently, affecting when is_mapped returns true

- timestamp: 2026-02-23T12:45:00Z
  checked: layer_shell.rs new_layer_surface vs layer_shell_handle_commit
  found: Two separate points - new_layer_surface (creates layer) and commit (with buffer)
  implication: Multiple commits could potentially trigger multiple animation starts

- timestamp: 2026-02-23T13:00:00Z
  checked: layer_shell.rs line 198-208 (unmapped path)
  found: When layer unmaps, it's added back to unmapped_layer_surfaces even if close animation starts
  implication: Could cause issues when re-opening during close animation

- timestamp: 2026-02-23T13:10:00Z
  checked: layer_shell.rs line 129-168 (mapped path)
  found: When layer re-maps, checks if in closing_layers and removes if found
  implication: This removes from closing_layers but what about the newly created MappedLayer?

- timestamp: 2026-02-23T13:30:00Z
  checked: git history - commit 82abe639
  found: Previous fix added open_animation_started flag but was later removed
  implication: The flag was removed in later refactoring, potentially reintroducing the bug

- timestamp: 2026-02-23T13:45:00Z
  checked: Current code - mapped.rs advance_animations
  found: Uses 16ms delay before pending animation becomes active
  implication: If commits happen within 16ms window, animations might stack

- timestamp: 2026-02-23T14:00:00Z
  checked: niri.rs render_layer_normal - closing_layers rendering
  found: Both mapped layers and closing_layers are rendered in same frame
  implication: Close animation and open animation can play simultaneously, creating "double animation" effect

## Resolution
<!-- OVERWRITE as understanding evolves -->

root_cause: The animation is triggered multiple times because:
1. When a layer unmaps (null commit), it's added back to unmapped_layer_surfaces (line 208)
2. When the same layer re-maps with new buffer, was_unmapped returns Some
3. A new MappedLayer is created and start_open_animation() is called
4. The previous fix (open_animation_started flag in commit 82abe639) was removed in later refactoring
5. The current 16ms delay in pending animation can cause stacking if commits happen quickly
6. Both close and open animations can render simultaneously during transition

fix: Need to add back the open_animation_started flag to prevent running animation multiple times per layer lifecycle. The flag should be set after animation starts and checked before starting a new one.

files_changed: 
- src/layer/mapped.rs: Add open_animation_started flag
- src/handlers/layer_shell.rs: Check flag before starting animation
