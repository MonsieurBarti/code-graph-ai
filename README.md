# code-graph

High-performance code intelligence engine that indexes TypeScript/JavaScript and Rust codebases into a queryable dependency graph. Built in Rust, designed for AI agents.

Gives [Claude Code](https://docs.anthropic.com/en/docs/claude-code) direct access to your codebase's structure via [MCP](https://modelcontextprotocol.io/) -- no source file reading needed. Fifteen MCP tools cover symbol search, reference tracing, blast radius analysis, circular dependency detection, dead code detection, graph export, batch queries, snapshot/diff, and multi-project management.

## Features

- **Multi-language parsing** -- TypeScript, TSX, JavaScript, JSX, and Rust via tree-sitter with full symbol extraction (functions, classes, interfaces, types, enums, components, methods, properties, structs, traits, impl blocks, macros, pub visibility)
- **Dependency graph** -- file-level and symbol-level edges: imports, calls, extends, implements, type references
- **Import resolution** -- TypeScript path aliases (tsconfig.json), barrel files (index.ts re-exports), monorepo workspaces, Rust crate-root module resolution with Cargo workspace discovery
- **Fifteen query types** -- find definitions, trace references, blast radius analysis, circular dependency detection, 360-degree symbol context, project statistics, graph export, file structure, file summaries, import analysis, batch queries, dead code detection, graph diff, project registration, project listing
- **MCP server** -- zero-config (defaults to cwd), exposes all queries as tools for Claude Code over stdio, optional `--watch` flag for auto-reindex
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

## Install

```bash
cargo install code-graph-cli
```

This installs the `code-graph` binary to `~/.cargo/bin/`.

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

# Find a symbol
code-graph find "UserService" /path/to/project

# What breaks if I change this?
code-graph impact "DatabaseConfig" /path/to/project

# Start MCP server for Claude Code (zero-config, defaults to cwd)
code-graph mcp

# Start MCP server with auto-reindex on file changes
code-graph mcp /path/to/project --watch

# Export dependency graph as Mermaid at package granularity
code-graph export . --format mermaid --granularity package

# Create a named graph snapshot for later comparison
code-graph snapshot create baseline .
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
```

### index

Index a project, discovering and parsing all TypeScript/JavaScript and Rust files.

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

### Output formats

All query commands support `--format`:

| Format | Description |
|--------|-------------|
| `compact` | One-line-per-result, token-optimized (default) |
| `table` | Human-readable columns with ANSI colors |
| `json` | Structured JSON for programmatic use |

## MCP integration

### Claude Code setup

Add to your Claude Code MCP config (`~/.claude/claude_desktop_config.json` or project `.mcp.json`):

```json
{
  "mcpServers": {
    "code-graph": {
      "command": "code-graph",
      "args": ["mcp"]
    }
  }
}
```

The server defaults to the current working directory. To specify a path or enable auto-reindex:

```json
{
  "mcpServers": {
    "code-graph": {
      "command": "code-graph",
      "args": ["mcp", "/path/to/your/project", "--watch"]
    }
  }
}
```

### Available tools

Once connected, Claude Code gets access to fifteen tools:

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
      "mcp__code-graph__list_projects"
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

1. **Walk** -- discovers TS/JS and Rust files respecting `.gitignore` and exclusion rules
2. **Parse** -- tree-sitter extracts symbols, imports, exports, and relationships from each file. TypeScript/JavaScript parsing covers functions, classes, interfaces, type aliases, enums, and components. Rust parsing covers functions, structs, enums, traits, impl blocks, type aliases, constants, statics, and macro definitions with visibility tracking.
3. **Resolve** -- maps import specifiers to actual files. For TypeScript/JavaScript: oxc_resolver handles path aliases, barrel files, and workspaces. For Rust: crate-root module tree walk with use-path classification (crate/super/self/external/builtin) and Cargo workspace discovery.
4. **Build graph** -- constructs a petgraph with file nodes, symbol nodes, and typed edges (imports, calls, extends, implements, type references)
5. **Cache** -- serializes the graph to disk with bincode for fast reloads
6. **Query** -- traverses the graph to answer structural questions without reading source files
7. **Watch** -- monitors filesystem events and incrementally updates the graph (re-parses only changed files)

## Project stats

| Metric | Value |
|--------|-------|
| Languages supported | TypeScript, JavaScript, Rust |
| Lines of Rust code | ~21,000 |
| Tests | 285 (264 unit + 21 integration) |
| CLI commands | 11 |
| MCP tools | 15 |
| Rust edition | 2024 |
| Binary size | ~12 MB (static, zero runtime deps) |

## License

MIT
