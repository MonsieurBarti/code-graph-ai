---
phase: 05-watch-mode-persistence
plan: 02
subsystem: infra
tags: [notify, notify-debouncer-mini, tokio, gitignore, watcher, incremental, petgraph]

# Dependency graph
requires:
  - phase: 05-watch-mode-persistence
    plan: 01
    provides: remove_file_from_graph, serde derives on CodeGraph, cache infrastructure
  - phase: 02-import-resolution
    provides: resolve_import, build_resolver, discover_workspace_packages, workspace_map_to_aliases
  - phase: 01-parser
    provides: parse_file, ParseResult with symbols/imports/relationships
affects: [05-03-watch-loop-integration]

# Tech tracking
tech-stack:
  added: [notify 8, notify-debouncer-mini 0.7]
  patterns:
    - "sync-to-tokio bridge: std mpsc channel + spawn_blocking for OS watcher events into async runtime"
    - "gitignore single source of truth: ignore::gitignore::Gitignore built once at watcher start, same rules as walk_project"
    - "incremental graph update: remove_file_from_graph + re-parse + scoped resolve (not full resolve_all)"

key-files:
  created:
    - src/watcher/mod.rs
    - src/watcher/event.rs
    - src/watcher/incremental.rs
  modified:
    - Cargo.toml
    - src/main.rs

key-decisions:
  - "notify 8 + notify-debouncer-mini 0.7 used — 75ms debounce within locked 50-100ms range"
  - "DebounceEventResult Err variant is single notify::Error (not Vec) — plan template had incorrect for-loop, fixed to direct eprintln"
  - "Rust 2024 edition: explicit ref binding in if-let patterns removed — implicit borrowing used (same fix pattern as 05-01)"
  - "is_external_package/extract_package_name duplicated from resolver::mod.rs (private there) — 15 lines, stable, no visibility change needed"
  - "classify_event uses file existence (path.exists()) to distinguish Modified vs Deleted — notify-debouncer-mini does not distinguish create/modify"
  - "ConfigChanged returns false from handle_file_event — caller is responsible for triggering full rebuild"

patterns-established:
  - "Watcher filter order: node_modules (hardcoded) → .code-graph (hardcoded) → .gitignore rules → config files → source extensions → existence check"
  - "Incremental update: remove_file_from_graph + parse + add_file + add_symbol + scoped resolve + wire_relationships_for_file + fix_unresolved_pointing_to"
  - "fix_unresolved_pointing_to: collect all UnresolvedImport nodes, build resolver once, check each against new file path — O(unresolved * resolver_call)"

requirements-completed: [INTG-04, INTG-05]

# Metrics
duration: 4min
completed: 2026-02-23
---

# Phase 5 Plan 02: File Watcher and Incremental Re-index Pipeline Summary

**Debounced OS watcher (75ms via notify-debouncer-mini) with sync-to-tokio bridge, WatchEvent classification filtering .gitignore/.code-graph/node_modules, and incremental graph update pipeline (remove + re-parse + scoped resolve + dependency fix-up)**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-23T12:53:17Z
- **Completed:** 2026-02-23T12:57:17Z
- **Tasks:** 2
- **Files modified:** 5 (3 created, 2 modified)

## Accomplishments
- Implemented `src/watcher/event.rs` with `WatchEvent` enum: Modified(PathBuf), Created(PathBuf), Deleted(PathBuf), ConfigChanged
- Implemented `src/watcher/mod.rs` with `start_watcher` returning `(WatcherHandle, tokio_mpsc::Receiver<WatchEvent>)` — debounces at 75ms, filters via gitignore + hardcoded exclusions, bridges OS sync events to tokio async channel
- Implemented `src/watcher/incremental.rs` with full incremental re-index: `handle_file_event` dispatches to `handle_modified`/`handle_deleted`, scoped import resolution (not full `resolve_all`), relationship wiring for single file, and `fix_unresolved_pointing_to` to heal previously-unresolved imports when a new file arrives
- All 89 pre-existing tests pass — no regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Watcher infrastructure with notify-debouncer-mini** - `27bc780` (feat)
2. **Task 2: Incremental re-index pipeline** - `550e66f` (feat)

