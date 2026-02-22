# Phase 1: Foundation & Core Parsing - Research

**Researched:** 2026-02-22
**Domain:** Rust binary, tree-sitter parsing, petgraph in-memory graph, file walking with gitignore, CLI
**Confidence:** HIGH

---

<user_constraints>

## User Constraints (from CONTEXT.md)

### Locked Decisions

**CLI output & feedback**
- Silent during indexing by default — no progress output
- Final summary shows breakdown by symbol type: "240 functions, 85 classes, 120 interfaces..." with total file count and elapsed time
- `-v` verbose flag from the start — shows file-by-file parsing output for debugging
- Human-readable output by default, `--json` flag for structured JSON output
- Error/skip counts appear in summary only when files were actually skipped

**Symbol granularity**
- Top-level and exported const arrow functions are symbols (the dominant modern TS pattern)
- Both class methods AND object literal methods are symbols — maximum granularity
- React components detected via JSX return and tagged as "component" type in addition to being a function
- Interface properties and methods tracked as child symbols — enables finding who uses a specific field
- Standard symbol types: functions, classes, interfaces, type aliases, enums, exported variables

**Project configuration**
- Config file: `code-graph.toml` at project root (like rustfmt.toml)
- Project root = current working directory where `code-graph index .` is run (no auto-detection magic)
- File exclusions: respect .gitignore AND always auto-exclude node_modules
- Additional exclusions configurable via code-graph.toml
- Basic monorepo awareness from Phase 1: detect workspaces from package.json and index all packages in one pass

**Error tolerance**
- Malformed/unparseable files: skip the entire file, log a warning, continue indexing
- Unsupported extensions (.vue, .svelte, .coffee): silently skip — only process .ts/.tsx/.js/.jsx
- Permission errors (unreadable files): same as parse errors — skip and include in error count
- Error count appears in final summary only when files were actually skipped
- Lenient overall: never fail the whole indexing run due to individual file issues

### Claude's Discretion

- Exact summary formatting and layout
- Internal graph data structures and memory layout
- Tree-sitter query patterns for symbol extraction
- Config file schema details beyond exclusions

### Deferred Ideas (OUT OF SCOPE)

None — discussion stayed within phase scope

</user_constraints>

---

<phase_requirements>

## Phase Requirements

| ID | Description | Research Support |
|----|-------------|-----------------|
| PARS-01 | Tool can index all .ts/.tsx/.js/.jsx files in a project, respecting .gitignore | `ignore` crate WalkBuilder with standard_filters + filter_entry for node_modules; file type matching via extension check |
| PARS-02 | Tool extracts symbols from each file: functions, classes, interfaces, type aliases, enums, and exported variables | tree-sitter Query S-expressions targeting `function_declaration`, `class_declaration`, `interface_declaration`, `type_alias_declaration`, `enum_declaration`, `lexical_declaration`; `arrow_function` on `const` for exported arrow functions; child symbols via `interface_body` traversal |
| PARS-03 | Tool extracts all import statements (ESM import, CJS require, dynamic import with string literal) | tree-sitter `import_statement` node for ESM; `call_expression` with callee identifier "require" for CJS; `call_expression` with `import` node for dynamic import; `string_fragment` child for the module path |
| PARS-04 | Tool extracts export statements (named exports, default exports, re-exports) | tree-sitter `export_statement` node; `export_clause` + `export_specifier` for named; `export_statement` with `"default"` for default; `export_statement` with `string` child for re-exports |

</phase_requirements>

---

## Summary

Phase 1 establishes the full parsing pipeline: walk the project tree, parse every TS/JS file with tree-sitter, extract symbols and import/export edges, store the result in a petgraph in-memory graph, and report a summary to the user. All four required technologies (file walking, parsing, symbol extraction, graph storage) are mature in the Rust ecosystem with high-quality crates available and well-documented APIs.

The `ignore` crate (from BurntSushi/ripgrep) provides a production-grade WalkBuilder that handles .gitignore semantics automatically. Tree-sitter 0.26.x has first-class Rust bindings, and the TypeScript grammar (v0.23.2) supports all modern TypeScript 5.x features including the `satisfies` operator and const type parameters. The tree-sitter-typescript grammar provides two separate parsers: one for `.ts` files and one for `.tsx` files — this distinction is mandatory and must be handled at the file-walk layer. The `petgraph` StableGraph is the right choice for this phase because node indices must remain stable as the graph is built incrementally.

