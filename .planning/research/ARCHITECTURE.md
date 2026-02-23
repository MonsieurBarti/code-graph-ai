# Architecture Patterns: Rust Language Support and Graph Export

**Domain:** Code intelligence engine — v1.1 integration design
**Researched:** 2026-02-23
**Confidence:** HIGH (based on direct codebase analysis + verified external sources)

## Context

This document supersedes the v1.0 pre-implementation architecture file. It is written against the actual shipped v1.0 codebase (9,397 LOC, 58 source files, 89 tests) and focuses specifically on how Rust language support and graph export integrate into the working architecture.

v1.0 key facts that constrain design:
- `parse_file` / `parse_file_parallel` dispatch by file extension string — no abstraction layer
- `language_for_extension` in `languages.rs` is the sole grammar registry
- Thread-local parsers in `parser/mod.rs` are hardcoded: `PARSER_TS`, `PARSER_TSX`, `PARSER_JS`
- `SymbolKind` enum contains TS/JS-specific variants (Component, Interface, TypeAlias)
- `ImportKind` is ESM/CJS/DynamicImport — all JS concepts
- `resolve_all` is built entirely around `oxc_resolver` (a TypeScript-aware resolver)
- Walker and watcher both have `SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx"]` hardcoded
- `GraphNode` enum has no language-specific variants — this is the one clean boundary

---

## 1. Language Abstraction

### Decision: Enum Dispatch, Not Trait Objects

**Recommended approach:** Add a `Language` enum for dispatch at the call site. Do NOT introduce a trait object (`dyn LanguagePlugin`) at this stage.

**Rationale:** The codebase uses pattern matching throughout (`match ext { "ts" => ..., "js" => ... }`). Adding a trait object would require boxing, dynamic dispatch, and making the entire parse pipeline `dyn`-aware. For two languages, that complexity is not justified. An enum is idiomatic, fast, and testable.

```rust
// New: src/parser/language_kind.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageKind {
    TypeScript,
    Tsx,
    JavaScript,
    Rust,
}

impl LanguageKind {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "ts"       => Some(Self::TypeScript),
            "tsx"      => Some(Self::Tsx),
            "js" | "jsx" => Some(Self::JavaScript),
            "rs"       => Some(Self::Rust),
            _          => None,
        }
    }

    pub fn language_str(&self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Tsx        => "tsx",
            Self::JavaScript => "javascript",
            Self::Rust       => "rust",
        }
    }
}
```

This replaces the scattered `match ext { ... }` patterns in:
- `parser/languages.rs` — `language_for_extension`
- `parser/mod.rs` — `parse_file`, `parse_file_parallel`
- `watcher/incremental.rs` — `handle_modified` language_str assignment
- `main.rs` — `build_graph` language_str assignment

### Thread-Local Parser Expansion

Add `PARSER_RS` to the thread-local block in `parser/mod.rs`:

```rust
thread_local! {
    static PARSER_TS:  RefCell<Parser> = RefCell::new(make_parser(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()));
    static PARSER_TSX: RefCell<Parser> = RefCell::new(make_parser(tree_sitter_typescript::LANGUAGE_TSX.into()));
    static PARSER_JS:  RefCell<Parser> = RefCell::new(make_parser(tree_sitter_javascript::LANGUAGE.into()));
    static PARSER_RS:  RefCell<Parser> = RefCell::new(make_parser(tree_sitter_rust::LANGUAGE.into()));
}
```

`parse_file_parallel` adds a `LanguageKind::Rust` arm dispatching to `PARSER_RS`.

### What Changes, What Stays Identical

| Component | Change Required | Notes |
|-----------|-----------------|-------|
| `parser/languages.rs` | Extend `language_for_extension` for `"rs"` | Trivial |
| `parser/mod.rs` | Add `PARSER_RS` thread-local; add Rust arm in both parse functions | Low-risk |
| `watcher/mod.rs` | Add `"rs"` to `SOURCE_EXTENSIONS` | 1-line change |
| `watcher/incremental.rs` | Add `"rs"` arm to `language_str` match | 1-line change |
| `walker.rs` | Add `"rs"` to `SOURCE_EXTENSIONS` | 1-line change |
| `main.rs` `build_graph` | Add `"rs"` arm to `language_str` match | 1-line change |
| `graph/node.rs` | No change — `GraphNode`, `FileInfo` are language-agnostic | Clean boundary |
| `graph/mod.rs` | No change — graph operations are language-agnostic | Clean boundary |
| `resolver/mod.rs` | Large: Rust resolver is separate (see section 2) | New module |
| `query/*` | No change — queries operate on graph nodes regardless of language | Clean boundary |
| `mcp/*` | No change — MCP tools query the graph; language is invisible | Clean boundary |

