---
phase: 02-import-resolution-graph-completion
plan: "04"
subsystem: resolver
tags: [rust, petgraph, barrel-files, named-reexport, graph-resolution, tree-sitter]

# Dependency graph
requires:
  - phase: 02-03
    provides: resolve_all() orchestrator and file-level ResolvedImport edges that this plan enriches

provides:
  - resolve_named_reexport_chains() in barrel.rs: chases ExportKind::ReExport entries to defining files and adds direct ResolvedImport edges
  - named_reexport_edges field on ResolveStats for diagnostic tracking
  - Cycle detection for circular named re-export chains
  - 4 unit tests for named re-export chain resolution

affects: [phase-03, phase-04, query-layer]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Named re-export map: barrel_path -> vec<(names, source_path)> built from parse_results before graph mutation"
    - "Collect-then-mutate: candidates collected from graph edges first, then graph mutated in second pass"
    - "Cycle detection: per-chain HashSet<PathBuf> visited set in recursive chase function"
    - "Dedup guard: check existing ResolvedImport edge before adding to avoid duplicates"

key-files:
  created: []
  modified:
    - src/resolver/barrel.rs
    - src/resolver/mod.rs

key-decisions:
  - "Named re-export map built as pre-pass from parse_results before scanning edges — avoids complex borrow issues"
  - "Collect candidates (importer, barrel, specifier) into vec before mutating graph — required by Rust borrow checker"
  - "Chase algorithm uses recursive inner function with mutable visited set passed by &mut reference"
  - "No dedup of edges added per name — dedup done via 'already_exists' check on graph before add_resolved_import"
  - "named_reexport_edges field added to ResolveStats for verbose diagnostics only; not surfaced in user-facing IndexStats output"
  - "ImportSpecifier.alias holds original exported name when aliased (import { Foo as F }) — used to match barrel re-export names"

patterns-established:
  - "Barrel chain passes: resolve_barrel_chains() then resolve_named_reexport_chains() as Step 4 + 4b in resolve_all()"

requirements-completed: [PARS-05, PARS-06, PARS-07, PARS-08, PARS-09]

# Metrics
duration: 25min
completed: 2026-02-22
---

# Phase 2 Plan 04: Named Re-Export Barrel Chain Resolution Summary

**Named re-export chasing via resolve_named_reexport_chains() that adds direct ResolvedImport edges from importers to defining files, closing the PARS-06 gap identified in 02-VERIFICATION.md**

## Performance

- **Duration:** 25 min
- **Started:** 2026-02-22T22:11:00Z
- **Completed:** 2026-02-22T22:36:38Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments

- Implemented `resolve_named_reexport_chains()` in `src/resolver/barrel.rs` that chases `ExportKind::ReExport` entries to the defining file, adding a direct `ResolvedImport` edge bypassing the barrel
- Wired the new function into `resolve_all()` as Step 4b immediately after the existing `resolve_barrel_chains()` call
- Added `named_reexport_edges` field to `ResolveStats` for verbose diagnostic output
- 4 unit tests: single-level chain, multi-level chain, cycle detection (no infinite loop), name-not-found (no spurious edge)
- All 57 tests pass (53 pre-existing + 4 new); end-to-end fixture confirms `import { UserService } from './services'` resolves through `services/index.ts` to `UserService.ts`

## Task Commits

1. **Task 1: Implement resolve_named_reexport_chains() in barrel.rs** - `074d9c0` (feat)
2. **Task 2: Wire named re-export resolution into resolve_all pipeline** - `e3d671a` (feat)

## Files Created/Modified

- `src/resolver/barrel.rs` — Added `resolve_named_reexport_chains()` public function, `chase_named_reexport()` and `chase_named_reexport_inner()` private helpers, `HashSet` and `petgraph::visit::EdgeRef` imports, and 4 new unit tests
- `src/resolver/mod.rs` — Added `named_reexport_edges: usize` to `ResolveStats`, added Step 4b call in `resolve_all()` with verbose logging

## Decisions Made

- **Collect-then-mutate pattern:** Rust borrow checker prevents holding an iterator over graph edges while mutating the graph. Collected candidates into `Vec<(PathBuf, PathBuf, String)>` first, then mutated graph in a second pass.
- **Named re-export map as pre-pass:** Built `barrel_reexports: HashMap<PathBuf, Vec<(Vec<String>, PathBuf)>>` from `parse_results` before touching the graph. Simpler and avoids combined mutable/immutable borrow issues.
- **Recursive chase with mutable visited set:** `chase_named_reexport()` calls `chase_named_reexport_inner()` recursively, threading a `&mut HashSet<PathBuf>` to detect cycles across arbitrarily deep chains.
- **Dedup before add:** Before calling `graph.add_resolved_import()`, check if a `ResolvedImport` edge already exists from importer to defining file — prevents adding duplicate edges if the pipeline is called multiple times or if oxc_resolver already resolved directly.
- **Alias handling:** For `import { Foo as F } from '...'`, the original exported name is in `specifier.alias` (type `Option<String>`). Use `alias.as_deref().unwrap_or(&name)` to get the name that must match the barrel's `ExportInfo.names`.
- **`named_reexport_edges` is diagnostic only:** The field is added to `ResolveStats` for internal verbose logging; it is not propagated to `IndexStats` or user-facing output since the edges are already counted in `resolved_imports`.

## Deviations from Plan

None — plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None — no external service configuration required.

## Next Phase Readiness

- PARS-05 through PARS-09 requirements satisfied. Named barrel re-export resolution is fully operational.
- Phase 2 gap identified in 02-VERIFICATION.md (Success Criterion 2 / PARS-06) is now closed.
- The graph now contains direct `ResolvedImport` edges from importers to defining files, making it possible for query-layer (Phase 3) to trace transitive dependencies through barrel files without needing to expand `BarrelReExportAll` edges at query time for named re-exports.
- Remaining limitation: alias re-exports (`export { Foo as Bar } from './module'`) where the barrel renames the export are not yet chased — the name matching would fail since `ExportInfo.names` stores the original name and we look for it by the alias. This is an edge case not covered by current requirements.

---
*Phase: 02-import-resolution-graph-completion*
*Completed: 2026-02-22*
