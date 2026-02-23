# Phase 4: MCP Integration - Research

**Researched:** 2026-02-23
**Domain:** Rust MCP server (rmcp), stdio transport, token-optimized tool responses
**Confidence:** HIGH

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

**Tool surface design**
- 1:1 mapping from CLI commands to MCP tools: `find_symbol`, `find_references`, `get_impact`, `detect_circular`, `get_context`, plus `get_stats` (6 tools total)
- No reindex/index tool via MCP — indexing is a CLI concern
- Symbol name is required; file path is optional for disambiguation (if omitted, searches globally)
- Default result limit on each tool with an optional `limit` param to override (Claude picks sensible defaults)

**Response format**
- Compact text lines: one result per line, fields separated by delimiters (e.g., `src/auth.ts:42 | UserService | class`)
- Always include a summary header line (total count, truncation info) before results
- Context/360-degree tool uses labeled sections (`--- callers ---`, `--- callees ---`, etc.) to separate groups
- All file paths are relative to the project root

**Error responses**
- Symbol not found: return error with up to 3 fuzzy match suggestions to reduce round-trips
- Use MCP protocol's `isError` flag for all error conditions (not inline text errors)
- No staleness tracking in Phase 4 — deferred to Phase 5 (watch mode)

**Startup & onboarding**
- MCP server invoked as subcommand: `code-graph mcp [path]` — defaults to cwd if path omitted
- Designed for user-level MCP registration: `{"command": "code-graph", "args": ["mcp"]}` — works across multiple projects
- Each MCP tool accepts an optional `project_path` param to scope queries to a specific project
- Can also be registered per-project with an explicit path in args: `{"command": "code-graph", "args": ["mcp", "/path/to/project"]}`

### Claude's Discretion
- No-index behavior: whether to return an actionable error or auto-index on first call
- Exact default result limits per tool type
- Delimiter choice for compact text format
- Tool parameter naming conventions

### Deferred Ideas (OUT OF SCOPE)
None — discussion stayed within phase scope
</user_constraints>

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|-----------------|
| INTG-01 | Tool runs as an MCP server over stdio, exposing graph queries as tools to Claude Code | rmcp `serve_server` + `stdio()` transport pattern; `Commands::Mcp` subcommand added to clap CLI |
| INTG-02 | MCP tool responses use a token-optimized compact format (target: ~60% fewer tokens than verbose JSON) | Existing `OutputFormat::Compact` formatters in `query/output.rs` reused as-is; compact lines are `~5-15` tokens vs JSON at `~25-50` per result |
| INTG-03 | MCP tool descriptions are concise (under 100 tokens each) to minimize per-turn overhead | Tool description string budgeting; token counting heuristic: 100 tokens ≈ 75 words; descriptions must be one tight sentence |
</phase_requirements>

---

## Summary

Phase 4 wires the existing query engine into an MCP stdio server so Claude Code can call `find_symbol`, `find_references`, `get_impact`, `detect_circular`, `get_context`, and `get_stats` as native tools. The project already has all the hard logic — this phase is primarily an integration layer.

The standard approach uses `rmcp` 0.16 (the official Anthropic Rust SDK, released 2026-02-17) with its `#[tool_router]` and `#[tool_handler]` proc-macros. The server binary is extended with a `mcp` subcommand that initializes the graph (either from a supplied path or cwd) and then serves over `stdin`/`stdout`. Each tool handler calls directly into the existing `query::*` modules and formats output with the existing compact formatters, returning plain-text `Content::text(...)` inside `CallToolResult`.

The critical constraint is token budget: tool _descriptions_ must stay under ~100 tokens each (one tight sentence), and tool _responses_ must use the existing compact line format (already built). The existing compact format already achieves the ~60% savings target — no new format work is required.

**Primary recommendation:** Add `rmcp` + `schemars` + `tokio` to `Cargo.toml`, add a `Commands::Mcp` variant to `cli.rs`, create `src/mcp/` module with a single server struct implementing `#[tool_router]` + `#[tool_handler]`, and keep all tool response text generation in the existing `query/output.rs` helpers.