---

## 2. Rust Module Resolver

### The Core Problem

TypeScript import resolution and Rust module resolution are fundamentally different:

| Dimension | TypeScript | Rust |
|-----------|-----------|------|
| Import syntax | `import { X } from './utils'` | `use crate::utils::X;` or `mod utils;` |
| Resolution mechanism | oxc_resolver follows Node.js algorithm + tsconfig | File-system walk + mod tree traversal |
| Module declaration | Implicit (file = module) | Explicit (`mod foo;` declares `foo.rs` or `foo/mod.rs`) |
| External deps | `node_modules/` lookup | `Cargo.toml [dependencies]` |
| Path syntax | String specifiers | Path segments (`crate::`, `super::`, `self::`) |

### Decision: Custom Rust Resolver, Not oxc_resolver Adapter

**Do not** attempt to adapt oxc_resolver for Rust. oxc_resolver is TypeScript/Node.js-aware and its internal model does not map to Rust's module tree. A custom resolver is ~200-400 LOC and provides cleaner semantics.

**New module:** `src/resolver/rust_resolver.rs`

### Rust Module Resolution Algorithm

The `mod` tree determines which files exist in the module namespace. Resolution requires two passes:

**Pass 1: Build the mod tree from source**

For each `.rs` file, extract `mod foo;` declarations via tree-sitter query. This tells us which child modules exist. Map each `mod foo;` to either:
- `<current_dir>/foo.rs`
- `<current_dir>/foo/mod.rs`

This produces a `ModTree: HashMap<PathBuf, Vec<PathBuf>>` (parent file → child module files).

```
Tree-sitter query for mod declarations:
(mod_item
  name: (identifier) @mod_name
  !body)   ; inline mod {} excluded — those have no file
```

Note: inline `mod foo { ... }` blocks do not correspond to files. Only `mod foo;` (without body) resolves to a file.

**Pass 2: Resolve `use` paths against the mod tree**

For each `use` declaration, extract the path segments from the `use_declaration` node. Classify the root segment:

| Root segment | Meaning | Resolution |
|-------------|---------|------------|
| `crate` | Crate root | Walk from `src/main.rs` or `src/lib.rs` |
| `super` | Parent module | Walk up one level in mod tree |
| `self` | Current module | Current file's module scope |
| `std` / `core` / `alloc` | Rust standard library | Mark as external (builtin), no file |
| Any other identifier | External crate or workspace crate | Look up in Cargo.toml `[dependencies]` |

```rust
// src/resolver/rust_resolver.rs
pub struct RustModTree {
    /// Maps each .rs file to the child module files it declares via `mod foo;`
    pub declared_modules: HashMap<PathBuf, Vec<PathBuf>>,
    /// Maps canonical module path (e.g. "crate::parser::mod") to file path
    pub path_to_file: HashMap<String, PathBuf>,
    /// The crate root file (src/main.rs or src/lib.rs)
    pub crate_root: PathBuf,
}

pub enum RustImportOutcome {
    ResolvedFile(PathBuf),          // use path resolved to a .rs file in the project
    ExternalCrate(String),          // external dependency (from Cargo.toml)
    BuiltinCrate(String),           // std, core, alloc
    Unresolved(String),             // could not resolve
}
```

**Cargo.toml parsing:** Read `[dependencies]` and `[workspace]` sections using the `toml` crate (already a dependency). Extract crate names for external package classification. For workspace members, resolve to local directories the same way as npm workspace packages.

**Crate root detection:** Look for `src/main.rs` first, then `src/lib.rs`. For workspace members, each member has its own `src/main.rs` or `src/lib.rs`.

### Import Extraction via tree-sitter

The tree-sitter Rust grammar uses `use_declaration` nodes. The `argument` field contains the use tree:

