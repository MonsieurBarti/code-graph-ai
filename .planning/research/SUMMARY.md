# Project Research Summary

**Project:** Code-Graph
**Domain:** Code intelligence engine / dependency graph tool with MCP integration
**Researched:** 2026-02-22
**Confidence:** HIGH

## Executive Summary

Code-Graph is a Rust-based code intelligence engine that indexes TypeScript/JavaScript codebases into a queryable dependency graph, exposed to Claude Code via MCP. The research confirms Rust as the optimal language choice: tree-sitter has first-class Rust bindings, petgraph provides graph algorithms out of the box, and Anthropic maintains an official Rust MCP SDK (rmcp). The architecture follows a 4-stage pipeline (Walk → Parse → Resolve → Link) with in-memory graph storage and zero-copy persistence.

The competitive landscape includes Axon (Python, comprehensive but slow), dependency-cruiser (JS, import-only), and code-graph-mcp (JS, moderate). Our key differentiators are: sub-second incremental re-indexing, token-optimized MCP responses (~60% fewer tokens than competitors), single-binary distribution with zero dependencies, and TypeScript-first accuracy (barrel files, path aliases, monorepo support).

The primary risks are TypeScript import resolution complexity (barrel files, path aliases, monorepo workspaces) and memory management in long-running watch mode. Both are addressable with the recommended architecture: symbol-level import tracking and string interning from day one.

## Key Findings

### Recommended Stack

**Language: Rust** — wins on tree-sitter integration (native bindings), graph algorithms (petgraph), memory efficiency (no GC), and performance ceiling. Trade-off: ~30-40% slower development than Go.

**Core technologies:**
- **tree-sitter** (0.24+): TS/JS parsing — industry standard, incremental parsing, thread-safe
- **petgraph** (0.6+): In-memory graph — DFS, BFS, cycle detection built-in, no external DB
- **rmcp** (latest): MCP server — official Anthropic Rust SDK, stdio transport
- **notify** (7.0+): File watching — cross-platform, debounced events
- **rkyv** (0.8+): Graph persistence — zero-copy deserialization for instant startup
- **tokio** (1.x): Async runtime — concurrent watcher + MCP server
- **serde** (1.x): Serialization — compact JSON output for MCP responses

### Expected Features

**Must have (table stakes):**
- File-level import/dependency graph
- Symbol extraction (functions, classes, types, exports)
- Go-to-definition and find-all-references
- Impact analysis (blast radius)
- MCP server with structured tool responses
- Incremental re-indexing with file watcher
- CLI for developer debugging

**Should have (competitive):**
- Token-optimized output format (~60% savings vs verbose JSON)
- Sub-second incremental re-index (<100ms single-file)
- Zero-dependency single binary
- TypeScript-first accuracy (barrel files, path aliases)
- Guided next-step hints in MCP responses

**Defer (v2+):**
- Multi-language support — TS/JS only in v1
- Vector/semantic search — structural queries are sufficient
- Visual graph rendering — DOT export at most
- LSP server mode — MCP is the integration target
- Git coupling analysis — high cost, unclear AI value

### Architecture Approach

The system follows a pipeline architecture with 4 major layers: CLI/MCP entry points → Query Engine → Graph Store (petgraph) → Indexing Pipeline (walk → parse → resolve → link). The graph is held in memory with `Arc<RwLock<GraphStore>>` for concurrent access. File watcher sends changed paths via tokio channels to the indexer, which does incremental updates. Persistence via rkyv enables fast cold starts.

**Major components:**
1. **Indexing Pipeline** — 4-stage: Walker (ignore crate) → Parser (tree-sitter) → Resolver (tsconfig-aware) → Linker (graph edges)
2. **Graph Store** — petgraph DiGraph with typed nodes (File, Function, Class, etc.) and edges (IMPORTS, CALLS, CONTAINS, etc.)
3. **Query Engine** — symbol lookup, impact traversal, reference search on the graph
4. **MCP Server** — rmcp stdio server exposing 5-6 tools to Claude Code
5. **File Watcher** — notify with debounce, triggers incremental re-index

### Critical Pitfalls

1. **Barrel file explosion** — `index.ts` re-exports create O(n^2) edges. Fix: symbol-level import resolution (track WHICH symbol, not just which file).
2. **TypeScript path alias resolution** — `@/components/*` paths break without tsconfig parsing including `extends` chains. Fix: full tsconfig resolver with inheritance.
3. **Memory leaks in watch mode** — graph nodes accumulate, strings not freed. Fix: string interning (lasso crate) from day one, periodic compaction.
4. **MCP tool description token overhead** — verbose tool descriptions consume hundreds of tokens per turn. Fix: keep each description under 100 tokens.
5. **Monorepo package resolution** — workspace packages resolve to node_modules instead of local paths. Fix: read `package.json` workspaces, build package map.

