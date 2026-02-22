# Stack Research

**Domain:** Code intelligence engine / dependency graph tool
**Researched:** 2026-02-22
**Confidence:** HIGH

## Go vs Rust Decision

### Comparison for This Specific Use Case

| Dimension | Go | Rust | Winner |
|-----------|-----|------|--------|
| **tree-sitter bindings** | go-tree-sitter (smacker/go-tree-sitter) — mature, actively maintained, clean API. Grammars available as Go modules. | tree-sitter crate (tree-sitter/tree-sitter) — first-class support, Rust is tree-sitter's native language. Best API, best performance. | Rust |
| **Graph storage** | No embedded graph DB in Go ecosystem. Would need: adjacency lists in memory + bbolt/BadgerDB for persistence. | petgraph crate — mature, fast, in-memory graph with algorithms (DFS, BFS, cycle detection, topological sort). Persistence via serde + custom storage. | Rust (petgraph) |
| **MCP SDK** | mcp-go (mark3labs/mcp-go) — actively maintained, supports stdio and SSE transports. | mcp-rust-sdk (anthropics/mcp-rust-sdk, formerly modelcontextprotocol/rust-sdk) — official Anthropic SDK, actively developed. Also rmcp (anthropics/rmcp). | Tie |
| **File watching** | fsnotify — mature, cross-platform, well-maintained. | notify crate — mature, cross-platform, well-maintained. | Tie |
| **Build & distribute** | `go build` → single static binary. Cross-compilation trivial (`GOOS=darwin GOARCH=arm64`). | `cargo build --release` → single binary. Cross-compilation needs cross or cargo-zigbuild. Slightly harder. | Go |
| **Concurrency (watch + serve)** | Goroutines — trivial to run watcher, MCP server, and indexer concurrently. | Tokio async runtime — powerful but more complex. Ownership model makes shared graph state harder. | Go |
| **Memory efficiency** | GC-managed. Typically 30-50% more memory than Rust for same workload. Still well within <100MB target. | Zero-cost abstractions, no GC. Lowest possible memory footprint. | Rust |
| **Development speed** | Faster to write, simpler error handling, easier to iterate. | Slower to write, borrow checker friction, but catches more bugs at compile time. | Go |
| **Parse performance** | Fast enough — tree-sitter is C under the hood. Go FFI overhead is minimal. | Fastest possible — native tree-sitter bindings, zero FFI overhead. | Rust |

### Recommendation: **Rust**

**Rationale:**
1. tree-sitter is written in C with Rust as the primary binding language — best API, zero overhead
2. petgraph provides graph algorithms out of the box (cycle detection, DFS traversal for impact analysis)
3. Memory efficiency matters for always-on daemon — Rust achieves <100MB naturally
4. The performance ceiling is higher — matters for 10K+ file codebases
5. Anthropic maintains an official Rust MCP SDK (rmcp)
6. serde ecosystem makes token-efficient serialization trivial (custom compact formats)

**Trade-off acknowledged:** Development will be ~30-40% slower than Go. The borrow checker will fight concurrent access to the graph during watch-mode updates. Mitigate with Arc<RwLock<Graph>> pattern.

## Recommended Stack

### Core Technologies

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| **Rust** | 1.82+ (2024 edition) | Implementation language | Best tree-sitter integration, lowest memory, petgraph ecosystem |
| **tree-sitter** | 0.24+ | TS/JS parsing | Industry standard for fast, incremental parsing. Used by every code intelligence tool. |
| **tree-sitter-typescript** | latest | TypeScript grammar | Official grammar, covers TSX. Maintained by tree-sitter org. |
| **tree-sitter-javascript** | latest | JavaScript grammar | Official grammar, covers JSX. |
| **petgraph** | 0.6+ | In-memory graph storage | Fast graph with DFS, BFS, cycle detection, topological sort. No external DB needed. |
| **rmcp** | latest | MCP server | Official Anthropic Rust MCP SDK. Supports stdio transport (what Claude Code uses). |
| **notify** | 7.0+ | File system watching | Cross-platform, debounced events, recursive directory watching. |
| **tokio** | 1.x | Async runtime | Required by MCP server. Handles concurrent watcher + server + indexer. |
| **serde** + **serde_json** | 1.x | Serialization | JSON output for MCP responses. Custom serializers for compact format. |

