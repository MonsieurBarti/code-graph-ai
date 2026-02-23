## Code navigation

Use code-graph MCP tools (`find_symbol`, `find_references`, `get_context`, `get_impact`) as the
primary way to navigate the codebase. Prefer these over glob/grep/read for finding definitions,
tracing references, and understanding blast radius. Fall back to file reading only when you need
the full source of a file (e.g., to edit it or review implementation details).
