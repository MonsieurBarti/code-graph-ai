# Pitfalls Research

**Domain:** Code intelligence engine / dependency graph tool — v1.1 Rust language support + graph export
**Researched:** 2026-02-23
**Confidence:** HIGH

---

> This document covers v1.1 milestone pitfalls only: adding Rust language support and graph export to
> an existing working TS/JS system. Greenfield pitfalls (barrel files, tsconfig paths, etc.) were
> covered in the v1.0 PITFALLS.md and are not repeated here unless they interact with Rust integration.

---

## Critical Pitfalls

### Pitfall 1: Breaking the TS/JS Pipeline While Adding Rust

**What goes wrong:**
The parser dispatch, walker, and resolver modules all have TS/JS-specific assumptions baked in as
implicit contracts. Adding Rust as a second language path causes TS/JS regressions if shared code is
changed without testing both paths. The most dangerous locations are `languages.rs`
(`language_for_extension`), `walker.rs` (`SOURCE_EXTENSIONS`), `parser/mod.rs`
(`parse_file_parallel` thread_local logic), and `resolver/mod.rs` (the `is_external_package`
heuristic, which assumes non-relative paths are npm packages — wrong for Rust `use` statements).

**Why it happens:**
Developers focus on making the new Rust path work. TS/JS is "already working" and not actively
tested during Rust development. Shared functions are modified to accommodate Rust semantics and
silently break TS/JS edge cases.

**How to avoid:**
1. Never modify existing `parse_file_parallel` without running the full TS/JS test suite immediately.
2. The resolver's `is_external_package` function (`!specifier.starts_with('.')`) MUST NOT be called
   for Rust files — Rust uses `use std::collections::HashMap;` (no dots, but not npm). Add a
   per-language dispatch before calling it.
3. Add `SOURCE_EXTENSIONS` to a per-language registry rather than a flat `&[&str]` constant — makes
   the contract explicit and prevents accidental Rust `.rs` addition polluting TS paths.
4. Run the existing 89 tests as a pre-commit gate throughout Rust development.

**Warning signs:**
- TS/JS `unresolved` count increases during Rust integration work (resolver dispatch leak).
- Existing tests fail that did not fail before the Rust branch was added.
- `is_external_package("std")` returning `true` (it does — no dot prefix, not npm).

**Phase to address:**
Rust Parsing phase — establish the language dispatch boundary before writing any Rust-specific code.

---

### Pitfall 2: Treating Rust Module Resolution Like Import Resolution

**What goes wrong:**
The `use` statement in Rust is NOT equivalent to `import` in TypeScript. `use std::io::Write;`
does not point to a file. The entire TS/JS resolution pipeline (oxc_resolver, workspace detection,
tsconfig paths) is irrelevant for Rust. Building a Rust module resolver by analogy to the TS
resolver produces wrong results for every category:

| Rust statement | Wrong analogy (TS) | Correct semantics |
|---|---|---|
| `use std::io::Write` | External npm package | Rust stdlib — terminal node |
| `use crate::parser::mod` | Relative path import | Same-crate file at `src/parser/mod.rs` |
| `use super::event::WatchEvent` | `../event` import | Parent module in the crate tree |
| `mod foo;` | Not applicable in TS | Declares that `src/foo.rs` or `src/foo/mod.rs` belongs to the module tree |
| `pub use self::foo::Bar;` | `export { Bar } from './foo'` | Re-exports Bar through THIS module's public API |

Crucially, `mod foo;` is a DECLARATION that FILE-SYSTEM discovery must happen. Without processing
`mod` declarations, files are never added to the Rust module tree, and every `use` path is wrong.

**Why it happens:**
The existing resolver abstraction (`resolve_all`, `resolve_import`) is designed around the
concept of "a string specifier that gets resolved to a file path." Rust does not work this way:
the module tree is built from `mod` declarations, and `use` statements navigate that tree.

**How to avoid:**
1. Implement a separate `RustModuleTree` that is built in a first pass over all `.rs` files by
   following `mod foo;` declarations from `lib.rs`/`main.rs` entry points.
2. Only after the module tree is built, process `use` statements against it.
3. Do NOT pass Rust files through `build_resolver` or `oxc_resolver` — it will always fail or
   produce wrong results.
4. Keep Rust resolution code entirely separate from TS resolution code. No shared resolver
   abstraction between the two languages.

**Warning signs:**
- `use` paths resolving to `UnresolvedImport` for basic `std` and `crate::` paths.
- `crate::` paths not finding files that are in the project.
- Files in the project that are never added to the graph (mod declarations not followed).

**Phase to address:**
Rust Module Resolution phase — separate implementation, not an extension of the TS resolver.

---

