# Technology Stack

**Analysis Date:** 2026-02-16

## Languages

**Primary:**
- Rust 1.85+ - Main compositor implementation

**Secondary:**
- C (via bindgen) - libinput, DRM headers
- GLSL - Custom rendering shaders

## Runtime

**Environment:**
- Linux with Wayland session
- Direct console (TTY) or nested (Winit)

**Package Manager:**
- Cargo (Rust)
- Lockfile: `Cargo.lock` (present)

## Frameworks

**Core:**
- Smithay (git master) - Wayland compositor framework
  - Backend: DRM, GBM, libinput, EGL
  - Renderer: GLES2/GLES3, multi-GPU
  - Desktop: Space, LayerMap, PopupManager

**Testing:**
- proptest 1.9.0 - Property-based testing
- insta 1.46.0 - Snapshot testing (5280+ snapshots)
- calloop-wayland-source - Wayland event testing

**Build/Dev:**
- cargo +nightly fmt - Code formatting (NOT stable)
- cargo clippy - Linting

## Key Dependencies

**Wayland Compositor:**
- smithay - Core framework
- smithay-drm-extras - DRM utilities
- wayland-backend - Protocol backend
- wayland-scanner - Protocol code generation

**Graphics:**
- glam 0.30.10 - Math library (vec2, mat3, etc.)
- drm-ffi 0.9.0 - DRM bindings

**Input:**
- input 0.9.1 - libinput wrapper
- keyframe 1.1.1 - Animation easing functions

**Configuration:**
- knuffel 3.2.0 - KDL config parsing
- miette 5.10.0 - Error reporting with diagnostics

**Serialization:**
- serde 1.0.228 - Serialization framework
- serde_json 1.0.149 - JSON for IPC

**IPC & Desktop Integration:**
- niri-ipc - Internal IPC types
- niri-config - Config crate
- zbus 5.13.0 - D-Bus (optional)
- pipewire 0.9.2 - Screen capture (optional)

**Utilities:**
- anyhow 1.0.100 - Error handling
- bitflags 2.10.0 - Flag types
- calloop 0.14.3 - Event loop
- tracy-client 0.18.4 - Profiling (optional)

**UI Rendering:**
- pango 0.21.5 - Text rendering
- pangocairo 0.21.5 - Cairo integration
- png 0.18.0 - Screenshot output

## Configuration

**Environment:**
- KDL format (not TOML/YAML)
- Default: `~/.config/niri/config.kdl`
- Live-reloading on config change

**Build:**
- `rustfmt.toml` - 100 char comment width, module imports
- `clippy.toml` - Allow interior mutability for Smithay types
- `Cargo.toml` - Workspace with 4 crates

## Platform Requirements

**Development:**
- Rust 1.85+
- cargo +nightly (for formatting)
- System libraries: libinput, GBM, DRM, EGL

**Production:**
- Linux with DRM/KMS
- libinput-compatible input devices
- GBM-capable GPU (Intel, AMD, NVIDIA)
- Optional: D-Bus (systemd), PipeWire (screencast)

## Additional Crates (Workspace)

**niri-config:**
- knuffel - KDL parsing
- csscolorparser - Color parsing
- regex - Pattern matching
- miette - Error diagnostics

**niri-ipc:**
- serde + serde_json - IPC serialization
- clap - CLI (optional)
- schemars - JSON schema (optional)

**niri-visual-tests:**
- GTK4 - Visual regression testing

---

*Stack analysis: 2026-02-16*