**Primary recommendation:** Use `ignore` for walking, `tree-sitter` + `tree-sitter-typescript` + `tree-sitter-javascript` for parsing, `petgraph::stable_graph::StableGraph` for the in-memory graph, `clap` (derive API) for CLI, and `serde` + `toml` + `serde_json` for config and package.json reading. Avoid building any custom file-ignore logic, query parsers, or graph implementations.

---

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `tree-sitter` | 0.26.x (latest 0.26.5) | Incremental parser runtime — parses source into AST, runs S-expression queries | Only production-grade incremental parser with Rust-native bindings; used by Neovim, Helix, GitHub Copilot |
| `tree-sitter-typescript` | 0.23.2 | TypeScript + TSX grammars | Official grammar from tree-sitter org; supports TS 5.x `satisfies`, const type params |
| `tree-sitter-javascript` | 0.25.0 | JavaScript + JSX grammars | Official grammar; needed for .js/.jsx files (TS grammar does not cover bare JS) |
| `ignore` | 0.4.x | File walking with .gitignore semantics | From ripgrep author; production-hardened, handles nested .gitignore, .git/info/exclude, global gitignore |
| `petgraph` | 0.6.x | Directed in-memory graph | Standard Rust graph crate; `StableGraph` keeps NodeIndex stable across insertions |
| `clap` | 4.x | CLI argument parsing + subcommands | Dominant Rust CLI library; derive macro gives cargo-like UX with minimal boilerplate |
| `serde` + `serde_json` | 1.0 | Parse package.json for monorepo workspaces field | Universally used in Rust ecosystem; serde_json::from_str into typed structs |
| `toml` | 0.9.x | Parse code-graph.toml config file | Native Rust TOML library from the toml-rs org; same as Cargo's config format |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `anyhow` | 1.x | Ergonomic error handling with context | Use for all IO, parse, and config errors; avoids custom error types in phase 1 |
| `serde` derive | 1.0 | Struct deserialization for config and package.json | Required alongside toml and serde_json |
| `std::time::Instant` | stdlib | Elapsed time for summary output | No dependency needed; `Instant::now()` + `.elapsed()` |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `ignore` | `walkdir` | walkdir does not handle .gitignore; would require hand-rolling ignore logic |
| `petgraph::StableGraph` | `petgraph::Graph` | Graph has wider algorithm support but indices shift on removal; StableGraph is correct for incremental builds |
| `clap` derive | `clap` builder API | Builder is more flexible; derive is cleaner for a structured CLI with known subcommands |
| `toml` | `config-rs` | config-rs handles multiple sources (env, files); overkill for a single .toml file |
| `anyhow` | `thiserror` | thiserror for library errors; anyhow for binary error handling — this is a binary |

**Installation:**
```toml
# Cargo.toml [dependencies]
tree-sitter = "0.26"
tree-sitter-typescript = "0.23"
tree-sitter-javascript = "0.25"
ignore = "0.4"
petgraph = { version = "0.6", features = ["stable_graph"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.9"
anyhow = "1"
```

---

## Architecture Patterns

### Recommended Project Structure

```
src/
├── main.rs              # Entry point: parse CLI, dispatch to commands
├── cli.rs               # Clap structs: Cli, Commands, IndexArgs
├── config.rs            # CodeGraphConfig: parse code-graph.toml, defaults
├── walker.rs            # File discovery: WalkBuilder, gitignore, extension filter
├── parser/
│   ├── mod.rs           # Parsing orchestration: parse_file() dispatch
│   ├── languages.rs     # Language instances: ts_language(), tsx_language(), js_language()
│   ├── symbols.rs       # Tree-sitter queries + extractors for symbols
│   └── imports.rs       # Tree-sitter queries + extractors for imports/exports
├── graph/
│   ├── mod.rs           # CodeGraph struct wrapping StableGraph
│   ├── node.rs          # Node enum: FileNode, SymbolNode
│   └── edge.rs          # Edge enum: Contains, Imports, Exports
└── output.rs            # Summary formatting: human-readable and --json
```

### Pattern 1: Language Selection by Extension

**What:** Choose the tree-sitter Language based on file extension at parse time. Tree-sitter-typescript exposes two separate language objects — one for `.ts` and one for `.tsx`. This is mandatory because TSX grammar enables JSX syntax which conflicts with TypeScript angle-bracket casts.