### Pitfall 3: Missing `mod` Declaration Processing (The Silent Gap)

**What goes wrong:**
A file `src/parser/rust.rs` exists on disk but is never added to the graph because no `mod rust;`
declaration was followed to discover it. The graph shows the project but silently omits modules
that are declared but not reachable from the crate root via `mod` chains.

This is the Rust-specific equivalent of the TS barrel file problem, but worse: TS files are
discovered by walking the filesystem. Rust files are only in scope if they appear in the `mod`
declaration tree. A Rust file can exist on disk but be completely outside the crate's compilation
unit if no `mod` statement references it.

The special cases that are easy to miss:
- `#[path = "custom/path.rs"] mod foo;` — the file is NOT at the expected location
- `mod foo { mod bar; }` — inline module with nested `mod` declaration inside it
- `cfg_attr`-gated modules: `#[cfg_attr(test, path = "test_impl.rs")] mod impl_;`
- `src/lib.rs` AND `src/main.rs` coexisting — two separate crate roots

**Why it happens:**
File-walking is the mental model from TS development. Developers walk `*.rs` files and try to
parse them all. This produces an incomplete module tree with wrong parent-child relationships
because the module hierarchy is defined by `mod` declarations, not filesystem layout.

**How to avoid:**
1. Start module tree construction from `src/lib.rs` or `src/main.rs` (or both for crates with
   both). Never start by walking `*.rs` files directly.
2. For each file, parse ALL `mod_item` nodes (tree-sitter node type). For each `mod foo;` (no
   body), derive the child file path using BOTH conventions:
   - `src/parent/foo.rs` (new style, preferred since Rust 1.30)
   - `src/parent/foo/mod.rs` (old style, must still support)
3. Handle `#[path = "..."]` attributes on `mod` items — the file is at an arbitrary location.
4. Fall back to filesystem walking only for orphaned `.rs` files (report them as warnings, not errors).

**Warning signs:**
- Symbol count is lower than expected for a Rust project.
- `use crate::X` paths produce `UnresolvedImport` even though `X` exists in the source.
- Files that exist on disk have no nodes in the graph.

**Phase to address:**
Rust Parsing phase — `mod` declaration traversal must be the foundation before any other Rust
parsing work begins.

---

### Pitfall 4: `pub use` Re-Export Chains Create Duplicate or Missing Symbols

**What goes wrong:**
Idiomatic Rust heavily uses `pub use` to flatten internal module hierarchies for public APIs.
A symbol `Bar` may be defined in `src/inner/bar.rs` but also re-exported at `src/lib.rs` via
`pub use inner::bar::Bar`. The tool either:
(a) Creates two Symbol nodes for the same `Bar` (one at definition, one at re-export), making
    impact analysis double-count, or
(b) Only records the re-export and misses the actual definition location, breaking "go to
    definition" semantics.

Glob re-exports (`pub use module::*`) compound this: the set of re-exported items is only known
after parsing the target module.

**Why it happens:**
TS handled barrel files with a dedicated `BarrelReExportAll` edge and named re-export chain pass.
Rust's `pub use` looks similar but has different semantics: in TS, `export * from './x'` is a
file-level operation. In Rust, `pub use module::Symbol` can re-export a specific item while also
keeping the original accessible. The graph model must distinguish definition location from
visibility location.

**How to avoid:**
1. Record ONE Symbol node at the definition site (the file where it is first declared).
2. Model `pub use crate::inner::Bar` as a `ReExport` edge from the re-exporting file to the
   definition Symbol node. Do NOT create a second Symbol node.
3. For glob re-exports (`pub use module::*`), expand the glob after the module tree is built —
   similar to the TS barrel chain pass but driven by Rust's module tree instead of file discovery.
4. Add a `ReExport { visibility: pub/pub_crate/pub_super }` edge type to EdgeKind rather than
   reusing existing edge types — the visibility distinction matters for impact analysis.

**Warning signs:**
- Symbol count is higher than `grep -r 'fn \|struct \|enum \|trait ' src/ | wc -l` would suggest.
- `find` queries return the re-export file instead of the definition file.
- Impact analysis shows a symbol affecting itself (circular re-export edge mishandled).

**Phase to address:**
Rust Module Resolution phase — design the re-export edge model before implementing symbol extraction.

---

### Pitfall 5: Edition-Unaware Path Resolution

**What goes wrong:**
A Rust file using Edition 2015 requires `extern crate foo;` before `use foo::Bar;` will work.
A file on Edition 2018 or later can write `use foo::Bar;` directly. The tool does not read the
`edition` field from `Cargo.toml`, treats all crates as Edition 2021, and fails to resolve
`extern crate` declarations or silently treats them as unknown symbols in 2015 crates.

