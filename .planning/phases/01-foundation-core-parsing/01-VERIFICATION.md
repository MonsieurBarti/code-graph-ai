---
phase: 01-foundation-core-parsing
verified: 2026-02-22T18:30:00Z
status: passed
score: 5/5 must-haves verified
re_verification: false
---

# Phase 1: Foundation & Core Parsing — Verification Report

**Phase Goal:** A runnable Rust binary that can walk a TypeScript/JavaScript project, parse every source file with tree-sitter, extract symbols, and persist them in an in-memory graph
**Verified:** 2026-02-22T18:30:00Z
**Status:** PASSED
**Re-verification:** No — initial verification

---

## Goal Achievement

### Observable Truths (Success Criteria from ROADMAP.md)

| #  | Truth                                                                                                          | Status     | Evidence                                                                                                                 |
|----|----------------------------------------------------------------------------------------------------------------|------------|--------------------------------------------------------------------------------------------------------------------------|
| 1  | Running `code-graph index .` on a TS project completes without errors and reports file and symbol counts       | VERIFIED   | Live run on `/tmp/cg-test2` produced "Indexed 4 files in 0.14s  4 functions, 2 classes, 2 interfaces..." with exit 0    |
| 2  | Tool discovers all .ts/.tsx/.js/.jsx files while correctly excluding paths in .gitignore                       | VERIFIED   | `node_modules/pkg/index.js` excluded, `dist/bundle.js` (in .gitignore) excluded; only 4 source files discovered         |
| 3  | Tool extracts functions, classes, interfaces, type aliases, enums, and exported variables from each file       | VERIFIED   | All 7 symbol kinds tested via `cargo test` (24 passing). Live run extracted all types. OnceLock query caching confirmed  |
| 4  | Tool extracts ESM imports, CJS require calls, and dynamic imports with string literals from each file          | VERIFIED   | 3 import kind tests pass (test_esm_named_imports, test_cjs_require, test_dynamic_import). Live run: 6 imports on 2 files |
| 5  | Tool extracts named exports, default exports, and re-exports from each file                                    | VERIFIED   | 4 export kind tests pass (test_named_export, test_default_export, test_reexport, test_reexport_all). Live: 5 exports     |

**Score:** 5/5 truths verified

---

### Required Artifacts

#### Plan 01-01 Artifacts

| Artifact          | Expected                                              | Status     | Details                                                                                              |
|-------------------|-------------------------------------------------------|------------|------------------------------------------------------------------------------------------------------|
| `Cargo.toml`      | Project manifest with all Phase 1 dependencies        | VERIFIED   | Contains tree-sitter=0.26, petgraph=0.6, clap=4, serde=1, ignore=0.4, glob=0.3, anyhow=1, toml=0.8 |
| `src/main.rs`     | Entry point dispatching CLI commands                  | VERIFIED   | 132 lines; full pipeline: Cli::parse → walk_project → CodeGraph → parse_file loop → print_summary   |
| `src/cli.rs`      | Clap derive CLI structs with index subcommand         | VERIFIED   | Exports Cli (Parser) and Commands enum with Index { path, verbose, json }                           |
| `src/config.rs`   | code-graph.toml parsing with serde                    | VERIFIED   | Exports CodeGraphConfig with exclude field; load() reads toml, defaults when absent                  |
| `src/walker.rs`   | File discovery with gitignore, node_modules, monorepo | VERIFIED   | walk_project() + detect_workspace_roots(); node_modules excluded by path component check             |

#### Plan 01-02 Artifacts

