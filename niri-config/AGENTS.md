# NIRI-CONFIG KNOWLEDGE BASE

## OVERVIEW

KDL configuration parsing crate using knuffel derive macros.

## STRUCTURE

```
niri-config/src/
├── lib.rs           # Config struct, file loading, include handling
├── animations.rs    # Animation settings (window, layer, workspace)
├── appearance.rs    # Colors, gradients, border styling
├── binds.rs         # Key bindings and Actions
├── input.rs         # Keyboard, touchpad, mouse, tablet config
├── output.rs        # Monitor modes, transforms, positioning
├── layout.rs        # Tiling layout, gaps, struts
├── window_rule.rs   # Window matching rules
├── layer_rule.rs    # Layer-shell rules
├── gestures.rs      # Touchpad/mouse gesture config
├── workspace.rs     # Workspace definitions
├── utils/           # MergeWith trait, helper types
└── error.rs         # Config parsing errors
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Add config option | `lib.rs` | Add to Config struct + Part type |
| Key bindings | `binds.rs` | Action enum, Modifier flags |
| Animation settings | `animations.rs` | Shader paths, easing curves |
| Colors/gradients | `appearance.rs` | Color type, gradient definitions |
| Input devices | `input.rs` | XkbConfig, device-specific settings |
| Monitor config | `output.rs` | Mode, transform, VRR settings |
| Config merging | `utils/merge_with.rs` | MergeWith trait for includes |

## CONVENTIONS

- **Split types pattern**: Most types have two variants (e.g., `Layout` + `LayoutPart`). The Part type is parsed from one file, then merged into the final type.
- **knuffel derive**: Use `#[knuffel(child)]`, `#[knuffel(argument)]` for KDL structure.
- **Default impls**: Set initial values before parsing; parsing updates with config values.
- **MergeWith trait**: Required for types supporting config includes (`@config` directive).
- **Validation**: Parse-time validation via miette diagnostics.

## ANTI-PATTERNS

- **Skip the Part type**: Always implement both Type and TypePart for includable configs.
- **Direct file I/O in parsers**: Use Decode trait, handle files only in lib.rs.
- **Panics in parsing**: Return DecodeError or miette::Error for user-facing diagnostics.
- **Implicit defaults**: Always use `#[knuffel(default = ...)]` for optional fields.
