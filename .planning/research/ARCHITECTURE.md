# Architecture Research

**Domain:** Code intelligence engine / dependency graph tool
**Researched:** 2026-02-22
**Confidence:** HIGH

## Standard Architecture

### System Overview

```
┌─────────────────────────────────────────────────────────────┐
│                      CLI / Entry Point                       │
│  index | query | impact | watch | mcp                       │
├─────────────────────────────────────────────────────────────┤
│                      MCP Server (rmcp)                       │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐       │
│  │ query    │ │ context  │ │ impact   │ │ stats    │       │
│  │ tool     │ │ tool     │ │ tool     │ │ tool     │       │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘       │
│       └─────────────┴─────────────┴─────────────┘           │
├─────────────────────────────────────────────────────────────┤
│                     Query Engine                             │
│  Symbol lookup | Impact traversal | Reference search         │
├─────────────────────────────────────────────────────────────┤
│                     Graph Store (petgraph)                    │
│  Nodes: File, Folder, Function, Class, Type, Export          │
│  Edges: IMPORTS, CALLS, EXPORTS, CONTAINS, EXTENDS           │
├─────────────────────────────────────────────────────────────┤
│                     Indexing Pipeline                         │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐       │
│  │ Walker   │→│ Parser   │→│ Resolver │→│ Linker   │       │
│  │ (ignore) │ │(tree-sit)│ │(imports) │ │(calls)   │       │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘       │
├─────────────────────────────────────────────────────────────┤
│                     File Watcher (notify)                     │
│  Debounced events → Incremental re-index                     │
├─────────────────────────────────────────────────────────────┤
│                     Persistence (rkyv)                        │
│  .code-graph/graph.bin | .code-graph/index.bin               │
└─────────────────────────────────────────────────────────────┘
```

### Component Responsibilities

| Component | Responsibility | Typical Implementation |
|-----------|----------------|------------------------|
| **CLI** | Parse commands, dispatch to appropriate subsystem | clap with subcommands |
| **MCP Server** | Expose graph queries as MCP tools over stdio | rmcp with tool handlers |
| **Query Engine** | Translate high-level queries into graph traversals | Methods on graph: find_symbol, impact_of, references_to |
| **Graph Store** | Hold the dependency graph in memory, support traversals | petgraph DiGraph with typed nodes/edges |
| **Indexing Pipeline** | Transform source files into graph nodes and edges | 4-stage pipeline: walk → parse → resolve → link |
| **File Watcher** | Detect file changes and trigger incremental re-index | notify with debounce, feeds changed paths to pipeline |
| **Persistence** | Save/load graph to disk for fast startup | rkyv zero-copy serialization to .code-graph/ |

## Recommended Project Structure

```
src/
├── main.rs                 # CLI entry point (clap)
├── lib.rs                  # Library root, re-exports
├── cli/                    # CLI command handlers
│   ├── mod.rs
│   ├── index.rs            # `code-graph index` command
│   ├── query.rs            # `code-graph query <symbol>`
│   ├── impact.rs           # `code-graph impact <symbol>`
│   └── watch.rs            # `code-graph watch` command
├── mcp/                    # MCP server and tool definitions
│   ├── mod.rs
│   ├── server.rs           # MCP server setup (rmcp)
│   └── tools/              # One file per MCP tool
│       ├── mod.rs
│       ├── query.rs
│       ├── context.rs
│       ├── impact.rs
│       └── stats.rs
├── graph/                  # Graph data structures
│   ├── mod.rs
│   ├── store.rs            # GraphStore: petgraph wrapper
│   ├── nodes.rs            # Node types (File, Function, Class, etc.)
│   ├── edges.rs            # Edge types (Imports, Calls, Contains, etc.)
│   └── query.rs            # Query methods on the graph
├── indexer/                # Indexing pipeline
│   ├── mod.rs
│   ├── walker.rs           # File system walker (ignore crate)
│   ├── parser.rs           # tree-sitter parsing, AST → symbols
│   ├── resolver.rs         # Import resolution (tsconfig paths, node_modules)
│   └── linker.rs           # Call graph linking (function calls → edges)
├── watcher/                # File watching and incremental updates
│   ├── mod.rs
│   └── incremental.rs      # Diff-based re-indexing logic
├── persist/                # Graph persistence
│   ├── mod.rs
│   └── storage.rs          # Save/load graph (rkyv)
└── format/                 # Output formatting
    ├── mod.rs
    ├── compact.rs           # Token-efficient format for MCP
    └── pretty.rs            # Human-readable format for CLI
```

### Structure Rationale

- **graph/:** Isolated from parsing — the graph doesn't know about tree-sitter. Clean boundary.
- **indexer/:** Pipeline stages are separate files — can test each stage independently.
- **mcp/tools/:** One file per tool — easy to add new tools, each tool is self-contained.
- **format/:** Separate from graph — formatting is a concern of the output layer, not the data layer.