| Artifact                  | Expected                                          | Status     | Details                                                                                         |
|---------------------------|---------------------------------------------------|------------|-------------------------------------------------------------------------------------------------|
| `src/parser/mod.rs`       | parse_file() orchestration                        | VERIFIED   | parse_file() calls language_for_extension, set_language, parse, extract_symbols/imports/exports |
| `src/parser/languages.rs` | Language selection by file extension              | VERIFIED   | ts→LANGUAGE_TYPESCRIPT, tsx→LANGUAGE_TSX, js/jsx→LANGUAGE; mandatory grammar split enforced     |
| `src/parser/symbols.rs`   | Tree-sitter queries for symbol extraction         | VERIFIED   | 3 query strings (TS/TSX/JS), 3 OnceLock statics, extract_symbols(), 9 unit tests                |
| `src/graph/mod.rs`        | CodeGraph wrapping StableGraph with lookup index  | VERIFIED   | StableGraph<GraphNode, EdgeKind, Directed>, file_index, symbol_index, all methods; 4 unit tests  |
| `src/graph/node.rs`       | GraphNode enum (File, Symbol) with all kinds      | VERIFIED   | GraphNode::File(FileInfo), GraphNode::Symbol(SymbolInfo); SymbolKind has 9 variants              |
| `src/graph/edge.rs`       | EdgeKind enum (Contains, Imports, Exports, ChildOf) | VERIFIED | All 4 variants present with correct field definitions                                            |

#### Plan 01-03 Artifacts

| Artifact               | Expected                                        | Status     | Details                                                                                         |
|------------------------|-------------------------------------------------|------------|-------------------------------------------------------------------------------------------------|
| `src/parser/imports.rs` | Import and export extraction                   | VERIFIED   | extract_imports() (ESM/CJS/dynamic) + extract_exports() (named/default/reexport/all); 11 tests  |
| `src/output.rs`         | Summary formatting (human-readable and JSON)   | VERIFIED   | print_summary() with cargo-style and JSON modes; IndexStats with serde::Serialize               |
| `src/main.rs`           | Full indexing pipeline orchestration           | VERIFIED   | 132 lines; walk→parse→graph→stats→summary; --verbose and --json wired; skipped counter          |

---

### Key Link Verification

#### Plan 01-01 Key Links

| From            | To              | Via                           | Status  | Evidence                                                           |
|-----------------|-----------------|-------------------------------|---------|--------------------------------------------------------------------|
| `src/main.rs`   | `src/cli.rs`    | `Cli::parse()` dispatch       | WIRED   | `use cli::{Cli, Commands}; Cli::parse()` on line 20               |
| `src/walker.rs` | `src/config.rs` | Config exclusions in walker   | WIRED   | `use crate::config::CodeGraphConfig` on line 5; used in `is_excluded_by_config()` |

#### Plan 01-02 Key Links

| From                      | To                       | Via                                        | Status  | Evidence                                                              |
|---------------------------|--------------------------|--------------------------------------------|---------|-----------------------------------------------------------------------|
| `src/parser/mod.rs`       | `src/parser/languages.rs`| `language_for_extension()` call            | WIRED   | `use languages::language_for_extension` + call on line 49            |
| `src/parser/symbols.rs`   | `src/graph/node.rs`      | Symbol extraction produces SymbolKind      | WIRED   | `use crate::graph::node::{SymbolInfo, SymbolKind}` + all variants used |
| `src/graph/mod.rs`        | `src/graph/node.rs`      | StableGraph parameterized with GraphNode   | WIRED   | `StableGraph<GraphNode, EdgeKind, Directed>` on line 16              |

#### Plan 01-03 Key Links

| From                   | To                   | Via                                       | Status  | Evidence                                                                 |
|------------------------|----------------------|-------------------------------------------|---------|--------------------------------------------------------------------------|
| `src/main.rs`          | `src/parser/mod.rs`  | `parse_file()` called per file            | WIRED   | `parser::parse_file(file_path, &source)` on line 68                    |
| `src/main.rs`          | `src/graph/mod.rs`   | CodeGraph populated with results          | WIRED   | `graph.add_file(...)`, `graph.add_symbol(...)`, `graph.add_child_symbol(...)` on lines 80-87 |
| `src/main.rs`          | `src/output.rs`      | `print_summary` called after indexing     | WIRED   | `use output::{IndexStats, print_summary}` + call on line 127           |
| `src/parser/imports.rs`| `src/graph/edge.rs`  | Import/export data maps to EdgeKind       | PARTIAL | EdgeKind variants (Imports, Exports) are defined but not yet added to graph in main.rs. Counts accumulated only. This is a designed deferral — plan states "actual resolution is Phase 2" |

---

### Requirements Coverage

