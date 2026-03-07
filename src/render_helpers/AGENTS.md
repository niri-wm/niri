# RENDER HELPERS

GPU-accelerated rendering utilities for OpenGL/GLES output.

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Custom shaders | `shaders/` | Border, shadow, resize, open/close animation shaders |
| Renderer traits | `renderer.rs` | NiriRenderer, AsGlesRenderer abstractions |
| Shader compilation | `shader_element.rs` | Generic shader render element with uniforms |
| Gradient borders | `border.rs` | Oklab/Oklch gradients with rounded corners |
| Window shadows | `shadow.rs` | Gaussian-blurred rounded rectangle shadows |
| Texture management | `texture.rs` | Fractional scale texture buffers |
| Surface rendering | `surface.rs` | Wayland surface tree to texture snapshots |
| Resize animations | `resize.rs` | Shader-based window resize effects |
| Clipped surfaces | `clipped_surface.rs` | Rounded corner surface clipping |
| Damage tracking | `damage.rs` | Output damage accumulation for partial redraws |
| Offscreen rendering | `offscreen.rs` | Render-to-texture utilities |
| Primary GPU textures | `primary_gpu_texture.rs` | GPU-specific texture optimizations |

## ANTI-PATTERNS

- **Shader failures are silent**: Failed shader compiles log warnings but don't panic - always check `Shaders::program()` returns `Some`
- **EGL context required**: Shaders store in EGL user_data - accessing before `shaders::init()` panics
- **Manual uniform updates**: Border/shadow params require calling `update_inner()` after changes
- **Unsafe GLES calls**: `shader_element.rs` uses raw GL FFI - coordinate system mismatches cause visual glitches
