# Refactor Spec: Split Layer Open/Close Animation Render Types

Status: Proposed
Owner: layer-anims branch
Last updated: 2026-02-18

## 1. Problem Statement

Layer animations currently share one type and one render-element path for both open and close:

- `LayerAnimation` uses `is_open: bool` to branch behavior in one implementation (`src/layer/layer_animation.rs:22`, `src/layer/layer_animation.rs:26`).
- `LayerSurfaceRenderElement` has one animation variant (`Animation = LayerAnimationRenderElement`) (`src/layer/mapped.rs:84`).
- `MappedLayer::render_with_animation()` contains both open and close rendering logic in one method (`src/layer/mapped.rs:467`).

This differs from the window path, where open and close are explicitly separated:

- Open animation has dedicated types in tile rendering (`src/layout/opening_window.rs:21`, `src/layout/tile.rs:126`).
- Close animation has dedicated types in space-level rendering (`src/layout/closing_window.rs:28`, `src/layout/scrolling.rs:99`, `src/layout/floating.rs:74`).

Important clarification: `TileRenderElement` does not contain a close-animation variant. Window close animations are rendered from `ScrollingSpaceRenderElement` and `FloatingSpaceRenderElement` through `ClosingWindowRenderElement`.

## 2. Goals

1. Separate layer open and close animation rendering into distinct types and render-element enums.
2. Remove `is_open` as the primary runtime discriminator for layer animation behavior.
3. Make layer render enums lifecycle-explicit (`OpeningAnimation` vs `ClosingAnimation`) instead of generic `Animation`.
4. Keep existing runtime behavior and lifecycle semantics (including `closing_layers`) unchanged.
5. Land refactor in small, reviewable, atomic commits.

## 3. Non-Goals

1. Do not redesign `closing_layers` container or ownership model in this refactor (`src/niri.rs:247`).
2. Do not introduce window transaction blocker semantics for layers.
3. Do not redesign shader program APIs (`ProgramType::LayerOpen`, `ProgramType::LayerClose`) in this refactor.
4. Do not change animation timings or config schema (`layer_open`, `layer_close`).

## 4. Current Architecture (Reference)

### 4.1 Layer path today

- Animation state fields in `MappedLayer`:
  - `open_animation: Option<LayerAnimation>` (`src/layer/mapped.rs:53`)
  - `close_animation: Option<LayerAnimation>` (`src/layer/mapped.rs:56`)
- Single animation render enum:
  - `LayerAnimationRenderElement` (`src/layer/layer_animation.rs:31`)
- Layer surface enum exposes one animation variant:
  - `Animation = LayerAnimationRenderElement` (`src/layer/mapped.rs:84`)

### 4.2 Window path today (target style)

- Open:
  - `OpenAnimation` (`src/layout/opening_window.rs:21`)
  - `OpeningWindowRenderElement` (`src/layout/opening_window.rs:28`)
  - Wired into `TileRenderElement::Opening` (`src/layout/tile.rs:126`)
- Close:
  - `ClosingWindow` (`src/layout/closing_window.rs:28`)
  - `ClosingWindowRenderElement` (`src/layout/closing_window.rs:58`)
  - Wired into space enums (`src/layout/scrolling.rs:99`, `src/layout/floating.rs:74`)

## 5. Target Architecture

### 5.1 New layer animation types

Introduce two dedicated structs:

1. `OpeningLayerAnimation`
2. `ClosingLayerAnimation`

These replace the current single `LayerAnimation` type for active state in `MappedLayer`.

### 5.2 New layer animation render enums

Introduce two dedicated render-element enums:

1. `OpeningLayerRenderElement`
2. `ClosingLayerRenderElement`

### 5.3 LayerSurfaceRenderElement shape

Change `LayerSurfaceRenderElement` from:

```rust
Animation = LayerAnimationRenderElement
```

to:

```rust
OpeningAnimation = OpeningLayerRenderElement
ClosingAnimation = ClosingLayerRenderElement
```

while preserving existing non-animation variants:

