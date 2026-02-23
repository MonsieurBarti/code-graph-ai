# Code-Graph

## What This Is

A high-performance Rust code intelligence engine that indexes TypeScript/JavaScript codebases into a queryable dependency graph. It gives Claude Code direct access to a pre-built structural graph via MCP, enabling smarter navigation and impact analysis while minimizing token consumption. Includes a file watcher for automatic incremental re-indexing and bincode-based graph persistence for instant cold starts.

## Core Value

Claude Code can understand any codebase's structure and dependencies without reading source files — querying a local graph instead, saving tokens and time on every interaction.

## Requirements

### Validated

- ✓ Fast tree-sitter-based parsing of TypeScript/JavaScript codebases — v1.0
- ✓ Dependency graph covering files, symbols, imports, and call relationships — v1.0
- ✓ Background watch mode that re-indexes incrementally on file changes (<100ms) — v1.0
- ✓ MCP server exposing graph queries to Claude Code as native tools — v1.0
- ✓ Impact analysis: "what breaks if I change X?" — v1.0
- ✓ Smart navigation: jump to the right symbol without reading the file — v1.0
- ✓ Token-efficient output format (71% savings vs JSON) — v1.0
- ✓ Embedded graph storage via petgraph + bincode persistence — v1.0
- ✓ CLI for direct developer use (index, find, refs, impact, circular, context, stats, watch) — v1.0

### Active

- [ ] Parallel parsing via rayon for multi-core utilization
- [ ] Memory optimization for <100MB RSS on 10K-file projects
- [ ] Single static binary with zero runtime dependencies
- [ ] `cargo install code-graph` distribution

### Out of Scope

- Multi-language support beyond TS/JS — defer to v2 after nailing one language
- Cloud/remote features — everything runs locally
- Visual graph rendering/UI — Claude Code integration is the interface
- AI-powered code summarization — focus on structural data, not NLP
- Full TypeScript type inference — tree-sitter is a parser, not a type checker
- Cross-repo analysis — local-only constraint

## Context

**Shipped v1.0** with 9,353 LOC Rust across 33 source files. 89 tests passing.
**Tech stack:** Rust, tree-sitter, petgraph (StableGraph), oxc_resolver, rmcp, notify-debouncer-mini, bincode, tokio.
**Architecture:** CLI binary with 8 subcommands (index, find, refs, impact, circular, context, stats, watch, mcp). MCP stdio server with 6 tools. File watcher with 75ms debounce and incremental re-index pipeline. Bincode graph cache with atomic writes and staleness diff for cold starts.

## Constraints

- **Language:** Rust
- **Performance:** Must index a 10K-file TS codebase in under 30 seconds; incremental re-index under 100ms
- **Memory:** Background daemon should use <100MB RSS for typical projects
- **Dependencies:** Minimal — embedded storage, no external database servers
- **Distribution:** Single binary, zero runtime dependencies
- **Integration:** MCP protocol for Claude Code (standard, not custom)

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Rust over Go | Native tree-sitter bindings, petgraph algorithms, zero GC, performance ceiling | ✓ Good — 9.3K LOC, fast compilation, all targets met |
| MCP as integration method | Standard protocol for Claude Code, works across clients | ✓ Good — 6 tools via rmcp, zero-config |
| TS/JS only for v1 | Nail one language perfectly before expanding | ✓ Good — deep support with barrel resolution, path aliases |
| petgraph StableGraph | In-memory graph with stable node indices, serde support | ✓ Good — enables incremental updates without index invalidation |
| bincode for persistence | Fast serialization, compact binary format, atomic writes | ✓ Good — instant cold starts with staleness diff |
| Token-efficient compact format | Core differentiator — design output for LLM consumption | ✓ Good — 71% token savings vs JSON measured |
| oxc_resolver for import resolution | TypeScript-aware, handles tsconfig paths/extends/aliases | ✓ Good — covers barrel files, workspace packages |
| RwLock over Mutex for graph cache | Concurrent MCP tool reads during watcher updates | ✓ Good — clone-drop-work-swap pattern prevents lock contention |

---
*Last updated: 2026-02-23 after v1.0 milestone*