---

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| rmcp | 0.16.0 | MCP protocol: server lifecycle, tool dispatch, stdio transport | Official Anthropic Rust SDK; proc-macros eliminate boilerplate |
| tokio | 1 (full) | Async runtime required by rmcp | rmcp is async-first; `full` feature needed for `stdin`/`stdout` |
| schemars | 1.0 | JSON Schema generation for tool input types | rmcp's `#[tool]` macro requires `JsonSchema` derive on param structs |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| serde / serde_json | 1 (already in Cargo.toml) | Param struct serialization | Required by `Parameters<T>` wrapper |
| anyhow | 1 (already in Cargo.toml) | Error handling in main | Already present; MCP server returns `Result<(), anyhow::Error>` from main |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| rmcp 0.16 (official SDK) | mcp-sdk-rs, rust-mcp-sdk | rmcp is now the official Anthropic SDK (moved to modelcontextprotocol org); alternatives are unmaintained or less featureful |
| `#[tool_router]` macro | Manual `ServerHandler::call_tool` dispatch | Macro eliminates 50+ lines of boilerplate match arms and schema generation |

### Installation

Add to `/workspace/Cargo.toml`:
```toml
rmcp = { version = "0.16", features = ["server", "transport-io", "macros"] }
tokio = { version = "1", features = ["full"] }
schemars = "1"
```

The project already has `serde`, `serde_json`, and `anyhow` — no changes needed there.

---

## Architecture Patterns

### Recommended Project Structure
```
src/
├── mcp/
│   ├── mod.rs          # pub use; re-exports CodeGraphServer
│   ├── server.rs       # CodeGraphServer struct + #[tool_router] impl + #[tool_handler] impl
│   └── params.rs       # Input param structs (derive Deserialize, JsonSchema)
├── cli.rs              # Add Commands::Mcp variant
└── main.rs             # Add Mcp arm that calls mcp::run(path)
```

### Pattern 1: rmcp Tool Router
**What:** `#[tool_router]` on an `impl` block auto-generates `ListTools` and `CallTool` dispatch. `#[tool_handler]` wires the router into `ServerHandler`.
**When to use:** Any time you expose a fixed set of Rust functions as MCP tools.

```rust
// Source: https://docs.rs/rmcp/latest/rmcp/index
use rmcp::{ErrorData as McpError, model::*, tool, tool_router, tool_handler,
           handler::server::tool::ToolRouter, ServerHandler};
use serde::Deserialize;
use schemars::JsonSchema;

#[derive(Deserialize, JsonSchema)]
struct FindSymbolParams {
    /// Symbol name or regex pattern
    symbol: String,
    /// Optional file path scope (relative to project root)
    path: Option<String>,
    /// Max results to return
    limit: Option<usize>,
}

#[derive(Clone)]
pub struct CodeGraphServer {
    project_root: PathBuf,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl CodeGraphServer {
    fn new(project_root: PathBuf) -> Self {
        Self { project_root, tool_router: Self::tool_router() }
    }

    #[tool(description = "Find symbol definitions by name or regex. Returns file:line and kind.")]
    async fn find_symbol(
        &self,
        params: Parameters<FindSymbolParams>,
    ) -> Result<CallToolResult, McpError> {
        // call into query::find::find_symbol, format with compact formatter
        // return CallToolResult::success(vec![Content::text(output)])
    }
}

#[tool_handler]
impl ServerHandler for CodeGraphServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("code-graph: query a TypeScript/JavaScript dependency graph".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
```

### Pattern 2: stdio Server Entrypoint
**What:** `serve_server(service, stdio()).await` blocks until the client disconnects.
**When to use:** Any stdio MCP server — this is the canonical startup pattern.

```rust
// Source: https://docs.rs/rmcp/latest/rmcp/service/fn.serve_server
use rmcp::transport::stdio;

pub async fn run(project_root: PathBuf) -> anyhow::Result<()> {
    let service = CodeGraphServer::new(project_root);
    let server = rmcp::serve_server(service, stdio()).await?;
    server.waiting().await?;
    Ok(())
}
```

### Pattern 3: isError for Protocol-Level Errors
**What:** Return `CallToolResult` with `is_error: Some(true)` (not an `Err`) for domain errors like "symbol not found". Reserve `Err(McpError)` for protocol/transport failures.

```rust
// Source: rmcp docs.rs + MCP spec
fn not_found_result(symbol: &str, suggestions: &[&str]) -> CallToolResult {
    let mut msg = format!("symbol '{}' not found.", symbol);
    if !suggestions.is_empty() {
        msg.push_str(&format!(" Did you mean: {}?", suggestions.join(", ")));
    }
    CallToolResult {
        content: vec![Content::text(msg)],
        is_error: Some(true),
        ..Default::default()
    }
}
```