## Architectural Patterns

### Pattern 1: Pipeline Architecture (Indexing)

**What:** Source files flow through a 4-stage pipeline: Walk → Parse → Resolve → Link
**When to use:** Initial full index and per-file re-index
**Trade-offs:** Clear separation of concerns. Each stage is testable. Slightly more code than a monolithic indexer, but much easier to debug.

**Stages:**

```
Walk (collect .ts/.js files, respect .gitignore)
  ↓ Vec<PathBuf>
Parse (tree-sitter → extract symbols, imports, exports, calls per file)
  ↓ Vec<FileInfo { path, symbols, imports, exports, calls }>
Resolve (map import specifiers to actual file paths using tsconfig)
  ↓ Vec<ResolvedFile { ..., resolved_imports: Vec<(ImportSpec, PathBuf)> }>
Link (insert nodes/edges into graph)
  ↓ Graph updated
```

### Pattern 2: Read-Write Lock for Concurrent Access

**What:** Graph wrapped in `Arc<RwLock<GraphStore>>`. Watcher takes write lock briefly, queries take read locks.
**When to use:** When running watch mode + MCP server concurrently (the normal operating mode).
**Trade-offs:** Simple to implement. Write lock blocks queries momentarily during re-index. Acceptable because re-index is fast (<100ms for single file).

### Pattern 3: Content-Addressed Caching

**What:** Hash file contents (xxhash) and skip re-parsing if hash unchanged. Store hash → parsed result.
**When to use:** During incremental re-index and cold start (skip files already in persisted graph).
**Trade-offs:** Small memory overhead for hash table. Massive speedup when many files unchanged.

### Pattern 4: Compact Response Format

**What:** MCP tool responses use a minimal structured format optimized for token count.
**When to use:** Every MCP tool response.

**Example — find_references response:**

```json
{"sym":"UserService","refs":[["src/api/routes.ts",42],["src/middleware/auth.ts",18],["src/tests/user.test.ts",7]],"total":3}
```

vs verbose (what Axon returns):

```json
{"symbol":{"name":"UserService","kind":"class","file":"src/services/user.ts","line":15},"references":[{"file":"src/api/routes.ts","line":42,"context":"import { UserService } from '../services/user'"},{"file":"src/middleware/auth.ts","line":18,"context":"const svc = new UserService()"}],"total_references":3}
```

**Token savings:** ~60% fewer tokens for the same information.

## Data Flow

### Full Index Flow

```
CLI: `code-graph index`
    ↓
Walker: find all .ts/.js/.tsx/.jsx files (respect .gitignore, tsconfig include/exclude)
    ↓ Vec<PathBuf>
Parser: for each file, tree-sitter parse → extract symbols, imports, calls
    ↓ Vec<FileInfo>
Resolver: for each import, resolve to absolute path (tsconfig paths, node_modules, index.ts)
    ↓ Vec<ResolvedFile>
Linker: insert File/Folder nodes, Symbol nodes, Import/Call/Contains edges
    ↓ Graph in memory
Persist: serialize graph to .code-graph/graph.bin
    ↓
Done — graph ready for queries
```

### Incremental Update Flow

```
Watcher: file change detected (notify)
    ↓ PathBuf of changed file
Hash Check: content hash changed?
    ↓ Yes
Remove: delete all nodes/edges originating from this file
    ↓
Re-Parse: tree-sitter parse changed file only
    ↓
Re-Resolve: resolve imports for this file only
    ↓
Re-Link: insert new nodes/edges for this file
    ↓
Cascade: find files that import this file → check if their edges need updating
    ↓
Persist: save updated graph
```

### MCP Query Flow

```
Claude Code: calls MCP tool (e.g., `impact("UserService")`)
    ↓ stdio JSON-RPC
MCP Server: route to impact tool handler
    ↓
Query Engine: find "UserService" node → BFS/DFS upstream through IMPORTS/CALLS edges
    ↓ Vec<ImpactedSymbol>
Formatter: compact JSON response
    ↓
Claude Code: receives structured response (no file reads needed)
```

### Key Data Flows

1. **Index → Persist → Load:** Full index writes graph to disk. Next startup loads from disk, only re-indexes changed files (hash comparison). Target: cold start <2s for cached 10K-file project.
2. **Watch → Incremental → Query:** File save triggers re-index of changed file + dependents. Graph updated in-place. Next query sees fresh data. Target: <100ms from save to query-ready.

## Graph Schema

### Node Types