**When to use:** In `parser/mod.rs`, before calling `parser.set_language()`.

**Example:**
```rust
// Source: tree-sitter-typescript crate docs + GitHub README
use tree_sitter::Language;

fn language_for_extension(ext: &str) -> Option<Language> {
    match ext {
        "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "js" | "jsx" => Some(tree_sitter_javascript::LANGUAGE.into()),
        _ => None,
    }
}
```

### Pattern 2: Parser Setup and Parsing

**What:** One `tree_sitter::Parser` per file (or per-thread with thread-local). Set language, parse UTF-8 bytes, get the syntax tree.

**When to use:** In `parser/mod.rs` `parse_file()` function.

**Example:**
```rust
// Source: https://docs.rs/tree-sitter/latest/tree_sitter/index.html
use tree_sitter::Parser;

let mut parser = Parser::new();
parser
    .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
    .expect("Error loading TypeScript grammar");

let source = std::fs::read(path)?;
let tree = parser.parse(&source, None)
    .ok_or_else(|| anyhow::anyhow!("tree-sitter parse returned None"))?;
let root_node = tree.root_node();
```

### Pattern 3: Tree-sitter S-Expression Queries

**What:** Compile a `Query` once per language (not per file). Run `QueryCursor::new()` per file to iterate matches. Extract text with `node.utf8_text(&source)`.

**When to use:** In `parser/symbols.rs` and `parser/imports.rs` — precompile queries as `OnceLock<Query>` statics.

**Example:**
```rust
// Source: https://docs.rs/tree-sitter/latest/tree_sitter/struct.Query.html
use tree_sitter::{Query, QueryCursor};
use std::sync::OnceLock;

static SYMBOL_QUERY: OnceLock<Query> = OnceLock::new();

fn symbol_query(language: &Language) -> &Query {
    SYMBOL_QUERY.get_or_init(|| {
        Query::new(language, r#"
            (function_declaration name: (identifier) @name) @func
            (class_declaration name: (type_identifier) @name) @class
            (interface_declaration name: (type_identifier) @name) @interface
            (type_alias_declaration name: (type_identifier) @name) @type_alias
            (enum_declaration name: (identifier) @name) @enum
            (lexical_declaration
              (variable_declarator
                name: (identifier) @name
                value: (arrow_function)) @arrow) @exported_arrow
        "#).expect("invalid query")
    })
}

fn extract_symbols(tree: &Tree, source: &[u8], query: &Query) -> Vec<Symbol> {
    let mut cursor = QueryCursor::new();
    let mut symbols = vec![];
    for (m, _) in cursor.matches(query, tree.root_node(), source) {
        // extract @name capture text
        for capture in m.captures {
            let name = capture.node.utf8_text(source).unwrap_or("").to_string();
            // build Symbol from capture index + name
            symbols.push(/* ... */);
        }
    }
    symbols
}
```

### Pattern 4: File Walking with gitignore + node_modules exclusion

**What:** Use `WalkBuilder` with standard_filters (auto-reads .gitignore) plus a `filter_entry` closure to force-exclude `node_modules` even if not in .gitignore. Then filter yielded entries by extension.

**When to use:** In `walker.rs`.

**Example:**
```rust
// Source: https://docs.rs/ignore/latest/ignore/struct.WalkBuilder.html
use ignore::WalkBuilder;

fn discover_files(root: &Path, extra_excludes: &[String]) -> Vec<PathBuf> {
    let mut builder = WalkBuilder::new(root);
    builder
        .standard_filters(true)   // respects .gitignore automatically
        .filter_entry(|e| {
            // always exclude node_modules regardless of .gitignore
            e.file_name().to_str()
                .map(|n| n != "node_modules")
                .unwrap_or(true)
        });

    builder.build()
        .filter_map(|result| result.ok())
        .filter_map(|entry| {
            let path = entry.path().to_path_buf();
            let ext = path.extension()?.to_str()?;
            matches!(ext, "ts" | "tsx" | "js" | "jsx").then_some(path)
        })
        .collect()
}
```

### Pattern 5: Graph Node/Edge Design

**What:** Use `StableGraph<Node, Edge, Directed>`. Two node kinds: `FileNode` (path + metadata) and `SymbolNode` (name, kind, location, parent file). Edge kinds: `Contains` (file → symbol), `Imports` (file → file), `Exports` (file → symbol). Store a `HashMap<PathBuf, NodeIndex>` alongside the graph for O(1) lookup.

