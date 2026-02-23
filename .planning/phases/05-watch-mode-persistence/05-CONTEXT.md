# Phase 5: Watch Mode & Persistence - Context

**Gathered:** 2026-02-23
**Status:** Ready for planning

<domain>
## Phase Boundary

Background file watcher with incremental re-indexing (<100ms for single-file changes) and graph persistence to disk for instant cold starts. The watcher keeps the graph current automatically while the daemon runs, and the persisted cache eliminates re-parsing unchanged files on restart. Performance benchmarks (10K files, parallel parsing, memory limits) and distribution are Phase 6.

</domain>

<decisions>
## Implementation Decisions

### Daemon lifecycle
- Watcher embedded in the MCP server process (single process, watcher runs as internal thread)
- Watcher starts lazily — triggered on first index or query, not on MCP server startup
- Also provide standalone `code-graph watch` CLI command for watching without MCP (dual mode: embedded + standalone)
- Standalone watch command prints status to terminal, useful for debugging and manual use

### Persistence format
- Cache lives in `.code-graph/` directory in project root (similar to `.git/` convention)
- Serialization format: bincode (fast serialize/deserialize, compact on disk, maximizes cold start speed)
- Staleness detection: mtime + file size (stat call only, no content hashing — covers 99% of real changes)
- On load: smart diff — compare cached file list vs current file list, re-parse changed/new files, remove deleted entries. Handles branch switches gracefully without full re-index

### Incremental re-index scope
- Propagation: re-parse changed file + update direct dependents (files that import the changed file, whose resolution might have changed)
- File deletions: remove file's symbols and edges from graph, mark imports pointing to it as unresolved
- New files: parse immediately + check if existing unresolved imports now resolve to this file, fix those edges
- Renames: treated as delete + create (simpler, most watchers report it this way)

### Watch filtering
- Watcher respects same .gitignore rules used during initial indexing (single source of truth)
- Rapid saves handled with debounce (~50-100ms after last change before re-indexing)
- Config file changes (tsconfig.json, package.json) trigger full re-index (affect path resolution globally)
- node_modules always excluded from watching (hardcoded, regardless of .gitignore)

### Claude's Discretion
- Watcher activity reporting strategy (log file, terminal output, or silent)
- Exact debounce timing within the ~50-100ms range
- Bincode schema versioning / migration approach
- Temp file handling during cache writes (atomic write strategy)
- Exact watcher library choice (notify crate or alternative)

</decisions>

<specifics>
## Specific Ideas

No specific requirements — open to standard approaches

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 05-watch-mode-persistence*
*Context gathered: 2026-02-23*
