# Feature Research

**Domain:** Code intelligence engine / dependency graph tool with MCP integration
**Researched:** 2026-02-22
**Confidence:** HIGH (multiple authoritative sources: Axon GitHub, Constellation docs, code-graph-mcp, dependency-cruiser, LSP spec, MCP ecosystem research)

---

## Feature Landscape

### Table Stakes (Users Expect These)

Features any code intelligence tool must have. Missing these = tool is not credible.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| **File-level import/dependency graph** | Every tool in the category does this (dependency-cruiser, madge, ts-graph all lead with it). Without it, you have no graph. | MEDIUM | Parse all `import`/`require`/`export` statements. Track module resolution via `tsconfig.json` path aliases. Must handle ESM, CJS, and dynamic imports. |
| **Symbol extraction (functions, classes, types)** | Users expect to ask "where is `FooService` defined?" — requires indexed symbols. ts-morph, Axon, and all LSP servers do this as a baseline. | MEDIUM | Extract: function decls, class decls, interfaces, type aliases, enums, exported constants. Needs name + file + line number minimum. |
| **Go-to-definition (symbol → location)** | LSP has standardized this expectation. Any code navigation tool that can't resolve `findDefinition(symbolName)` is incomplete. Claude Code's LSP integration (shipped Dec 2025) raised the bar. | MEDIUM | Given a symbol name or identifier, return file path + line. Must handle re-exports and barrel files. |
| **Find-all-references (who uses this symbol)** | Axon, Constellation, code-graph-mcp, LSP — all treat this as non-negotiable. AI agents need it to understand impact before editing. | MEDIUM | Return all usages of a symbol across the codebase. More useful than grep because it's semantic (knows `Foo` the class vs `foo` the variable). |
| **Circular dependency detection** | dependency-cruiser, madge, and typescript-graph all detect this. It's one of the top reasons developers reach for these tools. | LOW | Run cycle detection on the import graph (DFS). Return all cycles as arrays of module paths. |
| **Incremental re-indexing / watch mode** | Axon explicitly supports this; code-graph-mcp uses a debounced file watcher. Users expect the graph to stay current without full re-index. Without it, the graph goes stale within minutes. | HIGH | Watch filesystem events, invalidate affected nodes, re-parse only changed files and their dependents. Sub-1-second target for single-file changes. |
| **CLI query interface** | All competitor tools (dependency-cruiser, madge, Axon, code-graph-mcp) have CLI. Developers need to debug the graph directly and trust its output. | LOW | Commands: `index`, `query <symbol>`, `impact <symbol>`, `stats`. JSON output mode for scripting. |
| **MCP server with structured tool responses** | This is the delivery mechanism for Claude Code. Axon and Constellation both confirm MCP is how AI agents access code intelligence. Without MCP, the tool requires custom integration per client. | MEDIUM | Expose tools via MCP protocol. Each tool returns structured JSON. Tool descriptions must be concise (tool metadata inflates tokens — confirmed by arxiv 2602.14878v1). |
| **Caller/callee analysis (call graph)** | Axon's `axon_context`, Constellation's caller analysis, and code-graph-mcp's `find_callers`/`find_callees` all include this. AI agents need to understand "what calls this function" to reason about behavior. | HIGH | Requires parsing function call expressions, not just imports. Confidence scoring needed (dynamic calls are hard). Handled well by tree-sitter but still computationally expensive. |
| **Project statistics / codebase overview** | code-graph-mcp has `project_statistics`, Axon has `axon_list_repos`. AI agents need a fast "get the lay of the land" tool that doesn't require reading files. | LOW | Return: file count, symbol count, top-level module structure, language breakdown, largest files, most-imported modules. |

---

### Differentiators (Competitive Advantage)

