# Stack Research

**Domain:** Code intelligence engine — v1.1 additions: Rust language support + graph export
**Researched:** 2026-02-23
**Confidence:** HIGH (all findings verified against crates.io, docs.rs, cargo tree output from live project)

---

## Context: What Already Exists (DO NOT Re-Research)

The v1.0 stack is fully validated and in production. This document covers ONLY the new dependencies needed for v1.1.

**Existing stack (do not change):**
- `tree-sitter = "0.26"` → resolves to 0.26.5
- `tree-sitter-typescript = "0.23"` → resolves to 0.23.2
- `tree-sitter-javascript = "0.25"` → resolves to 0.25.0
- `tree-sitter-language = "0.1"` → resolves to 0.1.7 (the shared bridge crate)
- `petgraph = "0.6"` (with `stable_graph`, `serde-1` features) → resolves to 0.6.5
- `toml = "0.8"` → resolves to 0.8.23 (already present, used in config.rs)
- rayon, bincode, serde, tokio, rmcp, clap, etc. — all unchanged

---

## New Dependencies Required

### 1. Rust Grammar — tree-sitter-rust

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `tree-sitter-rust` | `"0.24"` | Parse Rust source files | Official grammar maintained by tree-sitter org. Only Rust grammar crate. Parses all Rust constructs needed: `use_declaration`, `mod_item`, `function_item`, `struct_item`, `trait_item`, `impl_item`, `enum_item`. |

**Compatibility confirmed:** `tree-sitter-rust 0.24.0` depends on `tree-sitter-language ^0.1` (not `tree-sitter` directly). The `tree-sitter-language 0.1.7` bridge crate decouples grammar ABI from the `tree-sitter` runtime version. Confirmed by `cargo tree` on the live project — the same pattern already works for `tree-sitter-typescript 0.23.2` and `tree-sitter-javascript 0.25.0` alongside `tree-sitter 0.26.5`. No version conflict.

**Node types available (verified against tags.scm and node-types.json):**
```
use_declaration    → field: argument (path, scoped_use_list, use_list, use_wildcard, use_as_clause)
mod_item           → fields: name (identifier), body? (declaration_list), visibility_modifier?
function_item      → field: name (identifier), tags.scm: @definition.function
struct_item        → field: name (type_identifier), tags.scm: @definition.class
enum_item          → field: name (type_identifier), tags.scm: @definition.class
trait_item         → field: name (type_identifier), tags.scm: @definition.interface
impl_item          → fields: trait? (type_identifier), type (type_identifier)
```

**Tags query patterns (from TAGS_QUERY constant):**
```scheme
(function_item name: (identifier) @name) @definition.function
(struct_item name: (type_identifier) @name) @definition.class
(enum_item name: (type_identifier) @name) @definition.class
(trait_item name: (type_identifier) @name) @definition.interface
(mod_item name: (identifier) @name) @definition.module
(impl_item trait: (type_identifier) @name) @reference.implementation
(impl_item type: (type_identifier) @name !trait) @reference.implementation
```

**Use TAGS_QUERY directly** — tree-sitter-rust ships a `TAGS_QUERY` constant (equivalent to what exists for the TS/JS grammars). No need to write custom queries for symbol extraction.

**Cargo.toml addition:**
```toml
tree-sitter-rust = "0.24"
```

---

### 2. Cargo.toml Parsing — cargo_toml (NEW dependency)

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `cargo_toml` | `"0.22"` | Parse Cargo.toml for workspace members, package names, and inter-crate dependencies | Dedicated Cargo manifest parser with workspace inheritance support. Covers `[workspace].members`, `[dependencies]`, and path dependencies correctly. |

**Why not use the existing `toml = "0.8"` crate?**
The `toml` crate does raw TOML deserialization into generic `Value` or custom structs. It would require manually defining structs matching Cargo.toml schema and handling workspace inheritance (where member packages inherit deps from `[workspace.dependencies]`). `cargo_toml` already implements these structs and handles inheritance.

**Key API surface (verified against docs.rs v0.22.3):**
```rust
// Load workspace root
let manifest = cargo_toml::Manifest::from_path("Cargo.toml")?;

// Access workspace members (relative paths, may include globs)
if let Some(workspace) = &manifest.workspace {
    let members: &Vec<String> = &workspace.members;  // e.g. ["crates/*", "tools/mytool"]
    let deps = &workspace.dependencies;               // workspace-level deps
}

// Access package name and dependencies
if let Some(package) = &manifest.package {
    let name = &package.name;
}
let deps = &manifest.dependencies;  // DepsSet (BTreeMap<String, Dependency>)
```

**What it covers for Rust module resolution:**
- Workspace root detection → `[workspace].members` glob expansion
- Package name extraction → `[package].name` per member
- Inter-crate path dependencies → `Dependency::Detailed { path: Some(...) }`
- Workspace dependency inheritance → `complete_from_path()` handles `{ workspace = true }`

