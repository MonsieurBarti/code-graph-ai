# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-22)

**Core value:** Claude Code can understand any codebase's structure and dependencies without reading source files — querying a local graph instead, saving tokens and time on every interaction.
**Current focus:** Phase 5 — Watch Mode Persistence

## Current Position

Phase: 5 of 6 (Watch Mode Persistence)
Plan: 1 of 3 in current phase
Status: Executing
Last activity: 2026-02-23 — Completed 05-01-PLAN.md (graph serialization + cache persistence layer)

Progress: [██████████] 60%

## Performance Metrics

**Velocity:**
- Total plans completed: 11
- Average duration: 17 min
- Total execution time: ~3.0 hours

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 01 | 3 completed (DONE) | 64 min | 21 min |
| 02 | 4 completed (DONE) | 73 min | 18 min |
| 03 | 3 completed (DONE) | 139 min | 46 min |

**Recent Trend:**
- Last 5 plans: 02-03 (38 min), 02-04 (25 min), 03-01 (7 min), 03-02 (5 min)
- Trend: Infrastructure/extension plans fast (~5-7min); integration plans ~25-38min

*Updated after each plan completion*
| Phase 01 P01 | 4 | 2 tasks | 5 files |
| Phase 01 P02 | 29 | 2 tasks | 7 files |
| Phase 01 P03 | 31 | 2 tasks | 4 files |
| Phase 02 P01 | 5 | 2 tasks | 8 files |
| Phase 02 P02 | 5 | 1 tasks | 2 files |
| Phase 02 P03 | 38 | 2 tasks | 4 files |
| Phase 02 P04 | 25 | 2 tasks | 2 files |
| Phase 03 P01 | 7 | 2 tasks | 7 files |
| Phase 03 P02 | 5 | 2 tasks | 7 files |
| Phase 03 P03 | 127 | 1 tasks | 5 files |
| Phase 04 P01 | 3 | 2 tasks | 5 files |
| Phase 04 P02 | 23 | 2 tasks | 4 files |
| Phase 05 P01 | 3 | 2 tasks | 7 files |

## Accumulated Context

### Decisions

Decisions are logged in PROJECT.md Key Decisions table.
Recent decisions affecting current work:

