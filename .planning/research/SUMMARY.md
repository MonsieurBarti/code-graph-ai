# Project Research Summary

**Project:** code-graph v1.1 — Rust Language Support + Graph Export
**Domain:** Code intelligence engine — incremental language expansion on a production v1.0 system
**Researched:** 2026-02-23
**Confidence:** HIGH

## Executive Summary

This is a well-defined incremental expansion of an existing production system. v1.0 ships with a fully working TS/JS indexer, petgraph-based code graph, 6 MCP tools, CLI with 9 commands, watch mode, and 89 passing tests across 9,397 LOC. The v1.1 milestone adds two independent capabilities: (1) Rust language support, and (2) graph export in DOT and Mermaid formats. Both have HIGH-confidence implementation paths with no ambiguous technology choices — the existing codebase constrains the design space effectively.

The recommended approach is enum-based language dispatch (not trait objects), a custom Rust module resolver (no library equivalent to oxc_resolver exists for Rust), and a new `src/export/` module for graph rendering. The graph data model requires only additive changes: new `SymbolKind` variants (`Struct`, `Trait`, `Impl`, `Macro`) and one new edge kind (`TraitImpl`). Only 2 new Cargo dependencies are needed: `tree-sitter-rust = "0.24"` and `cargo_toml = "0.22"`. DOT export uses the already-present `petgraph::dot` API. Mermaid export is 30 lines of string formatting — no crate justified.

The principal risk is not in the new Rust code but in inadvertently breaking the TS/JS pipeline while integrating Rust. The existing 89-test suite is the primary regression guard and must pass throughout development. A secondary risk is the Rust module resolver: `mod` declarations must be followed from crate roots, not from filesystem walks — this is a fundamentally different model than TS `import` resolution and getting it wrong produces a silently incomplete graph. Graph export has a third risk: default granularity must be `file` or `package` (not `symbol`), or the first real-world demo produces unrenderable output exceeding Mermaid's 500-edge limit.

---

## Key Findings

### Recommended Stack

The v1.0 stack is unchanged. v1.1 requires exactly 2 new Cargo dependencies. `tree-sitter-rust = "0.24"` uses the existing `tree-sitter-language 0.1.7` bridge pattern already proven by the typescript and javascript grammar crates. `cargo_toml = "0.22"` is needed for Cargo workspace member discovery and edition detection; the existing `toml = "0.8"` crate lacks workspace inheritance handling and would require manual struct definitions. DOT export is already available via `petgraph::dot::Dot::with_attr_getters` — no new dependency. Mermaid export does not justify any crate dependency.

**Core technologies:**
- `tree-sitter-rust = "0.24"`: Parse Rust source files — only official Rust grammar, ships `TAGS_QUERY` constant, verified compatible with `tree-sitter-language 0.1.7` bridge already in the project
- `cargo_toml = "0.22"`: Parse `Cargo.toml` for workspace members, package names, edition, and path dependencies — workspace inheritance handling that the `toml` crate lacks
- `petgraph::dot` (built-in, no new dep): DOT format export — `StableGraph` already implements required traits in petgraph 0.6.5
- Raw string generation (no dep): Mermaid format export — Mermaid flowchart syntax is too simple to justify a v0.1.2 immature crate

**Critical version requirements:**
- `tree-sitter-rust = "0.24"` resolves to 0.24.0, depends on `tree-sitter-language ^0.1` (not `tree-sitter` directly) — no version conflict with existing stack

### Expected Features

Features are cleanly separated into the two v1.1 capabilities. Rust language support has a total dependency order (walker extension → grammar registration → symbol extraction → mod resolver → use resolution → impl linkage). Graph export is architecturally independent and can be developed in parallel against TS-only graphs.

**Must have (v1.1 table stakes):**
- Rust symbol extraction: fn, struct, enum, trait, type, const, static, macro_rules! — without this, no Rust file is useful
- Rust use declaration parsing (simple, nested braces, glob, alias) — enables import graph
- `mod` declaration to file resolution — the core hard problem; critical path for the entire Rust module graph
- `pub use` re-export handling — without this, `lib.rs` files appear empty
- `impl` block method extraction — without this, no methods are visible for find/refs/impact
- Cargo workspace package discovery — enables package-level grouping in export
- `code-graph export` with DOT output at symbol/file/package granularity
- `code-graph export` with Mermaid output at symbol/file/package granularity