**What it does NOT cover** (must be implemented as custom logic):
- File-to-module mapping (the `mod` tree walk — see Architecture)
- `use` path resolution to source files
- `pub use` re-export tracking

**Cargo.toml addition:**
```toml
cargo_toml = "0.22"
```

---

### 3. DOT Export — petgraph::dot (NO new dependency)

**petgraph already includes DOT export.** No additional crate needed.

`petgraph::dot::Dot` is available in the existing `petgraph = "0.6"` dependency (resolves to 0.6.5). `StableGraph` implements `IntoNodeReferences` and `IntoEdgeReferences` (confirmed — this parity was added and is present in 0.6+).

**API to use:**
```rust
use petgraph::dot::{Dot, Config};

// Basic export (uses Debug formatting for node/edge weights)
let dot_string = format!("{:?}", Dot::with_config(&graph, &[Config::EdgeNoLabel]));

// Custom labels (needed for code-graph — format as symbol names, file paths)
let dot_string = format!("{}", Dot::with_attr_getters(
    &graph,
    &[Config::NodeNoLabel, Config::EdgeNoLabel],
    |_, edge| format!("label = {:?}", edge.weight().label()),
    |_, node| format!("label = {:?}", node.weight().name()),
));
```

**Config options available:**
- `Config::EdgeNoLabel` — suppress edge weight labels
- `Config::NodeNoLabel` — suppress node weight labels
- Custom labels via `with_attr_getters` closures

**Limitation:** petgraph's Dot output is noted as "mostly intended for debugging." It is sufficient for the use case (structural visualization), but the output will be basic Graphviz DOT without subgraph/cluster support for package grouping. Package-level granularity will require building a new `Graph` with aggregated nodes, not wrapping the existing `StableGraph` directly.

**No Cargo.toml change needed** — petgraph is already a dependency.

---

### 4. Mermaid Export — Write raw strings (NO new dependency)

**Recommendation: generate Mermaid text directly. Do not add a Mermaid crate.**

**Rationale:**
- Mermaid syntax for flowcharts/graph diagrams is extremely simple to generate as strings
- `mermaid_builder` (v0.1.2, the only viable option) is immature, low downloads, and adds a dependency for something achievable in 30 lines of Rust
- `mermaid-rs-renderer` renders SVG — not what we need (we output text for users to paste/render themselves)
- The output format is straightforward:

```
flowchart LR
  A["file_a.rs"] --> B["file_b.rs"]
  A --> C["file_c.rs"]
```

**Implementation pattern:**
```rust
pub fn to_mermaid(graph: &StableGraph<Node, Edge>) -> String {
    let mut out = String::from("flowchart LR\n");
    for node in graph.node_indices() {
        let weight = &graph[node];
        writeln!(out, "  {}[\"{}\"]\n", node.index(), weight.label()).ok();
    }
    for edge in graph.edge_references() {
        writeln!(out, "  {} --> {}", edge.source().index(), edge.target().index()).ok();
    }
    out
}
```

Package-level grouping uses `subgraph` blocks in Mermaid — also trivial to emit as strings.

**No Cargo.toml change needed.**

---

## Rust Module Resolution Strategy

**There is no oxc_resolver equivalent for Rust.** This is a deliberate design choice for the Rust module system — module resolution is part of `rustc`'s compilation phase, not a separate resolvable library. **Custom implementation is required.**

The resolution algorithm is deterministic and well-specified:

### Step 1: Workspace Discovery (cargo_toml)
```
Find Cargo.toml → check [workspace].members → expand globs → list of member crate roots
```

### Step 2: Crate Root Identification
```
Each member Cargo.toml → [[bin]].path OR [lib].path OR default:
  - Library crate: src/lib.rs
  - Binary crate: src/main.rs
```

### Step 3: Module Tree Walk (tree-sitter-rust + filesystem)
```
Start at crate root file.
For each `mod_item` node with no body (i.e., `mod foo;` not `mod foo { ... }`):
  - Look for file: <current_dir>/foo.rs   (modern convention, Rust 1.30+)
  - Look for file: <current_dir>/foo/mod.rs  (legacy convention)
  - First found wins. Record file → module path mapping.
  - Recurse into found file.
For each `mod_item` with a body (`mod foo { ... }`):
  - Inline module. Parse body in same file context.
```

### Step 4: Use Declaration Resolution (tree-sitter-rust)
```
For each `use_declaration`:
  - Extract path segments from `argument` field (scoped_identifier, scoped_use_list, use_as_clause)
  - First segment is either:
    - `crate::` → resolve within current crate
    - `super::` → resolve in parent module
    - `self::` → resolve in current module
    - Known external crate name (from Cargo.toml [dependencies]) → cross-crate edge
    - `std::`, `core::`, `alloc::` → stdlib (skip, not in graph)
  - Map to file node via module-path → file mapping from Step 3
```