```
(use_declaration
  argument: (scoped_identifier | use_list | use_as_clause | identifier | scoped_use_list))
```

Extract the full path by recursively walking the use tree node to produce the canonical path string (e.g. `"crate::parser::imports"`).

For the existing `ImportInfo` struct: a Rust `use` statement maps cleanly to `ImportInfo { kind: ImportKind::Use, module_path: "crate::parser::imports", specifiers: [...] }`. Add `ImportKind::Use` to the enum.

### Integration with resolve_all

`resolve_all` in `resolver/mod.rs` currently runs unconditionally on all files. For v1.1, gate by language:

```rust
pub fn resolve_all(graph: &mut CodeGraph, project_root: &Path,
                   parse_results: &HashMap<PathBuf, ParseResult>, verbose: bool) -> ResolveStats {
    // Partition by language
    let ts_js_results: HashMap<_, _> = parse_results.iter()
        .filter(|(p, _)| is_ts_js(p)).collect();
    let rust_results: HashMap<_, _> = parse_results.iter()
        .filter(|(p, _)| p.extension().map(|e| e == "rs").unwrap_or(false)).collect();

    // Run existing pipeline for TS/JS (no changes)
    if !ts_js_results.is_empty() { resolve_ts_js(...); }

    // Run Rust-specific pipeline
    if !rust_results.is_empty() { resolve_rust(..., &rust_results); }
}
```

The Rust pipeline does NOT use `oxc_resolver`. It uses `RustModTree` + `RustImportOutcome`.

---

## 3. Graph Model: Node and Edge Extensions

### Decision: Extend SymbolKind, Do NOT Add CrateNode or ModuleNode

The current `GraphNode::File` + `GraphNode::Symbol` model is sufficient for Rust. The graph should represent what matters for code intelligence, not mirror the language's module hierarchy exactly.

**Do not add `CrateNode` or `ModuleNode` as new `GraphNode` variants.** Reasoning:

1. `GraphNode::File` already represents a `.rs` file. `FileInfo.language = "rust"` distinguishes it.
2. A `mod foo;` is already captured: `foo.rs` is a `FileNode`. The `mod` declaration itself is redundant — the file being in the graph IS the module.
3. Inline `mod { ... }` blocks without file correspondence are edge cases that add graph complexity without query value. Skip them in v1.1.
4. `ExternalPackageInfo` already works for Cargo crates — just populate with crate name from Cargo.toml.

**Add Rust-specific `SymbolKind` variants:**

```rust
pub enum SymbolKind {
    // Existing TS/JS variants (unchanged)
    Function, Class, Interface, TypeAlias, Enum, Variable, Component, Method, Property,

    // New Rust variants
    Struct,    // struct Foo { ... }
    Trait,     // trait Bar { ... }
    Impl,      // impl Foo (or impl Trait for Foo) — see note below
    Macro,     // macro_rules! or proc macro
    TypeParam, // used in child symbols of generic bounds (future — skip v1.1)
}
```

**Impl blocks — recommended approach:**

`impl Foo` and `impl Trait for Foo` are important for Rust navigation. Two options:

- **Option A (recommended):** Represent the `impl` block as a `SymbolKind::Impl` with name `"impl Foo"` or `"impl Display for Foo"`. Methods inside become `SymbolKind::Method` child symbols. Add an `Implements` edge from the struct to the trait if `impl Trait for Type`.
- **Option B:** Skip `impl` as a symbol node; only add its methods as children of the struct. This loses information about which trait the methods come from.

Option A is recommended because it preserves trait implementation relationships. The name `"impl Display for Point"` is unambiguous and queryable.

**Trait methods as child symbols:** Extract `function_item` nodes inside `declaration_list` (body of `trait_item` or `impl_item`) as `SymbolKind::Method` child symbols, matching the existing pattern for TypeScript class methods.

**Struct fields:** Extract `field_declaration` nodes from `field_declaration_list` as `SymbolKind::Property` child symbols, matching TypeScript interface properties.

### New SymbolKind Mapping to tree-sitter Nodes