**Should have (v1.1.x after validation):**
- Trait implementation edges (`impl Trait for Type`) — add when users ask "who implements this trait?"
- `#[derive(...)]` attribute capture on Symbol nodes
- `--focus-on` and `--max-depth N` export flags for large codebases

**Defer (v2+):**
- Inline module body as a separate graph node (complex, most Rust uses file-declaring `mod`)
- Proc macro expansion (requires spawning `cargo expand`, violates zero-dependency goal)
- Cross-crate analysis into external crates.io dependencies

### Architecture Approach

The existing `GraphNode`, `CodeGraph`, `query/*`, `mcp/*`, and `cache/*` modules require zero changes — they are language-agnostic clean boundaries. The integration work is concentrated in three areas: (1) the language dispatch layer in `parser/`, (2) a new `src/resolver/rust_resolver.rs` module separate from the TS/JS resolver, and (3) a new `src/export/` module with DOT and Mermaid renderers. The `LanguageKind` enum replaces scattered `match ext` patterns throughout the codebase without requiring dynamic dispatch or trait objects.

**Major components:**
1. `src/parser/language_kind.rs` (new) — `LanguageKind` enum, single source of truth for extension-to-language dispatch; replaces scattered match patterns in `languages.rs`, `parser/mod.rs`, `watcher/incremental.rs`, `main.rs`
2. `src/parser/rust/` (new, 3 files) — tree-sitter symbol extraction (`symbols.rs`), use declaration extraction (`imports.rs`), wired into existing parse pipeline via new `PARSER_RS` thread-local
3. `src/resolver/rust_resolver.rs` (new) — `RustModTree` builder from `mod` declarations, Cargo.toml parser, `use` path classifier (crate/super/self/external/builtin); entirely separate from oxc_resolver pipeline
4. `src/export/` (new, 3 files) — `ExportConfig` entry point, `dot.rs` using `petgraph::dot::Dot::with_attr_getters`, `mermaid.rs` as custom renderer; new `export` CLI subcommand
5. Modified shared files (low-risk, additive) — `SymbolKind` +4 variants, `EdgeKind` +1 variant (`TraitImpl`), `SOURCE_EXTENSIONS` + `"rs"`, `CONFIG_FILES` + `"Cargo.toml"`, `CACHE_VERSION` bump

### Critical Pitfalls

1. **Breaking the TS/JS pipeline during Rust integration** — the resolver's `is_external_package` heuristic treats Rust `use std::...` paths as npm packages. Gate resolver dispatch by language before calling any TS/JS resolver function. Run the 89-test suite continuously throughout Rust development. Add `"rs"` to `SOURCE_EXTENSIONS` and `PARSER_RS` to `thread_local!` in the same commit with a parse test for the `.rs` extension.

2. **Walking `.rs` files instead of following `mod` declarations** — filesystem glob of `*.rs` produces a graph with wrong module hierarchy and broken `use crate::` resolution. Files can exist on disk but be outside the compilation unit. Start module tree construction from `src/lib.rs` or `src/main.rs` only. Support both `src/foo.rs` and `src/foo/mod.rs` conventions. Track visited paths to prevent infinite loops on `#[path]` cycles.

3. **`pub use` re-export creating duplicate or missing Symbol nodes** — one Symbol node at definition site; model `pub use` as a `ReExport` edge to the definition node. Two-pass: build module tree first, then expand glob re-exports.

4. **Graph export unrenderable at default settings** — symbol-level export of a 50-file project produces 400+ nodes, exceeds Mermaid's 500-edge hard limit. Default granularity MUST be `file`, not `symbol`. Pre-count nodes/edges before output; emit clear error with suggestion if limits exceeded. Never use `splines=ortho` in DOT output.

5. **Bincode cache corruption when adding new node/edge variants** — bincode has zero backward/forward compatibility guarantees; field order mismatches produce wrong data silently. Every PR adding `GraphNode` or `EdgeKind` variants MUST bump `CACHE_VERSION` in the same commit. Add CI test that loads an old-version cache file and asserts graceful `None` return.

---