Additional edition-specific differences:
- Edition 2015: `use` paths are relative to crate root in `use` but absolute in code
- Edition 2018+: `use` and code paths are unified; `crate::` keyword unambiguous
- Edition 2015 crates use `mod submod { extern crate X; }` inside module blocks

**Why it happens:**
The edition field is in `Cargo.toml`, not in the source file. Tools that focus on file parsing
forget to connect `Cargo.toml` metadata to the per-file parsing context.

**How to avoid:**
1. Parse `Cargo.toml` (using `toml` crate) to extract `edition` (default: 2015 if absent).
2. Pass edition as context to the Rust import extractor.
3. In Edition 2015: treat `extern crate X` as an external dependency declaration (equivalent to
   an npm package node). In Edition 2018+: suppress `extern crate` nodes (they are no-ops).
4. Resolve `::foo` (leading double-colon) as an external crate reference (Edition 2015 style).
5. Read edition from the `[workspace]` section for workspace members that inherit it.

**Warning signs:**
- Legacy crates (actix-web 1.x, old diesel examples) show high unresolved import counts.
- `extern crate` appearing in the symbol list as an unknown node type.
- `::std::` paths (Edition 2015 absolute path syntax) failing to resolve.

**Phase to address:**
Rust Parsing phase — read `Cargo.toml` metadata before starting file parsing, not after.

---

### Pitfall 6: Macro-Generated Modules Are Invisible to Tree-Sitter

**What goes wrong:**
Procedural macros and `macro_rules!` macros can generate entire module declarations, function
definitions, and `impl` blocks that are invisible to tree-sitter. A `#[derive(Debug)]` macro
generates an `impl Debug for Foo` block. A `module_path!()` call is a compile-time value.
Common crates generate substantial amounts of code via macros:

- `serde`: generates `Serialize`/`Deserialize` implementations
- `thiserror`: generates `Display` and `From` implementations
- `sqlx`: can generate query-specific types at compile time
- `tokio::main`: rewrites the `main` function
- `actix-web` route macros: generate handler registration code

The tool reports "no implementations of Serialize found" for a type that has `#[derive(Serialize)]`,
because the `impl Serialize for Foo` is macro-expanded, not present in the source AST.

**Why it happens:**
Tree-sitter is a parser, not a compiler. It sees the source text as written, not after macro
expansion. rust-analyzer acknowledges this as "the hardest nut to crack" for language tooling —
macros create interdependencies between name resolution and expansion that require a full compiler
pipeline to resolve correctly.

**How to avoid:**
1. Document this limitation explicitly. The graph represents syntactically visible code, not
   macro-expanded code. This is a known constraint of tree-sitter-based tools.
2. For `#[derive(...)]` attributes: parse the derive list as a hint. `#[derive(Serialize)]` on
   `struct Foo` implies an `impl Serialize for Foo` exists — record a synthetic `DerivedImpl` edge.
3. For proc-macro attributes (e.g., `#[tokio::main]`): record as metadata on the symbol node but
   do not attempt expansion.
4. For `macro_rules!` that declare modules: accept they will not be in the graph. This is the
   same limitation TS has with computed `require()` calls.
5. Add `derived_traits: Vec<String>` to `SymbolInfo` to surface derive info in stats and queries.

**Warning signs:**
- Struct symbols with no `impl` connections despite having `#[derive(...)]`.
- Users expect "find all Serialize implementations" to work but gets no results.
- Impact analysis misses that changing a struct definition would break serialization code.

**Phase to address:**
Rust Parsing phase — scope the limitation explicitly, add derive hint extraction as a defined
feature boundary.

---

### Pitfall 7: Thread-Local Parser Registry Doesn't Scale to Multiple Languages

**What goes wrong:**
The current `thread_local!` block in `parser/mod.rs` declares three parsers: `PARSER_TS`,
`PARSER_TSX`, `PARSER_JS`. Adding Rust requires a fourth: `PARSER_RS`. This is fine, but the
`parse_file_parallel` dispatch (`match ext { "ts" => ..., "tsx" => ..., "js"|"jsx" => ... }`)
must be extended. The danger is partial extension: Rust files silently fall through to the `_`
arm and fail with "unsupported extension" rather than using the Rust parser.

A subtler issue: the `thread_local!` parsers are initialized with a specific language grammar at
initialization time. If a rayon thread first handles a `.ts` file and later gets a `.rs` file,
it cannot reuse `PARSER_TS` for Rust — it correctly uses `PARSER_RS`. But if Rust files are
rare in a mostly-TS project, `PARSER_RS` may never initialize on most threads, adding memory
overhead on threads that do get RS files. This is a minor issue, not a blocker.