| SymbolKind | tree-sitter node | Query pattern |
|------------|-----------------|---------------|
| `Struct` | `struct_item` | `(struct_item name: (type_identifier) @name) @symbol` |
| `Trait` | `trait_item` | `(trait_item name: (type_identifier) @name) @symbol` |
| `Function` | `function_item` (top-level) | `(function_item name: (identifier) @name) @symbol` |
| `Method` | `function_item` in `declaration_list` | child of `impl_item` or `trait_item` |
| `Enum` | `enum_item` | `(enum_item name: (type_identifier) @name) @symbol` |
| `TypeAlias` | `type_item` | `(type_item name: (type_identifier) @name) @symbol` |
| `Impl` | `impl_item` | `(impl_item type: (type_identifier) @name) @symbol` |
| `Macro` | `macro_definition` | `(macro_definition name: (identifier) @name) @symbol` |

**pub visibility:** In Rust, `pub` is visibility. Use tree-sitter's `visibility_modifier` child to set `is_exported`. `pub(crate)`, `pub(super)` map to `is_exported = true` (visible outside file). Private items (no modifier) map to `is_exported = false`.

**No is_default field for Rust:** `is_default` is TS-specific (default exports). Set to `false` for all Rust symbols.

### New Edge Kind for Rust

Add one edge kind to handle Rust trait implementations:

```rust
pub enum EdgeKind {
    // ... existing variants unchanged ...

    /// struct/type implements trait via `impl Trait for Type`
    /// From: SymbolNode (Impl block or Struct) → To: SymbolNode (Trait)
    TraitImpl,
}
```

The existing `Extends` and `Implements` edges are TS-specific semantics. `TraitImpl` is semantically distinct.

---

## 4. Export Pipeline: Where Graph Export Lives

### Decision: New `src/export/` Module with CLI Subcommand

Graph export does not belong in the formatter (`query/output.rs`), which formats query results. Export renders the entire graph structure. It is a separate concern.

**New module tree:**

```
src/export/
├── mod.rs          # pub fn export_graph(graph, config) -> Result<String>
├── dot.rs          # DOT format renderer
└── mermaid.rs      # Mermaid format renderer
```

**New CLI subcommand** added to `cli.rs`:

```rust
/// Export the dependency graph in DOT or Mermaid format for visualization.
Export {
    path: PathBuf,

    /// Output format: dot or mermaid.
    #[arg(long, value_enum, default_value_t = ExportFormat::Dot)]
    format: ExportFormat,

    /// Granularity level: symbol, file, or package.
    #[arg(long, value_enum, default_value_t = Granularity::File)]
    granularity: Granularity,

    /// Filter to a specific file or directory (relative to project root).
    #[arg(long)]
    file: Option<PathBuf>,

    /// Output file path (prints to stdout if omitted).
    #[arg(short, long)]
    output: Option<PathBuf>,
},
```

### DOT Format Implementation

petgraph's built-in `petgraph::dot::Dot` struct generates basic DOT but with limited label control. For v1.1, use `Dot::with_attr_getters` for custom node/edge labeling:

```rust
use petgraph::dot::{Dot, Config};

let dot = Dot::with_attr_getters(
    &filtered_graph,
    &[Config::EdgeNoLabel, Config::NodeNoLabel],
    &|_, edge| format!("label=\"{:?}\"", edge.weight()),
    &|_, (_, node)| match node {
        GraphNode::File(f)   => format!("label=\"{}\" shape=box", f.path.file_name()),
        GraphNode::Symbol(s) => format!("label=\"{}\" shape=ellipse", s.name),
        _                    => String::new(),
    },
);
```

Petgraph's `Dot` type implements `Display`, so `format!("{}", dot)` produces the full DOT string. Confidence: HIGH (verified against official petgraph docs).

### Mermaid Format Implementation

Petgraph does not have built-in Mermaid support. Implement a custom renderer in `export/mermaid.rs`. Mermaid flowchart syntax is straightforward:

```
flowchart LR
    A["src/parser.rs"] --> B["src/graph.rs"]
    B --> C["src/resolver.rs"]
```

Node IDs must be alphanumeric (sanitize file paths). Walk the graph with `graph.node_indices()` and `graph.edge_references()`.

### Granularity Levels

| Level | Nodes | Edges | Use Case |
|-------|-------|-------|---------|
| `symbol` | Every symbol node | All edge types including Calls/Extends | Deep dependency analysis |
| `file` | File nodes only | ResolvedImport edges between files | Architecture overview |
| `package` | ExternalPackage + directory groupings | Import edges between packages/dirs | High-level monorepo view |

