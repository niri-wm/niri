# VISUAL TESTS

**Scope:** niri-visual-tests/ - GTK visual regression testing

## OVERVIEW

GTK4/Adwaita application for manual visual testing. Renders niri layout/rendering with mock windows.

## STRUCTURE

```
niri-visual-tests/
├── Cargo.toml
└── src/
    └── main.rs
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Visual tests | `src/main.rs` | GTK app |

## CONVENTIONS

- GTK4 + libadwaita
- Not automated - developer inspection only

## ANTI-PATTERNS

- Requires GTK4 runtime
- Manual verification only
