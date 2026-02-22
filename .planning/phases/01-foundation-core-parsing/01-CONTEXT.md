# Phase 1: Foundation & Core Parsing - Context

**Gathered:** 2026-02-22
**Status:** Ready for planning

<domain>
## Phase Boundary

A runnable Rust binary that walks a TypeScript/JavaScript project, parses every source file with tree-sitter, extracts symbols and imports/exports, and stores them in an in-memory graph. The `code-graph index .` command completes the full indexing pipeline. Import resolution, queries, and MCP exposure are separate phases.

</domain>

<decisions>
## Implementation Decisions

### CLI output & feedback
- Silent during indexing by default — no progress output
- Final summary shows breakdown by symbol type: "240 functions, 85 classes, 120 interfaces..." with total file count and elapsed time
- `-v` verbose flag from the start — shows file-by-file parsing output for debugging
- Human-readable output by default, `--json` flag for structured JSON output
- Error/skip counts appear in summary only when files were actually skipped

### Symbol granularity
- Top-level and exported const arrow functions are symbols (the dominant modern TS pattern)
- Both class methods AND object literal methods are symbols — maximum granularity
- React components detected via JSX return and tagged as "component" type in addition to being a function
- Interface properties and methods tracked as child symbols — enables finding who uses a specific field
- Standard symbol types: functions, classes, interfaces, type aliases, enums, exported variables

### Project configuration
- Config file: `code-graph.toml` at project root (like rustfmt.toml)
- Project root = current working directory where `code-graph index .` is run (no auto-detection magic)
- File exclusions: respect .gitignore AND always auto-exclude node_modules
- Additional exclusions configurable via code-graph.toml
- Basic monorepo awareness from Phase 1: detect workspaces from package.json and index all packages in one pass

### Error tolerance
- Malformed/unparseable files: skip the entire file, log a warning, continue indexing
- Unsupported extensions (.vue, .svelte, .coffee): silently skip — only process .ts/.tsx/.js/.jsx
- Permission errors (unreadable files): same as parse errors — skip and include in error count
- Error count appears in final summary only when files were actually skipped
- Lenient overall: never fail the whole indexing run due to individual file issues

### Claude's Discretion
- Exact summary formatting and layout
- Internal graph data structures and memory layout
- Tree-sitter query patterns for symbol extraction
- Config file schema details beyond exclusions

</decisions>

<specifics>
## Specific Ideas

- Output feel should be like cargo or gh cli — clean, professional, not noisy
- `--json` flag behavior modeled after gh cli (human default, JSON when flagged)
- Monorepo detection reads `workspaces` field from package.json

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 01-foundation-core-parsing*
*Context gathered: 2026-02-22*
