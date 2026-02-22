---
phase: 01-foundation-core-parsing
plan: 02
subsystem: parser
tags: [rust, tree-sitter, petgraph, symbol-extraction, tsx, jsx, typescript, javascript]

# Dependency graph
requires:
  - "01-01 (Cargo project scaffold, CLI, file walker)"
provides:
  - "tree-sitter parser infrastructure selecting correct grammar by file extension"
  - "OnceLock-cached symbol queries for TS, TSX, and JS grammars"
  - "extract_symbols() returning (SymbolInfo, Vec<SymbolInfo>) tuples for parent+child symbols"
  - "parse_file() orchestration returning ParseResult with symbols and tree"
  - "CodeGraph with StableGraph, file_index, symbol_index, add_file/add_symbol/add_child_symbol"
affects: [01-03, all-subsequent-phases]

# Tech tracking
tech-stack:
  added:
    - "tree-sitter StreamingIterator — QueryMatches uses streaming (not standard) iterator in 0.26"
    - "Language::name() — used to distinguish JS from TS/TSX for query selection"
  patterns:
    - "OnceLock<Query> for compiled query caching (one per language grammar)"
    - "StreamingIterator via tree_sitter::StreamingIterator import"
    - "detect_export() checks node itself + ancestors for export_statement"
    - "Child symbols extracted via interface_body/class_body traversal"
    - "JSX component heuristic: contains_jsx() recursive descent on function body"

key-files:
  created:
    - src/parser/mod.rs
    - src/parser/languages.rs
    - src/parser/symbols.rs
    - src/graph/mod.rs
    - src/graph/node.rs
    - src/graph/edge.rs
  modified:
    - src/main.rs

key-decisions:
  - "Use Language::name() (returns 'javascript'/'typescript'/'typescript_tsx') to distinguish grammars for query selection — version() method does not exist in tree-sitter 0.26"
  - "detect_export() starts from sym_node itself (not parent) because @symbol capture IS the export_statement for arrow-fn patterns"
  - "OnceLock<Query> static per language — compiled once, reused across all files"
  - "De-duplicate symbols by (name, row) to handle overlapping query patterns (arrow fn + exported var patterns both match)"

patterns-established:
  - "TreeSitter 0.26 uses StreamingIterator — must import tree_sitter::StreamingIterator to iterate QueryMatches"
  - "Child symbols (interface properties, class methods) stored via ChildOf edges for query-time traversal"

requirements-completed: [PARS-02]

# Metrics
duration: 29min
completed: 2026-02-22
---

# Phase 1 Plan 02: Tree-sitter Parser Infrastructure and Symbol Extraction Summary

**Tree-sitter parser with grammar-per-extension selection, OnceLock query caching, full symbol extraction (functions, classes, interfaces, type aliases, enums, variables, arrow functions, React components), and petgraph-backed CodeGraph data structures**

## Performance

- **Duration:** 29 min
- **Started:** 2026-02-22T16:53:55Z
- **Completed:** 2026-02-22T17:22:00Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments

- Implemented `src/graph/node.rs`: `SymbolKind` (9 variants), `SymbolInfo`, `FileInfo`, `GraphNode` with full Debug/Clone derives
- Implemented `src/graph/edge.rs`: `EdgeKind` enum (Contains, Imports, Exports, ChildOf)
- Implemented `src/graph/mod.rs`: `CodeGraph` wrapping `StableGraph<GraphNode, EdgeKind, Directed>` with file_index/symbol_index lookup maps, add_file/add_symbol/add_child_symbol/file_count/symbol_count/symbols_by_kind methods — 4 unit tests pass
- Implemented `src/parser/languages.rs`: `language_for_extension()` routing `.ts` to `LANGUAGE_TYPESCRIPT`, `.tsx` to `LANGUAGE_TSX`, `.js/.jsx` to `LANGUAGE` — mandatory grammar split enforced
- Implemented `src/parser/symbols.rs`: three separate query strings (TS/TSX/JS), three `OnceLock<Query>` statics, `extract_symbols()` with StreamingIterator usage, JSX component heuristic, interface/class child extraction, export detection — 9 unit tests pass
- Implemented `src/parser/mod.rs`: `parse_file()` orchestrating grammar selection, tree-sitter parsing, and symbol extraction, returning `ParseResult { symbols, tree }`
- Registered `mod graph;` and `mod parser;` in `main.rs`

## Task Commits