## Implications for Roadmap

Based on the dependency graph in FEATURES.md and the build order from ARCHITECTURE.md, there is a clear natural phase structure. Phases 1-4 cover Rust language support in strict dependency order. Phase 5 (graph export) is architecturally independent and can be parallelized with Phases 2-4.

### Phase 1: Language Dispatch Foundation

**Rationale:** Every subsequent Rust change depends on `.rs` files being discovered and dispatched correctly. This is also the highest-risk area for TS/JS regression — establishing the dispatch boundary first contains that risk. This phase has zero new Rust-specific logic and is entirely infrastructure.

**Delivers:** `.rs` files are discovered by walker and watcher, dispatched to the correct parser (returns parse error until grammar is wired), and `Cargo.toml` changes trigger full re-index. All 89 existing tests continue to pass.

**Addresses:** Walker extension, SOURCE_EXTENSIONS, CONFIG_FILES addition, `LanguageKind` enum creation, thread-local `PARSER_RS` registration

**Avoids:** Pitfall 1 (TS/JS regression), Pitfall 7 (thread-local parser registry desync) — establish the boundary before writing Rust-specific code

**Research flag:** Standard patterns — no research needed. Direct codebase changes documented in ARCHITECTURE.md.

### Phase 2: Rust Symbol Extraction

**Rationale:** Symbol extraction is independently testable against tree-sitter output before the resolver exists. It unblocks the resolver (which wires symbols to use paths). `SymbolKind` and `EdgeKind` extensions trigger a `CACHE_VERSION` bump here.

**Delivers:** Rust fn, struct, enum, trait, type, const, static, macro_rules! symbols appear in the graph with correct `is_exported` mapping from Rust visibility modifiers. `impl` blocks emit `SymbolKind::Impl` with methods as `SymbolKind::Method` children. `stats` command shows Rust symbol counts.

**Addresses:** All Rust symbol kind table-stakes features, `SymbolKind` extension, `CACHE_VERSION` bump

**Avoids:** Pitfall 11 (bincode cache corruption) — must bump version in same commit as SymbolKind change

**Research flag:** Standard patterns — tree-sitter query patterns fully documented in STACK.md and ARCHITECTURE.md. No additional research needed.

### Phase 3: Rust Module Resolver

**Rationale:** This is the hardest phase. The `mod` declaration traversal algorithm is the foundation for correct `use` path resolution. Must be built as a completely separate module from the TS/JS resolver — no shared abstraction. Requires two-pass: build `RustModTree` first, then resolve `use` statements against it.

**Delivers:** `use crate::`, `use super::`, `use self::` paths resolve to file nodes. External crate `use` statements produce `ExternalPackage` nodes. `std`/`core`/`alloc` produce builtin terminal nodes. Cargo workspace members resolve to local sources. `pub use` creates `ReExport` edges, not duplicate Symbol nodes. `TraitImpl` edges connect struct to trait for `impl Trait for Type` declarations.

**Addresses:** mod declaration parsing, use declaration resolution, pub use re-export handling, Cargo workspace package discovery, cross-crate edges, `TraitImpl` edge kind addition

**Avoids:** Pitfall 2 (treating Rust module resolution like TS import resolution — separate resolver), Pitfall 3 (missing mod declaration traversal — start from crate root), Pitfall 4 (pub use duplicate symbols — one node, ReExport edge), Pitfall 5 (edition-unaware parsing — read edition from Cargo.toml), Pitfall 12 (Cargo workspace resolution — build workspace map before use resolution)

**Research flag:** Needs careful implementation validation. The mod tree traversal algorithm is specified in the Rust Reference (HIGH confidence) but has documented edge cases: `#[path = "..."]` attributes, `src/lib.rs` + `src/main.rs` coexisting, inline mod with nested file-declaring mod, Edition 2015 `extern crate`. Recommend dogfooding on the tool's own Rust source immediately after implementation.

### Phase 4: Watch Mode Rust Integration + CLI Parity Verification

**Rationale:** Watch mode for Rust has a Rust-specific cascading problem — adding `mod X;` to a file must trigger discovery of `X`'s source file, which the existing TS incremental handler does not do. Acceptable v1.1 mitigation: full re-index when `lib.rs` or `main.rs` is modified. CLI parity verifies all 9 existing commands work on Rust files.