### Pattern 4: Compact Text Response Builder
**What:** Re-use the existing `query/output.rs` compact formatters by redirecting `println!` to a `String` buffer (or refactoring to return `String` directly).

Two implementation options:
1. **Refactor `format_*` functions** to return `String` instead of printing — cleaner, recommended.
2. **Capture stdout** into a buffer per-call — works but more complex.

Refactoring is the correct choice: the formatters already produce the right compact text. Add a `format_*_to_string(...)` variant or change the signature to accept a `&mut impl Write` or return `String`.

### Anti-Patterns to Avoid
- **Verbose JSON responses from MCP tools:** The `OutputFormat::Json` path is for the CLI `--format json` flag. Never use it for MCP tool responses — it will fail INTG-02 and produce bloated output.
- **Inline error text in success response:** Never embed `"ERROR: symbol not found"` as plain text in a success `CallToolResult`. Use `is_error: Some(true)` so Claude can distinguish tool errors from results.
- **Large `instructions` field in `ServerInfo`:** Keep `get_info()` instructions under ~200 tokens. The instructions field is loaded on every turn.
- **Re-indexing on every tool call:** The graph must be built once at server startup and held in `Arc<CodeGraph>` (or stored in the server struct). Each tool call queries the in-memory graph.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| MCP protocol JSON-RPC framing | Custom stdio parser | `rmcp` + `transport-io` feature | Handles initialize/initialized handshake, capabilities negotiation, content encoding |
| Tool input JSON Schema | Manual schema strings | `schemars::JsonSchema` derive + rmcp `#[tool]` macro | Schema generated automatically from Rust struct; stays in sync with param types |
| Tool description token counting | Character count heuristic | Write descriptions, count manually using ratio: 1 token ≈ 4 chars (English prose) | A 75-word description fits comfortably under 100 tokens |
| Fuzzy symbol matching | Levenshtein from scratch | Pull symbol names from graph + simple edit-distance or prefix filter | The graph already stores all symbol names; a small helper over existing data is sufficient |

**Key insight:** All query logic (find, refs, impact, circular, context, stats) is already implemented and tested in `src/query/`. Phase 4 is a thin MCP adapter layer — keep it thin.

---

## Common Pitfalls

### Pitfall 1: Graph Not Built at Startup
**What goes wrong:** Each tool call triggers `build_graph()` — 500ms+ per call on real projects, making MCP impractical.
**Why it happens:** Naively porting the CLI pattern where every command rebuilds the graph.
**How to avoid:** Call `build_graph()` once in `CodeGraphServer::new()` (or in `run()` before creating the server) and store `Arc<CodeGraph>` in the server struct. Since the server is `Clone` (required by `#[tool_router]`), wrap in `Arc`.
**Warning signs:** Tool calls taking >100ms; identical results on repeated calls triggering file I/O.

