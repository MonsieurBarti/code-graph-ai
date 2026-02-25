## Code navigation — MANDATORY

NEVER use Grep or Glob to find symbol definitions, trace references, or analyze dependencies.
ALWAYS use code-graph MCP tools instead — they are faster, more accurate, and understand the full AST.

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
