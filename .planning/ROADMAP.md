# Roadmap: Code-Graph

## Milestones

- ✅ **v1.0 MVP** — Phases 1-5 (shipped 2026-02-23)

## Phases

<details>
<summary>✅ v1.0 MVP (Phases 1-5) — SHIPPED 2026-02-23</summary>

- [x] Phase 1: Foundation & Core Parsing (3/3 plans) — completed 2026-02-22
- [x] Phase 2: Import Resolution & Graph Completion (4/4 plans) — completed 2026-02-22
- [x] Phase 3: Query Engine & CLI (3/3 plans) — completed 2026-02-23
- [x] Phase 4: MCP Integration (2/2 plans) — completed 2026-02-23
- [x] Phase 5: Watch Mode & Persistence (3/3 plans) — completed 2026-02-23

</details>

### Phase 6: Performance & Distribution
**Goal**: The tool meets all performance benchmarks (10K files indexed under 30s, daemon under 100MB RSS) and ships as a zero-dependency single binary installable via cargo
**Depends on**: Phase 5
**Requirements**: PERF-01, PERF-02, PERF-03, DIST-01, DIST-02
**Success Criteria** (what must be TRUE):
  1. `code-graph index .` on a 10,000-file TypeScript codebase completes in under 30 seconds
  2. The background daemon uses less than 100MB RSS on a typical project of up to 10K files
  3. `cargo install code-graph` succeeds and places a working binary in PATH with no additional runtime dependencies
  4. The installed binary is fully self-contained (no dynamic library dependencies on tree-sitter or any other C library at runtime)
**Plans**: TBD

## Progress

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 1. Foundation & Core Parsing | v1.0 | 3/3 | Complete | 2026-02-22 |
| 2. Import Resolution & Graph Completion | v1.0 | 4/4 | Complete | 2026-02-22 |
| 3. Query Engine & CLI | v1.0 | 3/3 | Complete | 2026-02-23 |
| 4. MCP Integration | v1.0 | 2/2 | Complete | 2026-02-23 |
| 5. Watch Mode & Persistence | v1.0 | 3/3 | Complete | 2026-02-23 |
| 6. Performance & Distribution | 3/3 | Complete   | 2026-02-23 | - |