For `file` and `package` granularity, pre-filter the `CodeGraph` before passing to the renderer. Create a filtered subgraph using petgraph's `FilterNode` / `FilterEdge` utilities, or copy relevant nodes/edges into a new `StableGraph` for clean rendering.

### Integration with `build_graph` and cache

The export command calls `build_graph` (same pipeline as all other commands) then passes the graph to `export::export_graph`. No special caching needed — `build_graph` already saves to `.code-graph/` cache, so subsequent export calls will use the cached graph (via the existing cache module).

---

## 5. Watcher Changes for Rust

### Decision: Minimal Changes — Rust Files Use Same Watcher Infrastructure

The watcher infrastructure (`notify`, debouncer, `WatchEvent`, `start_watcher`) is file-system-level and language-agnostic. The only language-specific parts are:

1. `SOURCE_EXTENSIONS` constant — add `"rs"`
2. `CONFIG_FILES` constant — add `"Cargo.toml"` for full re-index on workspace changes
3. `handle_modified` in `watcher/incremental.rs` — extend `language_str` match with `"rs"` arm

### Rust mod tree shift on file changes

**The problem:** In TypeScript, every file is independently importable. In Rust, a file only participates in the module tree if it is declared via `mod foo;` somewhere. If a new `.rs` file appears, nothing imports it until someone adds `mod foo;` — which triggers a re-parse of the parent file.

**The v1.1 solution:** Follow the same re-index pattern as TS/JS:

1. File modified → remove from graph, re-parse, re-resolve imports for that file
2. File created → same as modified (current code already handles create === modified via `path.exists()` check)
3. File deleted → remove from graph, mark importers as unresolved

**The edge case:** If `mod foo;` is added to `lib.rs`, `lib.rs` is the modified file. `handle_modified` re-parses `lib.rs`, which now extracts the `mod foo;` declaration. The Rust resolver then tries to resolve `foo` → `src/foo.rs`. If `src/foo.rs` is already in the graph (was indexed), the edge gets created. If not, it stays unresolved until `src/foo.rs` appears.

This is equivalent to how the existing `fix_unresolved_pointing_to` function handles the TS case. The Rust resolver needs a similar pass.

**`ConfigChanged` for Cargo.toml:** When `Cargo.toml` changes, trigger a full re-index (same as `tsconfig.json` / `package.json`). This handles workspace member additions and dependency changes.

**No new watcher logic required for v1.1.** The mod tree shift case is handled by the existing incremental pipeline structure. The only addition is `"Cargo.toml"` in `CONFIG_FILES`.

---

## 6. Component Boundaries and Data Flow

### Updated Architecture for v1.1

```
┌─────────────────────────────────────────────────────────────────┐
│                         CLI Entry Point                          │
│  index | find | refs | impact | circular | context | stats      │
│  watch | mcp | EXPORT (new)                                      │
├───────────────────────────┬─────────────────────────────────────┤
│      Indexing Pipeline    │         Export Pipeline (new)        │
│  walker (+ .rs files)     │  export/dot.rs                       │
│  parser (+ Rust grammar)  │  export/mermaid.rs                   │
│  resolver                 │  Granularity filter                  │
│  ├── ts_js (unchanged)    │  petgraph Dot API                    │
│  └── rust (new)           │                                      │
├───────────────────────────┴─────────────────────────────────────┤
│                    CodeGraph (unchanged)                          │
│  GraphNode: File | Symbol(+Struct,Trait,Impl,Macro) |           │
│             ExternalPackage | UnresolvedImport                   │
│  EdgeKind: Contains | ChildOf | ResolvedImport | Calls |        │
│            Extends | Implements | BarrelReExportAll | TraitImpl  │
├─────────────────────────────────────────────────────────────────┤
│           Cache | Watcher | MCP | Query (all unchanged)          │
└─────────────────────────────────────────────────────────────────┘
```

### New vs Modified vs Unchanged Components