### Supporting Libraries

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| **clap** | 4.x | CLI argument parsing | CLI interface (`index`, `query`, `impact`, `watch`) |
| **tracing** | 0.1+ | Structured logging | Debug indexing issues, performance profiling |
| **dashmap** | 6.x | Concurrent hash map | Symbol lookup table shared between watcher and query threads |
| **xxhash-rust** | 0.8+ | Fast hashing | File content hashing for change detection (faster than SHA) |
| **ignore** | 0.4+ | Gitignore-aware file walking | Respect .gitignore during indexing (from the ripgrep author) |
| **globset** | 0.4+ | Glob pattern matching | File inclusion/exclusion patterns |
| **memmap2** | 0.9+ | Memory-mapped file I/O | Fast file reading during indexing without loading full files into memory |
| **rkyv** | 0.8+ | Zero-copy deserialization | Fast graph persistence to disk (load graph without parsing) |

### Development Tools

| Tool | Purpose | Notes |
|------|---------|-------|
| **cargo** | Build system | Standard Rust toolchain |
| **cargo-watch** | Dev reload | Auto-rebuild on source changes |
| **criterion** | Benchmarking | Benchmark indexing speed, query latency |
| **insta** | Snapshot testing | Test graph output against known-good snapshots |

## Installation

```bash
# From source
cargo install code-graph

# Or build locally
git clone ...
cargo build --release
# Binary at target/release/code-graph
```

## Alternatives Considered

| Recommended | Alternative | When to Use Alternative |
|-------------|-------------|-------------------------|
| Rust | Go | If development speed is critical and team doesn't know Rust |
| petgraph | KuzuDB (embedded) | If Cypher query language is needed (Axon approach) — adds dep complexity |
| petgraph | Neo4j embedded | Never — too heavy for a CLI tool |
| rkyv (persistence) | SQLite/DuckDB | If complex queries on graph metadata needed beyond traversal |
| rmcp | Custom MCP impl | Never — the protocol is standardized, use the SDK |
| notify | inotify/kqueue direct | If cross-platform not needed and want lower-level control |
| tree-sitter | swc_ecma_parser | If only TS/JS forever and want pure-Rust parsing (no C dep). Less mature for this use case. |
| tree-sitter | TypeScript compiler API | If full type resolution needed. Much slower, much heavier. Defer to v2. |

## What NOT to Use

| Avoid | Why | Use Instead |
|-------|-----|-------------|
| **KuzuDB / any external graph DB** | Adds runtime dependency, complicates distribution, overkill for in-memory graph queries | petgraph + rkyv persistence |
| **Embedding models (fastembed, etc.)** | Violates memory constraint, adds model dependency, unnecessary for structural queries | Exact + fuzzy string matching on symbol names |
| **Python bindings / PyO3** | Defeats the purpose of building in Rust | Pure Rust implementation |
| **gRPC for MCP transport** | MCP uses stdio or HTTP+SSE, not gRPC | rmcp stdio transport |
| **Diesel / SeaORM** | No relational DB needed | petgraph for graph, serde for serialization |

## Stack Patterns by Variant

**If targeting maximum indexing speed:**
- Use rayon for parallel file parsing (tree-sitter parsers are thread-safe)
- Use memmap2 for file I/O
- Parse files in parallel, merge into graph sequentially

**If targeting minimum memory:**
- Use string interning (lasso crate) for repeated identifiers
- Store graph edges as indices, not cloned strings
- Use rkyv for zero-copy graph loading from disk

**If targeting easiest MCP integration:**
- Use rmcp with stdio transport (Claude Code default)
- Define tools as Rust structs with serde derive
- Return JSON responses directly

## Version Compatibility

| Package A | Compatible With | Notes |
|-----------|-----------------|-------|
| tree-sitter 0.24+ | tree-sitter-typescript latest | Ensure grammar version matches tree-sitter core |
| tokio 1.x | rmcp latest | Both use tokio async runtime |
| serde 1.x | rkyv 0.8+ | Can derive both for same structs |

## Sources

- tree-sitter Rust bindings: official tree-sitter crate (tree-sitter/tree-sitter GitHub)
- petgraph: docs.rs/petgraph — graph data structures and algorithms
- rmcp: github.com/anthropics/rmcp — official Anthropic Rust MCP SDK
- notify: docs.rs/notify — cross-platform filesystem notification
- rkyv: docs.rs/rkyv — zero-copy deserialization framework
- mcp-go: github.com/mark3labs/mcp-go — Go MCP SDK (for comparison)
- Axon architecture: github.com/harshkedia177/axon (Python reference implementation)

---
*Stack research for: code intelligence engine / dependency graph tool*
*Researched: 2026-02-22*
