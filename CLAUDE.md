<!-- code-graph:start -->

## Code navigation -- MANDATORY

NEVER use Grep or Glob to find symbol definitions, trace references, or analyze dependencies.
ALWAYS use code-graph CLI commands instead -- they are faster, more accurate, and understand the full AST.

| Task                             | Command                       | NOT this                                     |
| -------------------------------- | ----------------------------- | -------------------------------------------- |
| Find where something is defined  | `code-graph find <symbol>`    | ~~Grep for `class X`, `function X`, `fn X`~~ |
| Find what uses/imports something | `code-graph refs <symbol>`    | ~~Grep for `import`, `require`, identifier~~ |
| Understand a symbol fully        | `code-graph context <symbol>` | ~~Multiple Grep + Read calls~~               |
| Check what breaks if I change X  | `code-graph impact <symbol>`  | ~~Manual file-by-file tracing~~              |
| Detect circular deps             | `code-graph circular`         | ~~Grep for import cycles~~                   |
| Project overview                 | `code-graph stats`            | ~~Glob + count files~~                       |

Use Read/Grep/Glob ONLY for:

- Reading full file contents before editing
- Searching for string literals, comments, TODOs, error messages
- Non-structural text searches that have nothing to do with code navigation
  <!-- code-graph:end -->