| Component | Status | Work Required |
|-----------|--------|---------------|
| `src/parser/language_kind.rs` | **New** | `LanguageKind` enum, replaces scattered extension matches |
| `src/parser/rust/mod.rs` | **New** | Rust symbol/import/relationship extraction |
| `src/parser/rust/symbols.rs` | **New** | tree-sitter queries for Rust symbols |
| `src/parser/rust/imports.rs` | **New** | tree-sitter queries for `use_declaration` nodes |
| `src/resolver/rust_resolver.rs` | **New** | Mod tree builder, Cargo.toml parser, use path resolver |
| `src/export/mod.rs` | **New** | Export entry point |
| `src/export/dot.rs` | **New** | DOT format renderer using petgraph::dot |
| `src/export/mermaid.rs` | **New** | Mermaid format renderer (custom) |
| `src/graph/node.rs` | **Modified** | Add Struct, Trait, Impl, Macro to `SymbolKind` |
| `src/graph/edge.rs` | **Modified** | Add `TraitImpl` edge kind |
| `src/parser/languages.rs` | **Modified** | Add `"rs"` extension |
| `src/parser/mod.rs` | **Modified** | Add `PARSER_RS` thread-local, Rust dispatch arm |
| `src/walker.rs` | **Modified** | Add `"rs"` to `SOURCE_EXTENSIONS` |
| `src/watcher/mod.rs` | **Modified** | Add `"rs"` to `SOURCE_EXTENSIONS`, `"Cargo.toml"` to CONFIG_FILES |
| `src/watcher/incremental.rs` | **Modified** | Add `"rs"` to `language_str` match |
| `src/resolver/mod.rs` | **Modified** | Gate TS/JS vs Rust resolver dispatch |
| `src/cli.rs` | **Modified** | Add `Export` subcommand |
| `src/main.rs` | **Modified** | Add Export command handler |
| `src/graph/mod.rs` | Unchanged | Graph operations are language-agnostic |
| `src/query/*` | Unchanged | Queries work on graph nodes, language-invisible |
| `src/mcp/*` | Unchanged | MCP tools query graph, language-invisible |
| `src/cache/*` | Unchanged | Cache serializes the graph, language-invisible |
| `src/output.rs` | Unchanged | IndexStats fields may need Rust-specific additions |

---

## 7. Suggested Build Order

The order is determined by dependencies: each component needs its dependencies to be in place first.

### Step 1: Language Abstraction Layer (unblocks everything else)

1. Create `src/parser/language_kind.rs` with `LanguageKind` enum
2. Update `parser/languages.rs` to add `"rs"` → `tree_sitter_rust::LANGUAGE`
3. Add `tree-sitter-rust` to `Cargo.toml`
4. Add `PARSER_RS` thread-local to `parser/mod.rs`
5. Add `"rs"` to all `SOURCE_EXTENSIONS` arrays (walker, watcher)
6. Add `"Cargo.toml"` to `CONFIG_FILES` in watcher

**Why first:** Every subsequent step depends on `.rs` files being discovered and dispatched correctly. This is also the lowest-risk set of changes.

### Step 2: Rust Symbol Extraction (unblocks graph population)