1. **Task 1: Graph data structures (nodes, edges, CodeGraph)** - `8c3f66a` (feat)
2. **Task 2: Tree-sitter parser infrastructure and symbol extraction** - `4df2613` (feat)

## Files Created/Modified

- `src/graph/node.rs` — SymbolKind enum (Function/Class/Interface/TypeAlias/Enum/Variable/Component/Method/Property), SymbolInfo, FileInfo, GraphNode
- `src/graph/edge.rs` — EdgeKind enum (Contains, Imports{specifier}, Exports{name,is_default}, ChildOf)
- `src/graph/mod.rs` — CodeGraph struct, O(1) file/symbol lookup indexes, 4 tests
- `src/parser/languages.rs` — language_for_extension() with mandatory TS/TSX split
- `src/parser/symbols.rs` — OnceLock query cache, extract_symbols(), 9 tests
- `src/parser/mod.rs` — parse_file() returning ParseResult
- `src/main.rs` — added `mod graph;` and `mod parser;`

## Decisions Made

- `detect_export()` checks `sym_node` itself first, not just ancestors — because the `@symbol` capture for arrow-function patterns IS the `export_statement` node; ancestor-only walk missed it
- Used `Language::name()` instead of `Language::version()` for grammar identification — `version()` does not exist in tree-sitter 0.26; the name method returns "javascript"/"typescript"/"typescript_tsx"
- De-duplicate by (name, row) to handle overlapping query patterns that both match an exported arrow function
- Compiled three separate query strings (TS/TSX/JS) rather than one universal query — JS grammar lacks interface/type_alias/enum nodes, causing query compilation errors if those patterns are included

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed detect_export to check node itself, not just ancestors**
- **Found during:** Task 2 (test run — test_export_const_arrow_function and test_tsx_component_detection failed)
- **Issue:** `detect_export()` started walking from `node.parent()`. For exported arrow functions the `@symbol` capture IS the `export_statement`, so `parent()` is `program` — no `export_statement` found in ancestors.
- **Fix:** Changed loop to start from `Some(node)` so the node itself is checked first.
- **Files modified:** `src/parser/symbols.rs`
- **Commit:** `4df2613` (fix included in task commit)

**2. [Rule 3 - Blocking] Adapted to tree-sitter 0.26 StreamingIterator API**
- **Found during:** Task 2 (cargo build — `QueryMatches` is not an Iterator)
- **Issue:** The plan's research described `for m in matches {}` but tree-sitter 0.26 uses `StreamingIterator`, not standard `Iterator`. Also `Language::version()` does not exist (it's `abi_version()`), and `child(i)` takes `u32` not `usize`.
- **Fix:** Added `use tree_sitter::StreamingIterator`, changed loop to `while let Some(m) = matches.next()`, used `Language::name()` for language identification, cast index to `i as u32`.
- **Files modified:** `src/parser/symbols.rs`
- **Commit:** `4df2613`

---

**Total deviations:** 2 auto-fixed (Rule 1 - bug, Rule 3 - blocking API mismatch)
**Impact on plan:** Both required for correctness. No scope creep.

## Issues Encountered

- `tree-sitter 0.26` `QueryMatches` requires `StreamingIterator` trait in scope to iterate — research examples showed the old iterator pattern which no longer works
- `Language::name()` returns `"typescript_tsx"` for the TSX grammar — used this to distinguish TS from TSX when `is_tsx` parameter alone wasn't sufficient for query selection

## User Setup Required

None.

## Next Phase Readiness

- Parser produces `Vec<(SymbolInfo, Vec<SymbolInfo>)>` for any `.ts/.tsx/.js/.jsx` file
- CodeGraph data structures ready to store files, symbols, and edges
- Plan 03 (import/export extraction) can use `ParseResult.tree` directly — no re-parsing needed
- No blockers

## Self-Check: PASSED

All artifacts verified:
- FOUND: src/graph/node.rs
- FOUND: src/graph/edge.rs
- FOUND: src/graph/mod.rs
- FOUND: src/parser/mod.rs
- FOUND: src/parser/languages.rs
- FOUND: src/parser/symbols.rs
- FOUND: .planning/phases/01-foundation-core-parsing/01-02-SUMMARY.md
- FOUND commit 8c3f66a (Task 1: graph data structures)
- FOUND commit 4df2613 (Task 2: parser infrastructure)
- All 13 tests pass (4 graph + 9 parser)
