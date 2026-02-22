---
phase: 02-import-resolution-graph-completion
plan: "03"
subsystem: resolver
tags: [rust, oxc_resolver, petgraph, barrel-resolution, symbol-relationships, integration]
dependency_graph:
  requires:
    - 02-01-SUMMARY.md  # Graph types, resolver infrastructure, workspace detection
    - 02-02-SUMMARY.md  # Relationship extraction (RelationshipInfo, RelationshipKind)
  provides:
    - resolve_all() orchestrator in src/resolver/mod.rs
    - ResolveStats struct with resolved/unresolved/external/builtin/relationships counts
    - src/resolver/barrel.rs with resolve_barrel_chains() and BarrelReExportAll edge wiring
    - Updated src/main.rs with full two-pass pipeline (parse then resolve)
    - Updated src/output.rs IndexStats with 5 resolution fields
  affects:
    - src/main.rs (parse loop now retains ParseResults for resolution pass)
    - src/output.rs (IndexStats extended, print_summary updated)
tech_stack:
  added: []
  patterns:
    - Two-pass architecture: parse all files first, then resolve (enables cross-file lookup)
    - Single Resolver instance reused for all files (anti-pattern warning from research)
    - External package classification by specifier prefix (no . or /)
    - Scoped npm package name extraction (@scope/name vs name/subpath)
    - Barrel resolution via parse_results key lookup (no second resolver call needed)
    - petgraph::visit::EdgeRef trait required for .target() on EdgeReference
key_files:
  created:
    - src/resolver/barrel.rs
  modified:
    - src/resolver/mod.rs
    - src/main.rs
    - src/output.rs
decisions:
  - "Barrel pass uses parse_results HashMap lookup instead of second oxc_resolver call — faster and avoids resolver API complexity for files already indexed"
  - "External package classification based on specifier prefix (not . and not /) — correct heuristic since workspace aliases are handled upstream by resolver"
  - "Symbol relationship pass skips ambiguous multi-candidate calls (undocumented limitation: cross-file call ambiguity per research Open Question 3)"
  - "BarrelReExportAll path stored with canonical join result — may include ./ in output (cosmetic, no functional impact)"
metrics:
  duration: "38 min"
  completed: "2026-02-22"
  tasks_completed: 2
  files_modified: 4
---

# Phase 02 Plan 03: Full Resolution Pipeline Integration Summary

**One-liner:** End-to-end import resolution pipeline wiring oxc_resolver + workspace detection + barrel chain traversal + symbol relationship edges into the `code-graph index` command with resolution metrics output.

## What Was Built

### src/resolver/barrel.rs (new)

- `resolve_barrel_chains(graph, parse_results, verbose)` — iterates all `ExportKind::ReExportAll` exports across all parsed files
- For each `export * from './x'`, resolves the source specifier via extension-probing against `parse_results` keys and adds a `BarrelReExportAll` edge from the barrel file node to the source file node
- Best-effort: skips gracefully when source file not in graph (external barrel, unindexed file)
- `resolve_relative_specifier()` helper probes `.ts/.tsx/.js/.jsx/.mts/.mjs` extensions and `index.*` directory patterns
- 3 unit tests: edge added for ReExportAll, no edge for named re-exports, graceful skip for missing source

### src/resolver/mod.rs (updated)

- `resolve_all(graph, project_root, parse_results, verbose) -> ResolveStats` — 5-step pipeline:
  1. Workspace detection via `discover_workspace_packages()`
  2. Single `Resolver` instance via `build_resolver()` with workspace aliases
  3. File-level resolution: for every import, call `resolve_import()` and classify as Resolved/External/Builtin/Unresolved
  4. Barrel chain pass: `barrel::resolve_barrel_chains()`
  5. Symbol relationship pass: wire Extends/Implements/InterfaceExtends/Calls/TypeReference edges from `RelationshipInfo` data
- `ResolveStats` struct with 5 counters
- `is_external_package()` and `extract_package_name()` helpers for npm package classification
- 2 unit tests for helper functions

### src/main.rs (updated)

- Parse loop now stores each `ParseResult` in a `HashMap<PathBuf, ParseResult>` for the resolve step
- After parse loop: calls `resolver::resolve_all(&mut graph, &path, &parse_results, verbose)`
- Verbose mode logs resolution summary to stderr
- `IndexStats` construction populated with `ResolveStats` fields

### src/output.rs (updated)

- `IndexStats` struct extended with 5 new fields: `resolved_imports`, `unresolved_imports`, `external_packages`, `builtin_modules`, `relationship_edges`
- `print_summary()` adds "Resolution" section to human-readable output
- JSON output automatically includes all new fields via `#[derive(Serialize)]`

