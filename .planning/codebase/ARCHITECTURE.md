# Architecture

**Analysis Date:** 2026-02-16

## Pattern Overview

**Overall:** Event-driven Wayland compositor with Smithay backend

**Key Characteristics:**
- Monolithic core in `niri.rs` (~235k lines) handling the main compositor state
- Modular sub-systems: layout, input, render, handlers, protocols
- Event-driven architecture via calloop event loop
- Property-based testing for layout correctness
- Snapshot testing for rendering

## Layers

**Core Compositor:**
- Purpose: Central state management and event routing
- Location: `src/niri.rs`
- Contains: `Niri` struct (main compositor state), display initialization, event dispatch
- Depends on: All sub-systems (layout, input, render, animation)
- Used by: Backend drivers

**Window Layout (Tiling Engine):**
- Purpose: Scrollable-tiling window management with dynamic workspaces
- Location: `src/layout/`
- Contains: `Layout<W>` trait, `ScrollingSpace`, `FloatingSpace`, workspace management
- Key files: `scrolling.rs` (5600+ lines), `workspace.rs`, `monitor.rs`, `tile.rs`
- Depends on: `niri_config`, `niri_ipc`, animation, render_helpers

**Input Handling:**
- Purpose: Keyboard, pointer, touch, tablet input processing
- Location: `src/input/`
- Contains: Input state, grab handlers (move, resize, spatial movement), gestures
- Key files: `mod.rs` (221k lines - second largest), `move_grab.rs`, `resize_grab.rs`

**Wayland Protocol Handlers:**
- Purpose: Wayland protocol implementations (xdg-shell, layer-shell, etc.)
- Location: `src/handlers/`
- Contains: XDG shell, layer shell, compositor protocol handlers
- Key files: `xdg_shell.rs` (64k lines), `layer_shell.rs`, `compositor.rs`

**Rendering:**
- Purpose: GPU rendering, window compositing, effects
- Location: `src/render_helpers/`
- Contains: Render elements, shaders, borders, shadows, textures
- Key files: `shader_element.rs`, `border.rs`, `shadow.rs`, `offscreen.rs`

**Animation System:**
- Purpose: Window/workspace animations, transitions
- Location: `src/animation/`
- Contains: Animation timing, spring physics, bezier curves, clock
- Key files: `mod.rs`, `spring.rs`, `bezier.rs`, `clock.rs`

**Layer Shell (Overlays):**
- Purpose: Waybar, dunst, and other overlay window management
- Location: `src/layer/`
- Contains: Layer surface mapping, animation state
- Key files: `mapped.rs`, `mod.rs`, `layer_animation.rs`

**Custom Protocols:**
- Purpose: Extended Wayland protocols (workspace management, screencopy, etc.)
- Location: `src/protocols/`
- Contains: ext_workspace, foreign_toplevel, screencopy, output_management

**Configuration:**
- Purpose: KDL config parsing and validation
- Location: `niri-config/src/`
- Contains: Config structs, bindings, animations, window rules

**IPC:**
- Purpose: Client-compositor communication
- Location: `niri-ipc/src/`
- Contains: Request/Reply/Event enums, socket handling

## Data Flow

**Window Opening Flow:**
1. Client creates xdg-surface → `handlers/xdg_shell.rs`
2. `Layout::open_window()` → adds to workspace
3. `Tile` created with initial size/position
4. `opening_window.rs` handles animation state
5. Render elements created via `TileRenderElement`

**Input Event Flow:**
1. Backend receives input → `input/mod.rs`
2. `InputState::process_keyboard/pointer_event()`
3. Grab handlers check for active grabs (move, resize)
4. Key bindings matched via `binds.rs`
5. Action dispatched to layout/input/workspace

**Rendering Flow:**
1. Frame clock ticks → `Niri::render_frame()`
2. `render_helpers` collect all render elements
3. Each element prepared (textures, shaders)
4. Smithay renderer composites to GPU
5. `OutputDamageTracker` handles partial updates

## Key Abstractions

**LayoutElement Trait:**
- Purpose: Abstract window interface for testability
- Location: `src/layout/mod.rs`
- Examples: `Tile<W>`, `TestWindow` (in tests)
- Pattern: Trait-based dependency injection for testing

**RenderElement Types:**
- Purpose: Smithay-compatible rendering
- Examples: `TileRenderElement`, `LayerSurfaceRenderElement`, `SolidColorRenderElement`
- Pattern: Each component has corresponding `*RenderElement`

**Grab Handlers:**
- Purpose: Input gesture state machines
- Examples: `MoveGrab`, `ResizeGrab`, `SpatialMovementGrab`
- Pattern: State enum with transition methods

## Entry Points

**Main:**
- Location: `src/main.rs`
- Triggers: User runs `niri` binary
- Responsibilities: CLI parsing, logging init, config loading, compositor startup

**Backend Initialization:**
- Location: `src/backend/mod.rs`
- Triggers: Backend selection (TTY, Winit, Headless)
- Responsibilities: DRM/EGL initialization, input device setup

**Display Setup:**
- Location: `src/niri.rs` - `Niri::new()`
- Triggers: After config and backend init
- Responsibilities: Wayland display creation, protocol registration

## Error Handling

**Strategy:** Result-based with anyhow for compositing errors

**Patterns:**
- `anyhow::Result<T>` for fallible operations
- `bail!` macro for early returns with context
- `ensure!` macro for precondition checks
- `tracing::error!` for logging failures

## Cross-Cutting Concerns

**Logging:** tracing crate with `tracing::info!`, `warn!`, `error!`
**Validation:** Config validation via knuffel/miette error reporting
**Authentication:** N/A (no user auth - compositor runs per-session)

---

*Architecture analysis: 2026-02-16*