**When to use:** In `graph/mod.rs` as the `CodeGraph` struct.

**Example:**
```rust
// Source: https://docs.rs/petgraph/latest/petgraph/stable_graph/struct.StableGraph.html
use petgraph::stable_graph::StableGraph;
use petgraph::Directed;

#[derive(Debug)]
pub struct CodeGraph {
    pub graph: StableGraph<GraphNode, EdgeKind, Directed>,
    pub file_index: std::collections::HashMap<PathBuf, petgraph::stable_graph::NodeIndex>,
}

impl CodeGraph {
    pub fn new() -> Self {
        Self {
            graph: StableGraph::new(),
            file_index: HashMap::new(),
        }
    }

    pub fn add_file(&mut self, path: PathBuf) -> NodeIndex {
        let idx = self.graph.add_node(GraphNode::File(path.clone()));
        self.file_index.insert(path, idx);
        idx
    }
}
```

### Pattern 6: Clap Derive CLI with Subcommands

**What:** Use `#[derive(Parser)]` on a `Cli` struct with a `Commands` enum. The `index` subcommand takes a path argument plus `--verbose`/`-v` and `--json` flags.

**When to use:** In `cli.rs`.

**Example:**
```rust
// Source: https://docs.rs/clap/latest/clap/_cookbook/escaped_positional_derive/index.html
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "code-graph", version, about = "Index and query code structure")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Index a TypeScript/JavaScript project
    Index {
        /// Path to the project root
        path: PathBuf,
        /// Show file-by-file parsing output
        #[arg(short, long)]
        verbose: bool,
        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },
}
```

### Pattern 7: Monorepo Detection via package.json

**What:** After discovering the root, check for a `package.json` file. If it has a `workspaces` field (array of glob patterns or string), resolve each workspace path and include all sub-packages in the index pass.

**When to use:** In `walker.rs` or `config.rs` during root initialization.

**Example:**
```rust
// Source: serde_json documentation + npm workspaces spec
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct PackageJson {
    workspaces: Option<WorkspacesField>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WorkspacesField {
    Patterns(Vec<String>),
    Config { packages: Vec<String> },
}

fn detect_workspaces(root: &Path) -> Vec<PathBuf> {
    let pkg_path = root.join("package.json");
    let Ok(content) = std::fs::read_to_string(&pkg_path) else { return vec![] };
    let Ok(pkg) = serde_json::from_str::<PackageJson>(&content) else { return vec![] };

    match pkg.workspaces {
        None => vec![],
        Some(WorkspacesField::Patterns(patterns)) |
        Some(WorkspacesField::Config { packages: patterns }) => {
            // expand glob patterns relative to root
            patterns.iter()
                .flat_map(|p| glob::glob(&root.join(p).to_string_lossy()).ok()?.flatten())
                .filter(|p| p.is_dir())
                .collect()
        }
    }
}
```

### Anti-Patterns to Avoid

- **One Parser instance per file with `set_language` repeatedly:** `set_language` resets parser state but is safe. The real issue is creating a new Parser for every file which allocates unnecessarily. Use thread-local or pass parser as `&mut`.
- **Compiling Query per file:** Queries are expensive to compile. Compile once per language with `OnceLock` or `lazy_static`.
- **Using `petgraph::Graph` (not Stable):** Regular Graph shifts NodeIndex on removal. Even if Phase 1 never removes nodes, starting with Stable avoids a forced migration later.
- **Walking with `walkdir` and hand-rolling gitignore:** The `ignore` crate already handles all edge cases (nested .gitignore, .git/info/exclude, global core.excludesFile). Do not rebuild this.
- **Parsing `.ts` files with the TSX grammar:** TSX grammar cannot parse angle-bracket type assertions (`<T>expr`). Must use separate grammars per extension.
- **Blocking on file reads in a serial loop for Phase 1:** Serial is acceptable for Phase 1 correctness. Parallelism is a Phase 6 concern (PERF-02 with rayon).

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| gitignore rule parsing | Custom .gitignore parser | `ignore` crate `WalkBuilder` | gitignore has 30+ edge cases (negations, anchors, character classes, directory-only rules); ignore crate handles all of them |
| S-expression AST queries | Manual tree cursor traversal | tree-sitter `Query` + `QueryCursor` | Cursor traversal is O(n) over the whole tree; compiled queries are optimized NFA-based pattern matching |
| TOML config parsing | Manual string parsing | `toml` + `serde` | TOML has multiline strings, dotted keys, arrays of tables; spec-compliant parsing is non-trivial |
| package.json parsing | Manual JSON string scanning | `serde_json` | JSON edge cases (escaped strings, unicode), wastes time |
| Graph data structure | Custom adjacency list | `petgraph` | Graph algorithms (future phases: cycle detection, transitive closure) come for free |
| CLI flag parsing | `std::env::args()` manual parsing | `clap` | Help text, error messages, type coercion, completion — all free with clap |