**Why it happens:**
The thread-local registry grows by addition. Without a systematic per-language dispatch table,
developers add new language arms to the `match` in `parse_file_parallel` and forget to add
a corresponding thread-local parser, or vice versa.

**How to avoid:**
1. Refactor `parse_file_parallel` to use a macro or table-driven dispatch that cannot get
   out of sync: define languages in one place, generate both the `thread_local!` block and the
   `match` arm from it.
2. Alternatively, accept a small allocation penalty and use `parse_file` (non-parallel) for
   single-file incremental updates (this already happens in the watcher), and keep
   `parse_file_parallel` only for the bulk index pass.
3. Add a compile-time test: `parse_file_parallel` called with `.rs` extension must return `Ok`.

**Warning signs:**
- Rust files returning "unsupported file extension" errors during indexing.
- Mixed-language project where Rust files all fail but TS files succeed.
- `stats` output showing 0 Rust files indexed when `.rs` files are present.

**Phase to address:**
Rust Parsing phase — extend the thread-local registry and dispatch at the same time, in the
same commit, with a test for each extension.

---

### Pitfall 8: `mod` Declaration Changes Cascade Through the Module Tree

**What goes wrong:**
When a developer adds `mod new_module;` to `src/lib.rs`, the incremental re-indexer fires a
`WatchEvent::Modified` for `lib.rs`. The current incremental handler (`handle_modified`) removes
the old `lib.rs` node and re-indexes just that file. But the new `mod new_module;` declaration
implies that `src/new_module.rs` (or `src/new_module/mod.rs`) should now be added to the graph.
The incremental handler does not walk new `mod` declarations — it only re-parses the changed file
in isolation. The new module file is never indexed until the next full re-index.

The reverse is equally broken: removing `mod old_module;` from `lib.rs` should trigger removal
of the entire `old_module` subtree from the graph. The current `handle_deleted` only removes
a single file node.

**Why it happens:**
The existing incremental system was designed for TS/JS where file discovery is filesystem-based.
Adding a `import './newfile'` to a TS file doesn't create a new file — it references an existing
one. But adding `mod new_module;` to a Rust file IS a declaration that a new file should be
discovered and indexed.

**How to avoid:**
1. After re-parsing a modified Rust file, diff the old vs new `mod` declarations.
2. For newly added `mod X;` declarations: trigger discovery and indexing of the corresponding
   file(s) as if they were new files.
3. For removed `mod X;` declarations: trigger removal of the corresponding subtree from the graph.
4. This requires maintaining a `mod_children: HashMap<PathBuf, Vec<PathBuf>>` data structure
   to know which files were discovered via which `mod` declaration.
5. Consider: for the initial implementation, fall back to a full re-index when `lib.rs` or
   `main.rs` is modified (since these are the crate roots, changes are high-impact). Incremental
   mod-tree updates can be a follow-on improvement.

**Warning signs:**
- Adding a new Rust module doesn't appear in the graph without a full `code-graph index` run.
- `use crate::new_module::Foo` shows as unresolved after adding the module.
- Graph diverges from reality during extended watch sessions without explicit re-index.

**Phase to address:**
Watch Mode / Incremental Re-indexing phase — design the Rust-aware incremental handler before
implementing it, not after discovering the cascading problem.

---

### Pitfall 9: Graph Export Produces Unrenderable Output at Default Settings

**What goes wrong:**
The first naive implementation of `code-graph export` on a medium Rust project (50 files, 300+
symbols) generates a DOT file with 400+ nodes and 800+ edges. Rendering with `dot -Tsvg` takes
30+ seconds and produces an SVG that is unusable for architectural review. GitHub's Mermaid
renderer enforces a hard limit of 500 edges (`maxEdges` defaults to 500), causing "Too many edges"
errors. Even at lower counts, the default `dot` layout algorithm fails to cluster related nodes,
producing a "ball of string" diagram.

**Why it happens:**
The feature is specified as "three granularity levels: symbol, file, package." Developers
implement symbol-level export first (it maps most directly to the existing graph structure) and
ship it as the default. Symbol-level on any real project immediately hits rendering limits.

**How to avoid:**
1. The default granularity for `export` MUST be `package` or `file`, not `symbol`. Package-level
   exports of a 50-file project produce 5-10 nodes — always renderable.
2. Add explicit node/edge count checks before outputting:
   - Package level: always fine
   - File level: warn if >100 files ("consider --filter or --package to scope")
   - Symbol level: warn if >200 symbols ("this will not render well in most tools")
3. For Mermaid output, enforce the `maxEdges: 500` limit by counting edges before output and
   refusing with a helpful error if exceeded.
4. For DOT output, add `rankdir=LR` and avoid `splines=ortho` (which crashes Graphviz at ~6K nodes).
5. Support `--filter <package>` and `--depth N` flags from day one — export is only useful when
   scoped.