## Implications for Roadmap

### Phase 1: Project Foundation & Core Parsing
**Rationale:** Can't build anything without the parsing pipeline and basic graph structure.
**Delivers:** Rust project scaffold, tree-sitter TS/JS parsing, symbol extraction, basic graph structure, CLI `index` command.
**Addresses:** File-level import graph, symbol extraction.
**Avoids:** Building MCP too early (need a working graph first).

### Phase 2: Import Resolution & Graph Completion
**Rationale:** The graph is only useful if imports resolve correctly. This is the hardest part — barrel files, path aliases, monorepos.
**Delivers:** Full import resolution (tsconfig paths, barrel files, workspace packages), complete dependency graph, call graph basics.
**Addresses:** Go-to-definition, find-all-references, circular dependency detection.
**Avoids:** Barrel file explosion (symbol-level tracking), path alias bugs (full tsconfig parsing).

### Phase 3: MCP Server & Query Engine
**Rationale:** Once the graph is correct, expose it to Claude Code via MCP.
**Delivers:** MCP server with 5-6 tools, token-optimized response format, impact analysis queries.
**Addresses:** MCP integration, impact analysis, token-efficient output.
**Avoids:** MCP tool description bloat (token budget from day one).

### Phase 4: Watch Mode & Incremental Updates
**Rationale:** The graph must stay current for daily use. Watch mode makes it a living tool, not a one-shot indexer.
**Delivers:** File watcher, incremental re-indexing, graph persistence, daemon mode.
**Addresses:** Incremental re-index, sub-second updates.
**Avoids:** Memory leaks (string interning, compaction), full re-index on every change.

### Phase 5: CLI Polish & Distribution
**Rationale:** Make the tool installable and usable as a standalone CLI.
**Delivers:** CLI commands (query, impact, stats), single-binary distribution, installation docs.
**Addresses:** CLI interface, zero-dependency distribution.

### Phase 6: Performance Optimization & Hardening
**Rationale:** Optimize for the target benchmarks (10K files <30s, incremental <100ms, <100MB RSS).
**Delivers:** Parallel parsing (rayon), memory optimization, benchmark suite, edge case handling.
**Addresses:** Performance constraints, scaling to large codebases.

### Phase Ordering Rationale

- Parsing before resolution: can't resolve imports without parsed AST
- Resolution before MCP: MCP tools are useless with incorrect graph
- MCP before watch: manual re-index is fine for testing; watch is a UX enhancement
- Watch before polish: watch mode changes daemon architecture, affects CLI design
- Optimization last: premature optimization wastes effort; optimize with real benchmarks

### Research Flags

Phases likely needing deeper research during planning:
- **Phase 2:** TypeScript import resolution is complex — needs research into tsconfig spec, monorepo tools
- **Phase 3:** MCP protocol details, tool registration, stdio transport specifics

Phases with standard patterns (skip research-phase):
- **Phase 1:** tree-sitter parsing is well-documented, Rust project setup is standard
- **Phase 4:** File watching with notify is straightforward
- **Phase 5:** CLI with clap is well-documented

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Rust + tree-sitter + petgraph is well-proven combination |
| Features | HIGH | Competitive analysis across 6+ tools, clear differentiation |
| Architecture | HIGH | Pipeline pattern is standard for code analysis tools |
| Pitfalls | HIGH | TS/JS-specific pitfalls well-documented by community |

**Overall confidence:** HIGH

### Gaps to Address

- **rmcp maturity:** The Anthropic Rust MCP SDK is relatively new — verify API stability during Phase 3 planning
- **petgraph persistence:** rkyv integration with petgraph may need custom serialization — prototype early
- **Tree-sitter TypeScript grammar completeness:** Verify handling of latest TS features (satisfies, const type params)

## Sources

### Primary (HIGH confidence)
- tree-sitter Rust crate: tree-sitter/tree-sitter GitHub
- petgraph: docs.rs/petgraph
- rmcp: github.com/anthropics/rmcp
- Axon reference implementation: github.com/harshkedia177/axon
- dependency-cruiser: github.com/sverweij/dependency-cruiser
- code-graph-mcp: github.com/entrepeneur4lyf/code-graph-mcp

### Secondary (MEDIUM confidence)
- MCP tool description overhead: arxiv 2602.14878v1
- Constellation code intelligence: constellationdev.io
- Sourcerer token savings claims: skywork.ai

### Tertiary (LOW confidence)
- rkyv + petgraph integration: needs prototyping to validate

---
*Research completed: 2026-02-22*
*Ready for roadmap: yes*
