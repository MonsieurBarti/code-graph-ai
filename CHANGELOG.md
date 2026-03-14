# Changelog

All notable changes to code-graph are documented in this file.

## v3.0.0 (2026-03-14)

**Added:**
- Clone detection: identify structurally identical code via signature hashing across the codebase
- Background daemon with IPC (`daemon start|stop|status`) for persistent indexing without blocking the CLI
- Claude Code hooks integration (PreToolUse) replacing MCP-based tool dispatch
- Multi-project registry with aliases (`project add|remove|list|show`) and `--project` flag on all query commands
- Project auto-detection from current working directory (no explicit path required)
- `setup --uninstall` flag to cleanly remove hooks and permissions

**Changed:**
- CLI-first architecture with 25 subcommands: index, find, refs, impact, circular, stats, context, watch, snapshot, serve, export, structure, file-summary, imports, dead-code, diff, diff-impact, decorators, clusters, flow, project, setup, daemon, daemon-run, rename
- Token-optimized compact output moved from MCP tool responses to native CLI `--format compact` (default on all query commands)

**Removed:**
- MCP server and stdio transport (replaced by Claude Code hooks)
- `rmcp` dependency and all MCP tool definitions
- MCP configuration section (`[mcp]`) from code-graph.toml

## v2.0.0 (2026-03-03)

**Added:**
- Python language support (functions, classes, decorators, imports, PEP 695 type aliases)
- Go language support (functions, methods, struct tags, `//go:` directives, go.mod resolution)
- Decorator/attribute extraction across all 5 languages with framework detection
- Interactive web UI -- `code-graph serve` with Svelte + WebGL graph visualization
- RAG conversational agent with hybrid retrieval and multi-provider LLM support
- BM25 full-text search with tiered exact → trigram → BM25 → RRF pipeline
- 5 new MCP tools: `find_by_decorator`, `find_clusters`, `trace_flow`, `plan_rename`, `get_diff_impact`
- Confidence scoring (High/Medium/Low) on impact analysis results
- `setup` command for auto-configuring Claude Code, Cursor, and Windsurf
- `serve` command for web UI with `--port` and `--ollama` options
- Claude Code skills bundle and PreToolUse enrichment hook
- Feature flags: `web` and `rag`

**Changed:**
- Cache version 4 → 6 (auto-rebuilds on first run)
- SymbolInfo extended with `line_end`, `decorators`, framework labels
- New edge kinds: HasDecorator, ChildOf, Embeds, Implements

## v1.2.0 (2026-02-26)

**Added:**
- Zero-config MCP server (defaults to cwd)
- 5 new MCP tools: `get_structure`, `get_file_summary`, `get_imports`, `find_dead_code`, `get_diff`
- Batch queries -- up to 10 queries in single MCP call
- Trigram Jaccard fuzzy matching for typo-tolerant symbol search
- Multi-project server support (`register_project`, `list_projects`)
- Section-scoped `get_context` for 60-80% token savings
- Graph snapshot/diff system
- `[mcp]` config section in code-graph.toml

## v1.1.0 (2026-02-25)

**Added:**
- Rust language support (functions, structs, enums, traits, impl blocks, macros, visibility)
- Rust module resolution with crate-root walk and use-path classification
- Graph export to DOT and Mermaid at symbol/file/package granularity
- `export` and `snapshot` CLI commands

## v1.0.0 (2026-02-23)

**Added:**
- TypeScript/JavaScript parsing with tree-sitter (functions, classes, interfaces, types, enums, components)
- Import resolution with tsconfig paths, barrel files, monorepo workspaces
- Dependency graph with file-level and symbol-level edges
- 6 CLI commands: `index`, `find`, `refs`, `impact`, `circular`, `context`, `stats`
- MCP server with 6 tools over stdio
- File watcher with incremental re-indexing
- Bincode disk cache for fast cold starts
- Token-optimized compact output format