**Warning signs:**
- Export tests only use toy graphs with <10 nodes.
- No test that runs `dot` or validates the output is actually renderable.
- Default demo uses a small fixture that hides the scaling problem.

**Phase to address:**
Graph Export phase — define the rendering limits and default granularity BEFORE writing the output
generator. The defaults are the product decision, not an implementation detail.

---

### Pitfall 10: File-Level Export Conflates Rust Files with TS/JS Files

**What goes wrong:**
At file-level granularity, the export includes all indexed files. In a multi-language project
(TS + Rust), the export graph mixes TS files (with `ResolvedImport` edges) and Rust files (with
`ModUse` edges). The resulting DOT/Mermaid diagram is unreadable because the two language graphs
use different node shapes/colors but are visually intermingled without clustering by language.

Additionally, Rust `use` statements navigate a module tree, not a file system. A `use crate::X`
edge in the exported graph points to a file node, but the Rust developer reading the diagram
thinks in terms of modules (e.g., `parser::mod`), not file paths (e.g., `src/parser/mod.rs`).
The export should present Rust nodes by module path (`crate::parser`) not by file path.

**Why it happens:**
The existing `FileInfo` node stores a `language: String` field (already in `node.rs`). The export
code iterates all FileNodes without filtering or grouping by language. The node label uses the
file path for all nodes regardless of language semantics.

**How to avoid:**
1. For Rust file nodes in export output: use module path (`crate::parser::symbols`) as the node
   label, not the filesystem path (`src/parser/symbols.rs`). Derive the module path from the
   `RustModuleTree` built during indexing.
2. In DOT output: use subgraph clusters to group Rust modules under their crate name, and TS
   files under their package name. This makes mixed-language exports readable.
3. Add `--language rust|ts|all` filter flag to export command.
4. In Mermaid output: use different node shapes for Rust (`[[ ]]` for modules) vs TS (`( )` for
   files) to visually distinguish them.

**Warning signs:**
- Export tests only run against single-language projects.
- Exported DOT shows `src/parser/symbols.rs` as a node label (not `crate::parser::symbols`).
- Mixed-language project export looks like two unrelated graphs in one diagram.

**Phase to address:**
Graph Export phase — design the multi-language presentation model before implementing the
output formatter.

---

### Pitfall 11: Bincode Cache Breaks When New Node/Edge Types Are Added

**What goes wrong:**
Adding `Rust`-specific graph node types (e.g., a new `GraphNode::RustModule` variant) or new
edge types (e.g., `EdgeKind::ModDeclaration`) changes the serialized binary format. Existing
`.code-graph/graph.bin` caches will fail to deserialize — they have the old schema. The current
code handles this correctly via `CACHE_VERSION` checking (load returns `None` on version mismatch,
triggering a full rebuild). BUT: the version bump must happen atomically with the struct change,
in the same commit. Forgetting to bump `CACHE_VERSION` causes silent deserialization corruption
(bincode does not include schema metadata — field order mismatches produce wrong data, not errors).

**Why it happens:**
Bincode has zero backward/forward compatibility guarantees. Any change to the serialized struct
(adding/removing/reordering fields or enum variants) silently produces wrong deserialized data
if the version is not bumped. Developers add a new enum variant to `GraphNode` and test it
without deleting their existing cache, seeing correct results because the new variant happens
to be last and old caches never contain it.

**How to avoid:**
1. Every PR that adds a new `GraphNode` variant, `EdgeKind` variant, or changes any serialized
   struct MUST bump `CACHE_VERSION` in `cache/envelope.rs` in the same commit.
2. Add a CI test that intentionally loads a v_N cache file with v_(N+1) code and asserts it
   returns `None` (graceful fallback, not a panic or wrong data).
3. Consider adding a `[serde(deny_unknown_fields)]` or similar guard — though bincode does not
   support this, it is a reminder to use a schema-aware format if backward compat becomes a
   real requirement.
4. Document in `cache/envelope.rs`: "Add a new variant? BUMP CACHE_VERSION. This is not optional."

**Warning signs:**
- A developer reports "weird graph data" after pulling changes that added new node types.
- CI passes but local development has stale cache artifacts from a previous version.
- `graph.bin` file size doesn't change after adding a new node type (variant never serialized).

**Phase to address:**
Rust Parsing phase — the first PR that adds Rust graph nodes MUST bump `CACHE_VERSION`.

---

### Pitfall 12: Cargo Workspace Resolution Differs from npm Workspace Resolution

