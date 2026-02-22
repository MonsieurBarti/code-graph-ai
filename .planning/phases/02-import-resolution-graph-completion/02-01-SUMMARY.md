---
phase: 02-import-resolution-graph-completion
plan: 01
subsystem: graph
tags: [rust, petgraph, oxc_resolver, graph-types, workspace-detection, monorepo, typescript-resolution]

# Dependency graph
requires:
  - phase: 01-foundation-core-parsing
    provides: CodeGraph struct with file/symbol nodes, EdgeKind enum, GraphNode enum, Cargo.toml with petgraph

provides:
  - Extended EdgeKind enum with ResolvedImport, Calls, Extends, Implements, BarrelReExportAll variants
  - Extended GraphNode enum with ExternalPackage and UnresolvedImport variants
  - CodeGraph helper methods for all new Phase 2 edge/node types with deduplication
  - oxc_resolver dependency with TypeScript-aware configuration wrapper
  - Workspace package detection for npm/yarn/pnpm monorepos
  - src/resolver/ module with file_resolver.rs and workspace.rs

affects:
  - 02-02 (import resolution pipeline will consume build_resolver and discover_workspace_packages)
  - 02-03 (barrel chain traversal uses BarrelReExportAll edge)
  - 02-04 (symbol relationship extraction uses Calls/Extends/Implements edges)

# Tech tracking
tech-stack:
  added:
    - oxc_resolver 3.0.3 (TypeScript-aware import resolution, tsconfig paths, project references)
  patterns:
    - Resolver constructed once at pipeline start and reused across all files (single-instance pattern)
    - ExternalPackage nodes deduplicated via external_index HashMap on CodeGraph
    - Minimal YAML line parser for pnpm-workspace.yaml (no serde_yaml dependency)
    - ResolutionOutcome enum discriminates Resolved / BuiltinModule / Unresolved outcomes

key-files:
  created:
    - src/resolver/mod.rs
    - src/resolver/file_resolver.rs
    - src/resolver/workspace.rs
  modified:
    - src/graph/edge.rs
    - src/graph/node.rs
    - src/graph/mod.rs
    - Cargo.toml
    - src/main.rs

key-decisions:
  - "oxc_resolver = 3 (edition 2021 compatible) chosen over 11.x (requires edition 2024 which was unstable on Rust 1.84)"
  - "Rust toolchain upgraded from 1.84.1 to stable 1.93.1 which supports edition 2024"
  - "pnpm-workspace.yaml parsed with minimal line parser — no serde_yaml added; 10-line parser covers 95% of real projects"
  - "ExternalPackage nodes deduplicated by package name via external_index HashMap on CodeGraph"
  - "builtin_modules: true enabled on resolver so Node.js builtins trigger ResolveError::Builtin for proper classification"
  - "workspace_aliases fed into ResolveOptions::alias so workspace package names bypass node_modules lookup"

patterns-established:
  - "ResolutionOutcome enum: typed discriminated union for resolve results (Resolved / BuiltinModule / Unresolved)"
  - "Workspace source-preferred mapping: src/ dir used when exists, otherwise package root"
  - "Graph helper methods follow existing add_file/add_symbol pattern: single method adds node + edge atomically"

requirements-completed: [PARS-05, PARS-07, PARS-08]

# Metrics
duration: 5min
completed: 2026-02-22
---

# Phase 02 Plan 01: Graph Type Extensions and Resolver Infrastructure Summary

**oxc_resolver wrapper + workspace detection infrastructure for TypeScript-aware import resolution with extended CodeGraph edge/node types**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-22T10:13:38Z
- **Completed:** 2026-02-22T10:18:51Z
- **Tasks:** 2
- **Files modified:** 8

## Accomplishments

- Extended CodeGraph with 5 new EdgeKind variants (ResolvedImport, Calls, Extends, Implements, BarrelReExportAll) and 2 new GraphNode variants (ExternalPackage, UnresolvedImport) with helper methods for each
- Built oxc_resolver wrapper (src/resolver/file_resolver.rs) with TypeScript-first extension order, .js-to-.ts alias, tsconfig project reference auto-discovery, and typed ResolutionOutcome enum
- Built workspace package detection (src/resolver/workspace.rs) for npm/yarn/pnpm monorepos with minimal YAML parser, glob expansion, and src/-preferred mapping
- All 36 tests pass: 24 Phase 1 tests (no regressions) + 3 new graph tests + 9 new resolver/workspace tests