Features that set this tool apart from Axon and the existing ecosystem. These are where we win.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| **Token-optimized output format** | Axon and code-graph-mcp return human-readable responses. This tool designs output for LLM consumption: minimal tokens, maximum signal. Sourcerer reports 95-98% token savings vs raw file reads. Our graph queries should deliver equivalent savings. | MEDIUM | Design a compact structured format: symbol records as flat arrays, not verbose JSON objects. Support a `compact` mode vs `verbose` mode. Strip whitespace and comments from code snippets when included. Return only what was asked for — no bloat. |
| **Impact analysis (blast radius)** | Axon has `axon_impact`, Constellation has `impactAnalysis`. BUT both are Python/cloud-hosted. We deliver this locally, sub-second, in a compiled binary. Constellation says impact analysis is "10-20x faster than text search" — our implementation should push further. | HIGH | Traverse the dependency graph upstream from a changed symbol. Return transitive dependents at configurable depth. Differentiate: direct dependents vs transitive. Flag when impact is "unbounded" (exported from barrel file imported everywhere). |
| **Sub-second incremental re-index** | Python-based Axon is slow on large codebases (its own acknowledged weakness). A Go/Rust implementation with proper file watching and graph diffing can achieve <100ms re-index for single-file changes in 10K-file repos. This is a primary reason to build in a compiled language. | HIGH | Use file hash-based change detection, not just mtime. Only re-traverse the affected subgraph. Cache parsed ASTs keyed by file hash. |
| **TypeScript-first accuracy** | Axon supports Python primarily, with TS/JS as secondary. dependency-cruiser and madge do imports only, not call graphs. ts-morph is comprehensive but library-only (no indexing, no server). We build for TS/JS first, getting edge cases right: path aliases, barrel files, `satisfies`, conditional types affecting exports. | HIGH | Use the TypeScript compiler API (via tree-sitter or `tsc` interface) to resolve types and re-exports accurately. Handle `index.ts` barrel files. Resolve `tsconfig.json` `paths` and `baseUrl`. |
| **Zero-dependency single binary** | Axon requires Python, KuzuDB, vector DB, and several heavy libraries. Our tool ships as one binary with embedded graph storage. This is a major DX win for teams that don't want to manage Python environments. | MEDIUM | Embed storage (bbolt, DuckDB, or similar). Statically link the tree-sitter grammar. Single `go install` or `cargo install` setup. |
| **Guided next-step tool hints** | Axon's most clever UX feature: every tool response includes "next step" suggestions (e.g., after `axon_context`, suggest calling `axon_impact`). This reduces AI agent hallucination and wasted tool calls. Worth adopting and extending. | LOW | Include a `next_steps` array in each tool response, listing relevant follow-up tool calls with suggested arguments. Helps AI agents navigate investigation workflows without re-planning. |
| **Dead code detection** | Axon has `axon_dead_code` but it requires their full 11-phase pipeline including community detection. We can build a simpler, faster version: symbols that are defined but never imported or called within the project. | MEDIUM | Track export/import counts per symbol. Flag symbols with zero inbound references that aren't framework entry points (e.g., not `main`, not route handlers). Framework-aware heuristics for TS: exported but unused = dead if not in a library. |

---

### Anti-Features (Commonly Requested, Often Problematic)

Features that seem like good ideas but should be explicitly deferred for v1.