| Requirement | Plan    | Description                                                                          | Status      | Evidence                                                                          |
|-------------|---------|--------------------------------------------------------------------------------------|-------------|-----------------------------------------------------------------------------------|
| PARS-01     | 01-01   | Index all .ts/.tsx/.js/.jsx files, respecting .gitignore                             | SATISFIED   | walker.rs uses ignore::WalkBuilder with .gitignore; node_modules hard-excluded   |
| PARS-02     | 01-02   | Extract symbols: functions, classes, interfaces, type aliases, enums, exported vars  | SATISFIED   | symbols.rs queries + 9 tests; all 6 + arrow functions, components, child symbols |
| PARS-03     | 01-03   | Extract all import statements (ESM import, CJS require, dynamic import)              | SATISFIED   | imports.rs extract_imports(); 5 import tests pass                                |
| PARS-04     | 01-03   | Extract export statements (named, default, re-exports)                               | SATISFIED   | imports.rs extract_exports(); 4 export tests pass                                |

All 4 Phase 1 requirements are satisfied. No orphaned requirements found.

---

### Anti-Patterns Scan

| File                      | Line | Pattern                         | Severity | Impact                              |
|---------------------------|------|---------------------------------|----------|-------------------------------------|
| `src/graph/edge.rs`       | 8,11 | `Imports` and `Exports` variants never constructed | INFO | Designed deferral to Phase 2; counts used in stats, edges deferred to import resolution |
| `src/graph/mod.rs`        | 75   | `symbol_count()` method unused  | INFO     | Defined for future query use; not a stub — method is fully implemented            |
| Various                   | —    | Fields read only in tests       | INFO     | Dead code warnings at build time; all fields are meaningful data, not placeholders |

**No blockers or stubs found.** All "dead code" warnings are intentional scaffolding for Phase 2 (import resolution edges) and are documented as such in the plan.

---

### Behavioral Notes (Non-Blocking)

**Malformed file handling:** The plan specifies "malformed files are skipped with warning, indexing continues." In practice, tree-sitter 0.26 performs error recovery on syntactically invalid files — it produces a partial tree with ERROR nodes rather than returning `None`. As a result, a file like `{{{` is parsed (file count increments), but 0 symbols are extracted from it. The tool does not crash and does continue. The `skipped` counter only increments for read errors or genuine `None` returns from tree-sitter. This behavior matches the spirit of the requirement (no crash, indexing continues) even if the exact mechanism differs. Documented in 01-03-SUMMARY.md key-decisions.

**Workspace detection:** `detect_workspace_roots()` is implemented and detects monorepo packages, but the result is currently discarded (`let _ = detect_workspace_roots(root)`). The file walk always starts from the project root (which covers all workspace subdirectories). This was an intentional design decision documented in 01-01-SUMMARY.md — avoids duplicate file discovery. The function is preserved for Phase 2 per-package scoping.

---

### Human Verification Required

The following items cannot be fully verified programmatically:

#### 1. CLI Help Text Quality

**Test:** Run `./target/debug/code-graph index --help`
**Expected:** Help text is clear, cargo-style, lists all flags (path, --verbose/-v, --json) with descriptions
**Why human:** Text quality and readability cannot be asserted programmatically

#### 2. Summary Output Readability

**Test:** Run `code-graph index .` on a real TypeScript project (not the temp test)
**Expected:** Output is cargo-style, professional, counts are accurate and readable
**Why human:** Visual formatting and accuracy judgment requires human review

#### 3. .gitignore Edge Cases

**Test:** Test with a nested .gitignore file, a global gitignore, and a project with no .gitignore
**Expected:** All three cases handled correctly; files are excluded only when they should be
**Why human:** Cannot enumerate all gitignore edge cases in automated checks

---

## Gaps Summary

No gaps found. All 5 success criteria are verified. All 4 requirements (PARS-01 through PARS-04) are satisfied. All 13 key artifact files exist with substantive, non-stub implementations. All key links are wired (with one designed partial — EdgeKind::Imports/Exports stored in graph struct for Phase 2, count-only in Phase 1). The binary builds clean (0 errors, 8 benign dead-code warnings) and all 24 tests pass.

---

_Verified: 2026-02-22T18:30:00Z_
_Verifier: Claude (gsd-verifier)_
