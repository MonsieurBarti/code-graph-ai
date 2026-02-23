---
phase: 03-query-engine-cli
plan: "03"
subsystem: query-engine
tags: [cli, query, context, graph-walk, edges, callers, callees, extends, implements]
dependency_graph:
  requires: [graph/mod.rs, graph/node.rs, graph/edge.rs, query/find.rs, query/refs.rs, query/output.rs, src/cli.rs, src/main.rs]
  provides: [symbol_context(), SymbolContext, CallInfo, format_context_results(), context CLI command]
  affects: [src/query/mod.rs, src/query/output.rs, src/query/find.rs, src/main.rs]
tech_stack:
  added: []
  patterns: [bidirectional edge walk for caller/callee extraction, Extends/Implements edge traversal in both directions, deduplication via HashSet keyed on (name, file, line)]
key_files:
  created:
    - src/query/context.rs
  modified:
    - src/query/mod.rs
    - src/query/output.rs
    - src/query/find.rs
    - src/main.rs
decisions:
  - "symbol_context() walks both symbol-to-symbol Calls edges AND file-to-symbol Calls edges from the symbol's parent file — required because Phase 2 resolver emits file-level Calls for unscoped call resolution"
  - "FindResult needed Clone derive to be stored in SymbolContext.definitions Vec<FindResult> — added #[derive(Clone)] to FindResult"
  - "One SymbolContext per matched symbol name — allows regex patterns matching multiple symbols to return multiple context blocks"
  - "Sections only rendered when non-empty in all three output modes — no empty 'Calls (0):' noise in table/compact output"
metrics:
  duration_min: 127
  completed_date: "2026-02-23"
  tasks_completed: 1
  files_changed: 5
---

# Phase 3 Plan 03: context command — 360-degree symbol view

**One-liner:** symbol_context() combining definition lookup, find_refs() for references, and bidirectional Calls/Extends/Implements edge walks for callers/callees/inheritance, wired as the final Context CLI subcommand completing all 7 Phase 3 commands.

## What Was Built

### Task 1: Implement symbol_context, output formatter, and wire Context command

**src/query/context.rs** (new file):
- `CallInfo` struct: symbol_name, kind, file_path, line — used for callers, callees, extends, implements entries
- `SymbolContext` struct: symbol_name + definitions (Vec<FindResult>) + references (Vec<RefResult>) + callees + callers + extends + implements + extended_by + implemented_by
- `symbol_context()`: builds 360-degree view in one pass:
  - **Definitions**: iterates symbol_indices, finds parent file via Contains edge walk, constructs FindResult entries; deduplicates by (file, line)
  - **References**: delegates to `find_refs()` (reuses refs.rs logic — import refs via ResolvedImport edges + call refs via incoming Calls edges)
  - **Callers**: walks incoming Calls edges to each symbol node; builds CallInfo from caller Symbol nodes
  - **Callees**: walks outgoing Calls from symbol node (symbol-to-symbol) AND outgoing Calls from the symbol's parent file node (file-level Calls from Phase 2 resolver); deduplicates by (name, file, line)
  - **Extends/Implements**: outgoing Extends/Implements edges for extends/implements lists; incoming for extended_by/implemented_by
- Private helpers: `find_containing_file()`, `find_containing_file_idx()`, `build_call_info()` — follow Contains/ChildOf edge chains
- 5 unit tests: callers detection, callees detection (symbol-to-symbol Calls), extends bidirectional, implements bidirectional, empty graph

**src/query/output.rs** (extended):
- `format_context_results()`: formats `&[SymbolContext]` in three modes:
  - **Compact**: prefixed lines (`symbol`, `def`, `ref`, `calls`, `called-by`, `extends`, `implements`, `extended-by`, `implemented-by`); sections omitted when empty; summary line `N refs, M callers, K callees`
  - **Table**: section-per-group layout with bold headers, empty sections omitted, kind label in title line
  - **JSON**: full structured object per symbol with all eight relationship arrays

**src/query/find.rs** (extended):
- Added `#[derive(Clone)]` to `FindResult` — required for storage in `SymbolContext.definitions`

**src/query/mod.rs** (extended):
- Added `pub mod context;` declaration

**src/main.rs** (wired):
- `Commands::Context`: validates regex, builds graph, calls `match_symbols()`, maps to `symbol_context()` per name, calls `format_context_results()`; exits with stderr message if no symbols matched
- Removed `todo!("Phase 3 Plan 03")` placeholder — all 6 query commands now fully wired

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] FindResult missing Clone derive**
- **Found during:** Task 1, first cargo build
- **Issue:** `SymbolContext` derives `Clone` and contains `Vec<FindResult>`, but `FindResult` in find.rs did not implement `Clone` — E0277 compile error
- **Fix:** Added `#[derive(Clone)]` to `FindResult` in find.rs
- **Files modified:** src/query/find.rs
- **Commit:** 3f72daf (same task commit)

## Test Results

- **Original tests:** 82 passed
- **New unit tests added:** 5 (context module)
- **Total:** 87 passed, 0 failed

## Verification

All plan verification criteria confirmed:

1. `cargo build` — clean compilation (pre-existing warnings only)
2. `cargo test` — 87 tests pass
3. `cargo run -- context UserService /tmp/test-p3-ctx` — shows definition, import ref, and summary
4. `cargo run -- context UserService /tmp/test-p3-ctx --format json` — structured JSON with all sections
5. `cargo run -- context UserService /tmp/test-p3-ctx --format table` — human-readable sections
6. All 7 subcommands: index, find, refs, impact, circular, stats, context — all produce correct output
7. Compact output: prefixed lines, relative paths, no decoration — token-optimized
8. No `todo!()` macros in main.rs or anywhere in src/

## Commits

| Task | Commit | Description |
|------|--------|-------------|
| Task 1 | 3f72daf | feat(03-03): implement context command — 360-degree symbol view |

## Self-Check: PASSED

- src/query/context.rs: FOUND
- src/query/mod.rs: FOUND (pub mod context added)
- src/query/output.rs: FOUND (format_context_results added)
- src/query/find.rs: FOUND (Clone derive added to FindResult)
- src/main.rs: FOUND (Context match arm wired, todo! removed)
- .planning/phases/03-query-engine-cli/03-03-SUMMARY.md: FOUND
- Commit 3f72daf: FOUND