**What goes wrong:**
The existing workspace detection (`discover_workspace_packages`) reads `package.json` workspaces.
For a Rust Cargo workspace, the analogous file is `Cargo.toml` with a `[workspace]` section.
If the project is a Cargo workspace (e.g., the codebase itself), the Rust resolver needs to
understand that `use code_graph_cli::graph::CodeGraph` refers to the sibling crate
`code_graph_cli`, whose source is at a local path, not on crates.io.

The workspace topology for Rust:
- Root `Cargo.toml` has `[workspace] members = ["crate-a", "crate-b"]`
- Each member has its own `Cargo.toml` with `[dependencies] crate-a = { path = "../crate-a" }`
- `use crate_a::Foo` is an external crate reference that resolves to a local workspace member

Without Cargo workspace awareness, these cross-member `use` statements are treated as external
crates (terminal nodes) instead of resolved local sources.

**Why it happens:**
The Cargo workspace discovery is simpler than npm workspace discovery (no glob patterns, no
nested workspace configs) but requires parsing `Cargo.toml` TOML files instead of
`package.json` JSON files. Developers port the npm workspace logic without reading Cargo's spec.

**How to avoid:**
1. Parse root `Cargo.toml` using the `toml` crate (already available if using `cargo_toml` or
   `toml` directly).
2. For each workspace member, read its `Cargo.toml` to get the package name (which may differ
   from the directory name).
3. Build a `cargo_workspace_map: HashMap<String, PathBuf>` mapping crate name to local source
   directory (member_path/src/).
4. In the Rust resolver: before treating a `use` target as external/crates.io, check if the
   crate name is in `cargo_workspace_map`. If so, resolve to local source.

**Warning signs:**
- Cross-crate `use` statements within a Cargo workspace shown as external terminal nodes.
- Impact analysis does not propagate changes across crate boundaries.
- The tool indexes `code-graph` itself and shows `rmcp::` as external when it should show
   internal workspace members.

**Phase to address:**
Rust Module Resolution phase — Cargo workspace awareness must precede `use` statement resolution.

---

## Technical Debt Patterns

| Shortcut | Immediate Benefit | Long-term Cost | When Acceptable |
|----------|-------------------|----------------|-----------------|
| Walk all `.rs` files instead of following `mod` declarations | Simpler discovery, no tree traversal | Wrong module tree, orphaned files in graph, broken `use crate::` resolution | Never — the Rust module system requires mod-declaration traversal |
| Reuse TS resolver for Rust `use` statements | No new code | Every Rust import is unresolved or wrong | Never — resolvers are fundamentally different |
| Skip `#[path = "..."]` attribute handling | Simpler parser | ~5% of Rust projects use custom paths; those break silently | Acceptable for v1.1 if documented as known gap |
| Symbol-level export as default granularity | Maps directly to graph structure | First real project use produces unrenderable output | Never for default — file/package should be default |
| Skip edition detection, assume Edition 2021 | No `Cargo.toml` parsing needed | 2015-era crates (many popular ones) break | Acceptable for v1.1 if 2015 crates documented as unsupported |
| Derive hint as full impl edge | Shows derive in impact analysis | Creates misleading "implements Serialize" edges | Acceptable if edge is typed as `DerivedImpl` not `Implements` |
| Full re-index on `lib.rs`/`main.rs` change | Correct module tree always | <100ms guarantee broken for large projects | Acceptable initial implementation; defer mod-diff to follow-on |

## Performance Traps