### Step 5: Cross-Crate Edges
```
Cargo.toml [dependencies] with path = "..." → dependency edge between package nodes
```

**Confidence:** HIGH — this algorithm is specified in the Rust Reference (doc.rust-lang.org/reference/items/modules.html). No ambiguity. No external resolver crate needed.

---

## Complete Cargo.toml Diff

```toml
# ADD these lines to [dependencies]:
tree-sitter-rust = "0.24"
cargo_toml = "0.22"

# CHANGE nothing else — petgraph, toml, tree-sitter all unchanged
```

---

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| DOT export | `petgraph::dot` (built-in) | `dot-writer` crate, raw string building | petgraph already has it, and the graph is already a petgraph StableGraph |
| Mermaid export | Raw string generation | `mermaid_builder 0.1.2` | Crate is v0.1.2, low maturity, adds dependency for 30 lines of string formatting |
| Cargo.toml parsing | `cargo_toml 0.22` | `toml 0.8` (already present) | `toml` requires manual struct definitions + no workspace inheritance handling |
| Cargo.toml parsing | `cargo_toml 0.22` | `cargo` crate (the actual Cargo lib) | `cargo` crate is enormous, not intended as a library, unstable API |
| Rust module resolution | Custom implementation | Non-existent library | No `oxc_resolver` equivalent for Rust exists — the problem requires custom implementation |

---

## What NOT to Add

| Avoid | Why | Use Instead |
|-------|-----|-------------|
| `mermaid_builder` | v0.1.2, immature, solves a 30-line problem | Write Mermaid strings directly |
| `mermaid-rs-renderer` | Renders SVG, not text output | Raw Mermaid text generation |
| `cargo` (the Cargo lib crate) | Enormous dep, unstable API, not meant as a library | `cargo_toml` for manifest parsing |
| `dot-writer` or similar DOT crates | petgraph already exports DOT | `petgraph::dot::Dot` |
| `syn` or `proc-macro2` | Full Rust AST parser — overkill, not a parser | `tree-sitter-rust` for structural parsing |
| `rustc-ap-*` crates | Compiler internals, unstable | tree-sitter-rust for parsing, cargo_toml for dependencies |

---

## Version Compatibility

| Package | Version | Compatible With | Notes |
|---------|---------|-----------------|-------|
| `tree-sitter-rust` | 0.24.0 | `tree-sitter 0.26.5`, `tree-sitter-language 0.1.7` | Bridge via `tree-sitter-language ^0.1` — no direct `tree-sitter` dependency at runtime. Verified by cargo tree pattern matching existing typescript/javascript grammars. |
| `cargo_toml` | 0.22.3 | `toml 0.8.x` (transitive) | No conflict with existing `toml = "0.8"` dep |
| `petgraph::dot` | 0.6.5 | `StableGraph` | `StableGraph` implements `IntoNodeReferences` in 0.6+, confirmed in source |

---

## Sources

- `cargo tree` on live project — confirmed `tree-sitter-language 0.1.7` bridge pattern works for all grammar crates alongside `tree-sitter 0.26.5`
- [tree-sitter-rust docs.rs](https://docs.rs/tree-sitter-rust/latest/tree_sitter_rust/) — LANGUAGE, NODE_TYPES, TAGS_QUERY constants; version 0.24.0
- [tree-sitter-rust tags.scm](https://github.com/tree-sitter/tree-sitter-rust/blob/master/queries/tags.scm) — symbol tagging query patterns (HIGH confidence)
- [tree-sitter-rust node-types.json](https://github.com/tree-sitter/tree-sitter-rust/blob/master/src/node-types.json) — confirmed `use_declaration`, `mod_item` field names (HIGH confidence)
- [cargo_toml lib.rs](https://lib.rs/crates/cargo_toml) — version 0.22.3, workspace inheritance support (HIGH confidence)
- [cargo_toml Manifest docs](https://docs.rs/cargo_toml/latest/cargo_toml/struct.Manifest.html) — `Manifest`, `Workspace`, `Package`, `Dependency` structs (HIGH confidence)
- [petgraph::dot docs](https://docs.rs/petgraph/latest/petgraph/dot/struct.Dot.html) — `Dot`, `Config`, `with_attr_getters` (HIGH confidence)
- [Rust Reference: Modules](https://doc.rust-lang.org/reference/items/modules.html) — canonical module resolution algorithm (HIGH confidence)
- [mermaid_builder docs](https://docs.rs/mermaid-builder/latest/mermaid_builder/) — v0.1.2, flowchart support confirmed but not recommended (MEDIUM confidence)

---

*Stack research for: code intelligence engine v1.1 (Rust language support + graph export)*
*Researched: 2026-02-23*
