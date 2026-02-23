---
phase: 04-mcp-integration
plan: 01
subsystem: infrastructure
tags: [mcp, tokio, rmcp, schemars, cli, output-formatters]
dependency-graph:
  requires: []
  provides: [rmcp-deps, tokio-runtime, mcp-cli-subcommand, format-to-string-api]
  affects: [src/main.rs, src/cli.rs, src/query/output.rs, Cargo.toml]
tech-stack:
  added: [rmcp 0.16, tokio 1 (full), schemars 1]
  patterns: [async-main, String-returning-formatters, mod-declaration-placeholder]
key-files:
  created: [src/mcp/mod.rs]
  modified: [Cargo.toml, src/cli.rs, src/main.rs, src/query/output.rs]
decisions:
  - "format_*_to_string functions write summary header FIRST (CONTEXT.md locked decision)"
  - "format_impact_to_string uses flat format (no tree mode) for MCP — indentation adds parsing ambiguity"
  - "format_context_to_string uses labeled section delimiters per CONTEXT.md locked decision"
  - "Converted main() to #[tokio::main] async fn — all existing sync subcommands work correctly inside async runtime"
metrics:
  duration: 3 min
  completed: 2026-02-23
---

# Phase 4 Plan 01: MCP Infrastructure Layer Summary

Added MCP infrastructure: rmcp 0.16 + tokio + schemars dependencies, async main with Commands::Mcp CLI subcommand, placeholder mcp module, and 6 `format_*_to_string` sibling functions returning String for MCP tool response consumption.

## Tasks Completed

| # | Task | Commit | Files |
|---|------|--------|-------|
| 1 | Add MCP deps, Mcp CLI subcommand, async main | e14a286 | Cargo.toml, Cargo.lock, src/cli.rs, src/main.rs, src/mcp/mod.rs |
| 2 | Add 6 format_*_to_string functions to output.rs | 305a246 | src/query/output.rs |

## What Was Built

### Task 1: Dependencies and CLI Subcommand

**Cargo.toml** — Added 3 new dependencies:
- `rmcp = { version = "0.16", features = ["server", "transport-io", "macros"] }` — official Anthropic Rust MCP SDK
- `tokio = { version = "1", features = ["full"] }` — async runtime required by rmcp
- `schemars = "1"` — JSON Schema generation for MCP tool parameter structs

**src/cli.rs** — Added `Commands::Mcp` variant:
```rust
/// Start an MCP stdio server exposing graph queries as tools for Claude Code.
Mcp {
    /// Path to the project root (defaults to current directory if omitted).
    path: Option<PathBuf>,
},
```

**src/main.rs** — Converted to async main and added Mcp arm:
- `fn main() -> Result<()>` → `#[tokio::main] async fn main() -> Result<()>`
- Added `mod mcp;` declaration
- Added `Commands::Mcp { path }` match arm with placeholder message

**src/mcp/mod.rs** — Created placeholder module file with comment only; Plan 02 populates it.

### Task 2: String-Returning Output Formatters

Added 6 sibling functions in `src/query/output.rs`. All return `String` via `std::fmt::Write`, keeping existing `println!`-based CLI formatters untouched:

| Function | Signature | Summary Header |
|----------|-----------|---------------|
| `format_find_to_string` | `(results: &[FindResult], project_root: &Path) -> String` | `{N} definitions found` |
| `format_stats_to_string` | `(stats: &ProjectStats) -> String` | `{N} files, {M} symbols` |
| `format_refs_to_string` | `(results: &[RefResult], project_root: &Path) -> String` | `{N} references found` |
| `format_impact_to_string` | `(results: &[ImpactResult], project_root: &Path) -> String` | `{N} affected files` |
| `format_circular_to_string` | `(cycles: &[CircularDep], project_root: &Path) -> String` | `{N} circular dependencies found` |
| `format_context_to_string` | `(contexts: &[SymbolContext], project_root: &Path) -> String` | `{N} symbols` |

The `format_context_to_string` function uses labeled sections (`--- definitions ---`, `--- references ---`, `--- callers ---`, `--- callees ---`, `--- extends ---`, `--- implements ---`, `--- extended-by ---`, `--- implemented-by ---`) per the CONTEXT.md locked decision, so Claude can parse relationship groups unambiguously.

## Decisions Made

1. **Summary header FIRST**: Per CONTEXT.md locked decision, every `format_*_to_string` writes the count summary as the very first line before any result lines. This lets Claude count-check results at a glance without scanning to the end.

2. **Flat format for impact**: `format_impact_to_string` intentionally uses the flat (non-tree) mode. Tree indentation adds parsing ambiguity in MCP context where Claude reads raw text; flat lists are unambiguous.

3. **Labeled sections for context**: `format_context_to_string` uses `--- section-name ---` delimiters between relationship groups rather than the compact CLI format. This matches the CONTEXT.md locked decision and allows Claude to parse sections reliably.

4. **Async main (not block_on in arm)**: Converted `main()` to `#[tokio::main]` rather than using `tokio::runtime::Runtime::new()?.block_on(...)` in the Mcp arm only. The research noted both approaches work; async main is cleaner and all existing sync CLI operations work correctly inside a tokio runtime.

## Verification

- `cargo build` — clean build, zero errors, 7 pre-existing dead-code warnings (unchanged)
- `cargo test` — 87 tests pass, 0 failed, 0 regressions
- `./target/debug/code-graph --help` — shows `mcp` subcommand
- `./target/debug/code-graph mcp` — prints "MCP server not yet implemented" (no panic)
- 6 `pub fn format_*_to_string` functions confirmed present in `src/query/output.rs`

## Deviations from Plan

None — plan executed exactly as written.

## Self-Check: PASSED

- [x] `src/mcp/mod.rs` exists
- [x] Commits e14a286 and 305a246 exist in git log
- [x] 6 `format_*_to_string` functions present in `src/query/output.rs`
- [x] `cargo build` succeeds
- [x] `cargo test` passes (87/87)