**Delivers:** Watch mode handles `.rs` file changes. All 9 CLI commands (`find`, `refs`, `impact`, `circular`, `context`, `stats`, `watch`, `index`, `mcp`) produce correct output for Rust symbols. Mixed TS+Rust projects index and query correctly.

**Addresses:** Incremental handler extension, `fix_unresolved_pointing_to` Rust analog, per-language stats in `stats` command output

**Avoids:** Pitfall 8 (cascading mod declaration changes) — accept full re-index on crate root modification as acceptable v1.1 tradeoff, documented as known limitation

**Research flag:** Standard patterns for the watcher extension. The cascading mod change problem is a known limitation to document explicitly rather than solve fully in v1.1.

### Phase 5: Graph Export (DOT + Mermaid)

**Rationale:** Architecturally independent of Rust support — can be developed in parallel with Phases 2-4. Only needs the `CodeGraph` and `petgraph::StableGraph` to exist (both in place from v1.0). Default granularity must be `file` and scale limits must be defined BEFORE output generation is implemented — they are product decisions, not implementation details.

**Delivers:** `code-graph export --format dot|mermaid --granularity symbol|file|package` command. Default granularity `file`. Pre-flight edge count check with clear error for Mermaid (>500 edges) and DOT (>500 nodes warn, do not truncate). Mixed-language projects show Rust nodes by module path (`crate::parser`) not filesystem path. Package clustering via DOT subgraph and Mermaid `subgraph` blocks.

**Addresses:** DOT format output, Mermaid format output, three granularity levels, node/edge count guards, multi-language export presentation

**Avoids:** Pitfall 9 (unrenderable export defaults — file granularity as default, pre-flight count gates), Pitfall 10 (mixed-language export conflation — language-aware node labels and subgraph clustering)

**Research flag:** Standard patterns for the export pipeline. DOT and Mermaid syntax are fully documented and validated. The scale limits (Mermaid 500-edge hard limit, Graphviz `splines=ortho` crash threshold) are confirmed from official sources. No additional research needed.

### Phase Ordering Rationale

- Phase 1 before everything: contains TS/JS regression risk before any Rust-specific code is written
- Phase 2 before Phase 3: symbol extraction is prerequisite for resolver (resolver links use paths to Symbol nodes)
- Phase 3 before Phase 4: resolver correctness must be verified before testing watch mode behavior
- Phase 5 parallel with 2-4: export operates on the existing graph, can be tested against TS-only graphs, merge after Rust support is stable
- `CACHE_VERSION` bump belongs in Phase 2 (first PR adding new SymbolKind/EdgeKind variants)

### Research Flags

Phases needing careful validation during execution:
- **Phase 3 (Rust Module Resolver):** Complex edge cases in `mod` declaration traversal — `#[path]` attributes, `mod` inside inline blocks, Edition 2015. Recommend immediate dogfooding on the tool's own Rust source after each sub-implementation.

Phases with standard well-documented patterns (skip additional research):
- **Phase 1:** Pure codebase wiring, documented change list in ARCHITECTURE.md
- **Phase 2:** tree-sitter query patterns fully specified in STACK.md and ARCHITECTURE.md
- **Phase 4:** Watcher extension is additive; incremental limitations are design decisions, not technical unknowns
- **Phase 5:** DOT/Mermaid syntax, scale limits, and petgraph API all confirmed HIGH confidence

---

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | All dependencies verified against crates.io, docs.rs, and `cargo tree` on the live project. Only 2 new deps needed. Alternatives clearly ruled out with rationale. |
| Features | HIGH | Node types verified against tree-sitter-rust node-types.json and tags.scm. Feature scope is well-defined with clear v1.1 vs v1.1.x vs v2+ boundaries. Competitor feature analysis complete. |
| Architecture | HIGH | Based on direct analysis of the actual v1.0 codebase (9,397 LOC, 58 source files). Component boundaries identified from source, not inference. Build order has clear dependency rationale. |
| Pitfalls | HIGH | 12 pitfalls identified with specific symptoms, prevention steps, and phase assignments. Sources include official Rust Reference, rust-analyzer blog, and Mermaid/Graphviz issue trackers. |

