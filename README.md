# code-graph

High-performance code intelligence engine that indexes TypeScript/JavaScript codebases into a queryable dependency graph. Built in Rust, designed for AI agents.

Gives [Claude Code](https://docs.anthropic.com/en/docs/claude-code) direct access to your codebase's structure via [MCP](https://modelcontextprotocol.io/) — no source file reading needed. Find symbols, trace references, analyze blast radius, and detect circular dependencies through pre-built graph queries.

## Features

- **Tree-sitter parsing** — TypeScript, TSX, JavaScript, JSX with full symbol extraction (functions, classes, interfaces, types, enums, components, methods, properties)
- **Dependency graph** — file-level and symbol-level edges: imports, calls, extends, implements, type references
- **Import resolution** — TypeScript path aliases (tsconfig.json), barrel files (index.ts re-exports), monorepo workspaces
- **Six query types** — find definitions, trace references, blast radius analysis, circular dependency detection, 360-degree symbol context, project statistics
- **MCP server** — exposes all queries as tools for Claude Code over stdio
- **File watcher** — incremental re-indexing on file changes with 75ms debounce
- **Disk cache** — bincode serialization for instant cold starts
- **Token-optimized output** — compact format designed for AI agent consumption

## Install

### From source

```bash
cargo install --path .
```

### Build manually

```bash
git clone https://github.com/user/code-graph.git
cd code-graph
cargo build --release
# Binary at target/release/code-graph
```

Requires Rust 1.85+ (edition 2024). No system dependencies — tree-sitter grammars are bundled.

## Quick start

```bash
# Index a project
code-graph index /path/to/project

# Find a symbol
code-graph find "UserService" /path/to/project

# What breaks if I change this?
code-graph impact "DatabaseConfig" /path/to/project

# Start MCP server for Claude Code
code-graph mcp /path/to/project
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
```

### index

Index a project, discovering and parsing all TypeScript/JavaScript files.

```bash
code-graph index . --verbose    # Print each discovered file
code-graph index . --json       # Output as JSON
```

### find

Find symbol definitions by name or regex pattern.

```bash
code-graph find "UserService" .
code-graph find "User.*Service" . -i            # Case-insensitive regex
code-graph find "authenticate" . --kind function # Filter by kind
code-graph find "Button" . --file src/components # Scope to directory
```

Symbol kinds: `function`, `class`, `interface`, `type`, `enum`, `variable`, `component`, `method`, `property`

### refs

Find all files and call sites that reference a symbol.

```bash
code-graph refs "UserService" .
code-graph refs "useAuth" . --format table    # Human-readable table
```

### impact

Show the transitive blast radius — everything affected if a symbol changes.

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

360-degree view combining definition, references, callers, and callees.

```bash
code-graph context "Logger" .
```

### watch

Start a standalone file watcher that re-indexes incrementally on changes.

```bash
code-graph watch .
```

> The `mcp` command starts its own embedded watcher automatically — you don't need to run `watch` separately.

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
      "args": ["mcp", "/path/to/your/project"]
    }
  }
}
```

### Available tools

Once connected, Claude Code gets access to six tools:

| Tool | Description |
|------|-------------|
| `find_symbol` | Find symbol definitions by name or regex |
| `find_references` | Find all files and call sites referencing a symbol |
| `get_impact` | Get the transitive blast radius of changing a symbol |
| `detect_circular` | Detect circular dependency cycles |
| `get_context` | 360-degree view: definition + references + callers + callees |
| `get_stats` | Project overview: files, symbols, imports |

The MCP server loads from disk cache on startup for near-instant cold starts, runs an embedded file watcher for live updates, and suggests similar symbol names when a search yields no results.

## Configuration

Optional `code-graph.toml` in your project root:

```toml
[exclude]
paths = ["vendor/", "dist/", "build/"]
```

By default, code-graph respects `.gitignore` patterns and always excludes `node_modules/`.

## How it works

1. **Walk** — discovers TS/JS files respecting `.gitignore` and exclusion rules
2. **Parse** — tree-sitter extracts symbols, imports, exports, and relationships from each file
3. **Resolve** — maps import specifiers to actual files using oxc_resolver (handles path aliases, barrel files, workspaces)
4. **Build graph** — constructs a petgraph with file nodes, symbol nodes, and typed edges (imports, calls, extends, implements, type references)
5. **Cache** — serializes the graph to disk with bincode for fast reloads
6. **Query** — traverses the graph to answer structural questions without reading source files
7. **Watch** — monitors filesystem events and incrementally updates the graph (re-parses only changed files)