**Key insight:** In the Rust ecosystem, every one of these problems has a production-grade, zero-unsafe-contract crate. The build time cost of the dependency is lower than the correctness risk of the hand-rolled alternative.

---

## Common Pitfalls

### Pitfall 1: Using a Single Grammar for Both TS and TSX

**What goes wrong:** Parsing a `.tsx` file with the TypeScript grammar causes parse errors or missed JSX nodes. Parsing a `.ts` file with the TSX grammar causes angle-bracket type assertion (`<Type>expr`) to fail because the TSX grammar treats `<` as JSX.

**Why it happens:** TSX is a distinct dialect. The tree-sitter-typescript crate exposes `LANGUAGE_TYPESCRIPT` and `LANGUAGE_TSX` as two separate values.

**How to avoid:** Route on file extension before calling `parser.set_language()`. `.ts` → `LANGUAGE_TYPESCRIPT`, `.tsx` → `LANGUAGE_TSX`.

**Warning signs:** Parse errors in TSX files when using the TS grammar, or angle-bracket assertions showing as `ERROR` nodes in the tree.

### Pitfall 2: Query Compiled Per File (Performance)

**What goes wrong:** Compiling a `Query` inside the file-parsing loop causes O(files) query compilations, adding significant overhead for large projects.

**Why it happens:** `Query::new()` compiles the S-expression pattern into an NFA — it is not a cheap operation.

**How to avoid:** Compile queries once per language, store in `OnceLock<Query>` static or pass as references. Verify with a benchmark if unsure.

**Warning signs:** Parsing 1000 files takes 10+ seconds where tree-sitter alone would take < 1s.

### Pitfall 3: Missed Exported Arrow Functions

**What goes wrong:** A query targeting only `function_declaration` misses the dominant modern TypeScript pattern: `export const MyFn = () => {}` and `export const MyFn = async () => {}`.

**Why it happens:** Arrow functions are `lexical_declaration` > `variable_declarator` > `arrow_function` — they are NOT `function_declaration` nodes.

**How to avoid:** Include a separate query pattern for `lexical_declaration` with an `arrow_function` value. For exported-only detection, additionally check for `export_statement` wrapping.

**Warning signs:** Symbol counts are much lower than expected; common TS codebases have > 50% of functions as arrow functions.

### Pitfall 4: node_modules Not in .gitignore

**What goes wrong:** Some projects have `node_modules` checked in (rare but exists) or the .gitignore is not at the root. The walker processes thousands of node_modules files, inflating counts and causing memory/time issues.

**Why it happens:** `WalkBuilder::standard_filters(true)` respects .gitignore, but only if node_modules is listed there. If it isn't, node_modules is processed.

**How to avoid:** Always apply a `filter_entry` closure that hard-excludes `node_modules` by directory name, independent of .gitignore. This is a non-negotiable always-exclude.

**Warning signs:** File counts in the thousands for small projects; parse times measured in minutes.

### Pitfall 5: JSX Detection for React Components

**What goes wrong:** Detecting React components requires heuristics, not a single clean node type. A function that returns JSX is a component, but JSX can appear in many positions.

**Why it happens:** The grammar represents JSX as `jsx_element` or `jsx_fragment` nodes anywhere inside function bodies. There is no dedicated `react_component` node type.

**How to avoid:** Use a tree-sitter query that matches functions/arrow functions containing `jsx_element` or `jsx_fragment` in their body (deep descendant match). Tag matched symbols with `kind: "component"` in addition to `"function"`. This is a heuristic — accept false positives.

**Warning signs:** Zero components detected in a React codebase.

### Pitfall 6: CJS require() Not Captured

**What goes wrong:** `const x = require("./module")` is a `call_expression` with identifier `"require"`, not an `import_statement`. A query targeting only `import_statement` misses all CJS imports.

