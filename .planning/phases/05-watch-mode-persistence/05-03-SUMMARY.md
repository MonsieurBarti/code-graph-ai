---
phase: 05-watch-mode-persistence
plan: 03
subsystem: mcp-integration
tags: [mcp, watcher, cache, rwlock, cold-start, cli]
requirements: [INTG-04, INTG-05, PERF-04]

dependency_graph:
  requires: [05-01, 05-02]
  provides: [mcp-watcher-integration, watch-cli, cold-start-cache]
  affects: [src/mcp/server.rs, src/cli.rs, src/main.rs]

tech_stack:
  added: []
  patterns:
    - RwLock read/write lock split for concurrent cache access
    - Double-check locking pattern (read -> write -> re-check)
    - Clone-under-read-lock + drop-lock + work-in-spawn_blocking + re-lock-to-swap
    - Lazy watcher initialization (single atomic init via tokio Mutex)
    - Staleness diff with 10% change threshold for cold start

key_files:
  created: []
  modified:
    - src/mcp/server.rs
    - src/cli.rs
    - src/main.rs

decisions:
  - "RwLock upgrade from Mutex: graph_cache uses Arc<RwLock<HashMap>> — allows concurrent reads during all MCP tool calls"
  - "Lazy watcher start: ensure_watcher_running called after first graph load, not at server startup"
  - "Lock discipline: write lock held ONLY for HashMap insert (nanoseconds), never during parse/resolve/IO (milliseconds)"
  - "Staleness diff threshold: if >=10% files changed on cold start, do full rebuild instead of scoped re-resolve"
  - "Double-check locking: read lock fast path -> drop -> write lock -> re-check before building to prevent redundant builds"
  - "Cache save in Index command: graph saved to .code-graph/graph.bin after indexing for MCP cold start benefit"

metrics:
  duration_minutes: 5
  completed_date: "2026-02-23"
  tasks_completed: 2
  files_modified: 3
---

# Phase 5 Plan 3: MCP Integration — Watcher + Cache Summary

**One-liner:** MCP server cold-start with disk cache + staleness diff, lazy embedded watcher using RwLock for concurrent read access, and standalone `code-graph watch` CLI command.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | MCP server — RwLock, cold start, lazy watcher, event loop | 8c72401 | src/mcp/server.rs |
| 2 | Watch CLI command and cache save in index command | b073f63 | src/cli.rs, src/main.rs |

## What Was Built

### Task 1: MCP Server Integration

**src/mcp/server.rs** was substantially refactored:

1. **RwLock upgrade:** `graph_cache` changed from `Arc<Mutex<HashMap>>` to `Arc<RwLock<HashMap>>`. All concurrent MCP tool calls now hold read locks simultaneously — no serialization during normal operation.

2. **watcher_handle field:** Added `Arc<tokio::sync::Mutex<Option<WatcherHandle>>>` — a tokio (not std) Mutex since it's only written once (lazy init) and rarely accessed.

3. **resolve_graph cold start flow:**
   - Fast path: acquire read lock, return cached graph immediately if present
   - Slow path: acquire write lock, double-check (another task may have populated), then `spawn_blocking` to try `load_cache` first
   - On disk cache hit: `apply_staleness_diff` re-parses changed files
   - On disk cache miss: full `build_graph`
   - Always saves updated graph to disk cache after build
   - Calls `ensure_watcher_running` after write lock is dropped

4. **apply_staleness_diff:** Walks current filesystem, compares mtime+size against cached metadata, classifies files as changed/deleted/unchanged. If `>=10%` changed: full rebuild (threshold avoids expensive scoped re-resolve for large diffs). Otherwise: remove deleted files, re-parse changed files, then run a full `resolve_all` pass on the updated graph.

5. **ensure_watcher_running:** Acquires watcher Mutex, returns immediately if already running. On first call: calls `start_watcher`, spawns a background task. The background task processes `WatchEvent`s with strict lock discipline:
   - `ConfigChanged`: `spawn_blocking` for full rebuild + `save_cache` (no lock held during CPU/IO), then `write().await` only for `HashMap::insert`
   - Other events: acquire read lock to clone `Arc<CodeGraph>`, drop read lock, call `(*old_arc).clone()` for owned graph, `spawn_blocking` for `handle_file_event` + `save_cache` (no lock held), then `write().await` only for `HashMap::insert`

### Task 2: Watch CLI Command + Index Cache Save

**src/cli.rs:** Added `Watch { path: PathBuf }` variant to `Commands` enum with documentation explaining it's a standalone debug tool (the MCP server has its own embedded watcher).

**src/main.rs:**
- `Commands::Watch` handler: indexes the project, saves initial cache, starts watcher, processes events in a loop with timing output per event type (Modified/Created/Deleted: shows relative path, elapsed ms, file/symbol counts; ConfigChanged: shows re-index time)
- `Commands::Index` handler: now saves cache to `.code-graph/graph.bin` after indexing — enables fast MCP cold starts for projects indexed via CLI
- All watch output goes to stderr per Phase 1 convention (stdout reserved for `--json` piping)

## Verification Results

- `cargo build`: clean, zero new errors (4 pre-existing warnings unchanged)
- `cargo test`: 89/89 passed
- `code-graph --help`: `watch` subcommand visible
- MCP server `resolve_graph`: tries disk cache first, applies staleness diff, falls back to full build
- Watcher starts lazily on first MCP tool call (not at server startup)
- Background event loop processes incremental updates per correct lock discipline
- RwLock allows concurrent reads during all tool calls
- Index command saves cache after indexing
- Watch command prints timing info per event to stderr

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] `if let Ok` used on Option return value in ensure_watcher_running**
- **Found during:** Task 1 compilation
- **Issue:** Plan template used `if let Ok(new_graph) = ... .ok().and_then(|r| r.ok())` which returns an `Option`, not a `Result`
- **Fix:** Changed to `if let Some(new_graph) = ...`
- **Files modified:** src/mcp/server.rs
- **Commit:** 8c72401 (fixed inline before commit)

None other — plan executed as written.

## Self-Check: PASSED

- src/mcp/server.rs: FOUND
- src/cli.rs: FOUND
- src/main.rs: FOUND
- 05-03-SUMMARY.md: FOUND
- Commit 8c72401: FOUND
- Commit b073f63: FOUND