| Trap | Symptoms | Prevention | When It Breaks |
|------|----------|------------|----------------|
| Parsing all `.rs` files with a single rayon pool that includes both TS and Rust | Thread starvation when one language dominates | Use per-language work queues or accept the shared pool (it's fine if tasks are CPU-bound) | Not a real problem until >10K mixed files |
| Recursive `mod` declaration traversal without visited set | Stack overflow on `mod foo { mod foo; }` pathological input or infinite-loop on circular `#[path]` | Track visited paths in a `HashSet<PathBuf>` during traversal | First project with a circular `#[path]` attribute |
| Expanding `pub use *` globs before the module tree is complete | Wrong or empty expansion (module not yet parsed) | Two-pass: build full module tree first, then expand globs | Any crate with cross-module glob re-exports |
| Generating symbol-level DOT export for a large codebase | `dot` process takes minutes, OOM, SVG is 50MB | Count nodes before output; reject if >500 symbols | Any real project with Rust support |
| Re-indexing `use` resolutions for every file when a single `.rs` file changes | O(n) resolution passes on file modify | Limit re-resolution to the changed file and its direct importers | >500 Rust files |
| Running `Cargo metadata` during indexing to discover workspace members | Adds 1-3s latency, requires cargo on PATH | Parse `Cargo.toml` directly (TOML parsing is cheap), avoid shelling out | Every index invocation |

## Integration Gotchas

| Integration | Common Mistake | Correct Approach |
|-------------|----------------|------------------|
| TS/JS resolver + Rust files | Calling `is_external_package` on Rust `use` paths — returns wrong answer (e.g., `std` treated as npm package) | Dispatch on file language BEFORE calling any resolver function |
| thread_local parser pool + new Rust grammar | Adding `PARSER_RS` to `thread_local!` block but forgetting to add `.rs` arm to `parse_file_parallel` match | Keep parser declaration and dispatch match in the same function, add test for each extension |
| bincode cache + new GraphNode variant | New variant added without bumping `CACHE_VERSION` — silent data corruption on deserialization | CI check: bump version in same commit as any GraphNode/EdgeKind change |
| Mermaid output + large Rust project | Output exceeds `maxEdges: 500` default, rendering fails with no clear error | Pre-count edges before output; error with node/edge count and granularity suggestion |
| DOT output + Graphviz rendering | Using `splines=ortho` at >3K nodes causes Graphviz to take minutes or crash | Never use `splines=ortho` by default; omit splines attribute for large graphs |
| Incremental watcher + `mod` declarations | Adding `mod X;` to a Rust file does not trigger indexing of `X` | Track which files were discovered via `mod` declarations; propagate watcher events to child modules |
| Walker + Rust files | `SOURCE_EXTENSIONS` must include `rs` for Rust files to be discovered at all | Add `rs` to extensions list AND ensure Walker exclusions (`target/`) are in place |
| Walker + Rust `target/` directory | Walking `target/` indexes thousands of generated Rust files (proc-macro expanded output, test artifacts) | Add `target` to hard exclusions alongside `node_modules` in walker |

## "Looks Done But Isn't" Checklist

- [ ] **Rust module tree**: Walk starts from `lib.rs`/`main.rs`, not from filesystem `*.rs` glob — verify `src/deeply/nested/mod.rs` is included.
- [ ] **`mod` with `#[path]` attribute**: Custom-path modules are discovered and indexed — verify with a fixture that uses `#[path = "../other.rs"] mod x;`.
- [ ] **Edition handling**: `Cargo.toml` is parsed and edition is read — verify `extern crate` in Edition 2015 crate is handled without crashing.
- [ ] **Rust vs TS resolver dispatch**: `is_external_package` is never called for Rust file imports — verify `use std::collections::HashMap` does not produce an npm-style external node.
- [ ] **`target/` exclusion**: Walker does not index files under `target/` — verify with a project that has been `cargo build`'d (target/ may be GB of generated code).
- [ ] **`pub use` re-export model**: Re-exported symbol has ONE graph node at definition site with a `ReExport` edge — verify no duplicate Symbol nodes.
- [ ] **Graph export defaults**: Default granularity is file or package, not symbol — verify that `code-graph export` on the tool's own source produces a renderable diagram.
- [ ] **Export edge count gate**: Exporting a large project at symbol granularity shows a clear error with count and suggestion, not a silent timeout — verify with 500+ symbol project.
- [ ] **Cache version bump**: Adding any new `GraphNode` or `EdgeKind` variant bumps `CACHE_VERSION` — verify by grepping git diff for both changes in the same commit.
- [ ] **Mermaid `maxEdges`**: Mermaid output stays under 500 edges by default or provides `--max-edges` override option — verify with edge count assertion in tests.
- [ ] **Mixed-language export**: A project with both `.ts` and `.rs` files produces a meaningful export where nodes are identifiable by language — verify with a multi-language fixture.
- [ ] **Cargo workspace**: `use workspace_member::Foo` in a Cargo workspace resolves to a local source node, not an external terminal node — verify on the tool's own workspace if applicable.

## Recovery Strategies

| Pitfall | Recovery Cost | Recovery Steps |
|---------|---------------|----------------|
| TS/JS regression from Rust integration | MEDIUM | Revert shared function changes; add language dispatch layer; re-run test suite |
| Wrong Rust resolver (TS resolver reused) | HIGH | Rewrite Rust resolution module from scratch; cannot be patched incrementally |
| `mod` declarations not followed | HIGH | Refactor entire Rust indexing pipeline to start from crate root; re-index all projects |
| `pub use` duplicate symbol nodes | MEDIUM | Add deduplication pass; remove duplicate nodes; bump cache version; re-index |
| Bincode schema corruption (forgot version bump) | LOW | Delete `.code-graph/graph.bin`; tool auto-rebuilds on next run |
| Symbol-level export unrenderable | LOW | Add node/edge count check; change default granularity; no data migration |
| `target/` indexed as Rust source | MEDIUM | Add `target` to hard exclusion list; re-index; prune cache |
| Mermaid output exceeds `maxEdges` | LOW | Add pre-flight edge count; emit error with suggestion |

## Pitfall-to-Phase Mapping

| Pitfall | Prevention Phase | Verification |
|---------|------------------|--------------|
| Breaking TS/JS pipeline | Rust Parsing (first phase) | All 89 existing tests pass throughout Rust development |
| Wrong resolver for Rust | Rust Module Resolution | `use std::io::Write` produces stdlib terminal node, not npm-style external |
| Missing `mod` declaration traversal | Rust Parsing | All files reachable from `lib.rs` via `mod` chain appear in graph |
| `pub use` duplicate symbols | Rust Module Resolution | Symbol count matches `grep` count of declarations; no duplicates |
| Edition-unaware parsing | Rust Parsing | Edition 2015 fixture crate indexes without errors |
| Macro-invisible symbols | Rust Parsing (scoped) | `#[derive(Debug)]` on a struct records derive hint, not a missing impl |
| Thread-local parser registry | Rust Parsing | `.rs` extension returns `Ok` from `parse_file_parallel` |
| Cascading `mod` change in watch mode | Watch Mode / Incremental | Adding `mod new_module;` to `lib.rs` causes `new_module.rs` to appear in graph |
| Unrenderable export defaults | Graph Export | `code-graph export` on tool's own source completes in <1s and renders in Graphviz |
| Mixed-language export confusion | Graph Export | Exported Mermaid diagram identifies nodes by language and uses module paths for Rust |
| Bincode cache version | Every new node/edge type PR | CI loads old cache version, asserts graceful `None` return |
| Cargo workspace resolution | Rust Module Resolution | Cross-crate `use` in workspace resolves to local source node |

## Sources

- Rust Reference — Modules: [doc.rust-lang.org/reference/items/modules.html](https://doc.rust-lang.org/reference/items/modules.html) — path attribute edge cases, inline vs file module context differences (HIGH confidence — official spec)
- Rust Reference — Use declarations: [doc.rust-lang.org/reference/items/use-declarations.html](https://doc.rust-lang.org/reference/items/use-declarations.html) — glob shadowing, `use {self}` namespace, underscore imports (HIGH confidence — official spec)
- Rust Edition Guide — Path changes: [doc.rust-lang.org/edition-guide/rust-2018/path-changes.html](https://doc.rust-lang.org/edition-guide/rust-2018/path-changes.html) — extern crate elimination, `crate::` keyword, unified path semantics (HIGH confidence — official)
- rust-analyzer blog — IDEs and Macros: [rust-analyzer.github.io/blog/2021/11/21/ides-and-macros.html](https://rust-analyzer.github.io//blog/2021/11/21/ides-and-macros.html) — macro expansion interdependency with name resolution, resource control challenges (HIGH confidence — authoritative source)
- tree-sitter-rust — node-types.json: [github.com/tree-sitter/tree-sitter-rust](https://github.com/tree-sitter/tree-sitter-rust) — `mod_item`, `use_declaration`, `use_wildcard`, `scoped_use_list` node structures (HIGH confidence — grammar source of truth)
- Graphviz forum — large graphs: [forum.graphviz.org/t/creating-a-dot-graph-with-thousands-of-nodes/1092](https://forum.graphviz.org/t/creating-a-dot-graph-with-thousands-of-nodes/1092) — practical 6K node crash threshold, `splines=ortho` danger (MEDIUM confidence — community verified)
- Mermaid GitHub — maxEdges limit: [github.com/mermaid-js/mermaid/issues/5042](https://github.com/mermaid-js/mermaid/issues/5042) — default 500 edge limit, secure config restriction (HIGH confidence — official issue tracker)
- Mermaid config schema: [mermaid.ai/open-source/config/schema-docs/config.html](https://mermaid.ai/open-source/config/schema-docs/config.html) — `maxEdges: 500`, `maxTextSize: 50000` defaults (HIGH confidence — official docs)
- rust-analyzer — CrateDefMap issue: [github.com/rust-lang/rust-analyzer/issues/5922](https://github.com/rust-lang/rust-analyzer/issues/5922) — `pub use` re-export cycle handling, import resolution complexity (MEDIUM confidence — issue tracker)
- bincode backward compat: [users.rust-lang.org/t/tools-for-supporting-version-migration-of-serialization-format/76292](https://users.rust-lang.org/t/tools-for-supporting-version-migration-of-serialization-format/76292) — zero backward compat guarantees, field order sensitivity (HIGH confidence — official forum + library docs)
- Existing codebase analysis: `/workspace/src/` — actual implementation reviewed for integration points, shared code paths, and data model assumptions (HIGH confidence — source of truth for this project)

---
*Pitfalls research for: code intelligence engine — v1.1 Rust language support + graph export*
*Researched: 2026-02-23*