**Overall confidence:** HIGH

### Gaps to Address

- **`#[path = "..."]` attribute handling:** The mod resolver must handle arbitrary file paths on `mod` items. PITFALLS.md recommends documenting as a known gap for v1.1 if not implemented. Decide at Phase 3 start whether to implement or explicitly document as unsupported.

- **Edition 2015 support:** PITFALLS.md allows "Edition 2015 crates unsupported" as an acceptable v1.1 boundary if documented. Reading the `edition` field from Cargo.toml is low cost — decide at Phase 3 start.

- **`pub use *` glob expansion data structure:** The two-pass requirement for glob re-exports is documented, but the exact representation for the `GlobImport` edge is unspecified. Define the model before Phase 3 implementation begins.

- **Mixed-language export node label source:** Rust nodes should display module path (`crate::parser`) not filesystem path. This requires the `RustModTree` to be available to the export renderer. The data flow from Phase 3 (resolver builds mod tree) to Phase 5 (export reads mod tree for labels) needs explicit design at Phase 5 start.

---

## Sources

### Primary (HIGH confidence)
- `cargo tree` on live project — verified `tree-sitter-language 0.1.7` bridge compatibility with all grammar crates
- [tree-sitter-rust docs.rs](https://docs.rs/tree-sitter-rust/latest/tree_sitter_rust/) — LANGUAGE, TAGS_QUERY constants; version 0.24.0
- [tree-sitter-rust node-types.json](https://github.com/tree-sitter/tree-sitter-rust/blob/master/src/node-types.json) — confirmed field names for all node types used
- [tree-sitter-rust tags.scm](https://github.com/tree-sitter/tree-sitter-rust/blob/master/queries/tags.scm) — symbol tagging query patterns
- [cargo_toml docs.rs](https://docs.rs/cargo_toml/latest/cargo_toml/struct.Manifest.html) — Manifest, Workspace, Package, Dependency structs; v0.22.3
- [petgraph::dot docs](https://docs.rs/petgraph/latest/petgraph/dot/struct.Dot.html) — Dot, Config, with_attr_getters API
- [Rust Reference: Modules](https://doc.rust-lang.org/reference/items/modules.html) — canonical module resolution algorithm
- [Rust Reference: Use Declarations](https://doc.rust-lang.org/reference/items/use-declarations.html) — use forms, glob semantics
- [Rust Edition Guide: Path Changes](https://doc.rust-lang.org/edition-guide/rust-2018/path-changes.html) — extern crate, crate:: keyword, unified paths
- [Cargo Book: Workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html) — workspace member resolution
- [Mermaid config schema](https://mermaid.ai/open-source/config/schema-docs/config.html) — maxEdges: 500, maxTextSize: 50000 defaults
- [Mermaid GitHub issue #5042](https://github.com/mermaid-js/mermaid/issues/5042) — 500 edge limit confirmed
- [Graphviz DOT Language](https://graphviz.org/doc/info/lang.html) — subgraph cluster syntax
- Existing codebase analysis `/workspace/src/` — all integration points verified from source

### Secondary (MEDIUM confidence)
- [rust-analyzer blog: IDEs and Macros](https://rust-analyzer.github.io/blog/2021/11/21/ides-and-macros.html) — macro expansion interdependency with name resolution
- [Graphviz forum: large graphs](https://forum.graphviz.org/t/creating-a-dot-graph-with-thousands-of-nodes/1092) — practical 6K node crash threshold, splines=ortho danger
- [rust-analyzer issue #5922](https://github.com/rust-lang/rust-analyzer/issues/5922) — pub use re-export cycle handling complexity
- [bincode backward compat discussion](https://users.rust-lang.org/t/tools-for-supporting-version-migration-of-serialization-format/76292) — zero backward compat guarantees
- [cargo-modules GitHub](https://github.com/regexident/cargo-modules) — competitor feature set verified
- [cargo-depgraph GitHub](https://github.com/jplatte/cargo-depgraph) — competitor feature set verified
- [mermaid_builder docs](https://docs.rs/mermaid-builder/latest/mermaid_builder/) — v0.1.2, ruled out as dependency

---
*Research completed: 2026-02-23*
*Ready for roadmap: yes*
