---
phase: 05-watch-mode-persistence
plan: 01
subsystem: database
tags: [petgraph, bincode, serde, tempfile, cache, serialization, persistence]

# Dependency graph
requires:
  - phase: 04-mcp-integration
    provides: CodeGraph struct and all graph types (node.rs, edge.rs, mod.rs)
provides:
  - Serde derives on all graph types enabling bincode serialization
  - Clone derive on CodeGraph for cache envelope
  - remove_file_from_graph method for incremental watcher updates
  - CacheEnvelope with atomic save_cache/load_cache via bincode + tempfile rename
  - .code-graph/graph.bin persistence layer with version checking
affects: [05-02-file-watcher, 05-03-incremental-reindex]

# Tech tracking
tech-stack:
  added: [bincode 2 (serde feature), tempfile 3, petgraph serde-1 feature]
  patterns: [atomic file writes via tempfile rename, cache versioning with CACHE_VERSION constant]

key-files:
  created:
    - src/cache/mod.rs
    - src/cache/envelope.rs
  modified:
    - Cargo.toml
    - src/graph/node.rs
    - src/graph/edge.rs
    - src/graph/mod.rs
    - src/main.rs

key-decisions:
  - "bincode 2 with serde feature used (not derive feature) — all serialization via serde path only"
  - "Atomic write pattern: NamedTempFile in cache dir then persist() rename — prevents corrupt cache on crash"
  - "CACHE_VERSION = 1 constant — load_cache returns None on version mismatch forcing full rebuild"
  - "remove_file_from_graph uses Contains edges to find owned symbols, ChildOf incoming edges for child symbols"
  - "Rust 1.84.1 default toolchain doesn't support edition 2024 — rustup default stable set to 1.93.1"
  - "Rust 2024 edition: ref pattern in if-let removed (compiler error) — pattern binding implicitly borrows"

patterns-established:
  - "Cache save: collect_file_mtimes + CacheEnvelope + NamedTempFile + persist() for atomic disk writes"
  - "Cache load: read bytes + decode_from_slice + version guard + return None on any failure"
  - "Graph mutation: collect nodes to remove first (Vec), then mutate in second pass — avoids borrow issues"

requirements-completed: [PERF-04]

# Metrics
duration: 3min
completed: 2026-02-23
---

# Phase 5 Plan 01: Graph Serialization and Cache Persistence Summary

**Serde derives + bincode 2 cache layer: all graph types serializable, atomic save/load to .code-graph/graph.bin with version checking and file mtime tracking**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-23T12:46:46Z
- **Completed:** 2026-02-23T12:49:52Z
- **Tasks:** 2
- **Files modified:** 7 (5 modified, 2 created)

## Accomplishments
- Added `serde::Serialize + serde::Deserialize` derives to all graph types: `SymbolKind`, `SymbolInfo`, `FileInfo`, `ExternalPackageInfo`, `GraphNode`, `EdgeKind`, and `CodeGraph`
- Added `Clone` derive to `CodeGraph` and enabled petgraph `serde-1` feature for `StableGraph` serialization
- Implemented `remove_file_from_graph` on `CodeGraph` — surgically removes file node, all owned symbols (via Contains edges), child symbols (via ChildOf edges), cleans both `file_index` and `symbol_index`
- Built `src/cache/` module with `CacheEnvelope`, `save_cache` (atomic via `NamedTempFile` rename), and `load_cache` (version-checked, returns `None` on mismatch/corruption)
- All 89 tests pass (87 pre-existing + 2 new cache roundtrip tests)

## Task Commits

Each task was committed atomically:

1. **Task 1: Add serde derives to graph types and remove_file_from_graph** - `5847cf8` (feat)
2. **Task 2: Implement cache envelope with atomic save/load** - `5687405` (feat)

## Files Created/Modified
- `Cargo.toml` - Added `bincode = { version = "2", features = ["serde"] }`, `tempfile = "3"`, petgraph `serde-1` feature
- `src/graph/node.rs` - Serde derives on `SymbolKind`, `SymbolInfo`, `FileInfo`, `ExternalPackageInfo`, `GraphNode`
- `src/graph/edge.rs` - Serde derives on `EdgeKind`
- `src/graph/mod.rs` - Serde + Clone derives on `CodeGraph`, `remove_file_from_graph` method, imports `std::path::Path` and `petgraph::visit::EdgeRef`
- `src/cache/mod.rs` - Cache module with re-exports
- `src/cache/envelope.rs` - `FileMeta`, `CacheEnvelope`, `cache_path`, `collect_file_mtimes`, `save_cache`, `load_cache`, roundtrip tests
- `src/main.rs` - Added `mod cache;` declaration

## Decisions Made
- Used bincode 2 with `serde` feature (not `derive`) — all serialization goes through serde derives already on graph types
- Atomic write: `NamedTempFile::new_in(cache_dir)` then `tmp.persist(target)` — guarantees no partial writes even on crash
- `CACHE_VERSION = 1` constant — `load_cache` returns `None` on version mismatch, caller does full rebuild (safe degradation)
- `remove_file_from_graph` collects `Contains` edges from file node to find owned top-level symbols, then `edges_directed` with `Incoming` + `ChildOf` filter for child symbols — two-pass collect-then-remove to avoid borrow checker issues

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed `ref` binding pattern incompatible with Rust 2024 edition**
- **Found during:** Task 1 (build verification)
- **Issue:** `if let Some(GraphNode::Symbol(ref info))` caused compiler error in edition 2024: "cannot explicitly borrow within an implicitly-borrowing pattern"
- **Fix:** Removed `ref` keyword — pattern binding implicitly borrows in Rust 2024
- **Files modified:** `src/graph/mod.rs`
- **Verification:** `cargo build` succeeds
- **Committed in:** `5847cf8` (Task 1 commit)

**2. [Rule 3 - Blocking] Set rustup default to stable 1.93.1**
- **Found during:** Task 1 (initial build attempt)
- **Issue:** Default toolchain was 1.84.1 which does not support `edition = "2024"` in Cargo.toml
- **Fix:** `rustup update stable && rustup default stable` — installed 1.93.1
- **Files modified:** None (toolchain configuration)
- **Verification:** `cargo build` succeeds after toolchain switch
- **Committed in:** N/A (environment fix)

---

**Total deviations:** 2 auto-fixed (1 Rule 1 bug, 1 Rule 3 blocking)
**Impact on plan:** Both fixes required for compilation. No scope creep.

## Issues Encountered
- Rust 1.84.1 was the shell default despite 1.93.1 being the correct toolchain for this project — ran `rustup default stable` to set 1.93.1 permanently

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All graph types are now serializable via bincode — ready for Phase 5 Plan 2 (file watcher)
- `remove_file_from_graph` provides the incremental update primitive the watcher needs
- Cache save/load infrastructure ready for integration into the watch-mode rebuild loop
- No blockers or concerns

---
*Phase: 05-watch-mode-persistence*
*Completed: 2026-02-23*