## Task Commits

1. **Task 1: Extend graph types with Phase 2 edge and node variants** - `f094ffa` (feat)
2. **Task 2: Create resolver module with workspace detection and oxc_resolver wrapper** - `60cb88d` (feat)

## Files Created/Modified

- `src/graph/edge.rs` - Added ResolvedImport, Calls, Extends, Implements, BarrelReExportAll variants to EdgeKind
- `src/graph/node.rs` - Added ExternalPackageInfo struct and ExternalPackage, UnresolvedImport variants to GraphNode
- `src/graph/mod.rs` - Added external_index field and 7 new helper methods to CodeGraph, plus 3 unit tests
- `src/resolver/mod.rs` - Module declarations and public re-exports for resolver infrastructure
- `src/resolver/file_resolver.rs` - build_resolver(), resolve_import(), workspace_map_to_aliases(), ResolutionOutcome enum
- `src/resolver/workspace.rs` - discover_workspace_packages(), read_workspace_globs(), parse_pnpm_workspace_yaml()
- `Cargo.toml` - Added oxc_resolver = "3"
- `src/main.rs` - Added mod resolver declaration

## Decisions Made

- Rust toolchain upgraded from 1.84.1 to stable 1.93.1: Cargo 1.84 did not support the project's `edition = "2024"` setting, causing build failures. Upgrading to 1.93.1 (latest stable) resolved this without changing any project code.
- `oxc_resolver = "3"` selected: The research identified 3.x as the latest edition-2021-compatible version. Cargo resolved 3.0.3. The 11.x series requires features our project isn't using.
- Minimal YAML parser for pnpm-workspace.yaml: serde_yaml not added — the pnpm format is a simple list under `packages:` that a 20-line parser handles completely.
- `builtin_modules: true` added to resolver options: this was not in the original plan spec but is required for `ResolveError::Builtin` to be returned so builtins are classified correctly by `resolve_import()`.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Enabled builtin_modules on Resolver to match documented ResolutionOutcome::BuiltinModule behavior**

- **Found during:** Task 2 (file_resolver.rs implementation)
- **Issue:** The plan's ResolutionOutcome enum includes BuiltinModule variant, but oxc_resolver only returns ResolveError::Builtin when ResolveOptions::builtin_modules is set to true. Without it, Node.js builtins (e.g. "fs") would fall through to Unresolved instead of BuiltinModule.
- **Fix:** Added `builtin_modules: true` to ResolveOptions in build_resolver()
- **Files modified:** src/resolver/file_resolver.rs
- **Verification:** Resolver construction test passes; behavior confirmed via oxc_resolver source inspection
- **Committed in:** 60cb88d (Task 2 commit)

**2. [Rule 3 - Blocking] Upgraded Rust toolchain from 1.84.1 to stable 1.93.1**

- **Found during:** Initial build attempt (pre-Task 1)
- **Issue:** Cargo 1.84.1 does not support `edition = "2024"` in Cargo.toml (feature not stabilized in that version). All builds failed.
- **Fix:** Ran `rustup default stable` which installed Rust 1.93.1 (stable) with full edition 2024 support.
- **Files modified:** None (toolchain change only)
- **Verification:** `cargo build` succeeds; all 36 tests pass
- **Committed in:** Not committed (environment setup, not code change)

---

**Total deviations:** 2 auto-fixed (1 bug, 1 blocking)
**Impact on plan:** Both fixes necessary for correctness and build success. No scope creep.

## Issues Encountered

None beyond the deviations documented above.

## Next Phase Readiness

- Graph types and resolver infrastructure are complete — Plan 02-02 can wire the resolution pipeline into the index command
- build_resolver() and discover_workspace_packages() are ready for consumption; src/main.rs has mod resolver declared
- No blockers

---
*Phase: 02-import-resolution-graph-completion*
*Completed: 2026-02-22*
