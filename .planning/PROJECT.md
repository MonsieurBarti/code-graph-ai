# Code-Graph

## What This Is

A high-performance code intelligence engine that indexes TypeScript/JavaScript codebases into a queryable dependency graph. Built in a compiled language (Go or Rust — TBD during research), it replaces expensive file reads by giving Claude Code direct access to a pre-built structural graph via MCP, enabling smarter navigation and impact analysis while minimizing token consumption.

## Core Value

Claude Code can understand any codebase's structure and dependencies without reading source files — querying a local graph instead, saving tokens and time on every interaction.

## Requirements

### Validated

(None yet — ship to validate)

### Active

- [ ] Fast tree-sitter-based parsing of TypeScript/JavaScript codebases (seconds, not minutes)
- [ ] Dependency graph covering files, folders, functions, classes, imports, and call relationships
- [ ] Background watch mode that re-indexes incrementally on file changes
- [ ] MCP server exposing graph queries to Claude Code as native tools
- [ ] Impact analysis: "what breaks if I change X?"
- [ ] Smart navigation: jump to the right symbol without reading the file
- [ ] Token-efficient output format (compact structured data, maximum information per token)
- [ ] Low memory footprint suitable for always-on background daemon
- [ ] Embedded graph storage (no external database dependencies)
- [ ] CLI for direct developer use (query, inspect, debug the graph)

### Out of Scope

- Multi-language support beyond TS/JS — defer to v2 after nailing one language
- Cloud/remote features — everything runs locally
- Visual graph rendering/UI — Claude Code integration is the interface
- Real-time collaboration features — personal tool first
- AI-powered code summarization — focus on structural data, not NLP

## Context

**Inspiration:** [Axon](https://github.com/harshkedia177/axon) — a Python-based code intelligence engine using tree-sitter + KuzuDB + MCP. Good concept, but Python makes it slow and memory-heavy. Its 11-phase pipeline (file walking → structure → parsing → import resolution → call tracing → heritage → types → community detection → process detection → dead code → git coupling) is comprehensive but over-engineered for the core use case.

**Key Axon learnings to build on:**
- Tree-sitter is the right parsing foundation (bindings exist for Go and Rust)
- MCP integration is the right delivery mechanism for Claude Code
- Graph queries (impact, context, navigation) are the high-value tools
- Watch mode for live re-indexing is essential

**Key Axon weaknesses to address:**
- Python performance: slow indexing, high memory on large codebases
- Over-broad scope: 11-phase pipeline when core value is in 4-5 phases
- Token efficiency not prioritized: output format not optimized for LLM consumption
- TS/JS support is secondary (Python-first design)

**Integration with Claude Code:**
The tool will integrate as an MCP server, which is the standard protocol for extending Claude Code with external tools. This makes it appear as native tooling — Claude can call graph queries alongside file reads, grep, etc. The MCP approach means:
- No custom hooks or plugins needed
- Works with any MCP-compatible client (Claude Code, Cursor, etc.)
- Tools appear in Claude's tool list automatically
- Zero-config once the server is running

**Target codebases:** From small libraries (<1K files) to large production monorepos (10K+ files), all TypeScript/JavaScript.

**User's workflow:** Primarily Claude Code for development. The graph should be transparent — Claude just knows it exists and queries it instead of reading files for structural understanding.

## Constraints

- **Language:** Go or Rust (to be decided during research based on tree-sitter ecosystem, graph storage options, and MCP SDK maturity)
- **Performance:** Must index a 10K-file TS codebase in under 30 seconds; incremental re-index under 1 second
- **Memory:** Background daemon should use <100MB RSS for typical projects
- **Dependencies:** Minimal — embedded storage, no external database servers
- **Distribution:** Single binary, zero runtime dependencies
- **Integration:** MCP protocol for Claude Code (standard, not custom)

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Go vs Rust | Research needed: tree-sitter bindings maturity, graph DB options, MCP SDK availability, build complexity | — Pending |
| MCP as integration method | Standard protocol for Claude Code, works across clients, no custom plugin maintenance | — Pending |
| TS/JS only for v1 | Nail one language perfectly before expanding — user's primary stack | — Pending |
| Embedded graph storage | No external deps, single binary distribution, fast local queries | — Pending |
| Token-efficient output format | Core differentiator — design output for LLM consumption, not human readability | — Pending |

---
*Last updated: 2026-02-22 after initialization*
