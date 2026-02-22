---
phase: 01-foundation-core-parsing
plan: 03
subsystem: parser
tags: [rust, tree-sitter, imports, exports, esm, cjs, dynamic-import, serde-json, pipeline]

# Dependency graph
requires:
  - "01-01 (CLI, file walker, Cargo scaffold)"
  - "01-02 (tree-sitter parser, symbol extraction, CodeGraph)"
provides:
  - "extract_imports() for ESM static, CJS require, and dynamic import() extraction"
  - "extract_exports() for named, default, re-export, and re-export-all extraction"
  - "IndexStats struct with serde::Serialize for JSON output"
  - "print_summary() with human-readable cargo-style and --json modes"
  - "Full code-graph index . pipeline: walk -> parse -> extract -> graph -> summary"
affects: [02-import-resolution, 03-query-api, all-subsequent-phases]

# Tech tracking
tech-stack:
  added:
    - "serde Serialize derive on IndexStats for --json mode"
    - "serde_json::to_string_pretty for structured JSON output"
  patterns:
    - "Per-grammar OnceLock<Query> statics with is_tsx bool discriminator (not Language::name())"
    - "is_tsx bool parameter mirrors extract_symbols() convention across all extractor functions"
    - "Filter require() by function name in code, not #eq? predicate (tree-sitter 0.26 StreamingIterator)"
    - "Namespace import name extracted by child kind (identifier), not field name (no field assigned)"
    - "Error tolerance: read/parse failures increment skipped counter and continue"
    - "Verbose/warning output to stderr; human-readable and JSON output to stdout"

key-files:
  created:
    - src/parser/imports.rs
    - src/output.rs
  modified:
    - src/parser/mod.rs
    - src/main.rs

key-decisions:
  - "is_tsx bool param added to extract_imports/extract_exports — Language::name() returns None for TS/TSX in tree-sitter 0.26, so file extension is the only reliable discriminator"
  - "Three separate OnceLock sets (TS/TSX/JS) per query type to prevent cross-grammar cache contamination"
  - "#eq? predicate removed from CJS require query — tree-sitter 0.26 StreamingIterator does not auto-filter predicates; filter by function name in code instead"
  - "Namespace import identifier extracted by child kind (identifier), not field name — tree-sitter assigns no field name to the identifier in namespace_import nodes"
  - "Malformed files continue indexing — tree-sitter is permissive (error nodes); only read errors and actual None returns increment skipped counter"

patterns-established:
  - "All extraction functions (symbols, imports, exports) follow identical signature: (tree, source, language, is_tsx)"
  - "Query caches grouped by function type and grammar — TS_IMPORT_QUERY, TSX_IMPORT_QUERY, JS_IMPORT_QUERY pattern"
  - "Serial file processing — parallel is deferred to Phase 6 per research recommendation"

requirements-completed: [PARS-03, PARS-04]

# Metrics
duration: 31min
completed: 2026-02-22
---

# Phase 1 Plan 03: Import/Export Extraction and Full Indexing Pipeline Summary

**Tree-sitter import/export extraction (ESM/CJS/dynamic, named/default/re-export) wired into a complete code-graph index . pipeline with cargo-style summary, --json mode, and malformed-file tolerance**

## Performance

- **Duration:** 31 min
- **Started:** 2026-02-22T17:29:24Z
- **Completed:** 2026-02-22T18:00:54Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments

- Implemented `src/parser/imports.rs`: ImportInfo/ExportInfo structs with full kind enums, `extract_imports()` covering all 3 import types (ESM static, CJS require, dynamic), `extract_exports()` covering all 4 export types (named, default, re-export, re-export-all), 11 unit tests (9 spec + 2 regression)
- Implemented `src/output.rs`: `IndexStats` with serde Serialize, `print_summary()` with cargo-style human-readable format and --json mode, skipped-file warning to stderr
- Updated `src/parser/mod.rs`: `ParseResult` extended with `imports` and `exports` fields, `parse_file()` calls all three extractors
- Updated `src/main.rs`: full indexing pipeline (walk -> read -> parse -> graph -> stats -> summary) with serial file processing, graceful skip-on-error, verbose per-file output, and elapsed-time tracking

## Task Commits

Each task was committed atomically:

1. **Task 1: Import and export extraction queries** - `fbe0fdf` (feat)
2. **Task 2: Full indexing pipeline and summary output** - `94414fa` (feat)

**Auto-fix: OnceLock cross-grammar contamination** - `bc38411` (fix)

**Plan metadata:** (docs commit — see below)

## Files Created/Modified