| Node Type | Properties | Notes |
|-----------|------------|-------|
| **File** | path, hash, size, last_indexed | One per source file |
| **Folder** | path | Directory structure |
| **Function** | name, file, line, col, exported, async, params_count | Named functions and arrow functions |
| **Class** | name, file, line, col, exported | Class declarations |
| **Method** | name, class, file, line, col, static, visibility | Class methods |
| **Interface** | name, file, line, col, exported | TypeScript interfaces |
| **TypeAlias** | name, file, line, col, exported | TypeScript type aliases |
| **Enum** | name, file, line, col, exported | TypeScript enums |
| **Variable** | name, file, line, col, exported, kind | const/let/var exports |

### Edge Types

| Edge Type | From → To | Properties | Notes |
|-----------|-----------|------------|-------|
| **IMPORTS** | File → File | specifier, is_type_only | `import { X } from './y'` |
| **CONTAINS** | File → Symbol | — | File contains this symbol definition |
| **EXPORTS** | File → Symbol | name, is_default, is_re_export | File exports this symbol |
| **CALLS** | Symbol → Symbol | line, confidence | Function A calls function B |
| **EXTENDS** | Class → Class | — | `class A extends B` |
| **IMPLEMENTS** | Class → Interface | — | `class A implements B` |
| **USES_TYPE** | Symbol → TypeAlias/Interface | — | Function uses this type in signature |
| **PARENT** | Folder → Folder/File | — | Directory structure |

### Index Structures (beside the graph)

| Structure | Purpose | Implementation |
|-----------|---------|----------------|
| **Symbol Index** | name → Vec<NodeId> | DashMap for concurrent lookup |
| **File Index** | path → NodeId | HashMap |
| **Hash Index** | path → content_hash | HashMap for change detection |

## Scaling Considerations

| Scale | Architecture Adjustments |
|-------|--------------------------|
| <1K files | Everything in memory. Full re-index is fine (<1s). No optimization needed. |
| 1K-10K files | Incremental indexing essential. Parallel parsing (rayon). Persistence for fast startup. |
| 10K+ files | String interning (lasso crate) to reduce memory. Lazy loading of rarely-queried subgraphs. Consider sharded graph for very large monorepos. |

### Scaling Priorities

1. **First bottleneck:** Parse time on full index. Fix: parallel parsing with rayon (tree-sitter is thread-safe).
2. **Second bottleneck:** Memory for large graphs. Fix: string interning, compact node representation.
3. **Third bottleneck:** Import resolution for deep barrel files. Fix: cache resolved paths, detect barrel re-export chains.

## Anti-Patterns

### Anti-Pattern 1: Storing Full AST in Graph

**What people do:** Store the entire tree-sitter AST for each file in the graph.
**Why it's wrong:** ASTs are huge. A 500-line file produces thousands of AST nodes. The graph should store extracted facts (symbols, relationships), not raw ASTs.
**Do this instead:** Extract symbols and relationships during parsing, discard the AST immediately.

### Anti-Pattern 2: Synchronous Full Re-Index on Every File Change

**What people do:** Watch mode triggers complete re-index on any file save.
**Why it's wrong:** Kills performance on large codebases. User saves one file, waits 30 seconds.
**Do this instead:** Incremental: remove old data for changed file, re-parse only that file, cascade to direct dependents only.

### Anti-Pattern 3: Using an External Database for Graph Storage

**What people do:** Use Neo4j, KuzuDB, or similar for the graph.
**Why it's wrong:** Adds deployment complexity, startup time, and memory overhead. For a CLI tool that indexes a single project, an in-memory graph with persistence is faster and simpler.
**Do this instead:** petgraph in memory + rkyv serialization to disk.

## Integration Points

### External Services

| Service | Integration Pattern | Notes |
|---------|---------------------|-------|
| **Claude Code** | MCP stdio transport | Claude spawns our binary, communicates via stdin/stdout JSON-RPC |
| **File system** | notify crate | Watch for changes, read files for indexing |
| **tsconfig.json** | Custom parser | Read paths, baseUrl, include/exclude for import resolution |
| **package.json** | serde_json | Read for dependency list, entry points |
| **.gitignore** | ignore crate | Respect gitignore during file walking |

### Internal Boundaries

| Boundary | Communication | Notes |
|----------|---------------|-------|
| Indexer ↔ Graph | Function calls (same process) | Indexer writes to graph, graph is the single source of truth |
| MCP Server ↔ Graph | Read lock on graph | Server never writes to graph, only reads |
| Watcher ↔ Indexer | Channel (tokio::mpsc) | Watcher sends changed paths, indexer processes them |
| CLI ↔ Graph | Direct function calls | CLI commands query the graph directly (no MCP overhead) |

## Sources

- petgraph docs: docs.rs/petgraph
- tree-sitter architecture: tree-sitter.github.io/tree-sitter
- rmcp MCP SDK: github.com/anthropics/rmcp
- Axon pipeline (reference): github.com/harshkedia177/axon
- rkyv zero-copy: docs.rs/rkyv
- notify file watcher: docs.rs/notify

---
*Architecture research for: code intelligence engine / dependency graph tool*
*Researched: 2026-02-22*