**Why it happens:** CJS require is syntactically a function call, not an import statement.

**How to avoid:** Write a separate query for `call_expression` where the function callee is the identifier `require` and the first argument is a string literal. Extract the `string_fragment` child.

**Warning signs:** Projects with mixed ESM/CJS show zero imports from CJS files.

---

## Code Examples

Verified patterns from official sources and Context7:

### Tree-sitter Query: Symbol Extraction

```rust
// Source: Context7 /websites/rs_tree-sitter + /tree-sitter/tree-sitter-typescript grammar corpus
const SYMBOL_QUERY_TS: &str = r#"
    ; Top-level function declarations
    (function_declaration name: (identifier) @name) @symbol

    ; Exported arrow function constants
    (export_statement
      (lexical_declaration
        (variable_declarator
          name: (identifier) @name
          value: [(arrow_function) (function)]))) @symbol

    ; Class declarations
    (class_declaration name: (type_identifier) @name) @symbol

    ; Interface declarations
    (interface_declaration name: (type_identifier) @name) @symbol

    ; Type alias declarations
    (type_alias_declaration name: (type_identifier) @name) @symbol

    ; Enum declarations
    (enum_declaration name: (identifier) @name) @symbol
"#;
```

### Tree-sitter Query: Import Extraction

```rust
// Source: Context7 /tree-sitter/tree-sitter-typescript grammar corpus
const IMPORT_QUERY: &str = r#"
    ; ESM static imports
    (import_statement
      source: (string (string_fragment) @module_path)) @import

    ; CJS require calls
    (call_expression
      function: (identifier) @require_fn
      arguments: (arguments (string (string_fragment) @module_path)))
    (#eq? @require_fn "require")

    ; Dynamic import with string literal
    (call_expression
      function: (import)
      arguments: (arguments (string (string_fragment) @module_path))) @dynamic_import
"#;
```

### Tree-sitter Query: Export Extraction

```rust
// Source: Context7 /tree-sitter/tree-sitter-typescript grammar corpus
const EXPORT_QUERY: &str = r#"
    ; Named exports: export { Foo, Bar }
    (export_statement
      (export_clause
        (export_specifier
          name: (identifier) @export_name))) @named_export

    ; Re-exports: export { Foo } from './module'
    (export_statement
      (export_clause
        (export_specifier
          name: (identifier) @export_name))
      source: (string (string_fragment) @reexport_source)) @reexport

    ; Default exports
    (export_statement
      "default"
      [(identifier) (function_declaration) (class_declaration)] @default_export)
"#;
```

### WalkBuilder with node_modules Exclusion

```rust
// Source: Context7 /websites/rs_ignore
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

pub fn walk_project(root: &Path) -> Vec<PathBuf> {
    WalkBuilder::new(root)
        .standard_filters(true)
        .filter_entry(|e| {
            e.file_name()
                .to_str()
                .map(|n| n != "node_modules")
                .unwrap_or(true)
        })
        .build()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let p = entry.path().to_path_buf();
            let ext = p.extension().and_then(|e| e.to_str())?;
            matches!(ext, "ts" | "tsx" | "js" | "jsx").then_some(p)
        })
        .collect()
}
```

### Summary Output Pattern (Cargo-style)

```rust
// Design based on Context.md: "like cargo or gh cli"
fn print_summary(stats: &IndexStats, elapsed: Duration, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(stats).unwrap());
        return;
    }

    println!(
        "Indexed {} files in {:.2}s",
        stats.file_count,
        elapsed.as_secs_f64()
    );
    println!(
        "  {} functions, {} classes, {} interfaces, {} types, {} enums, {} variables",
        stats.functions, stats.classes, stats.interfaces,
        stats.type_aliases, stats.enums, stats.variables
    );
    if stats.skipped > 0 {
        eprintln!("  {} files skipped (errors)", stats.skipped);
    }
}
```

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual tree cursor traversal | S-expression Query API | tree-sitter 0.13 (2020) | Orders of magnitude simpler; compiled query patterns, named captures |
| `walkdir` for file discovery | `ignore` crate with gitignore support | 2016+ (ripgrep) | .gitignore semantics handled automatically; no reimplementation needed |
| `petgraph::Graph` only | `petgraph::StableGraph` | petgraph 0.4+ | Stable node indices; graph can be mutated without invalidating external references |
| Regex-based JS/TS symbol extraction | tree-sitter incremental parsing | 2018+ | Handles all syntax edge cases, embedded languages, JSX; regex fails on nested structures |
| `structopt` for CLI | `clap` v4 derive API | clap 3.0 (2022) | structopt merged into clap; v4 is the current stable API |

