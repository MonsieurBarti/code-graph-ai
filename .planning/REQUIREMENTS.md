# Requirements: Code-Graph

**Defined:** 2026-02-22
**Core Value:** Claude Code can understand any codebase's structure and dependencies without reading source files — querying a local graph instead, saving tokens and time on every interaction.

## v1 Requirements

### Parsing

- [ ] **PARS-01**: Tool can index all .ts/.tsx/.js/.jsx files in a project, respecting .gitignore
- [ ] **PARS-02**: Tool extracts symbols from each file: functions, classes, interfaces, type aliases, enums, and exported variables
- [ ] **PARS-03**: Tool extracts all import statements (ESM import, CJS require, dynamic import with string literal)
- [ ] **PARS-04**: Tool extracts export statements (named exports, default exports, re-exports)
- [ ] **PARS-05**: Tool resolves TypeScript path aliases from tsconfig.json (paths, baseUrl, extends chains)
- [ ] **PARS-06**: Tool resolves barrel file imports to the actual defining file (symbol-level resolution through index.ts)
- [ ] **PARS-07**: Tool resolves monorepo workspace packages to local paths (package.json workspaces)
- [ ] **PARS-08**: Tool builds a complete file-level dependency graph with import edges
- [ ] **PARS-09**: Tool builds symbol-level relationships: contains, exports, calls, extends, implements

### Queries

- [ ] **QURY-01**: User can find the definition of a symbol (name → file:line location)
- [ ] **QURY-02**: User can find all references to a symbol across the codebase
- [ ] **QURY-03**: User can get the impact/blast radius of changing a symbol (transitive dependents)
- [ ] **QURY-04**: User can detect circular dependencies in the import graph
- [ ] **QURY-05**: User can get a 360-degree context view of a symbol (definition, callers, callees, type usage)

### Integration

- [ ] **INTG-01**: Tool runs as an MCP server over stdio, exposing graph queries as tools to Claude Code
- [ ] **INTG-02**: MCP tool responses use a token-optimized compact format (target: ~60% fewer tokens than verbose JSON)
- [ ] **INTG-03**: MCP tool descriptions are concise (under 100 tokens each) to minimize per-turn overhead
- [ ] **INTG-04**: Tool runs as a background daemon with file watching, re-indexing incrementally on file changes
- [ ] **INTG-05**: Incremental re-index completes in under 100ms for single-file changes
- [ ] **INTG-06**: Tool provides CLI commands: index (full index), query (symbol lookup), impact (blast radius), stats (overview)

### Performance

- [ ] **PERF-01**: Full index of a 10K-file TypeScript codebase completes in under 30 seconds
- [ ] **PERF-02**: Tool uses parallel parsing (rayon) to utilize multi-core CPUs
- [ ] **PERF-03**: Background daemon uses less than 100MB RSS for typical projects (<10K files)
- [ ] **PERF-04**: Graph persists to disk; cold start loads cached graph without re-parsing unchanged files

### Distribution

- [ ] **DIST-01**: Tool compiles to a single static binary with zero runtime dependencies
- [ ] **DIST-02**: Tool is installable via `cargo install code-graph`

## v2 Requirements

### Extended Analysis

- **EXAN-01**: Caller/callee call graph with confidence scoring (direct call = HIGH, alias = MEDIUM)
- **EXAN-02**: Dead code detection (symbols with zero inbound references, framework-aware)
- **EXAN-03**: Guided next-step hints in MCP responses (suggest follow-up tool calls)
- **EXAN-04**: Project statistics overview tool (file count, symbol count, module structure, most-imported modules)

### Multi-Language

- **LANG-01**: Go language support via tree-sitter-go grammar
- **LANG-02**: Python language support via tree-sitter-python grammar
- **LANG-03**: Rust language support via tree-sitter-rust grammar

### Export

- **EXPO-01**: DOT format export for graph visualization (pipe to Graphviz)

## Out of Scope

| Feature | Reason |
|---------|--------|
| Vector/semantic search / embeddings | Violates <100MB memory constraint, adds model dependency, unnecessary for structural queries |
| AI-powered code summarization | Defeats purpose — let Claude summarize from structural data |
| Visual graph rendering / UI | Orthogonal to AI agent use case, separate product |
| Cross-repo analysis | Requires network, auth, distributed storage — conflicts with local-only constraint |
| LSP server mode | MCP is the integration target; LSP is editor-focused, different protocol |
| Git coupling / change co-occurrence | High indexing cost, unclear AI agent value |
| Community detection / clustering | Filesystem hierarchy already provides natural clustering |
| Full TypeScript type inference | Requires loading TS compiler, too heavy for a CLI tool |
| Real-time type checking | tree-sitter is a parser, not a type checker; defer to TS compiler |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| PARS-01 | Phase 1 | Pending |
| PARS-02 | Phase 1 | Pending |
| PARS-03 | Phase 1 | Pending |
| PARS-04 | Phase 1 | Pending |
| PARS-05 | Phase 2 | Pending |
| PARS-06 | Phase 2 | Pending |
| PARS-07 | Phase 2 | Pending |
| PARS-08 | Phase 2 | Pending |
| PARS-09 | Phase 2 | Pending |
| QURY-01 | Phase 3 | Pending |
| QURY-02 | Phase 3 | Pending |
| QURY-03 | Phase 3 | Pending |
| QURY-04 | Phase 3 | Pending |
| QURY-05 | Phase 3 | Pending |
| INTG-01 | Phase 4 | Pending |
| INTG-02 | Phase 4 | Pending |
| INTG-03 | Phase 4 | Pending |
| INTG-04 | Phase 5 | Pending |
| INTG-05 | Phase 5 | Pending |
| INTG-06 | Phase 3 | Pending |
| PERF-01 | Phase 6 | Pending |
| PERF-02 | Phase 6 | Pending |
| PERF-03 | Phase 6 | Pending |
| PERF-04 | Phase 5 | Pending |
| DIST-01 | Phase 6 | Pending |
| DIST-02 | Phase 6 | Pending |

**Coverage:**
- v1 requirements: 26 total
- Mapped to phases: 26
- Unmapped: 0 ✓

---
*Requirements defined: 2026-02-22*
*Last updated: 2026-02-22 after initial definition*