1. Create `src/parser/rust/` submodule
2. Implement `symbols.rs`: tree-sitter queries for struct, enum, fn, trait, impl, macro
3. Implement `imports.rs`: tree-sitter queries for `use_declaration` nodes
4. Add `Struct`, `Trait`, `Impl`, `Macro` to `SymbolKind` in `graph/node.rs`
5. Wire Rust parser into `parse_file` and `parse_file_parallel`
6. Write tests against real Rust code snippets (including the project's own source)

**Why second:** Symbol extraction is needed before resolver (resolver wires symbols). It is also independently testable against tree-sitter output.

### Step 3: Rust Module Resolver (unblocks import edges)

1. Create `src/resolver/rust_resolver.rs`
2. Implement `RustModTree` builder (walk files, extract `mod foo;` declarations)
3. Implement `use` path classification (crate/super/self/external/builtin)
4. Implement Cargo.toml `[dependencies]` reader (reuse `toml` crate)
5. Implement workspace detection from `[workspace]` section
6. Wire into `resolver/mod.rs` with language-gated dispatch
7. Add `TraitImpl` edge kind to `graph/edge.rs`
8. Write tests for mod tree building, use path resolution

**Why third:** Depends on symbols being in the graph. Cargo.toml parsing is self-contained.

### Step 4: Watcher Incremental for Rust (unblocks watch mode)

1. Verify `handle_modified` works for `.rs` files with the new parser/resolver
2. Add `fix_unresolved_pointing_to` behavior for Rust mod tree changes (if needed)
3. Manual test: modify a `.rs` file, verify incremental re-index

**Why fourth:** Depends on parser and resolver being correct. Low new code — mostly wiring.

### Step 5: Graph Export (independent of Rust support)

1. Create `src/export/mod.rs` with `ExportConfig` struct and entry point
2. Implement `export/dot.rs` using `petgraph::dot::Dot::with_attr_getters`
3. Implement `export/mermaid.rs` as custom renderer
4. Add granularity filtering (file-level subgraph extraction)
5. Add `Export` subcommand to `cli.rs` and handler in `main.rs`
6. Write tests: export a known graph and assert DOT/Mermaid string structure

**Why fifth (and could be parallel with Steps 2-4):** Export is architecturally independent. It only needs the `CodeGraph` to exist. It can be built and tested against a TS/JS-only graph before Rust support is complete. **This is the recommended parallelization point if working with two developers.**

### Step 6: CLI/MCP Parity Verification

1. Run all existing CLI commands against a Rust project (the codebase itself)
2. Verify `find`, `refs`, `impact`, `circular`, `context`, `stats` produce correct output
3. Verify `watch` handles `.rs` file changes correctly
4. Verify `mcp` server responds to queries about Rust symbols

**Why last:** Integration verification. Depends on all prior steps.

---

## 8. Integration Points Summary

### Points Where TS/JS Pipeline Must Not Break

The explicit contract is "no unnecessary refactoring of working TS/JS pipeline." Every change to shared infrastructure must preserve TS/JS behavior:

| Change Point | Risk | Mitigation |
|-------------|------|-----------|
| Adding `LanguageKind::Rust` arm to extension dispatch | Low | All existing arms are unchanged; `"rs"` is a new branch |
| Adding `"rs"` to `SOURCE_EXTENSIONS` | Low | Filter is additive; existing extensions unchanged |
| Extending `SymbolKind` with Rust variants | Low | New enum variants; existing match arms need `_ =>` handling check |
| Extending `EdgeKind` with `TraitImpl` | Low | New variant; existing code never encounters it on TS files |
| Gating resolver dispatch by language | Medium | Must preserve oxc_resolver path for TS/JS files exactly as before |
| Modifying `resolve_all` signature | Medium | If signature changes, all call sites must update; prefer additive |

### Key Invariant: Parser Thread-Local Safety

The `thread_local!` + `RefCell<Parser>` pattern is critical for rayon performance. Adding `PARSER_RS` follows the same pattern exactly. Do not attempt to share parsers across languages or make them non-thread-local.

### Key Invariant: Graph Mutation is Sequential

`petgraph::StableGraph` is not `Send`. The build_graph function in `main.rs` follows the pattern:

```
par_iter (parse, rayon parallel) → collect → sequential graph mutation
```

This invariant must be preserved for Rust files. The Rust resolver also mutates the graph and must run in the sequential phase.

---

## Sources

- [petgraph::dot::Dot docs](https://docs.rs/petgraph/latest/petgraph/dot/struct.Dot.html) — `with_attr_getters` API confirmed, HIGH confidence
- [tree-sitter-rust tags.scm](https://github.com/tree-sitter/tree-sitter-rust/blob/master/queries/tags.scm) — node types confirmed: `struct_item`, `enum_item`, `function_item`, `trait_item`, `impl_item`, `mod_item`, HIGH confidence
- [tree-sitter-rust node-types.json](https://github.com/tree-sitter/tree-sitter-rust/blob/master/src/node-types.json) — field names confirmed: `use_declaration.argument`, `impl_item.type`, `impl_item.trait`, `mod_item.name`, `mod_item.body`, HIGH confidence
- [Rust Reference: Use declarations](https://doc.rust-lang.org/reference/items/use-declarations.html) — confirmed `crate`, `super`, `self` semantics, HIGH confidence
- [Cargo Book: Workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html) — workspace member resolution confirmed, HIGH confidence
- Codebase direct analysis: all component behaviors verified from source at `/workspace/src/`

---

*Architecture research for: code-graph v1.1 — Rust language support and graph export*
*Researched: 2026-02-23*