| Feature | Why Requested | Why Problematic | Alternative |
|---------|---------------|-----------------|-------------|
| **Semantic vector search / embeddings** | Axon uses BM25 + vector + fuzzy search. Developers assume "smarter search" means AI embeddings. | Embeddings require a model, model loading time, and significant memory. This directly violates our <100MB memory constraint and zero-dependency goal. For structural graph queries (find callers, find definition), vector search adds nothing — the graph is already semantic. | Structural graph queries + exact name matching + fuzzy name fallback (Levenshtein). This covers 95% of AI agent needs at a fraction of the cost. |
| **AI-powered code summarization** | "Summarize what this module does" is a natural request. | Summarization requires calling an LLM or loading a local model — exactly the thing we're offloading FROM Claude. It also goes stale and adds latency. The goal is structural data, not NLP analysis. | Return structured metadata: what the module exports, what it imports, how many functions, who calls it. Let Claude summarize from that. |
| **Visual graph rendering / UI** | dependency-cruiser, madge, and typescript-graph all offer graph visualization. Developers expect it. | Visual output (SVG, Graphviz, D3.js) is entirely orthogonal to the AI agent use case. It's a separate product. Building it adds scope without improving token savings or AI accuracy. | Expose a `dot` format export (trivial to implement) that developers can pipe to Graphviz themselves. This satisfies the need without building a UI. |
| **Cross-repo analysis** | Sourcegraph's killer feature. Developers with monorepos or polyrepos want cross-repo impact analysis. | Cross-repo requires network access, remote indexing, auth, and distributed graph storage — all in conflict with the "local, single binary, no dependencies" constraint. | Support monorepos (single `index` command from the root). For genuine cross-repo needs, that's a v3 problem. |
| **Multi-language support (Python, Go, Rust, etc.)** | tree-sitter supports 100+ languages. Developers will ask for Python support immediately. | Each language requires: grammar tuning, module resolution rules, framework-aware entry point detection, and different call graph semantics. Axon does Python + TS and does neither perfectly. Doing one language excellently is the differentiator. | Explicitly document v1 as TS/JS only. Design the parser interface as an abstraction so languages can be added in v2. |
| **Git coupling / change co-occurrence analysis** | Axon's Phase 11. "Files that change together" is interesting for understanding hidden dependencies. | Requires reading full git history, which is slow and memory-intensive on large repos. Adds significant indexing time without clear AI agent use case. | Defer entirely. Git coupling is a developer insight tool, not a token-saving AI navigation tool. |
| **Community detection / module clustering** | Axon uses the Leiden algorithm to cluster related symbols. Sounds useful for "what module does this belong to?" | Graph clustering algorithms are computationally expensive and produce results that require significant interpretation. The output is not obviously useful for AI agent navigation queries. | Provide module/folder hierarchy as the natural clustering mechanism — it already exists in the filesystem. |
| **Real-time type inference / type checking** | Developers want "what type does this function return?" with full generic resolution. | Full TypeScript type checking via `tsc` requires loading the full compiler, which is slow and heavy. tree-sitter parses syntax, not type semantics. | Capture declared return types from AST (what the developer wrote). Flag when return type is `any` or inferred. Full type inference is a stretch goal for v1.x. |
| **LSP server mode** | "Why not just be an LSP server?" — natural ask since LSP covers go-to-definition and references. | LSP is editor-focused: it operates on a single open file with cursor position. MCP is AI-agent-focused: it answers codebase-wide structural questions. They're complementary but different protocols. Building both in v1 doubles the surface area. | Be an MCP server first. The MCP use case (AI agent context, token savings) is the differentiator. LSP can be added in v2 if demand exists. |

---

## Feature Dependencies

```
[File-level import graph]
    └──requires──> [Incremental re-index / watch mode] (graph must stay current)
    └──enables──> [Circular dependency detection] (trivial once graph exists)

[Symbol extraction]
    └──requires──> [File-level import graph] (need to know where symbols come from)
    └──enables──> [Go-to-definition]
    └──enables──> [Find-all-references]
    └──enables──> [Dead code detection]

[Call graph / caller-callee]
    └──requires──> [Symbol extraction] (need symbols to build call edges)
    └──enables──> [Impact analysis] (impact traverses call graph, not just imports)

[Impact analysis]
    └──requires──> [File-level import graph] (import edges)
    └──requires──> [Call graph] (call edges — for full blast radius)
    └──requires──> [Symbol extraction] (need to identify what changed)

[MCP server]
    └──requires──> all analysis features (server wraps them as tools)
    └──enhances──> [Token-optimized output format] (format is MCP response design)

[Incremental re-index]
    └──requires──> [File-level import graph] (know which dependents to re-process)
    └──requires──> [Symbol extraction] (invalidate symbols from changed files)

[Dead code detection]
    └──requires──> [Symbol extraction] (need all symbols)
    └──requires──> [Find-all-references] (need reference counts per symbol)
    └──enhances──> [Impact analysis] (dead symbols have zero impact)

[Project statistics]
    └──requires──> [File-level import graph] (for module structure)
    └──requires──> [Symbol extraction] (for symbol counts)

[Guided next-step hints]
    └──requires──> [MCP server] (hints are part of tool responses)
    └──enhances──> all MCP tools
```

### Dependency Notes

