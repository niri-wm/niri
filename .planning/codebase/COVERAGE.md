# Documentation Coverage

**Analysis Date:** 2026-02-16

## Existing Documentation

### Project-Level
| Document | Status | Location |
|----------|--------|----------|
| README.md | ✅ Complete | Root - Features, getting started, media |
| AGENTS.md | ✅ Complete | Root - Overview, structure, where to look |
| CONTRIBUTING.md | ✅ Complete | Root - PR process, testing, review guidelines |
| LICENSE | ✅ Complete | Root - GPL-3.0-or-later |

### Module-Level (AGENTS.md files)
| Module | Status | Location |
|--------|--------|----------|
| Layout | ✅ Complete | `src/layout/AGENTS.md` |
| Input | ✅ Complete | `src/input/AGENTS.md` |
| Animation | ✅ Complete | `src/animation/AGENTS.md` |
| Backend | ✅ Complete | `src/backend/AGENTS.md` |
| Render Helpers | ✅ Complete | `src/render_helpers/AGENTS.md` |
| Config | ✅ Complete | `niri-config/AGENTS.md` |
| IPC | ✅ Complete | `niri-ipc/AGENTS.md` |
| Tests | ✅ Complete | `src/tests/AGENTS.md` |

### Configuration Files
| File | Status | Contents |
|------|--------|----------|
| `rustfmt.toml` | ✅ Complete | 5 lines - formatting config |
| `clippy.toml` | ✅ Complete | 6 lines - linting config |
| `Cargo.toml` | ✅ Complete | 188 lines - dependencies, features |
| `typos.toml` | ✅ Present | Spell checking config |
| `flake.nix` | ✅ Complete | Nix flake for development |

### External Documentation
| Resource | Status | Location |
|----------|--------|----------|
| Wiki | ✅ Complete | https://niri-wm.github.io/niri/ |
| Issues | ✅ Active | GitHub issues |
| Discussions | ✅ Active | GitHub discussions |
| Matrix | ✅ Active | #niri:matrix.org |

## This Analysis Documents

| Document | Status | Location |
|----------|--------|----------|
| ARCHITECTURE.md | ✅ Complete | `.planning/codebase/ARCHITECTURE.md` |
| STACK.md | ✅ Complete | `.planning/codebase/STACK.md` |
| PATTERNS.md | ✅ Complete | `.planning/codebase/PATTERNS.md` |
| COVERAGE.md | ✅ Complete | `.planning/codebase/COVERAGE.md` |

## Areas Needing Further Investigation

### Deep Dive Needed
1. **Wayland Protocol Details** - Protocol implementation patterns in `handlers/`
2. **Smithay Integration** - How niri uses Smithay's desktop/rendering APIs
3. **Render Pipeline** - Detailed GPU rendering flow, damage tracking
4. **Input State Machine** - Complex grab handlers, gesture recognition

### Test Coverage Details
- Property-based testing approach in `src/layout/tests.rs`
- Snapshot testing patterns (5280+ snapshots)
- Integration test infrastructure in `src/tests/`

### Configuration System
- KDL parsing internals in `niri-config/`
- Window rule matching
- Animation configuration

### IPC Protocol
- Version compatibility handling
- Event subscription model

## What's Well Documented

1. **Project Structure** - Clear module boundaries, AGENTS.md in each
2. **Key Algorithms** - Scrollable tiling, dynamic workspaces
3. **Contributing Process** - Detailed CONTRIBUTING.md
4. **Configuration** - KDL format, options in wiki
5. **Testing Strategy** - Property-based, snapshot, integration

## What's Less Documented

1. **Error Recovery** - How errors propagate, recovery strategies
2. **Performance Characteristics** - Bottlenecks, optimization notes
3. **Debugging Tools** - Tracy integration, debug rendering
4. **Release Process** - Versioning, packaging

## Recommendations for Future Documentation

1. Add architecture decision records (ADRs) for major design choices
2. Document the event loop flow in detail
3. Add debugging cookbook (how to debug common issues)
4. Document the test infrastructure more thoroughly

---

*Coverage analysis: 2026-02-16*
