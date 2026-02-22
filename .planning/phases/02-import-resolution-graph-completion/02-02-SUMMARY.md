---
phase: 02-import-resolution-graph-completion
plan: "02"
subsystem: parser
tags: [tree-sitter, relationships, calls, extends, implements, type-refs]
dependency_graph:
  requires:
    - 01-03-SUMMARY.md  # OnceLock/StreamingIterator/is_tsx patterns from Phase 1
  provides:
    - extract_relationships function in src/parser/relationships.rs
    - RelationshipInfo and RelationshipKind types
    - relationships field in ParseResult struct
  affects:
    - src/parser/mod.rs
tech_stack:
  added: []
  patterns:
    - OnceLock<Query> per grammar variant (TS/TSX/JS) — matches imports.rs and symbols.rs
    - StreamingIterator for tree-sitter 0.26 query cursor
    - is_tsx: bool param for grammar discrimination (Language::name() unreliable for TS/TSX)
    - Grammar-specific fallback queries (JS class heritage differs from TS layout)
key_files:
  created:
    - src/parser/relationships.rs
  modified:
    - src/parser/mod.rs
decisions:
  - "JavaScript grammar (tree-sitter-javascript 0.25) uses bare identifier in class_heritage — no extends_clause node like TypeScript; separate JS query required"
  - "extends_type_clause confirmed as correct node name for interface extends in TypeScript grammar (not extends_clause)"
  - "JS type annotation query returns None — JavaScript has no type annotations, skip that query pass entirely"
  - "from_name is None for Calls/MethodCall/TypeReference — context-free extraction; caller scope resolution deferred to Plan 03 graph wiring"
metrics:
  duration: "5 min"
  completed: "2026-02-22"
  tasks_completed: 1
  files_modified: 2
---

# Phase 2 Plan 02: Relationship Extraction Module Summary

**One-liner:** Tree-sitter query pass for symbol-level relationships — Calls, MethodCall, Extends, Implements, InterfaceExtends, TypeReference — using three separate OnceLock query sets per grammar variant.

## What Was Built

A new `src/parser/relationships.rs` module with:

- `RelationshipKind` enum: `Calls`, `MethodCall`, `Extends`, `Implements`, `InterfaceExtends`, `TypeReference`
- `RelationshipInfo` struct: `from_name: Option<String>`, `to_name: String`, `kind: RelationshipKind`, `line: usize`
- `extract_relationships(tree, source, language, is_tsx) -> Vec<RelationshipInfo>` — public extraction function
- Three tree-sitter query passes: calls query, inheritance query, type reference query
- Three OnceLock sets for TS/TSX/JS grammar variants (9 statics total, but JS skips type ref query)
- `(to_name, line, kind)` deduplication via `HashSet` — same approach as symbols.rs
- 12 unit tests covering all 6 relationship kinds plus edge cases

`src/parser/mod.rs` updated with:
- `pub mod relationships;` declaration
- `RelationshipInfo` and `extract_relationships` imports
- `relationships: Vec<RelationshipInfo>` field added to `ParseResult`
- `extract_relationships` called in `parse_file()` and stored in `ParseResult`

## Verification Results

- `cargo build`: clean compile (warnings only — pre-existing dead code from Phase 1/2 plan 1 infrastructure awaiting wiring)
- `cargo test -- relationships`: 12/12 passing
- `cargo test`: 48/48 passing (no regressions to Phase 1 functionality; 36 pre-existing + 12 new)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] JS grammar uses different class heritage layout**
- **Found during:** Task 1 (test run)
- **Issue:** The initial JS inheritance query used `(extends_clause value: (identifier))` which is correct for TypeScript grammar but the JavaScript grammar (tree-sitter-javascript 0.25) places `identifier` directly inside `class_heritage` with no `extends_clause` wrapper node
- **Fix:** Validated JS grammar via tree exploration, wrote separate JS-specific query: `(class_declaration name: (identifier) @class_name (class_heritage (identifier) @extends_name))`
- **Files modified:** `src/parser/relationships.rs` (inheritance_query JavaScript branch)
- **Commit:** b347e4d (included in task commit)

**2. [Rule 2 - Missing] JS type annotation query correctly omitted**
- **Found during:** Task 1 (implementation)
- **Issue:** JavaScript has no type annotations — compiling the TypeScript type_ref query against the JS grammar would fail. The plan noted to handle JS/TS discrimination but did not explicitly specify the JS behavior.
- **Fix:** `type_ref_query()` returns `None` for JavaScript grammar, skipping that query pass entirely. No JS_TYPE_REF_QUERY static allocated.
- **Files modified:** `src/parser/relationships.rs`

## Key Technical Discoveries

1. `extends_type_clause` (not `extends_clause`) is the correct node for interface extends in the TypeScript grammar — confirmed via live tree exploration before implementation.

2. The JavaScript grammar uses `identifier` for class names (not `type_identifier`), and `class_heritage` contains the parent `identifier` directly without an intervening `extends_clause` node.

3. The plan's query for calls uses `function:` field names — these work correctly in tree-sitter 0.26 for TypeScript grammar even though the raw AST shows them without field names when printed without field annotations.

## Self-Check: PASSED