- **Call graph requires symbol extraction:** You cannot track "FooService calls BarRepository" without first knowing that both FooService and BarRepository are symbols in the index. Symbol extraction must come first.
- **Impact analysis requires both import graph AND call graph:** Import-only impact (file A imports file B, so B's changes affect A) misses call-level impact. Full blast radius requires traversing both edge types.
- **Incremental re-index requires understanding the dependency subgraph:** To know what to re-process after file X changes, you need the existing graph to find X's dependents. Bootstrap order matters: full index first, then incremental.
- **Dead code detection conflicts with LSP server mode:** An LSP server operates file-by-file and cannot easily maintain a global reference count. Building LSP in v1 would compromise dead code accuracy. Keeping MCP-only avoids this conflict.

---

## MVP Definition

### Launch With (v1)

Minimum viable product to validate that graph queries save tokens and improve AI accuracy.

- [ ] **File-level import/dependency graph** — Without this, there is no graph. Everything else depends on it.
- [ ] **Symbol extraction (functions, classes, types, exports)** — Needed to answer "where is X defined?" which is the most common AI agent query.
- [ ] **Go-to-definition** — Replaces the most common use of file reads: "read file to find where FooService is defined."
- [ ] **Find-all-references** — Enables AI to understand scope of change without reading every file.
- [ ] **Impact analysis (import-level blast radius)** — The core value proposition. "What breaks if I change X?" without reading files.
- [ ] **MCP server with 5-6 core tools** — Delivery mechanism. Without MCP, the tool doesn't integrate with Claude Code.
- [ ] **Incremental re-index with file watcher** — Without this, the graph goes stale in any active coding session. Non-negotiable for daily use.
- [ ] **Token-optimized output format** — Core differentiator. Every tool response should be benchmarked for token count vs information density.
- [ ] **CLI: `index` and `query` commands** — Needed for developer trust and debugging. Must be able to inspect the graph directly.

### Add After Validation (v1.x)

Features to add once core is working and being used daily.

- [ ] **Caller/callee analysis (call graph)** — Trigger: impact analysis proves valuable, users want deeper traversal than import-level.
- [ ] **Dead code detection** — Trigger: users ask "is this safe to delete?" as a follow-up to impact analysis.
- [ ] **Circular dependency detection** — Trigger: natural extension of the import graph, low implementation cost once graph exists.
- [ ] **Guided next-step hints in MCP responses** — Trigger: AI agents make suboptimal tool call sequences that could be guided.
- [ ] **Project statistics overview tool** — Trigger: users want a "get oriented" tool for unfamiliar codebases.
- [ ] **`dot` format export for graph visualization** — Trigger: developers want to see the graph for debugging.

### Future Consideration (v2+)

Features to defer until product-market fit is established.

- [ ] **Full call graph with confidence scoring** — Why defer: high implementation complexity, risk of false positives degrading trust in the tool.
- [ ] **Multi-language support** — Why defer: doing TS/JS right is the differentiator. Premature expansion dilutes quality.
- [ ] **LSP server mode** — Why defer: MCP is the primary integration target. LSP adds surface area without advancing the token-savings mission.
- [ ] **Cross-repo / monorepo federation** — Why defer: requires distributed graph storage and is architecturally complex.
- [ ] **Git coupling analysis** — Why defer: high indexing cost, unclear AI agent value vs implementation complexity.

---

## Feature Prioritization Matrix

| Feature | User Value | Implementation Cost | Priority |
|---------|------------|---------------------|----------|
| File-level import graph | HIGH | MEDIUM | P1 |
| Symbol extraction | HIGH | MEDIUM | P1 |
| Go-to-definition | HIGH | LOW | P1 |
| Find-all-references | HIGH | MEDIUM | P1 |
| Impact analysis (import-level) | HIGH | MEDIUM | P1 |
| MCP server (core tools) | HIGH | MEDIUM | P1 |
| Incremental re-index / watch mode | HIGH | HIGH | P1 |
| Token-optimized output format | HIGH | LOW | P1 |
| CLI (index + query) | MEDIUM | LOW | P1 |
| Caller/callee call graph | HIGH | HIGH | P2 |
| Dead code detection | MEDIUM | MEDIUM | P2 |
| Circular dependency detection | MEDIUM | LOW | P2 |
| Guided next-step hints | MEDIUM | LOW | P2 |
| Project statistics tool | MEDIUM | LOW | P2 |
| Dot format export | LOW | LOW | P2 |
| Full TypeScript type resolution | MEDIUM | HIGH | P3 |
| Multi-language support | LOW | HIGH | P3 |
| LSP server mode | LOW | HIGH | P3 |
| Git coupling analysis | LOW | HIGH | P3 |
| Vector/semantic search | LOW | HIGH | P3 |

**Priority key:**
- P1: Must have for launch (v1)
- P2: Should have, add after validation (v1.x)
- P3: Nice to have, future consideration (v2+)

---

## Competitor Feature Analysis

| Feature | Axon | dependency-cruiser | Constellation | code-graph-mcp | Our Approach |
|---------|------|-------------------|---------------|----------------|--------------|
| Import graph | Yes (TS + Python) | Yes (TS/JS only) | Yes | Yes | Yes — TS/JS first, highest accuracy |
| Symbol extraction | Yes | No | Yes | Yes | Yes — all exportable symbols |
| Go-to-definition | Yes (`axon_query`) | No | Yes | Yes (`find_definition`) | Yes — MCP tool with compact response |
| Find-all-references | Yes (`axon_context`) | No | Yes | Yes (`find_references`) | Yes — returns file:line pairs only |
| Impact analysis | Yes (`axon_impact`) | No | Yes | No | Yes — primary differentiator |
| Caller/callee | Yes (`axon_context`) | No | Yes | Yes | v1.x — after import graph validated |
| Dead code | Yes (`axon_dead_code`) | No | Yes | No | v1.x |
| Circular dep detection | No (not documented) | Yes | Yes | No | v1.x — trivial once graph exists |
| Watch mode | Yes | No | Unclear | Yes (debounced) | Yes — <1 second re-index target |
| MCP integration | Yes (7 tools) | No | Yes | Yes (9 tools) | Yes — 6 tools v1, guided hints |
| Performance | Slow (Python) | Fast (JS) | Managed cloud | Moderate (JS) | Fast — Go/Rust compiled binary |
| Token-optimized output | No | No | Partial | No | Yes — core design principle |
| Single binary | No (Python + deps) | No (Node) | No (cloud) | No (Node) | Yes — zero runtime deps |
| Local only | Yes | Yes | No (cloud) | Yes | Yes — privacy first |
| CLI | Yes | Yes | No | No | Yes — for developer trust |
| Vector search | Yes (3-way fusion) | No | No | No | No — anti-feature for v1 |
| Visual output | No | Yes (DOT/SVG) | No | No | DOT export only (v1.x) |
| Multi-language | Yes (Py + TS/JS) | Yes (JS variants) | Unclear | Yes (25+) | No — TS/JS only v1 |

---

## Sources

- Axon MCP tools and pipeline: [Glama MCP listing for Axon](https://glama.ai/mcp/servers/@harshkedia177/axon) (HIGH confidence — official listing)
- Axon GitHub features: [GitHub harshkedia177/axon](https://github.com/harshkedia177/axon) (HIGH confidence)
- mcp-server-tree-sitter features: [GitHub wrale/mcp-server-tree-sitter FEATURES.md](https://github.com/wrale/mcp-server-tree-sitter/blob/main/FEATURES.md) (HIGH confidence)
- code-graph-mcp tools: [GitHub entrepeneur4lyf/code-graph-mcp](https://github.com/entrepeneur4lyf/code-graph-mcp) (HIGH confidence)
- Constellation code intelligence: [constellationdev.io](https://constellationdev.io/) (MEDIUM confidence — marketing site, features verified against Axon patterns)
- dependency-cruiser features: [GitHub sverweij/dependency-cruiser](https://github.com/sverweij/dependency-cruiser) (HIGH confidence)
- Madge features: [npm madge](https://www.npmjs.com/package/madge) (HIGH confidence)
- LSP capabilities and Claude Code LSP integration: [LSP official page](https://microsoft.github.io/language-server-protocol/) + [Claude Code LSP Dec 2025](https://www.aifreeapi.com/en/posts/claude-code-lsp) (HIGH confidence for LSP spec; MEDIUM for Claude Code integration)
- Token savings with structured MCP responses: [Anthropic code execution with MCP](https://www.anthropic.com/engineering/code-execution-with-mcp) (HIGH confidence)
- MCP tool description token overhead: [arxiv 2602.14878v1](https://arxiv.org/html/2602.14878v1) — "MCP Tool Descriptions Are Smelly" (MEDIUM confidence — recent preprint)
- Sourcerer 95-98% token savings: [Skywork AI - Sourcerer MCP](https://skywork.ai/skypage/en/sourcerer-mcp-server-ai-engineer-code-search/1979038890849849344) (LOW confidence — single source, verify claim)
- ts-morph features: [ts-morph.com](https://ts-morph.com/) + [GitHub dsherret/ts-morph](https://github.com/dsherret/ts-morph) (HIGH confidence)

---

*Feature research for: code intelligence engine / dependency graph tool (MCP-integrated, TS/JS)*
*Researched: 2026-02-22*