### Pitfall 2: `Clone` Requirement on Server Struct
**What goes wrong:** Compile error: "the trait `Clone` is not implemented for `CodeGraph`".
**Why it happens:** `#[tool_router]` requires the server struct to implement `Clone`. `CodeGraph` contains non-`Clone` types (petgraph's `StableGraph`).
**How to avoid:** Wrap the `CodeGraph` in `Arc<CodeGraph>`. `Arc<T>` is always `Clone`. Store `Arc<CodeGraph>` in the server struct.

```rust
#[derive(Clone)]
pub struct CodeGraphServer {
    graph: Arc<CodeGraph>,
    project_root: Arc<PathBuf>,
    tool_router: ToolRouter<Self>,
}
```

### Pitfall 3: Tool Description Token Budget Exceeded
**What goes wrong:** INTG-03 fails; per-turn overhead is not negligible.
**Why it happens:** Descriptions written conversationally ("This tool allows you to find a symbol definition by providing its name...") instead of tersely ("Find symbol definitions by name or regex. Returns file:line and kind.").
**How to avoid:** Keep each `description = "..."` under 75 words. Count words during review. The second example above is 11 words — well within budget.
**Warning signs:** Tool description prose exceeding 2–3 lines in the source.

### Pitfall 4: `project_path` vs Startup Path Confusion
**What goes wrong:** Tool's `project_path` param overrides a startup-pinned path unexpectedly, or the param is ignored silently.
**Why it happens:** Two modes exist (startup-pinned via `mcp [path]` arg, and per-call override via `project_path` param) and the resolution logic is ambiguous.
**How to avoid:** Define the resolution rule clearly in implementation:
  1. If `project_path` param is provided → use it (absolute or relative-to-cwd).
  2. Else if server was started with explicit path arg → use that.
  3. Else → use `std::env::current_dir()`.

### Pitfall 5: No-Index Behavior (Claude's Discretion Item)
**What goes wrong:** Server started with a path that has no `.code-graph-cache` or has never been indexed. Tool calls immediately fail with unhelpful errors.
**Why it happens:** `build_graph()` succeeds by re-parsing all files on every cold start (Phase 4 has no persistence), but if the directory doesn't exist or is empty, the graph is empty.
**How to avoid:** On startup, if `build_graph()` returns an empty graph (0 files), return an actionable error on the first tool call: `"Graph is empty. Run 'code-graph index <path>' first."` This is the recommended approach over silent auto-index (which would make startup latency unpredictable). (This is a Claude's Discretion item — the recommendation is: return actionable error, not auto-index.)

### Pitfall 6: schemars Version Mismatch
**What goes wrong:** Compile error in `#[derive(JsonSchema)]` or rmcp proc-macro panic.
**Why it happens:** rmcp 0.16 requires schemars 1.x. The older schemars 0.8 API is different.
**How to avoid:** Use `schemars = "1"` in Cargo.toml. Verify rmcp's dependency tree: `cargo tree | grep schemars`.

### Pitfall 7: Async Runtime Missing
**What goes wrong:** Compile error or panic: "there is no reactor running".
**Why it happens:** rmcp is async; `serve_server` is an async function. Without `#[tokio::main]`, there is no runtime.
**How to avoid:** The `mcp` subcommand entrypoint must be `async`. Since `main()` is currently synchronous, the `Mcp` match arm should call `tokio::runtime::Runtime::new()?.block_on(mcp::run(path))` — or convert main to `#[tokio::main]`. Converting main is cleaner and is the recommended approach (other subcommands are sync-safe inside an async runtime via `block_in_place` if needed — but in practice all query operations are CPU-bound with no blocking I/O after graph build).

---

## Code Examples

### Verified Pattern: Minimal stdio Server Entrypoint
```rust
// Source: https://docs.rs/rmcp/latest/rmcp/service/fn.serve_server
// Source: https://www.shuttle.dev/blog/2025/07/18/how-to-build-a-stdio-mcp-server-in-rust
use rmcp::transport::stdio;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = CodeGraphServer::new(project_root);
    let server = rmcp::serve_server(service, stdio()).await?;
    server.waiting().await?;
    Ok(())
}
```

### Verified Pattern: Tool with Typed Optional Params
```rust
// Source: https://docs.rs/rmcp/latest/rmcp/handler/server/wrapper/struct.Parameters
use rmcp::{tool, tool_router};
use serde::Deserialize;
use schemars::JsonSchema;

#[derive(Deserialize, JsonSchema)]
struct FindSymbolParams {
    /// Symbol name or regex pattern (e.g. "UserService" or "User.*")
    symbol: String,
    /// Limit results (default: 20)
    limit: Option<usize>,
    /// Optional file/directory scope (relative path)
    path: Option<String>,
    /// Optional project root when server runs in multi-project mode
    project_path: Option<String>,
}

#[tool_router]
impl CodeGraphServer {
    #[tool(description = "Find symbol definitions by name or regex. Returns file:line and kind.")]
    async fn find_symbol(
        &self,
        params: Parameters<FindSymbolParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let limit = p.limit.unwrap_or(20);
        // ... query graph, format output ...
        Ok(CallToolResult::success(vec![Content::text(output_text)]))
    }
}
```

### Verified Pattern: Claude Code User-Level Registration
```bash
# Source: https://code.claude.com/docs/en/mcp
claude mcp add --scope user --transport stdio code-graph -- code-graph mcp
# Per-project with pinned path:
claude mcp add --scope project --transport stdio code-graph -- code-graph mcp /path/to/project
```

The resulting `~/.claude.json` entry (user scope):
```json
{
  "mcpServers": {
    "code-graph": {
      "type": "stdio",
      "command": "code-graph",
      "args": ["mcp"]
    }
  }
}
```

### Verified Pattern: Project-Scoped `.mcp.json`
```json
{
  "mcpServers": {
    "code-graph": {
      "command": "code-graph",
      "args": ["mcp", "."]
    }
  }
}
```

### Compact Response Format (INTG-02 Token Analysis)
The existing compact format in `query/output.rs` already achieves the target. Example comparison:

**JSON (verbose, ~45 tokens for 1 result):**
```json
[{"name":"UserService","kind":"class","file":"src/auth/user.service.ts","line":12,"col":0,"exported":true,"default":false}]
```

**Compact (target, ~12 tokens for 1 result):**
```
def UserService src/auth/user.service.ts:12 class
```

10 results: JSON ≈ 450 tokens vs compact ≈ 120 tokens → ~73% reduction. Exceeds the 60% target.

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `4t145/rmcp` (community SDK) | `modelcontextprotocol/rust-sdk` (official) | 2025 | Use `rmcp` from official org; same crate name, same API |
| `schemars = "0.8"` | `schemars = "1"` | 2024-2025 | Breaking API change; rmcp 0.16 requires v1 |
| SSE transport | stdio transport | — | For local tools like code-graph, stdio is the correct transport; SSE is deprecated for new work |
| `tool_box!` macro (older rmcp) | `#[tool_router]` + `#[tool_handler]` proc macros | rmcp ~0.3+ | Attribute macros are cleaner; `tool_box!` still works but is the older pattern |

**Deprecated/outdated:**
- `tool_box!` declarative macro: functional but superseded by `#[tool_router]` — use attribute macros.
- SSE transport: deprecated per Claude Code docs; use stdio for local servers.

---

## Open Questions

1. **Async main vs sync main**
   - What we know: The current `main()` is synchronous (`fn main() -> Result<()>`). The `mcp` subcommand requires async for `serve_server`.
   - What's unclear: Whether converting `main` to `#[tokio::main]` causes issues with the existing synchronous CLI subcommands (Index, Find, Refs, Impact, Circular, Stats, Context) — all of which call `build_graph()` synchronously.
   - Recommendation: Convert to `#[tokio::main]`. The existing sync code runs fine inside a tokio runtime (no blocking I/O after graph build — all CPU-bound). Alternatively, add `tokio::runtime::Runtime::new()?.block_on(...)` only in the `Mcp` match arm, keeping main sync.

2. **Graph state on empty/missing project**
   - What we know: Claude's discretion to decide: auto-index or return actionable error.
   - What's unclear: Whether running `build_graph()` at server startup is acceptable startup latency (~100ms for small projects, up to a few seconds for large ones). This blocks the MCP `initialize` handshake.
   - Recommendation: Build graph lazily on first tool call, not at `new()`. Store `Option<Arc<CodeGraph>>` initialized to `None`. First call triggers build. Use `tokio::sync::Mutex` for the lazy init. This keeps startup instant and puts latency on the first query (acceptable).

3. **Output formatter refactoring scope**
   - What we know: `query/output.rs` formatters use `println!` directly.
   - What's unclear: How much refactoring is acceptable to make them return `String`.
   - Recommendation: Add `format_*_as_string(...)` sibling functions that write to a `String` via `use std::fmt::Write`. Keep existing `println!`-based functions for CLI. This avoids breaking CLI tests and keeps concerns separated.

---

## Sources

### Primary (HIGH confidence)
- `/websites/rs_rmcp_rmcp` (Context7) — tool macro patterns, Parameters wrapper, serve_server signature, ServerHandler
- https://docs.rs/crate/rmcp/latest — current version (0.16.0, released 2026-02-17), feature flags
- https://code.claude.com/docs/en/mcp — Claude Code MCP configuration scopes, JSON format, `claude mcp add` syntax

### Secondary (MEDIUM confidence)
- https://www.shuttle.dev/blog/2025/07/18/how-to-build-a-stdio-mcp-server-in-rust — full working example with Cargo.toml, verified against official docs
- https://hackmd.io/@Hamze/S1tlKZP0kx — rmcp official SDK guide, individual param style vs aggregated struct style
- https://github.com/modelcontextprotocol/rust-sdk — official SDK repo, confirmed 0.16.0 release date

### Tertiary (LOW confidence)
- None used for critical claims.

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — verified rmcp 0.16.0 current version via docs.rs; feature flags confirmed via Context7 + WebFetch
- Architecture: HIGH — rmcp proc-macro patterns verified via Context7; server struct Clone requirement verified via compiler semantics
- Pitfalls: HIGH — Clone pitfall is a known Rust compile-time issue; token budget is a hard project requirement; other pitfalls derived from code analysis
- Claude Code config format: HIGH — directly verified from official Claude Code docs

**Research date:** 2026-02-23
**Valid until:** 2026-04-23 (rmcp moves fast; re-verify feature flags if >30 days pass)