**Deprecated/outdated:**
- `structopt`: Merged into clap 3+; do not add as a separate dependency.
- `lazy_static!` for OnceLock: `std::sync::OnceLock` is stable since Rust 1.70; use it directly.
- `tree-sitter 0.20.x` API: 0.26.x has breaking changes in query cursor signatures. Use the latest.

---

## Open Questions

1. **React component detection accuracy**
   - What we know: JSX returns are represented as `jsx_element` or `jsx_fragment` nodes inside function bodies.
   - What's unclear: Whether a deep-descendant match query (matching jsx inside any nested expression) is supported cleanly in tree-sitter S-expression syntax, or if we need cursor traversal fallback.
   - Recommendation: Prototype the query with `(jsx_element)` anywhere inside an arrow function body during implementation. Accept false positives in phase 1; refine in later phases.

2. **Interface child symbols — performance cost**
   - What we know: Tracking `property_signature` and `method_signature` as child symbols enables "who uses this field" queries later.
   - What's unclear: For very large interfaces (100+ properties), how many nodes does this add and what is the memory impact at phase 1 scale?
   - Recommendation: Implement child symbol tracking from the start (per user decision) but add a counter to measure node inflation during verification.

3. **toml crate version 0.9.x vs 0.8.x compatibility**
   - What we know: `toml` 0.9 is the latest. The `toml::from_str` API is stable.
   - What's unclear: Exact serde derive compat surface. If the project's other dependencies pin an older toml, there could be version conflicts.
   - Recommendation: Start with `toml = "0.8"` (wider ecosystem compatibility) and upgrade to 0.9 if needed. Both expose the same `from_str` API.

---

## Sources

### Primary (HIGH confidence)

- Context7 `/websites/rs_tree-sitter` — Parser API, Query, QueryCursor, node text extraction
- Context7 `/tree-sitter/tree-sitter-typescript` — Grammar node types: import_statement, export_statement, function_declaration, class_declaration, interface_declaration, type_alias_declaration, enum_declaration, JSX variants
- Context7 `/websites/rs_petgraph` — StableGraph API: add_node, add_edge, NodeIndex stability
- Context7 `/websites/rs_ignore` — WalkBuilder API: standard_filters, filter_entry, add_custom_ignore_filename, overrides
- Context7 `/websites/rs_clap` — Derive API: Parser, Subcommand, ArgAction::Count
- https://docs.rs/tree-sitter/latest/tree_sitter/index.html — Confirmed tree-sitter 0.26.5 as current stable version; set_language, parse, root_node API
- https://deepwiki.com/tree-sitter/tree-sitter-typescript — Confirmed `satisfies_expression` and const type parameter support in TS 5.x; confirmed separate LANGUAGE_TYPESCRIPT and LANGUAGE_TSX exports
- https://tree-sitter.github.io/tree-sitter/using-parsers/queries/1-syntax.html — S-expression query syntax, wildcards, field names, negated fields

### Secondary (MEDIUM confidence)

- WebSearch: tree-sitter 0.26.5 is latest stable (2025-12); tree-sitter-typescript 0.23.2 (Nov 2024); tree-sitter-javascript 0.25.0 (Sep 2025)
- WebSearch: ignore crate WalkBuilder filter_entry + OverrideBuilder pattern for node_modules exclusion (verified against docs.rs API)
- WebSearch: petgraph StableGraph vs Graph index stability semantics (verified against docs.rs)
- WebSearch: serde_json from_str for package.json workspaces field (standard pattern)

### Tertiary (LOW confidence)

- WebSearch: CJS require detection via call_expression — grammar corpus shows call_expression structure; specific #eq? predicate for "require" identifier needs validation during implementation.

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — All libraries verified via Context7 + official docs with current version numbers
- Architecture: HIGH — Patterns derived from Context7 API docs and grammar corpus examples
- Pitfalls: HIGH (structural pitfalls) / MEDIUM (React component heuristics) — TS/TSX grammar split is documented; CJS require pattern is from grammar corpus

**Research date:** 2026-02-22
**Valid until:** 2026-03-22 (stable ecosystem; grammar versions may update but API is stable)