- `Wayland`
- `SolidColor`
- `Shadow`

### 5.4 MappedLayer state shape

Change fields from:

```rust
open_animation: Option<LayerAnimation>
close_animation: Option<LayerAnimation>
```

to:

```rust
open_animation: Option<OpeningLayerAnimation>
close_animation: Option<ClosingLayerAnimation>
```

Pending animation fields remain unchanged in this refactor:

- `pending_open_animation` (`src/layer/mapped.rs:59`)
- `pending_close_animation` (`src/layer/mapped.rs:62`)

## 6. Invariants (Must Hold)

1. Open and close animations remain mutually exclusive (`start_open_animation` clears close state; `start_close_animation` clears open state).
2. Close animations must continue rendering after unmap through `closing_layers` (`src/niri.rs:4244`).
3. Redraw detection for closing layers must continue to work (`src/niri.rs:4442`).
4. `target.should_block_out(...)` behavior remains unchanged (`src/layer/mapped.rs:480`).
5. Shader selection remains tied to `ProgramType::LayerOpen` and `ProgramType::LayerClose`.

## 7. Implementation Plan (Atomic Commits)

### Commit 1: Add New Types (No Wiring Yet)

Files:

- `src/layer/opening_layer.rs` (new)
- `src/layer/closing_layer.rs` (new)
- `src/layer/mod.rs`

Actions:

1. Add `OpeningLayerAnimation` + `OpeningLayerRenderElement`.
2. Add `ClosingLayerAnimation` + `ClosingLayerRenderElement`.
3. Export new modules/types from `src/layer/mod.rs`.
4. Keep existing `LayerAnimation` untouched.

Validation:

- `cargo check --release`

### Commit 2: Wire Open Path to New Type

Files:

- `src/layer/mapped.rs`

Actions:

1. Switch `open_animation` field type to `Option<OpeningLayerAnimation>`.
2. Update `advance_animations()` open branch to construct `OpeningLayerAnimation`.
3. Update open branch in `render_with_animation()` to emit `OpeningLayerRenderElement`.
4. Keep close path on legacy type for this commit.

Validation:

- `cargo check --release`
- `cargo test`

### Commit 3: Wire Close Path to New Type

Files:

- `src/layer/mapped.rs`
- `src/niri.rs` (if enum variant plumbing requires updates)

Actions:

1. Switch `close_animation` field type to `Option<ClosingLayerAnimation>`.
2. Update `advance_animations()` close branch to construct `ClosingLayerAnimation`.
3. Update close branch in `render_with_animation()` to emit `ClosingLayerRenderElement`.
4. Keep snapshot capture behavior unchanged for now.

Validation:

- `cargo check --release`
- `cargo test`

### Commit 4: Split LayerSurfaceRenderElement Variants

Files:

- `src/layer/mapped.rs`
- `src/niri.rs` (callers accepting/pushing layer elements)

Actions:

1. Replace `Animation` variant with explicit `OpeningAnimation` and `ClosingAnimation` variants.
2. Update all `push(...)` callsites in `MappedLayer` animation rendering.
3. Confirm `OutputRenderElements` integration compiles and behavior is unchanged (`src/niri.rs:6249`).

Validation:

- `cargo check --release`
- `cargo test`

### Commit 5: Remove Legacy LayerAnimation

Files:

- `src/layer/layer_animation.rs` (delete or reduce to shared helpers)
- `src/layer/mod.rs`

Actions:

1. Remove legacy `LayerAnimation` and `LayerAnimationRenderElement`.
2. Remove any remaining imports/usages.
3. Keep any extracted shared helpers only if still used.

Validation:

- `cargo check --release`
- `cargo test`

## 8. File-by-File Change Spec

### 8.1 `src/layer/opening_layer.rs` (new)

Must contain:

1. `OpeningLayerAnimation` struct with only open-animation concerns.
2. `OpeningLayerRenderElement` enum (offscreen + shader variants).
3. `render(...)` method equivalent to current open behavior.

Must not contain:

- Snapshot persistence for close path.
- `is_open` branching.

