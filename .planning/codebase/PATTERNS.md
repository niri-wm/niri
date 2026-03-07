# Coding Conventions

**Analysis Date:** 2026-02-16

## Naming Patterns

**Files:**
- `mod.rs` - Module entry point
- `*_grab.rs` - Input grab handlers (e.g., `move_grab.rs`, `resize_grab.rs`)
- `*_element.rs` - Render elements (e.g., `tile_element.rs`, `border.rs`)
- `*_handler.rs` - Protocol handlers (grouped in `handlers/`)
- snake_case: all lowercase with underscores

**Functions:**
- snake_case: `open_window()`, `handle_key_press()`, `render_frame()`
- Accessors: `fn name(&self) -> &Type` (getters)
- Mutations: `fn open(&mut self, ...)` (commands)
- `pub(crate)` for internal APIs

**Types:**
- PascalCase: `struct Layout`, `enum HitType`, `trait LayoutElement`
- Type aliases: `type Result<T> = std::result::Result<T, E>`
- Private fields with pub accessors pattern

**Constants:**
- SCREAMING_SNAKE_CASE: `const RESIZE_ANIMATION_THRESHOLD: f64 = 10.;`

## Code Style

**Formatting:**
- Tool: `cargo +nightly fmt --all` (NOT stable rustfmt)
- Config: `rustfmt.toml`
- Key settings:
  - `imports_granularity = "Module"` - Group by module
  - `group_imports = "StdExternalCrate"` - std, external, crate groups
  - `wrap_comments = true`
  - `comment_width = 100`

**Linting:**
- Tool: `cargo clippy --all --all-targets`
- Config: `clippy.toml`
- Key rules:
  - Allow `new_without_default` for types
  - Ignore interior mutability for `smithay::desktop::Window`, `smithay::output::Output`, `wayland_server::backend::ClientId`

## Import Organization

**Order:**
1. Standard library (`std::`, `core::`)
2. External crates (`smithay::`, `anyhow::`)
3. Local crate (`crate::`, `super::`)
4. Module imports

**Example from `src/niri.rs`:**
```rust
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet};

use anyhow::{bail, ensure, Context};
use smithay::backend::input::Keycode;
use smithay::desktop::{LayerMap, Space, Window};

use crate::animation::Clock;
use crate::backend::{Backend, Tty, Winit};
```

**Path Aliases:**
- Not typically used; prefer full paths

## Error Handling

**Patterns:**
```rust
// Use anyhow for composable errors
use anyhow::{bail, ensure, Context};

fn do_something() -> anyhow::Result<()> {
    ensure!(condition, "error message");
    
    some_fn()
        .context("additional context")?;
    
    Ok(())
}

// Match on Result in main paths
match self.layout.open_window(...) {
    Ok(window) => { /* success */ },
    Err(e) => { /* handle error */ },
}
```

**Logging:**
```rust
use tracing::{info, warn, error};

info!("listening on X11 socket: {name}");
warn!("refusing lock as another client is currently locking");
error!("quitting due to error: {e}");
```

**No Result Type Alias:**
- Most functions use `anyhow::Result<T>` directly
- Single custom alias in `src/utils/watcher.rs`: `type Result<T = (), E = Box<dyn Error>>`

## Comments

**When to Comment:**
- Module-level documentation in `mod.rs`
- Complex algorithm explanations
- "Why" not "what"
- Non-obvious invariants

**Doc Comments:**
- `///` for public API
- `//` for inline explanations

**Example from `src/layout/mod.rs`:**
```rust
//! Window layout logic.
//!
//! Niri implements scrollable tiling with dynamic workspaces. The scrollable tiling is mostly
//! orthogonal to any particular workspace system, though outputs living in separate coordinate
//! spaces suggest per-output workspaces.
//!
```

## Function Design

**Size:**
- Functions tend to be medium-sized (20-50 lines)
- Large files (niri.rs 6171 lines, input/mod.rs 221k lines) contain multiple related functions

**Parameters:**
- Take references for large types: `&self`, `&mut self`
- Clone for small types: `String`, `u32`
- Use newtypes for related parameters: `struct WorkspaceId(u32)`

**Return Values:**
- Return `Result<T, E>` for fallible operations
- Return references when appropriate: `fn workspace(&self) -> Option<&Workspace>`
- Return newtypes for opaque IDs: `OutputId`, `WorkspaceId`

## Module Design

**Exports:**
- `pub use` for re-exports in `mod.rs`
- `pub(crate)` for internal public APIs
- Private by default

**Barrel Files:**
- `mod.rs` serves as barrel for module
- Group related types with `pub mod sub;`

**Module Organization:**
```
mod foo;
pub mod bar;

pub use foo::Foo;
pub use bar::*;
```

## Trait Usage

**Common Traits:**
- `Debug` - All significant types
- `Clone` - For value types
- `Default` - For configurable types
- `PartialEq` / `Eq` - For comparisons
- `From` / `Into` - For conversions

**Custom Traits:**
- `LayoutElement` - Abstract window interface
- `LayoutElementRenderElement` - Rendering abstraction

## Testing Patterns

**Unit Tests:**
- In-source `#[cfg(test)]` modules
- Property-based: `proptest` with `#[derive(Arbitrary)]`
- Mock types for testing (e.g., `TestWindow`)

**Integration Tests:**
- Separate test binary: `src/tests/`
- Client-server tests: `client.rs`, `server.rs`
- Snapshot tests: `insta` crate

**Test Organization:**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    
    #[test]
    fn test_something() { ... }
}
```

---

*Convention analysis: 2026-02-16*