## Files Created/Modified
- `src/watcher/event.rs` - WatchEvent enum with Modified/Created/Deleted/ConfigChanged variants
- `src/watcher/mod.rs` - start_watcher, WatcherHandle, build_gitignore_matcher, classify_event with filter pipeline
- `src/watcher/incremental.rs` - handle_file_event, handle_modified, handle_deleted, wire_relationships_for_file, fix_unresolved_pointing_to
- `Cargo.toml` - Added notify = "8" and notify-debouncer-mini = "0.7"
- `src/main.rs` - Added mod watcher; declaration

## Decisions Made
- Used `notify-debouncer-mini 0.7` for 75ms debounce — simpler API than raw notify, no OS-level event kind discrimination needed
- `classify_event` uses `path.exists()` to distinguish Modified from Deleted — debouncer-mini does not provide create/modify distinction, both treated as Modified (remove + re-add)
- `ConfigChanged` returns `false` from `handle_file_event` so the caller can dispatch a full rebuild without the incremental handler having to know about `build_graph`
- Duplicated `is_external_package` and `extract_package_name` from `resolver::mod.rs` (private there) rather than changing their visibility — 15 lines, no coupling risk

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed DebounceEventResult Err variant type mismatch**
- **Found during:** Task 1 (build verification)
- **Issue:** Plan template iterated `for err in errs` where `errs` was `notify::Error` (not `Vec<notify::Error>`). `DebounceEventResult = Result<Vec<DebouncedEvent>, Error>` — the Err holds a single `Error`
- **Fix:** Changed `for err in errs { eprintln!(...) }` to `eprintln!("[watcher] error: {:?}", err)` with the single error
- **Files modified:** `src/watcher/mod.rs`
- **Verification:** `cargo build` succeeds
- **Committed in:** `27bc780` (Task 1 commit)

**2. [Rule 1 - Bug] Fixed Rust 2024 ref binding incompatibility in incremental.rs**
- **Found during:** Task 2 (build verification)
- **Issue:** Two `if let` patterns used explicit `ref` keyword inside implicitly-borrowing patterns — compile error in Rust 2024 edition (same issue as 05-01)
- **Fix:** Removed `ref` keyword: `EdgeKind::ResolvedImport { ref specifier }` → `{ specifier }` and `GraphNode::UnresolvedImport { ref specifier, ref reason }` → `&graph.graph[idx]` match with `{ specifier, reason }`
- **Files modified:** `src/watcher/incremental.rs`
- **Verification:** `cargo build` succeeds, 89 tests pass
- **Committed in:** `550e66f` (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (2 Rule 1 bugs)
**Impact on plan:** Both fixes required for compilation. Same Rust 2024 pattern established in 05-01 applies here too. No scope creep.

## Issues Encountered
- Plan template code used `for err in errs` iterating over `notify::Error` which is not iterable — checked crate source at `/usr/local/cargo/registry/src/.../notify-debouncer-mini-0.7.0/src/lib.rs` to confirm `DebounceEventResult = Result<Vec<DebouncedEvent>, Error>` type alias

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- `start_watcher` provides `WatcherHandle + tokio_mpsc::Receiver<WatchEvent>` — ready for Plan 03 integration into the watch-mode event loop
- `handle_file_event` provides the incremental update primitive — Plan 03 calls this on each received event
- `WatchEvent::ConfigChanged` signals Plan 03 to invoke `build_graph` for full rebuild
- No blockers or concerns

## Self-Check: PASSED

- src/watcher/mod.rs: FOUND
- src/watcher/event.rs: FOUND
- src/watcher/incremental.rs: FOUND
- 05-02-SUMMARY.md: FOUND
- Commit 27bc780: FOUND
- Commit 550e66f: FOUND
- cargo build: SUCCESS (0 errors, warnings only)
- cargo test: 89/89 PASS

---
*Phase: 05-watch-mode-persistence*
*Completed: 2026-02-23*
