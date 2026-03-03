# code-graph

High-performance code intelligence engine that indexes TypeScript, JavaScript, Rust, Python, and Go codebases into a queryable dependency graph. Built in Rust, designed for AI agents.

Gives [Claude Code](https://docs.anthropic.com/en/docs/claude-code) direct access to your codebase's structure via [MCP](https://modelcontextprotocol.io/) -- no source file reading needed. Twenty MCP tools cover symbol search, reference tracing, blast radius analysis, circular dependency detection, dead code detection, decorator search, clustering, call chain tracing, rename planning, diff impact, graph export, batch queries, snapshot/diff, and multi-project management.

## Features

- **Multi-language parsing** -- TypeScript, TSX, JavaScript, JSX, Rust, Python, and Go via tree-sitter with full symbol extraction (functions, classes, interfaces, types, enums, components, methods, properties, structs, traits, impl blocks, macros, pub visibility, async/sync functions, decorators, type aliases, struct tags)
- **Python parsing** -- functions (sync/async), classes, variables, type aliases (PEP 695), decorators with framework detection (Flask, FastAPI, Django)
- **Go parsing** -- functions, methods, type specs, struct tags, `//go:` directives as decorators, visibility by export convention, go.mod resolution
- **Decorator/attribute extraction** -- unified across all 5 languages with framework inference (NestJS, Flask, FastAPI, Actix, Angular)
- **Dependency graph** -- file-level and symbol-level edges: imports, calls, extends, implements, type references, has-decorator, child-of, embeds
- **Import resolution** -- TypeScript path aliases (tsconfig.json), barrel files (index.ts re-exports), monorepo workspaces, Rust crate-root module resolution with Cargo workspace discovery, Python package resolution, Go module resolution
- **Twenty query types** -- find definitions, trace references, blast radius analysis, circular dependency detection, 360-degree symbol context, project statistics, graph export, file structure, file summaries, import analysis, batch queries, dead code detection, graph diff, project registration, project listing, decorator search, clustering, call chain tracing, rename planning, diff impact
- **Interactive web UI** -- `code-graph serve` launches an Axum backend + Svelte frontend with WebGL graph visualization, file tree, code panel, search, and real-time WebSocket updates
- **RAG conversational agent** -- hybrid retrieval (structural graph + vector embeddings), multi-provider LLM support (Claude, OpenAI, Ollama), session memory, source citations
- **BM25 hybrid search** -- tiered pipeline: exact match → trigram fuzzy → BM25 → Reciprocal Rank Fusion
- **Confidence scoring** -- High/Medium/Low confidence tiers on impact analysis based on graph distance
- **MCP server** -- zero-config (defaults to cwd), exposes all queries as tools for Claude Code over stdio, optional `--watch` flag for auto-reindex
- **Editor auto-setup** -- `code-graph setup` configures MCP for Claude Code, Cursor, and Windsurf
- **Trigram fuzzy matching** -- Jaccard similarity for typo-tolerant symbol search with score-ranked suggestions
- **Batch queries** -- `batch_query` runs up to 10 queries in a single MCP call with single graph resolution
- **Dead code detection** -- `find_dead_code` identifies unreferenced symbols with entry-point exclusions
- **Graph snapshot/diff** -- create named snapshots and compare current graph state against baselines
- **Multi-project support** -- `register_project` and `list_projects` for managing multiple codebases from a single MCP server
- **Section-scoped context** -- `get_context` with targeted sections for 60-80% token savings per query
- **Graph export** -- DOT and Mermaid formats at symbol, file, or package granularity
- **Non-parsed file awareness** -- config files, docs, and assets visible in the graph
- **File watcher** -- incremental re-indexing on file changes with 75ms debounce
- **Disk cache** -- bincode serialization for instant cold starts
- **Token-optimized output** -- compact prefix-free format with context-aware next-step hints, designed for AI agent consumption (60-90% savings per session)
- **[mcp] config section** -- persistent project-level MCP defaults in `code-graph.toml`
- **Feature flags** -- `--features web` for web UI, `--features rag` for RAG agent

## Install

```bash
cargo install code-graph-cli
```

This installs the `code-graph` binary to `~/.cargo/bin/`.

To include the web UI or RAG agent, enable feature flags:

```bash
cargo install code-graph-cli --features web    # Web UI
cargo install code-graph-cli --features rag    # RAG agent (includes web)
```

### From source

```bash
git clone https://github.com/MonsieurBarti/code-graph-ai.git
cd code-graph-ai
cargo install --path .
```

### Build manually

```bash
git clone https://github.com/MonsieurBarti/code-graph-ai.git
cd code-graph-ai
cargo build --release
# Binary at target/release/code-graph
```

Requires Rust 1.85+ (edition 2024). No runtime dependencies -- tree-sitter grammars are statically linked.

## Quick start

```bash
# Index a TypeScript/JavaScript project
code-graph index /path/to/ts-project

# Index a Rust project
code-graph index /path/to/rust-project

# Index a Python project
code-graph index /path/to/python-project

# Index a Go project
code-graph index /path/to/go-project

# Find a symbol
code-graph find "UserService" /path/to/project

# What breaks if I change this?
code-graph impact "DatabaseConfig" /path/to/project

# Start MCP server for Claude Code (zero-config, defaults to cwd)
code-graph mcp

# Start MCP server with auto-reindex on file changes
code-graph mcp /path/to/project --watch

# Launch the interactive web UI
code-graph serve

# Export dependency graph as Mermaid at package granularity
code-graph export . --format mermaid --granularity package

# Create a named graph snapshot for later comparison
code-graph snapshot create baseline .

# Auto-configure MCP for your editor
code-graph setup
```

## CLI reference

```
Usage: code-graph <COMMAND>

Commands:
  index     Index a project directory
  find      Find a symbol's definition (file:line location)
  refs      Find all references to a symbol across the codebase
  impact    Show the transitive blast radius of changing a symbol
  circular  Detect circular dependencies in the import graph
  stats     Project statistics overview
  context   360-degree view of a symbol: definition, references, callers, callees
  mcp       Start an MCP stdio server exposing graph queries as tools
  watch     Start a file watcher for incremental re-indexing
  export    Export dependency graph to DOT or Mermaid format
  snapshot  Create, list, or delete named graph snapshots
  setup     Auto-configure MCP for Claude Code, Cursor, or Windsurf
  serve     Launch the interactive web UI (requires --features web)
```

### index

Index a project, discovering and parsing all TypeScript/JavaScript, Rust, Python, and Go files.

```bash
code-graph index . --verbose    # Print each discovered file
code-graph index . --json       # Output as JSON
```

### find

Find symbol definitions by name or regex pattern. Supports trigram fuzzy matching for typo-tolerant search.

```bash
code-graph find "UserService" .
code-graph find "User.*Service" . -i            # Case-insensitive regex
code-graph find "authenticate" . --kind function # Filter by kind
code-graph find "Button" . --file src/components # Scope to directory
```

Symbol kinds: `function`, `class`, `interface`, `type`, `enum`, `variable`, `component`, `method`, `property`, `struct`, `trait`, `impl`, `macro`

### refs

Find all files and call sites that reference a symbol.

```bash
code-graph refs "UserService" .
code-graph refs "useAuth" . --format table    # Human-readable table
```

### impact

Show the transitive blast radius -- everything affected if a symbol changes.

```bash
code-graph impact "DatabaseConfig" .
code-graph impact "API" . --tree              # Hierarchical dependency chain
```

### circular

Detect circular dependency cycles in the import graph (file-level).

```bash
code-graph circular .
code-graph circular . --format json
```

### stats

Project overview: file count, symbol breakdown by kind, import summary.

```bash
code-graph stats .
code-graph stats . --format json
```

### context

360-degree view combining definition, references, callers, and callees. Supports section scoping for targeted queries with 60-80% token savings.

```bash
code-graph context "Logger" .
```

### mcp

Start an MCP stdio server exposing graph queries as tools for Claude Code.

```bash
code-graph mcp                          # Zero-config: defaults to cwd
code-graph mcp /path/to/project         # Specify project path
code-graph mcp /path/to/project --watch # Auto-reindex on file changes
```

### watch

Start a standalone file watcher that re-indexes incrementally on changes.

```bash
code-graph watch .
```

> The `mcp` command starts its own embedded watcher automatically when `--watch` is passed -- you don't need to run `watch` separately.

### export

Export the dependency graph to DOT or Mermaid format at symbol, file, or package granularity.

```bash
code-graph export . --format dot --granularity symbol
code-graph export . --format mermaid --granularity package
code-graph export . --format dot --granularity file --max-nodes 200 --max-edges 500
```

### snapshot

Create, list, or delete named graph snapshots for change tracking and comparison.

```bash
code-graph snapshot create baseline .    # Create a snapshot named "baseline"
code-graph snapshot list .               # List all snapshots
code-graph snapshot delete baseline .    # Delete the "baseline" snapshot
```

### setup

Auto-configure MCP integration for your editor.

```bash
code-graph setup              # Interactive: detects available editors
```

### serve

Launch the interactive web UI with graph visualization.

```bash
code-graph serve                        # Default port 3000
code-graph serve --port 8080            # Custom port
code-graph serve --ollama               # Enable Ollama for local RAG
```

> Requires building with `--features web`. Add `--features rag` for the conversational agent.

### Output formats

All query commands support `--format`:

| Format | Description |
|--------|-------------|
| `compact` | One-line-per-result, token-optimized (default) |
| `table` | Human-readable columns with ANSI colors |
| `json` | Structured JSON for programmatic use |

## MCP integration

### Claude Code setup

```bash
claude mcp add --scope user code-graph -- code-graph mcp --watch
```

Or use the auto-setup command:

```bash
code-graph setup
```

This registers `code-graph` as a user-scoped MCP server available in all your projects. The `--watch` flag enables auto-reindex on file changes.

### Available tools

Once connected, Claude Code gets access to twenty tools:

| Tool | Description |
|------|-------------|
| `find_symbol` | Find symbol definitions by name or regex |
| `find_references` | Find all files and call sites referencing a symbol |
| `get_impact` | Get the transitive blast radius of changing a symbol |
| `detect_circular` | Detect circular dependency cycles |
| `get_context` | 360-degree view: definition + references + callers + callees |
| `get_stats` | Project overview: files, symbols, imports |
| `export_graph` | Export dependency graph to DOT or Mermaid format |
| `get_structure` | File/directory tree with symbol counts |
| `get_file_summary` | Compact summary of a file's symbols and imports |
| `get_imports` | List imports for a file or across the project |
| `batch_query` | Run up to 10 queries in a single call |
| `find_dead_code` | Detect unreferenced symbols with entry-point exclusions |
| `get_diff` | Compare current graph against a named snapshot |
| `register_project` | Register an additional project for multi-project queries |
| `list_projects` | List all registered projects |
| `find_by_decorator` | Find symbols by decorator/attribute pattern |
| `find_clusters` | Hierarchical clustering by coupling/cohesion |
| `trace_flow` | Find call chains between two symbols |
| `plan_rename` | Plan symbol renames with impact analysis |
| `get_diff_impact` | Git-diff-based impact analysis |

The MCP server loads from disk cache on startup for near-instant cold starts, runs an embedded file watcher for live updates (with `--watch`), and suggests similar symbol names via trigram fuzzy matching when a search yields no results.

### Recommended CLAUDE.md instructions

Claude Code defaults to reading source files with its built-in glob/grep/read tools. Without explicit guidance, it won't use code-graph even when the MCP server is running. Add the following to your project's `CLAUDE.md` so Claude uses graph queries instead of file reading for codebase navigation:

```markdown
## Code navigation -- MANDATORY

NEVER use Grep or Glob to find symbol definitions, trace references, or analyze dependencies.
ALWAYS use code-graph MCP tools instead -- they are faster, more accurate, and understand the full AST.

| Task | Tool | NOT this |
|------|------|----------|
| Find where something is defined | `find_symbol` | ~~Grep for `class X`, `function X`, `fn X`~~ |
| Find what uses/imports something | `find_references` | ~~Grep for `import`, `require`, identifier~~ |
| Understand a symbol fully | `get_context` | ~~Multiple Grep + Read calls~~ |
| Check what breaks if I change X | `get_impact` | ~~Manual file-by-file tracing~~ |
| Detect circular deps | `detect_circular` | ~~Grep for import cycles~~ |
| Project overview | `get_stats` | ~~Glob + count files~~ |

Use Read/Grep/Glob ONLY for:
- Reading full file contents before editing
- Searching for string literals, comments, TODOs, error messages
- Non-structural text searches that have nothing to do with code navigation
```

### Permission whitelisting

By default, Claude Code asks for confirmation on every MCP tool call. To auto-approve code-graph tools (they are read-only and safe), add this to `.claude/settings.json` in your project root:

```json
{
  "permissions": {
    "allow": [
      "mcp__code-graph__find_symbol",
      "mcp__code-graph__find_references",
      "mcp__code-graph__get_impact",
      "mcp__code-graph__detect_circular",
      "mcp__code-graph__get_context",
      "mcp__code-graph__get_stats",
      "mcp__code-graph__export_graph",
      "mcp__code-graph__get_structure",
      "mcp__code-graph__get_file_summary",
      "mcp__code-graph__get_imports",
      "mcp__code-graph__batch_query",
      "mcp__code-graph__find_dead_code",
      "mcp__code-graph__get_diff",
      "mcp__code-graph__register_project",
      "mcp__code-graph__list_projects",
      "mcp__code-graph__find_by_decorator",
      "mcp__code-graph__find_clusters",
      "mcp__code-graph__trace_flow",
      "mcp__code-graph__plan_rename",
      "mcp__code-graph__get_diff_impact"
    ]
  }
}
```

## Configuration

Optional `code-graph.toml` in your project root:

```toml
[exclude]
paths = ["vendor/", "dist/", "build/"]

[mcp]
default_format = "compact"  # Output format: compact, table, json
watch = true                # Auto-start file watcher
```

By default, code-graph respects `.gitignore` patterns and always excludes `node_modules/` and `target/`.

## How it works

1. **Walk** -- discovers TS/JS, Rust, Python, and Go files respecting `.gitignore` and exclusion rules
2. **Parse** -- tree-sitter extracts symbols, imports, exports, and relationships from each file. TypeScript/JavaScript parsing covers functions, classes, interfaces, type aliases, enums, and components. Rust parsing covers functions, structs, enums, traits, impl blocks, type aliases, constants, statics, and macro definitions with visibility tracking. Python parsing covers functions (sync/async), classes, variables, type aliases (PEP 695), and decorators. Go parsing covers functions, methods, type specs, struct tags, and `//go:` directives.
3. **Resolve** -- maps import specifiers to actual files. For TypeScript/JavaScript: oxc_resolver handles path aliases, barrel files, and workspaces. For Rust: crate-root module tree walk with use-path classification (crate/super/self/external/builtin) and Cargo workspace discovery. For Python: package resolution with `__init__.py` detection and relative imports. For Go: go.mod module resolution with package path mapping.
4. **Build graph** -- constructs a petgraph with file nodes, symbol nodes, and typed edges (imports, calls, extends, implements, type references, has-decorator, child-of, embeds)
5. **Cache** -- serializes the graph to disk with bincode for fast reloads
6. **Query** -- traverses the graph to answer structural questions without reading source files
7. **Watch** -- monitors filesystem events and incrementally updates the graph (re-parses only changed files)

## Project stats

| Metric | Value |
|--------|-------|
| Languages supported | TypeScript, JavaScript, Rust, Python, Go |
| Lines of Rust code | ~38,000 |
| Tests | 492 |
| CLI commands | 13 |
| MCP tools | 20 |
| Rust edition | 2024 |
| Binary size | ~12 MB (static, zero runtime deps) |

## License

MIT
