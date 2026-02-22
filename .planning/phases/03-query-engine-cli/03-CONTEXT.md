# Phase 3: Query Engine & CLI - Context

**Gathered:** 2026-02-22
**Status:** Ready for planning

<domain>
## Phase Boundary

All graph queries (definition, references, impact/blast radius, circular dependencies, 360-degree context, stats) exposed as separate CLI subcommands. Developers and Claude can query the indexed graph from the terminal. MCP integration is Phase 4 — this phase delivers CLI-only access.

</domain>

<decisions>
## Implementation Decisions

### Output format & display
- Default output format is **compact/token-optimized** — designed for minimal token consumption (AI agent use case)
- Human-readable mode via `--format table` flag
- Three format values: `compact` (default), `table`, `json`
- Color/ANSI styling enabled in table format with **auto-detection** (color when stdout is a terminal, plain when piped)
- No color in compact format (token waste)

### Symbol matching
- When multiple symbols match a name, **list all matches** (don't error or require qualification)
- **Regex support** for symbol patterns (e.g., `User.*Service`)
- **Case-sensitive by default**, `-i` flag for case-insensitive
- **`--kind` flag** to filter results by symbol kind (e.g., `--kind function,class`)
- **`--file` flag** to scope search to a specific file or directory path

### CLI command structure
- **Separate subcommands**, not flags on a single `query` command
- Subcommand names: `find`, `refs`, `impact`, `circular`, `stats`, `context`
  - `find <symbol>` — locate definition (file:line)
  - `refs <symbol>` — find all references
  - `impact <symbol>` — transitive blast radius
  - `circular` — detect circular dependencies
  - `stats` — project overview (file count, symbol count, module structure)
  - `context <symbol>` — 360-degree view (definition + refs + callers/callees in one shot)
- `index` subcommand already exists from Phase 1

### Impact analysis
- Default output is **flat list** (compact), `--tree` flag for hierarchical dependency chain view
- **Circular dependency detection at file level** (A.ts → B.ts → A.ts), not symbol level

### Claude's Discretion
- Compact format exact layout (Claude designs for maximum token efficiency)
- Impact analysis default depth strategy (unlimited vs capped)
- Summary count display strategy (when to show "N files affected")
- Tree view indentation and formatting
- Error message design
- Help text verbosity

</decisions>

<specifics>
## Specific Ideas

- Default format should be optimized for AI consumption (token-minimal) — this is the primary use case before MCP wraps it
- "I want it to use as few tokens as possible by default, but when used by a human we should have something we can easily understand"
- `--format table` as the explicit human-friendly switch, not `--pretty`

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 03-query-engine-cli*
*Context gathered: 2026-02-22*