- [Research]: Language is Rust — wins on tree-sitter native bindings, petgraph graph algorithms, zero GC memory, performance ceiling
- [Research]: Core stack confirmed: tree-sitter + petgraph + rmcp + notify + rkyv + tokio
- [Research]: Import resolution is the highest-risk area — barrel files, path aliases, monorepo workspaces all need correct handling in Phase 2
- [01-01]: Use require_git(false) on WalkBuilder so .gitignore is respected even in non-git directories
- [01-01]: Walk from project root only (not workspace subdirs separately) to avoid duplicate file discovery
- [01-01]: Hard-exclude node_modules via path component check — not relying on .gitignore entry
- [01-01]: Verbose output goes to stderr (not stdout) so stdout is clean for piping --json output
- [01-02]: detect_export() checks sym_node itself first (not parent) — @symbol capture IS export_statement for arrow-fn patterns
- [01-02]: Language::name() used for grammar identification — version() does not exist in tree-sitter 0.26
- [01-02]: OnceLock<Query> static per language — compiled once, reused across all files
- [01-02]: De-duplicate symbols by (name, row) to handle overlapping query patterns
- [01-02]: tree-sitter 0.26 QueryMatches uses StreamingIterator (not standard Iterator) — must import tree_sitter::StreamingIterator
- [01-03]: Language::name() returns None for TypeScript and TSX grammars in tree-sitter 0.26 — use is_tsx bool param derived from file extension for TS/TSX discrimination (not Language::name())
- [01-03]: All extractor functions (symbols, imports, exports) use is_tsx: bool as 4th parameter for per-grammar OnceLock selection
- [01-03]: tree-sitter 0.26 StreamingIterator does not auto-filter #eq? predicates — filter function name in Rust code instead
- [01-03]: tree-sitter namespace_import identifier has no field name — find by child kind, not child_by_field_name()
- [02-01]: oxc_resolver = "3" (edition 2021 compatible) used — 11.x series requires features not needed here
- [02-01]: Rust toolchain upgraded from 1.84.1 to stable 1.93.1 to support edition = "2024" in Cargo.toml
- [02-01]: pnpm-workspace.yaml parsed with minimal 20-line line parser — no serde_yaml added
- [02-01]: ExternalPackage nodes deduplicated by package name via external_index HashMap on CodeGraph
- [02-01]: builtin_modules: true enabled on Resolver so Node.js builtins classify as BuiltinModule not Unresolved
- [02-01]: workspace source-preferred mapping: src/ dir used when it exists, otherwise package root
- [Phase 02]: JS grammar (tree-sitter-javascript 0.25) uses different class_heritage layout — no extends_clause node, requires separate grammar-specific query
- [Phase 02]: extends_type_clause confirmed as correct node name for interface extends in TS grammar (validated via live tree exploration)
- [Phase 02]: from_name is None for Calls/MethodCall/TypeReference in context-free extraction pass — caller scope resolution deferred to Plan 03 graph wiring
- [02-03]: Barrel pass uses parse_results HashMap lookup instead of second oxc_resolver call — faster and avoids resolver API complexity for already-indexed files
- [02-03]: External package classification based on specifier prefix (not . and not /) — workspace aliases handled upstream by resolver
- [02-03]: Symbol relationship pass skips ambiguous multi-candidate calls — cross-file call ambiguity documented limitation (research Open Question 3)
- [02-03]: petgraph::visit::EdgeRef trait must be explicitly imported for .target() on EdgeReference
- [02-04]: Named re-export map built as pre-pass from parse_results before scanning edges — avoids combined borrow issues during graph mutation
- [02-04]: Collect-then-mutate: candidates collected from graph edges into Vec first, then graph mutated in second pass — required by Rust borrow checker
- [02-04]: ImportSpecifier.alias holds original exported name when aliased (import { Foo as F }) — use alias.as_deref().unwrap_or(&name) to get name matching barrel exports
- [02-04]: named_reexport_edges field on ResolveStats is diagnostic only — not surfaced in IndexStats user-facing output
- [Phase 03]: Symbol before path positional arg order — matches documented CLI: code-graph find <symbol> <path>
- [Phase 03]: find_containing_file() uses edges_directed() filtering to EdgeKind::Contains only — prevents Calls(File->Symbol) edges from being misidentified as the containing file
- [Phase 03]: build_graph() returns CodeGraph only — Index command keeps its own inline pipeline for stats computation
- [03-02]: match_symbols() helper in find.rs collects all NodeIndices for regex-matched symbols — used by refs and impact to avoid duplication
- [03-02]: blast_radius() uses custom BFS on incoming edges (not petgraph Bfs + Reversed) to filter to ResolvedImport edges only
- [03-02]: find_circular() builds temporary non-stable petgraph::Graph for kosaraju_scc — file-only nodes + ResolvedImport edges only
- [03-02]: IntoEdgeReferences trait must be explicitly imported for edge_references() on StableGraph
- [03-02]: edge_ref.weight() used instead of graph.graph[edge_ref.id()] for EdgeKind matching — avoids type inference issue
- [Phase 03]: symbol_context() walks both symbol-to-symbol Calls edges AND file-to-symbol Calls from parent file — required by Phase 2 resolver's file-level Calls emission
- [Phase 03]: FindResult needed Clone derive for storage in SymbolContext.definitions — added #[derive(Clone)] to FindResult
- [Phase 04-01]: format_*_to_string functions write summary header FIRST per CONTEXT.md locked decision
- [Phase 04-01]: Converted main() to #[tokio::main] async fn — cleaner than block_on approach
- [Phase 04-01]: format_impact_to_string uses flat format (no tree) for MCP — reduces parsing ambiguity
- [Phase 04-01]: format_context_to_string uses labeled section delimiters per CONTEXT.md locked decision
- [Phase 04-02]: Return Result<String, String> from tool handlers — Err maps to isError:true via IntoCallToolResult blanket impl
- [Phase 04-02]: #[tool_handler] annotation required on ServerHandler impl to wire call_tool/list_tools/get_tool
- [Phase 04-02]: Graph built in spawn_blocking to avoid blocking async executor during CPU-bound graph construction
- [05-01]: bincode 2 with serde feature used — all graph type serialization goes through serde derives (not bincode derive macros)
- [05-01]: Atomic cache write via NamedTempFile::new_in(cache_dir) + persist() rename — prevents corrupt cache on crash
- [05-01]: CACHE_VERSION = 1 constant — load_cache returns None on version mismatch, triggers full rebuild (safe degradation)
- [05-01]: remove_file_from_graph collects Contains/ChildOf edges then removes in second pass — collect-then-mutate avoids borrow checker issues
- [05-01]: Rust 2024 edition: explicit ref binding in if-let patterns removed — implicit borrowing used instead

### Pending Todos

None.

### Blockers/Concerns

- [Research flag - RESOLVED]: rmcp 0.16 API verified during Phase 4 implementation — #[tool_router] + #[tool_handler] macros work correctly
- [Research flag - RESOLVED]: rkyv integration with petgraph — using bincode + serde instead (simpler, no custom derives needed, serde-1 feature on petgraph handles StableGraph)
- [Research flag - RESOLVED]: tree-sitter TypeScript grammar handles latest TS features — verified during 01-02 implementation

## Session Continuity

Last session: 2026-02-23
Stopped at: Completed 05-01-PLAN.md
Resume file: .planning/phases/05-watch-mode-persistence/05-01-SUMMARY.md