- `src/parser/imports.rs` — ImportInfo, ExportInfo, ImportKind, ExportKind; extract_imports(); extract_exports(); 11 tests
- `src/output.rs` — IndexStats (serde::Serialize), print_summary() with human-readable and JSON modes
- `src/parser/mod.rs` — ParseResult extended with imports/exports fields; parse_file() calls extract_imports/exports
- `src/main.rs` — Full index pipeline orchestration: walk->parse->graph->summary; verbose and JSON flags wired

## Decisions Made

- `is_tsx: bool` parameter added to `extract_imports` and `extract_exports` — `Language::name()` returns `None` for TypeScript and TSX grammars in tree-sitter 0.26 (returns `Some("javascript")` only for JS). Using `is_tsx` derived from file extension is the reliable discriminator, consistent with `extract_symbols()`.
- Three separate `OnceLock<Query>` statics per query type (TS/TSX/JS) to prevent cross-grammar contamination — a query compiled for the TSX grammar cannot be used on a TypeScript tree.
- Removed `#eq?` predicate from CJS require query — tree-sitter 0.26 `StreamingIterator` does not auto-filter text predicates; filter for `"require"` function name in code instead.
- Namespace import identifier found by child kind, not `child_by_field_name("name")` — the tree-sitter TypeScript grammar does not assign a field name to the identifier in `namespace_import` nodes.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed OnceLock cross-grammar query contamination**
- **Found during:** Task 2 (end-to-end verification — imports showed 0 when TSX files processed before TS)
- **Issue:** `LangGroup` detection used `Language::name().unwrap_or("")` with `.contains("tsx")` check. But `Language::name()` returns `None` for both TypeScript and TSX grammars (only JS returns `Some`). Both TS and TSX fell to the same `TypeScript` group, sharing a single `OnceLock<Query>` initialized with whichever grammar ran first. When a TSX file was processed first, the TSX-compiled query was then used on TS trees (query mismatch = no matches).
- **Fix:** Added `is_tsx: bool` parameter to `extract_imports()` and `extract_exports()`, mirroring the established `extract_symbols()` convention. `lang_group` now uses `is_tsx` for TS/TSX discrimination, `Language::name()` only for JS detection.
- **Files modified:** `src/parser/imports.rs`, `src/parser/mod.rs`
- **Verification:** Added `test_tsx_then_ts_imports` regression test (passes). End-to-end `code-graph index /tmp/cg-test` shows 3 imports correctly.
- **Committed in:** `bc38411` (dedicated fix commit)

**2. [Rule 1 - Bug] Fixed namespace import specifier extraction**
- **Found during:** Task 1 (test `test_esm_namespace_import` failed with 0 specifiers)
- **Issue:** `namespace_import` identifier extracted via `child_by_field_name("name")` which returned `None`. Tree dump showed the identifier is a plain child with no field name assigned in the grammar.
- **Fix:** Added `extract_namespace_import_name()` helper that finds the `identifier` child by kind, not field name.
- **Files modified:** `src/parser/imports.rs`
- **Verification:** `test_esm_namespace_import` passes: `import * as path from 'path'` → 1 import, 1 namespace specifier.
- **Committed in:** `fbe0fdf` (Task 1 commit)

**3. [Rule 1 - Bug] Removed #eq? predicate from CJS require query**
- **Found during:** Task 1 (test `test_cjs_require` failed with 0 imports)
- **Issue:** tree-sitter 0.26 `StreamingIterator` does not auto-filter matches using text predicates like `#eq?`. All `call_expression(identifier, ...)` patterns matched regardless of the identifier name.
- **Fix:** Removed `#eq? @fn "require"` from query. Capture `@fn` and filter for `"require"` in Rust code instead.
- **Files modified:** `src/parser/imports.rs`
- **Verification:** `test_cjs_require` passes: `const fs = require('fs')` → 1 CJS import.
- **Committed in:** `fbe0fdf` (Task 1 commit)

---

**Total deviations:** 3 auto-fixed (Rule 1 - bugs)
**Impact on plan:** All fixes necessary for correct import extraction. No scope creep. The OnceLock fix is a key discovery documented in STATE.md: `Language::name()` returns `None` for TS/TSX in tree-sitter 0.26.

## Issues Encountered

- `Language::name()` returning `None` for TypeScript/TSX grammars is a tree-sitter 0.26 behavior difference from the documented API. Future phases using OnceLock query caching must use `is_tsx: bool` + JS name check pattern, not `Language::name()` alone.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Full `code-graph index .` pipeline is operational — Phase 1 is complete
- `ParseResult` carries symbols, imports, exports — Phase 2 (import resolution) can use these directly
- Import/export data types are defined — Phase 2 can add graph edges from ImportInfo.module_path
- All Phase 1 success criteria from ROADMAP.md are met
- 24 tests passing (4 graph + 9 symbols + 11 imports/exports)
- No blockers

---
*Phase: 01-foundation-core-parsing*
*Completed: 2026-02-22*
