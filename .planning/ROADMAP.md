# Roadmap: Code-Graph

## Overview

Code-Graph is built in six phases that follow the natural dependency order of the system. Parsing must exist before resolution, resolution before queries, queries before MCP exposure, MCP before watch mode (which changes daemon architecture), and watch mode before performance hardening. Each phase delivers a coherent, verifiable capability. By the end of Phase 4 Claude Code can query a correct dependency graph via MCP. Phases 5-6 make the tool production-grade: always-on watch mode, sub-second incremental updates, single-binary distribution, and benchmark-validated performance.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [x] **Phase 1: Foundation & Core Parsing** - Rust scaffold, tree-sitter parsing, symbol extraction, and an in-memory graph (completed 2026-02-22)
- [ ] **Phase 2: Import Resolution & Graph Completion** - Full import resolution (tsconfig paths, barrel files, monorepo workspaces) and complete symbol-level graph
- [ ] **Phase 3: Query Engine & CLI** - All graph queries (definition, references, impact, circular deps, context) exposed via CLI commands
- [ ] **Phase 4: MCP Integration** - rmcp stdio server exposing graph tools to Claude Code with token-optimized responses
- [ ] **Phase 5: Watch Mode & Persistence** - File watcher, incremental re-indexing (<100ms), graph persistence for fast cold starts
- [ ] **Phase 6: Performance & Distribution** - Parallel parsing, memory optimization, single-binary distribution via cargo install

## Phase Details

### Phase 1: Foundation & Core Parsing
**Goal**: A runnable Rust binary that can walk a TypeScript/JavaScript project, parse every source file with tree-sitter, extract symbols, and persist them in an in-memory graph
**Depends on**: Nothing (first phase)
**Requirements**: PARS-01, PARS-02, PARS-03, PARS-04
**Success Criteria** (what must be TRUE):
  1. Running `code-graph index .` on a TS project completes without errors and reports file and symbol counts
  2. The tool discovers all .ts/.tsx/.js/.jsx files while correctly excluding paths in .gitignore
  3. The tool extracts functions, classes, interfaces, type aliases, enums, and exported variables from each file
  4. The tool extracts ESM imports, CJS require calls, and dynamic imports with string literals from each file
  5. The tool extracts named exports, default exports, and re-exports from each file
**Plans:** 3/3 plans complete
Plans:
- [ ] 01-01-PLAN.md — Project scaffold, CLI, config, and file walker (PARS-01)
- [ ] 01-02-PLAN.md — Parser infrastructure, graph structures, and symbol extraction (PARS-02)
- [ ] 01-03-PLAN.md — Import/export extraction and full indexing pipeline (PARS-03, PARS-04)

### Phase 2: Import Resolution & Graph Completion
**Goal**: The in-memory graph correctly resolves every import to its actual defining file and symbol, handling TypeScript path aliases, barrel files, and monorepo workspace packages
**Depends on**: Phase 1
**Requirements**: PARS-05, PARS-06, PARS-07, PARS-08, PARS-09
**Success Criteria** (what must be TRUE):
  1. An import using a `@/` path alias (configured in tsconfig.json `paths`) resolves to the correct absolute file path
  2. An import from an `index.ts` barrel file resolves to the specific file that originally defines the imported symbol (not the barrel itself)
  3. An import referencing a workspace package name resolves to its local source path (not node_modules)
  4. The graph contains a complete file-level dependency edge for every import in the codebase
  5. The graph contains symbol-level relationship edges: contains, exports, calls, extends, implements
**Plans:** 1/3 plans executed
Plans:
- [ ] 02-01-PLAN.md — Graph types extension + resolver infrastructure (PARS-05, PARS-07, PARS-08)
- [ ] 02-02-PLAN.md — Symbol relationship extraction via tree-sitter queries (PARS-09)
- [ ] 02-03-PLAN.md — Resolution pipeline integration + barrel chasing + output (PARS-05, PARS-06, PARS-07, PARS-08, PARS-09)

### Phase 3: Query Engine & CLI
**Goal**: Developers and Claude can query the graph for any symbol's definition, references, impact radius, circular dependencies, and full context — all accessible via CLI commands
**Depends on**: Phase 2
**Requirements**: QURY-01, QURY-02, QURY-03, QURY-04, QURY-05, INTG-06
**Success Criteria** (what must be TRUE):
  1. `code-graph query <symbol>` returns the file path and line number of the symbol's definition
  2. `code-graph query --refs <symbol>` returns all files and locations that reference the symbol
  3. `code-graph impact <symbol>` returns the transitive set of files and symbols that would break if the symbol changed
  4. `code-graph query --circular` reports all circular dependency cycles in the import graph
  5. `code-graph stats` prints an overview of file count, symbol count, and module structure
**Plans**: TBD

### Phase 4: MCP Integration
**Goal**: Claude Code can query the dependency graph directly using native MCP tools, with responses that consume ~60% fewer tokens than verbose JSON
**Depends on**: Phase 3
**Requirements**: INTG-01, INTG-02, INTG-03
**Success Criteria** (what must be TRUE):
  1. Adding the code-graph MCP server to Claude Code's config makes graph query tools appear in Claude's tool list automatically
  2. Claude can call a `find_symbol` tool and receive a location response compact enough that a 10-symbol result fits in under 200 tokens
  3. Each MCP tool description is under 100 tokens so per-turn overhead is negligible
**Plans**: TBD

### Phase 5: Watch Mode & Persistence
**Goal**: The graph stays current automatically while the daemon runs, re-indexing changed files in under 100ms, and loads instantly from disk on cold start without re-parsing unchanged files
**Depends on**: Phase 4
**Requirements**: INTG-04, INTG-05, PERF-04
**Success Criteria** (what must be TRUE):
  1. Saving a file in an indexed project triggers an automatic incremental re-index with no user action
  2. A single-file change is fully re-indexed and the graph updated in under 100ms
  3. After stopping and restarting the daemon, the graph is available immediately (cold start reads persisted graph, skips parsing unchanged files)
**Plans**: TBD

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

**Execution Order:**
Phases execute in numeric order: 1 → 2 → 3 → 4 → 5 → 6

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Foundation & Core Parsing | 3/3 | Complete    | 2026-02-22 |
| 2. Import Resolution & Graph Completion | 1/3 | In Progress|  |
| 3. Query Engine & CLI | 0/TBD | Not started | - |
| 4. MCP Integration | 0/TBD | Not started | - |
| 5. Watch Mode & Persistence | 0/TBD | Not started | - |
| 6. Performance & Distribution | 0/TBD | Not started | - |
