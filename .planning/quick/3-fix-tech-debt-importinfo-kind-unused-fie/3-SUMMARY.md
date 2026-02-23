---
phase: quick-3
plan: 01
subsystem: tech-debt
tags: [clippy, dead-code, watcher, import-stats]

requires:
  - phase: none
    provides: n/a
provides:
  - "WatchEvent enum cleaned to 3 variants (no dead Created)"
  - "IndexStats includes ESM/CJS/dynamic import breakdown"
  - "ImportKind derives Hash for map key usage"
affects: [watcher, output, index-command]

tech-stack:
  added: []
  patterns:
    - "Import kind breakdown in IndexStats for richer CLI output"

key-files:
  created: []
  modified:
    - src/watcher/event.rs
    - src/watcher/incremental.rs
    - src/watcher/mod.rs
    - src/main.rs
    - src/parser/imports.rs
    - src/output.rs

key-decisions:
  - "Removed WatchEvent::Created entirely rather than keeping as alias -- notify-debouncer-mini cannot distinguish create vs modify"
  - "Added Hash derive to ImportKind to support future HashMap-based counting if needed"

patterns-established: []

requirements-completed: [QUICK-3]

duration: 3min
completed: 2026-02-23
---

# Quick Task 3: Fix Tech Debt Summary

**Removed dead WatchEvent::Created variant and surfaced ImportInfo.kind in IndexStats with ESM/CJS/dynamic breakdown**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-23T16:29:51Z
- **Completed:** 2026-02-23T16:33:09Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- Removed the never-constructed `WatchEvent::Created` variant and all its match arms across 4 files
- Surfaced `ImportInfo.kind` in production code by adding ESM/CJS/dynamic import counters to IndexStats
- Eliminated both `#[allow(dead_code)]` annotations that were suppressing clippy warnings
- Both human-readable and JSON output now include import kind breakdown

## Task Commits

Each task was committed atomically:

1. **Task 1: Remove WatchEvent::Created variant** - `03ec662` (fix)
2. **Task 2: Surface ImportInfo.kind in IndexStats output** - `6447c05` (feat)

## Files Created/Modified
- `src/watcher/event.rs` - Removed Created variant from WatchEvent enum
- `src/watcher/incremental.rs` - Simplified match to Modified-only (no Created)
- `src/watcher/mod.rs` - Updated comments to reflect 3-variant enum
- `src/main.rs` - Added import kind counting loop and new IndexStats fields
- `src/parser/imports.rs` - Added Hash derive to ImportKind, removed dead_code annotation
- `src/output.rs` - Added esm/cjs/dynamic fields to IndexStats and print_summary

## Decisions Made
- Removed WatchEvent::Created entirely rather than keeping as an alias -- notify-debouncer-mini cannot distinguish create vs modify, so the variant was never constructed
- Added Hash derive to ImportKind to enable HashMap key usage if needed in future

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All `#[allow(dead_code)]` annotations on these items eliminated
- Stricter clippy enforcement now possible
- Import kind stats available in CLI output for users

---
*Quick Task: 3-fix-tech-debt-importinfo-kind-unused-fie*
*Completed: 2026-02-23*