### 8.2 `src/layer/closing_layer.rs` (new)

Must contain:

1. `ClosingLayerAnimation` struct with close-only concerns.
2. `ClosingLayerRenderElement` enum.
3. `render(...)` method equivalent to current close behavior.

Must not contain:

- Open fallback behavior.

### 8.3 `src/layer/mapped.rs`

Must change:

1. Animation field types to new open/close structs.
2. Enum macro declaration for `LayerSurfaceRenderElement` to explicit open/close variants.
3. `render_with_animation()` internal dispatch.
4. `advance_animations()` constructors.

Must not change:

1. `closing_geometry` semantics.
2. `should_remove()` semantics (`src/layer/mapped.rs:265`).
3. pending 16ms gating behavior.

### 8.4 `src/niri.rs`

Must validate and update only if required by type changes:

1. `render_layer_normal(...)` integration (`src/niri.rs:4358`).
2. `closing_layers` render pass (`src/niri.rs:4244`).
3. `OutputRenderElements` type integration (`src/niri.rs:6246`).

Must not change:

1. `closing_layers` container structure.
2. Redraw heuristics logic.

## 9. Testing Plan

### 9.1 Automated

Run after each commit:

1. `cargo check --release`
2. `cargo test`

### 9.2 New tests to add

Create `src/tests/layer_animations.rs` and register in `src/tests/mod.rs`.

Test cases:

1. Open animation path renders with opening render element type.
2. Close animation path renders with closing render element type.
3. Reopen while closing clears close state and starts open state.
4. Close animation continues via `closing_layers` after unmap.
5. Blocked-out render target still follows existing fallback behavior.

### 9.3 Manual validation

1. Persistent launcher (Fuzzel) open/close/reopen loop.
2. Notification layer close animation visibility.
3. Multi-output close animation redraw continuity.

## 10. Risk Register

### Risk A: Variant mismatch breaks conversion paths

- Area: macro-generated enum conversions (`src/render_helpers/render_elements.rs:4`)
- Mitigation: keep variant changes localized, compile after each commit.

### Risk B: Close animation regresses due to mixed legacy/new state

- Area: `MappedLayer::render_with_animation()` close branch (`src/layer/mapped.rs:486`)
- Mitigation: migrate open and close in separate commits with tests between.

### Risk C: Redraw loop misses new closing type

- Area: `Niri::redraw` unfinished animation checks (`src/niri.rs:4442`)
- Mitigation: preserve existing `is_close_animation_ongoing()` contract.

## 11. Acceptance Criteria

All must be true:

1. No active runtime code path uses `LayerAnimation { is_open: ... }`.
2. `LayerSurfaceRenderElement` has explicit open/close animation variants.
3. Open and close layer render paths use dedicated render element types.
4. Existing open/close behavior and redraw behavior remain unchanged.
5. `cargo check --release` and `cargo test` pass.

## 12. Follow-Up Work (Out of Scope for This Spec)

1. Consolidate shared shader math helpers with window open/close implementations.
2. Consider moving close snapshot ownership fully into `ClosingLayerAnimation` if still split.
3. Evaluate replacing hardcoded `Duration::from_millis(16)` with a named constant.

## 13. Appendix: Evidence Anchors

Layer path anchors:

- `src/layer/layer_animation.rs:22`
- `src/layer/layer_animation.rs:26`
- `src/layer/mapped.rs:53`
- `src/layer/mapped.rs:56`
- `src/layer/mapped.rs:84`
- `src/layer/mapped.rs:467`

Window reference anchors:

- `src/layout/opening_window.rs:21`
- `src/layout/opening_window.rs:28`
- `src/layout/closing_window.rs:28`
- `src/layout/closing_window.rs:58`
- `src/layout/tile.rs:126`
- `src/layout/scrolling.rs:99`
- `src/layout/floating.rs:74`

Root rendering/lifecycle anchors:

- `src/niri.rs:247`
- `src/niri.rs:4244`
- `src/niri.rs:4358`
- `src/niri.rs:4442`
- `src/niri.rs:6249`