## Verification Results

- `cargo build`: clean compile (warnings only — pre-existing dead code awaiting Phase 3+ usage)
- `cargo test`: 53/53 passing (48 pre-existing + 3 barrel + 2 helper)
- `cargo run -- index /workspace`: 0 files (Rust project), completes cleanly
- `cargo run -- index /tmp/ts-fixture`: 3 files, 1 resolved, 1 external, 1 builtin, 1 BarrelReExportAll edge
- `cargo run -- index /tmp/ts-fixture --json`: all 5 resolution fields present in JSON output
- Verbose mode shows `barrel: /path/index.ts --[BarrelReExportAll]--> /path/utils.ts`

## Task Commits

1. **Task 1: Implement resolve_all orchestrator and barrel chain resolution** - `244aa27` (feat)
2. **Task 2: Integrate resolution pipeline into index command and update output** - `d221cdd` (feat)

## Files Created/Modified

- `src/resolver/barrel.rs` - Barrel chain resolution with BarrelReExportAll edge wiring
- `src/resolver/mod.rs` - resolve_all() orchestrator, ResolveStats, helper functions
- `src/main.rs` - Two-pass pipeline integration with parse_results HashMap
- `src/output.rs` - Extended IndexStats with resolution metrics, updated print_summary

## Decisions Made

- Barrel pass uses parse_results HashMap lookup instead of second oxc_resolver call: the resolver resolved imports during the file-level pass; for barrel ReExportAll we only need to check if the referenced source file was also indexed, which parse_results keys directly answer without another resolver invocation.
- External package classification based on specifier prefix: specifiers not starting with `.` or `/` are external packages. This correctly handles npm packages, scoped packages, and subpath imports while workspace aliases (which would otherwise look external) are handled upstream by the resolver itself returning a Resolved path.
- Symbol relationship pass skips ambiguous multi-candidate calls: when `to_name` matches multiple symbols in the index (same function name defined in multiple files), we skip the edge rather than creating spurious cross-file edges. This is the documented limitation from research (Open Question 3).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] petgraph EdgeRef trait not in scope for .target() calls**

- **Found during:** Task 1 (barrel.rs test compilation) and Task 1 (mod.rs implementation)
- **Issue:** The `petgraph::stable_graph::EdgeReference` type implements `EdgeRef` trait for `.target()` but the trait must be explicitly imported. Without `use petgraph::visit::EdgeRef`, calling `.target()` fails with E0599.
- **Fix:** Added `use petgraph::visit::EdgeRef;` to both `src/resolver/barrel.rs` (test module) and `src/resolver/mod.rs` (main implementation)
- **Files modified:** src/resolver/barrel.rs, src/resolver/mod.rs
- **Verification:** All 53 tests pass; cargo build clean

**2. [Rule 1 - Bug] Unused imports in barrel.rs (HashSet, ExportInfo)**

- **Found during:** Task 1 (cargo build warning cleanup)
- **Issue:** Initial barrel.rs imported `HashSet` (not needed — cycle detection approach shifted to per-file HashSet not needed in current design) and `ExportInfo` (only `ExportKind` is needed in the function signature).
- **Fix:** Removed unused imports
- **Files modified:** src/resolver/barrel.rs
- **Verification:** cargo build produces no unused import warnings for barrel.rs

---

**Total deviations:** 2 auto-fixed (1 bug × 2 occurrences, 1 cleanup)
**Impact on plan:** Both fixes trivial — trait import and dead import cleanup. No scope changes.

## Phase 2 Completion Status

After this plan, the Phase 2 requirements are fulfilled:
- **PARS-05**: Path alias imports resolved via tsconfig through oxc_resolver
- **PARS-06**: Barrel file `export * from` recorded as BarrelReExportAll edges (lazy resolution)
- **PARS-07**: Workspace package imports resolve to local source directories via workspace_map_to_aliases
- **PARS-08**: Every import has a graph edge (ResolvedImport, ExternalPackage, or UnresolvedImport)
- **PARS-09**: Symbol-level relationships (Calls, Extends, Implements) wired from RelationshipInfo data

## Self-Check: PASSED

**Files created:**
- `/workspace/src/resolver/barrel.rs` — FOUND
- `/workspace/src/resolver/mod.rs` — FOUND (modified)
- `/workspace/src/main.rs` — FOUND (modified)
- `/workspace/src/output.rs` — FOUND (modified)

**Commits:**
- `244aa27` — FOUND (feat(02-03): implement resolve_all orchestrator...)
- `d221cdd` — FOUND (feat(02-03): integrate resolution pipeline...)

**Tests:** 53/53 passing
**Build:** Clean (warnings only)
