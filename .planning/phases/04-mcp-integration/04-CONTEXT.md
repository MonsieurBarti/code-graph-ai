# Phase 4: MCP Integration - Context

**Gathered:** 2026-02-23
**Status:** Ready for planning

<domain>
## Phase Boundary

Expose the existing graph query engine as an MCP stdio server for Claude Code. The server surfaces graph queries as native MCP tools with token-optimized compact responses. Watch mode, persistence, and incremental re-indexing are separate phases.

</domain>

<decisions>
## Implementation Decisions

### Tool surface design
- 1:1 mapping from CLI commands to MCP tools: find_symbol, find_references, get_impact, detect_circular, get_context, plus get_stats (6 tools total)
- No reindex/index tool via MCP — indexing is a CLI concern
- Symbol name is required; file path is optional for disambiguation (if omitted, searches globally)
- Default result limit on each tool with an optional `limit` param to override (Claude picks sensible defaults)

### Response format
- Compact text lines: one result per line, fields separated by delimiters (e.g., `src/auth.ts:42 | UserService | class`)
- Always include a summary header line (total count, truncation info) before results
- Context/360-degree tool uses labeled sections (`--- callers ---`, `--- callees ---`, etc.) to separate groups
- All file paths are relative to the project root

### Error responses
- Symbol not found: return error with up to 3 fuzzy match suggestions to reduce round-trips
- Use MCP protocol's `isError` flag for all error conditions (not inline text errors)
- No staleness tracking in Phase 4 — deferred to Phase 5 (watch mode)

### Startup & onboarding
- MCP server invoked as subcommand: `code-graph mcp [path]` — defaults to cwd if path omitted
- Designed for user-level MCP registration: `{"command": "code-graph", "args": ["mcp"]}` — works across multiple projects
- Each MCP tool accepts an optional `project_path` param to scope queries to a specific project
- Can also be registered per-project with an explicit path in args: `{"command": "code-graph", "args": ["mcp", "/path/to/project"]}`

### Claude's Discretion
- No-index behavior: whether to return an actionable error or auto-index on first call
- Exact default result limits per tool type
- Delimiter choice for compact text format
- Tool parameter naming conventions

</decisions>

<specifics>
## Specific Ideas

- Tool descriptions must be under 100 tokens each (INTG-03 requirement) — concise enough that per-turn overhead is negligible
- Target ~60% fewer tokens than verbose JSON in responses (INTG-02 requirement)
- A 10-symbol result from find_symbol should fit under 200 tokens (success criterion from roadmap)

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 04-mcp-integration*
*Context gathered: 2026-02-23*
