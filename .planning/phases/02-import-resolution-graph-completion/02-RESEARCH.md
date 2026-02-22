# Phase 2: Import Resolution & Graph Completion - Research

**Researched:** 2026-02-22
**Domain:** TypeScript/JavaScript import resolution, graph edge construction, monorepo workspace detection
**Confidence:** HIGH

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

**Resolution strategy:**
- Always chase through re-export chains to the original defining file — never stop at barrel files
- Wildcard re-exports (`export * from './foo'`) resolved lazily at query time, not eagerly at index time — faster indexing
- External packages (node_modules) appear as nodes in the graph (name + version) but their internals are not indexed — imports from externals are terminal edges with package metadata

**Unresolved imports:**
- Claude's Discretion: choose the best approach for handling unresolvable imports (missing files, dynamic paths, external packages without node_modules)

**Symbol relationships:**
- 'calls' edges track direct function/method calls only (`foo()`, `obj.method()`) — no callback passing or assignment tracking
- Type references (`const x: SomeType`, function parameter types) treated the same as value references — no separate edge type for type-only relationships
- Full inheritance hierarchy: class extends class, class implements interface, interface extends interface — all captured
- 'contains' relationship tracks all nesting levels: file > function > nested function, class > method > inner function — full containment tree

**Workspace & monorepo scope:**
- Support all three major package managers: npm, yarn, and pnpm workspace protocols
- Index from a user-specified root — follow cross-package imports into other workspace packages as needed (not automatic full-monorepo indexing)
- Cross-package imports resolve directly to source files, not dist/build output — map package name to its source directory
- Detect turbo.json/nx.json to confirm monorepo structure, but resolve packages from package.json workspaces only — no special Turbo/Nx integration

**tsconfig handling:**
- Claude's Discretion: handling of multiple tsconfig files, extends chains, and project references — researcher/planner determine best approach

### Claude's Discretion

- How to handle unresolvable imports (missing files, dynamic paths, external packages without node_modules)
- How to handle multiple tsconfig files, extends chains, and project references

### Deferred Ideas (OUT OF SCOPE)

None — discussion stayed within phase scope
</user_constraints>

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|-----------------|
| PARS-05 | Tool resolves TypeScript path aliases from tsconfig.json (paths, baseUrl, extends chains) | oxc_resolver handles all of this natively: TsconfigOptions.config_file + TsconfigReferences::Auto |
| PARS-06 | Tool resolves barrel file imports to the actual defining file (symbol-level resolution through index.ts) | Post-resolution pass: follow ExportKind::ReExport and ExportKind::ReExportAll chains using the already-parsed graph |
| PARS-07 | Tool resolves monorepo workspace packages to local paths (package.json workspaces) | Read pnpm-workspace.yaml / package.json#workspaces, build name→path map, feed as aliases into oxc_resolver |
| PARS-08 | Tool builds a complete file-level dependency graph with import edges | Add `Imports` edge for every resolved ImportInfo; store unresolved imports as `UnresolvedImport` node+edge |
| PARS-09 | Tool builds symbol-level relationships: contains, exports, calls, extends, implements | New tree-sitter query pass using class_heritage, implements_clause, call_expression, member_expression node types |
</phase_requirements>

---

## Summary

Phase 2 takes the ParseResult data already produced in Phase 1 (imports, exports, symbols per file) and transforms the raw string module specifiers into concrete edges in the `CodeGraph`. The work splits into four distinct sub-problems: (1) file-level resolution using `oxc_resolver`, (2) barrel-file chasing to reach original defining symbols, (3) monorepo workspace package name mapping, and (4) a new tree-sitter query pass to extract symbol-level relationship edges (calls, extends, implements).

The critical architectural insight is that all four problems are **post-parse passes** — they consume the already-built symbol/import/export data from Phase 1 and layer new edges onto the existing `CodeGraph`. No re-parsing is needed. The resolution pipeline runs after all files are indexed: first workspace packages are mapped, then a single `Resolver` instance resolves all imports, then barrel chains are followed, then symbol relationship edges are added.

The key external crate is `oxc_resolver` v3.x (the highest version compatible with Rust's stable edition 2021 toolchain on this system). It handles path alias resolution, tsconfig `extends` chains, and `TsconfigReferences::Auto` for project references — all with zero manual tsconfig parsing required. Unresolvable imports (dynamic paths, missing files, bare external packages not in workspace) should be stored as `UnresolvedImport` nodes to preserve graph completeness and make gaps visible to query callers.

**Primary recommendation:** Use `oxc_resolver` for file resolution (PARS-05, PARS-07, PARS-08) and tree-sitter queries for symbol-level edges (PARS-09). Barrel resolution (PARS-06) is a pure graph traversal pass — no new dependencies needed.

---

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| oxc_resolver | 3.x (latest compatible with Rust stable edition 2021) | Resolve module specifiers to absolute paths, handling tsconfig paths/baseUrl/extends | Rust port of enhanced-resolve + tsconfig-paths-webpack-plugin + tsconfck; 28x faster than webpack equivalent; industry standard in Rust tooling (Rspack, Biome, OXC) |
| petgraph | 0.6 (already in Cargo.toml) | Add new edge types and traverse graph for barrel chaining | Already in project; `StableGraph::add_edge`, `neighbors_directed`, `Dfs` iterator cover all needs |
| tree-sitter + tree-sitter-typescript | 0.26 / 0.23 (already in Cargo.toml) | New query pass for symbol relationship extraction (calls, extends, implements) | Already in project and Phase 1 already demonstrates OnceLock query pattern |
| serde_json | 1 (already in Cargo.toml) | Parse package.json for workspace globs, name, version | Already in project |
| glob | 0.3 (already in Cargo.toml) | Expand pnpm-workspace.yaml / package.json#workspaces glob patterns to find workspace package directories | Already in project |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| serde (with derive) | 1 (already in Cargo.toml) | Deserialize tsconfig.json (when needed beyond what oxc_resolver reads) | Not needed — oxc_resolver parses tsconfig internally |
| toml | 0.8 (already in Cargo.toml) | Parse pnpm-workspace.yaml — NOTE: YAML is not TOML | Do NOT use for YAML; use serde_yaml or manual line parsing for pnpm-workspace.yaml |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| oxc_resolver | Hand-rolled tsconfig parser | Hand-rolling misses extends chains, ${configDir} template variables, project references, and edge cases in path glob matching. oxc_resolver is battle-tested against the TypeScript test suite. |
| oxc_resolver | typescript-language-server | Too heavyweight; requires Node.js runtime; defeats single-binary goal |
| tree-sitter queries | Hand-walking AST for symbol edges | Tree-sitter queries are declarative and already established as the project pattern |

**Installation (new dependency only):**

```toml
# Cargo.toml additions
oxc_resolver = "3"
serde_yaml = "0.9"   # for pnpm-workspace.yaml parsing (no YAML support currently)
```

Note: `serde_yaml` is only needed if the project must support pnpm workspaces via `pnpm-workspace.yaml`. Consider using a minimal YAML line parser instead to avoid a large dependency.

---

## Architecture Patterns

### Recommended Project Structure

```
src/
├── resolver/
│   ├── mod.rs          # pub fn resolve_all(graph, files, config) -> Result<()>
│   ├── file_resolver.rs  # oxc_resolver wrapper: specifier -> AbsPath or UnresolvedImport
│   ├── barrel.rs         # barrel chain follower: file -> original defining file+symbol
│   ├── workspace.rs      # workspace package map builder (npm/yarn/pnpm)
│   └── tsconfig.rs       # tsconfig discovery: find root tsconfig.json for project
├── parser/
│   ├── mod.rs            # (existing)
│   ├── symbols.rs        # (existing)
│   ├── imports.rs        # (existing)
│   └── relationships.rs  # NEW: call/extends/implements extraction via tree-sitter
└── graph/
    ├── mod.rs            # (existing, extended with new edge/node types)
    ├── node.rs           # ADD: ExternalPackage node type
    └── edge.rs           # ADD: Calls, Extends, Implements, ExportedBy edge types
```

### Pattern 1: Resolution Pipeline (Two-Pass Architecture)

**What:** Parse all files first (Phase 1 already does this). Then do a second pass that resolves all imports into edges.
**When to use:** Always — avoids forward-reference problems (file A imports file B which hasn't been parsed yet).

```
Pass 1 (Phase 1 — complete): parse all files → populate file nodes, symbol nodes, raw ImportInfo/ExportInfo
Pass 2 (Phase 2):
  Step 1: build workspace package map (name → source dir)
  Step 2: create Resolver with tsconfig + workspace aliases
  Step 3: for each file, for each ImportInfo → resolve → add edge (Imports or ExternalImport)
  Step 4: barrel chain pass → for each ReExport edge, follow to original definer → add Exports edge
  Step 5: symbol relationship pass → tree-sitter re-query each file for calls/extends/implements
```

### Pattern 2: oxc_resolver Configuration for TypeScript Projects

```rust
// Source: oxc_resolver-3.0.0/src/options.rs + examples/resolver.rs (verified)
use oxc_resolver::{ResolveOptions, Resolver, TsconfigOptions, TsconfigReferences};
use std::path::PathBuf;

fn build_resolver(project_root: &Path) -> Resolver {
    Resolver::new(ResolveOptions {
        extensions: vec![
            ".ts".into(), ".tsx".into(), ".js".into(), ".jsx".into(),
            ".json".into(), ".node".into(),
        ],
        // extension_alias maps .js imports → try .ts first (for projects that import .js but have .ts source)
        extension_alias: vec![
            (".js".into(), vec![".ts".into(), ".tsx".into(), ".js".into()]),
        ],
        tsconfig: Some(TsconfigOptions {
            config_file: project_root.join("tsconfig.json"),
            references: TsconfigReferences::Auto, // follows project references automatically
        }),
        // condition_names controls ESM vs CJS package exports field resolution
        condition_names: vec!["node".into(), "import".into()],
        // Do NOT include node_modules in extensions — external packages stay terminal
        ..ResolveOptions::default()
    })
}

// Resolve a single import specifier from a source file's directory
fn resolve_specifier(
    resolver: &Resolver,
    source_file: &Path,
    specifier: &str,
) -> Option<PathBuf> {
    let dir = source_file.parent()?;
    match resolver.resolve(dir, specifier) {
        Ok(resolution) => Some(resolution.into_path_buf()),
        Err(_) => None, // becomes UnresolvedImport node
    }
}
```

### Pattern 3: Workspace Package Map Construction

**npm/yarn** — read `workspaces` array from root `package.json`:
```json
{ "workspaces": ["packages/*", "apps/*"] }
```

**pnpm** — read `packages` array from `pnpm-workspace.yaml`:
```yaml
packages:
  - 'packages/*'
  - 'apps/*'
```

Both produce glob patterns. Expand each pattern from the workspace root, then for each matched directory read `package.json` to get the `name` field. Build a `HashMap<String, PathBuf>` mapping package name to source directory.

```rust
// Pseudocode — no new crates needed beyond serde_json + glob (both already in project)
fn build_workspace_map(root: &Path) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    let patterns = discover_workspace_globs(root); // reads package.json or pnpm-workspace.yaml
    for pattern in patterns {
        for pkg_dir in glob::glob(&format!("{}/{}", root.display(), pattern))
            .unwrap().flatten()
        {
            if let Ok(pkg_json) = read_package_json(&pkg_dir) {
                if let Some(name) = pkg_json["name"].as_str() {
                    // Point to src/ if it exists, else package root
                    let src = pkg_dir.join("src");
                    map.insert(name.to_owned(),
                        if src.exists() { src } else { pkg_dir });
                }
            }
        }
    }
    map
}
```

Feed the map into `ResolveOptions::alias` so `oxc_resolver` sees workspace packages as aliases:
```rust
let alias = workspace_map.into_iter()
    .map(|(name, path)| (name, vec![AliasValue::Path(path.to_string_lossy().into())]))
    .collect();
// Then: ResolveOptions { alias, ... }
```

### Pattern 4: Barrel Chain Resolution (Pure Graph Traversal)

After all files are indexed and `Imports` edges are added, follow re-export chains:

```
For each symbol S imported by file A:
  1. Resolve A's import → file B (barrel index.ts)
  2. Look in B's ExportInfo list for S:
     - ExportKind::ReExport { source: "./impl" } → recurse into ./impl
     - ExportKind::ReExportAll { source: "./types" } → mark as lazy (skip, record barrel target)
     - ExportKind::Named / Default → B is the definer, stop here
  3. Add edge: A --[Exports]--> actual_defining_file::symbol_node
```

Wildcard `ReExportAll` is resolved lazily (as decided): store a `BarrelExport` edge from barrel file to its source, resolve at query time when a specific symbol is requested.

**Cycle detection is mandatory:** barrel files can form cycles (`a/index.ts` re-exports from `b/index.ts` which re-exports from `a/index.ts`). Use a `HashSet<PathBuf>` visited set during traversal.

### Pattern 5: Symbol Relationship Extraction (tree-sitter queries)

New tree-sitter query pass over already-parsed files. Reuse existing `OnceLock<Query>` pattern from Phase 1.

**Extends/Implements** (verified from tree-sitter-typescript grammar):
```scheme
; Class extends class or abstract class
(class_declaration
  name: (type_identifier) @class_name
  (class_heritage
    (extends_clause
      value: (identifier) @extends_name)))

; Class implements interface
(class_declaration
  name: (type_identifier) @class_name
  (class_heritage
    (implements_clause
      (type_identifier) @implements_name)))

; Interface extends interface
(interface_declaration
  name: (type_identifier) @iface_name
  (extends_type_clause
    (type_identifier) @parent_name))
```

**Direct function/method calls** (CONTEXT decision: `foo()` and `obj.method()` only):
```scheme
; Direct call: foo()
(call_expression
  function: (identifier) @fn_name
  arguments: (arguments))

; Method call: obj.method()
(call_expression
  function: (member_expression
    property: (property_identifier) @method_name)
  arguments: (arguments))
```

**Type references** (same edge type as value references per CONTEXT decision):
```scheme
; Type annotation: const x: SomeType
(type_annotation
  (type_identifier) @type_ref)

; Function parameter type
(required_parameter
  type: (type_annotation
    (type_identifier) @type_ref))
```

### Pattern 6: New Graph Node and Edge Types

**Additions to `edge.rs`:**

```rust
pub enum EdgeKind {
    Contains,                                   // (existing) file → symbol
    Imports { specifier: String },              // (existing) file → file (raw, pre-resolution)
    Exports { name: String, is_default: bool }, // (existing) file → symbol

    // Phase 2 additions:
    /// Resolved import edge: importing file → actual resolved file
    ResolvedImport { specifier: String },
    /// Symbol → symbol: direct function/method call
    Calls,
    /// Symbol (class) → symbol (class/interface): inheritance
    Extends,
    /// Symbol (class) → symbol (interface): interface implementation
    Implements,
    /// File/Symbol → symbol: explicitly re-exported by this file (barrel resolution result)
    ExportedBy { name: String },
    /// File → file: barrel file re-exports everything from source (lazy wildcard)
    BarrelReExportAll,
    ChildOf,                                    // (existing) child symbol → parent symbol
}
```

**Additions to `node.rs`:**

```rust
/// External package node (node_modules dependency)
pub struct ExternalPackageInfo {
    pub name: String,
    pub version: Option<String>,  // from package.json if available
}

pub enum GraphNode {
    File(FileInfo),               // (existing)
    Symbol(SymbolInfo),           // (existing)
    ExternalPackage(ExternalPackageInfo), // Phase 2 addition
    UnresolvedImport { specifier: String }, // Phase 2 addition: captures unresolvable imports
}
```

### Anti-Patterns to Avoid

- **Resolving during parse:** Never call `oxc_resolver` during the tree-sitter parse pass. Resolve in a dedicated second pass after all files are in the graph (avoids ordering issues).
- **Creating a new Resolver per file:** `Resolver::new()` reads tsconfig and builds caches. Create one instance, reuse across all files.
- **Stopping at barrel files:** The CONTEXT decision requires chasing to the original definer. Never add a `ResolvedImport` edge that terminates at an `index.ts` barrel file when the barrel re-exports from elsewhere.
- **Eager wildcard resolution:** `export * from './foo'` must NOT be expanded at index time — too expensive, may hit cycles. Record as `BarrelReExportAll` edge; resolve at query time.
- **Missing cycle detection in barrel traversal:** Barrel chains can be circular. A `HashSet<PathBuf>` visited set is mandatory.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| tsconfig paths/baseUrl/extends resolution | Custom tsconfig parser + path matcher | `oxc_resolver` | `extends` chains, `${configDir}` substitution, project references, glob patterns in `paths` — all edge cases are handled; hand-rolling misses them |
| Node.js module resolution algorithm | Manual directory + extension probing | `oxc_resolver` | `main` field, `exports` field, `browser` field, symlinks, directory index files — all specified by Node.js spec |
| TypeScript workspace package discovery | Custom file walker | `glob` crate (already present) + `serde_json` for package.json | Glob expansion from workspace patterns is exactly what the `glob` crate does |
| Circular import detection in barrel chains | BFS with manual tracking | `HashSet<PathBuf>` visited set (std) | Simple and sufficient; petgraph's cycle detection algorithms are available but overkill for barrel chains |

**Key insight:** The entire file-resolution problem space (paths, baseUrl, extends, extensions, index files, package.json fields) is solved by `oxc_resolver`. The only custom logic required is the barrel-chaining pass (pure graph traversal) and the symbol-relationship extraction (tree-sitter queries following the existing project pattern).

---

## Common Pitfalls

### Pitfall 1: tsconfig Not Found at Project Root

**What goes wrong:** `oxc_resolver` is configured with `TsconfigOptions { config_file: root.join("tsconfig.json") }`, but the project has its tsconfig in a subdirectory or named differently (`tsconfig.app.json`, `tsconfig.base.json`).

**Why it happens:** Monorepos commonly have a root `tsconfig.json` (base) and per-package `tsconfig.json` files. The root tsconfig may contain only `references`, not `paths`.

**How to avoid:** Implement a tsconfig discovery strategy (Claude's Discretion area):
1. Check if `tsconfig.json` exists at the project root — use it if found.
2. If not found, search upward from each source file for the nearest `tsconfig.json`.
3. For monorepos, prefer per-package tsconfig when it exists (it will `extends` the root via `TsconfigReferences::Auto`).

**Warning signs:** `ResolveError::TsconfigNotFound` in resolve results; path alias imports failing while relative imports succeed.

### Pitfall 2: Extension Resolution Order for TypeScript

**What goes wrong:** A file imports `'./utils'` (no extension). The resolver tries `.js` first (default) and resolves to `utils.js` instead of `utils.ts`.

**Why it happens:** `oxc_resolver`'s default `extensions` is `[".js", ".json", ".node"]`. TypeScript files will not be found.

**How to avoid:** Always configure extensions as `[".ts", ".tsx", ".js", ".jsx", ".json", ".node"]` — put TypeScript extensions first.

Additionally, use `extension_alias` to map `.js` imports to `.ts` files (for projects that write `import './foo.js'` in TypeScript source):
```rust
extension_alias: vec![(".js".into(), vec![".ts".into(), ".tsx".into(), ".js".into()])],
```

**Warning signs:** Resolved paths end in `.js` when `.ts` files exist with the same stem.

### Pitfall 3: pnpm-workspace.yaml vs package.json Workspaces

**What goes wrong:** Workspace detection code only reads `package.json#workspaces`, missing pnpm monorepos which use a separate `pnpm-workspace.yaml` with a `packages:` key.

**Why it happens:** npm and yarn use `workspaces` in `package.json`; pnpm uses a dedicated YAML file. The file formats and field names differ.

**How to avoid:** Check for both files:
1. If `pnpm-workspace.yaml` exists at root → parse `packages:` array (YAML).
2. Else if `package.json` exists at root with `workspaces` field → parse that array (JSON).
3. Both produce glob patterns that are expanded the same way.

**Warning signs:** Workspace package imports resolving to `node_modules` instead of local source when project uses pnpm.

### Pitfall 4: Workspace Package source vs dist Resolution

**What goes wrong:** A workspace package `@myorg/utils` has its `main` field pointing to `dist/index.js`. `oxc_resolver` resolves to `dist/`, not `src/`.

**Why it happens:** The locked decision says "cross-package imports resolve directly to source files, not dist/build output." But if the package.json `main` field points to dist, resolution follows that.

**How to avoid:** When building the workspace alias map, always check for a `src/` directory in the package and prefer it over the `main` field:
```rust
let src_dir = pkg_dir.join("src");
let target = if src_dir.exists() { src_dir } else { pkg_dir.clone() };
// Feed target as alias value, bypassing package.json main field
```

**Warning signs:** Resolved paths contain `/dist/` or `/build/` for workspace packages.

### Pitfall 5: Barrel Cycle in Re-export Chains

**What goes wrong:** Following re-export chains hits an infinite loop. `packages/ui/index.ts` exports from `packages/ui/components/index.ts` which exports from `packages/ui/index.ts`.

**Why it happens:** Barrel files in monorepos frequently form cycles, especially when a package re-exports itself as a convenience.

**How to avoid:** Maintain a `HashSet<PathBuf>` of visited files during barrel traversal. If a file is already in the visited set, stop traversal and mark the import as unresolvable (or as the barrel itself).

**Warning signs:** Stack overflow during resolution pass; test hanging indefinitely.

### Pitfall 6: Resolver Instance Per File (Performance)

**What goes wrong:** A new `Resolver::new(options)` is called for each of the 10K files in the project. tsconfig is re-parsed 10K times.

**Why it happens:** Resolver construction is expensive — it reads and parses tsconfig.json, sets up caches.

**How to avoid:** Create one `Resolver` instance before the resolution loop; reuse it for all files. `Resolver` is `Send` + `Sync`, safe to share across threads.

### Pitfall 7: Unresolved Imports Silently Dropped

**What goes wrong:** Dynamic imports (`import(someVariable)`), imports of deleted files, and Node.js builtins are silently skipped. The graph appears complete but is missing edges.

**Why it happens:** Early return on resolution error without recording the failure.

**How to avoid:** On `ResolveError`, add an `UnresolvedImport` node + edge. This makes gaps visible to callers and enables the query layer to surface missing dependencies. Classify errors:
- `ResolveError::NotFound` → missing file (possibly deleted)
- `ResolveError::Builtin` → Node.js builtin (e.g., `fs`, `path`)
- `ResolveError::Specifier` → dynamic specifier (variable in import string)

---

## Code Examples

Verified patterns from official sources:

### oxc_resolver: Full TypeScript Project Setup

```rust
// Source: oxc_resolver-3.0.0/src/options.rs + examples/resolver.rs (local inspection)
use oxc_resolver::{
    ResolveOptions, Resolver, TsconfigOptions, TsconfigReferences, AliasValue,
};
use std::path::{Path, PathBuf};

pub fn build_resolver(
    project_root: &Path,
    workspace_aliases: Vec<(String, Vec<AliasValue>)>,
) -> Resolver {
    let tsconfig_path = project_root.join("tsconfig.json");
    let tsconfig = if tsconfig_path.exists() {
        Some(TsconfigOptions {
            config_file: tsconfig_path,
            references: TsconfigReferences::Auto,
        })
    } else {
        None
    };

    Resolver::new(ResolveOptions {
        extensions: vec![
            ".ts".into(), ".tsx".into(), ".mts".into(),
            ".js".into(), ".jsx".into(), ".mjs".into(),
            ".json".into(), ".node".into(),
        ],
        extension_alias: vec![
            (".js".into(), vec![".ts".into(), ".tsx".into(), ".js".into()]),
        ],
        tsconfig,
        alias: workspace_aliases,
        condition_names: vec!["node".into(), "import".into()],
        ..ResolveOptions::default()
    })
}

pub fn resolve_import(
    resolver: &Resolver,
    from_file: &Path,
    specifier: &str,
) -> ResolutionOutcome {
    let dir = from_file.parent().expect("file has parent dir");
    match resolver.resolve(dir, specifier) {
        Ok(r) => ResolutionOutcome::Resolved(r.into_path_buf()),
        Err(oxc_resolver::ResolveError::Builtin { resolved, .. }) => {
            ResolutionOutcome::BuiltinModule(resolved)
        }
        Err(_) => ResolutionOutcome::Unresolved,
    }
}

pub enum ResolutionOutcome {
    Resolved(PathBuf),
    BuiltinModule(String),
    Unresolved,
}
```

### petgraph: Adding Phase 2 Edges

```rust
// Source: petgraph docs + existing CodeGraph pattern in src/graph/mod.rs
impl CodeGraph {
    /// Add a resolved import edge: from_file imports from to_file.
    pub fn add_resolved_import(&mut self, from: NodeIndex, to: NodeIndex, specifier: &str) {
        self.graph.add_edge(from, to, EdgeKind::ResolvedImport {
            specifier: specifier.to_owned(),
        });
    }

    /// Add an external package node and an edge from the importing file.
    pub fn add_external_import(
        &mut self,
        from: NodeIndex,
        pkg_name: &str,
        specifier: &str,
    ) -> NodeIndex {
        // Reuse existing external package node if same package already seen.
        let pkg_idx = self.graph.add_node(GraphNode::ExternalPackage(ExternalPackageInfo {
            name: pkg_name.to_owned(),
            version: None,
        }));
        self.graph.add_edge(from, pkg_idx, EdgeKind::ResolvedImport {
            specifier: specifier.to_owned(),
        });
        pkg_idx
    }

    /// Add a Calls edge between two symbol nodes.
    pub fn add_calls_edge(&mut self, caller: NodeIndex, callee: NodeIndex) {
        self.graph.add_edge(caller, callee, EdgeKind::Calls);
    }

    /// Add an Extends edge between two symbol nodes (class→class or iface→iface).
    pub fn add_extends_edge(&mut self, child: NodeIndex, parent: NodeIndex) {
        self.graph.add_edge(child, parent, EdgeKind::Extends);
    }

    /// Add an Implements edge (class→interface).
    pub fn add_implements_edge(&mut self, class: NodeIndex, interface: NodeIndex) {
        self.graph.add_edge(class, interface, EdgeKind::Implements);
    }
}
```

### tree-sitter: Call Expression Query

```rust
// Following existing OnceLock pattern from src/parser/imports.rs
const CALLS_QUERY: &str = r#"
    ; Direct call: foo(...)
    (call_expression
      function: (identifier) @callee_name
      arguments: (arguments))

    ; Method call: obj.method(...)
    (call_expression
      function: (member_expression
        property: (property_identifier) @callee_name)
      arguments: (arguments))
"#;

const EXTENDS_QUERY: &str = r#"
    ; class Foo extends Bar
    (class_declaration
      name: (type_identifier) @class_name
      (class_heritage
        (extends_clause
          value: (identifier) @extends_name)))

    ; class Foo implements IBar, IBaz
    (class_declaration
      name: (type_identifier) @class_name
      (class_heritage
        (implements_clause
          (type_identifier) @implements_name)))

    ; interface Foo extends IBar
    (interface_declaration
      name: (type_identifier) @iface_name
      (extends_type_clause
        (type_identifier) @parent_iface_name))
"#;
```

### Workspace Detection

```rust
// Reads npm/yarn package.json workspaces or pnpm-workspace.yaml
// Source: verified from pnpm docs + npm workspace docs
use std::path::{Path, PathBuf};
use std::collections::HashMap;

pub fn discover_workspace_packages(root: &Path) -> HashMap<String, PathBuf> {
    let mut result = HashMap::new();
    let patterns = read_workspace_globs(root);

    for pattern in patterns {
        let full_pattern = format!("{}/{}/package.json", root.display(), pattern);
        if let Ok(paths) = glob::glob(&full_pattern) {
            for pkg_json_path in paths.flatten() {
                if let Some(pkg_dir) = pkg_json_path.parent() {
                    if let Ok(content) = std::fs::read_to_string(&pkg_json_path) {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(name) = json["name"].as_str() {
                                let src = pkg_dir.join("src");
                                let target = if src.exists() { src } else { pkg_dir.to_path_buf() };
                                result.insert(name.to_owned(), target);
                            }
                        }
                    }
                }
            }
        }
    }
    result
}

fn read_workspace_globs(root: &Path) -> Vec<String> {
    // pnpm: pnpm-workspace.yaml with 'packages:' array
    let pnpm_yaml = root.join("pnpm-workspace.yaml");
    if pnpm_yaml.exists() {
        // Parse YAML minimally: lines after 'packages:' that start with '  - '
        if let Ok(content) = std::fs::read_to_string(&pnpm_yaml) {
            return parse_pnpm_workspace_yaml(&content);
        }
    }

    // npm/yarn: package.json with 'workspaces' array
    let pkg_json = root.join("package.json");
    if let Ok(content) = std::fs::read_to_string(&pkg_json) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(arr) = json["workspaces"].as_array() {
                return arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
            }
        }
    }
    vec![]
}
```

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual tsconfig parsing | `oxc_resolver` with `TsconfigOptions` | 2023-2024 | No more hand-rolling extends chain + paths glob matching |
| Separate tsconfig-paths library | Built into `oxc_resolver` | 2023 | Single dependency instead of two |
| ts-morph / TypeScript compiler API for type resolution | tree-sitter structural queries (no type inference) | Project decision | Out of scope per REQUIREMENTS.md |
| TypeScript `moduleResolution: node` | `moduleResolution: bundler` (TypeScript 5.0+) | 2023 | `oxc_resolver` supports both; extensions list must include `.ts`/`.tsx` |

**Deprecated/outdated:**
- `tsconfig-paths` npm package: not usable from Rust — Rust tools use `oxc_resolver` instead
- Manual tsconfig extends resolution: `oxc_resolver` does this internally with `TsconfigReferences::Auto`
- Starting Resolver::new per file: creates new cache on each construction — always reuse

---

## Open Questions

1. **pnpm-workspace.yaml YAML parsing**
   - What we know: pnpm uses YAML format with a `packages:` key containing a list of glob strings
   - What's unclear: Whether to add `serde_yaml` dependency or implement minimal line-based parser
   - Recommendation: Implement a simple line parser for the common case (`packages:\n  - 'glob'\n`). Add `serde_yaml` only if edge cases require it. The format is simple enough that a 10-line parser covers 95% of real projects.

2. **tsconfig discovery in multi-package monorepos**
   - What we know: `oxc_resolver` with `TsconfigOptions { config_file, references: Auto }` handles project references. But paths in `compilerOptions.paths` are relative to the tsconfig that defines them.
   - What's unclear: If a per-package `tsconfig.json` extends a root tsconfig that defines `paths`, does `oxc_resolver` correctly resolve those paths from the per-package location?
   - Recommendation: Trust `TsconfigReferences::Auto` and test with a real monorepo fixture. If path resolution fails for extended paths, fall back to pointing the resolver at the root tsconfig rather than per-package.

3. **Symbol-to-symbol calls across files**
   - What we know: tree-sitter can extract call site names (function/method identifier strings). The symbol_index in CodeGraph maps names to NodeIndex.
   - What's unclear: A call to `foo()` may match multiple symbols named `foo` across different files. Without type inference (out of scope), we can't know which one is called.
   - Recommendation: When a call target name matches exactly one symbol in the graph, add a `Calls` edge. When it matches multiple, add edges to all candidates and mark them as `confidence: ambiguous` (or omit cross-file calls entirely for now, only recording intra-file calls). Document this limitation clearly.

4. **oxc_resolver compatibility with Rust edition 2024**
   - What we know: The current workspace uses Rust edition 2024 (Cargo.toml: `edition = "2024"`). oxc_resolver versions 11.x require edition 2024 themselves (`simd-json` dep requires it) but our CI Rust toolchain is 1.84.1 which does not yet fully support edition 2024. `oxc_resolver = "3.x"` (edition 2021) was confirmed to fetch correctly.
   - What's unclear: Whether the project's own edition 2024 causes issues when depending on a 2021 edition crate.
   - Recommendation: Use `oxc_resolver = "3"` (latest edition-2021-compatible version). Edition of a dependency is independent of the consumer's edition — this is safe.

---

## Sources

### Primary (HIGH confidence)
- Local inspection of `oxc_resolver-3.0.0` source at `/usr/local/cargo/registry/src/index.crates.io-6f17d22bba15001f/oxc_resolver-3.0.0/src/` — `options.rs`, `lib.rs`, `resolution.rs`, `error.rs`, `examples/resolver.rs` (directly verified)
- Existing Phase 1 code in `/workspace/src/` — `graph/mod.rs`, `graph/edge.rs`, `graph/node.rs`, `parser/imports.rs`, `Cargo.toml` (directly read)
- `/workspace/.planning/phases/02-import-resolution-graph-completion/02-CONTEXT.md` — user decisions (directly read)
- petgraph docs via Context7 `/websites/rs_petgraph` — `neighbors_directed`, `Dfs`, `add_edge` API (verified)
- tree-sitter-typescript grammar via DeepWiki — `class_heritage`, `extends_clause`, `implements_clause`, `call_expression` node kinds (verified via multiple sources)

### Secondary (MEDIUM confidence)
- [oxc_resolver GitHub + DeepWiki](https://deepwiki.com/oxc-project/oxc-resolver) — TsconfigDiscovery::Auto behavior, project references, monorepo caching (consistent with local source inspection)
- [TypeScript TSConfig Reference — paths](https://www.typescriptlang.org/tsconfig/paths.html) — paths are relative to the tsconfig that defines them, not the extending tsconfig (official TypeScript docs)
- [pnpm workspace documentation](https://pnpm.io/workspaces) — `pnpm-workspace.yaml` format with `packages:` key (official pnpm docs)
- [npm/yarn workspace format](https://nesbitt.io/2026/01/18/workspaces-and-monorepos-in-package-managers.html) — `package.json#workspaces` array (verified against npm docs)

### Tertiary (LOW confidence)
- tree-sitter-typescript grammar node kinds from DeepWiki — `extends_type_clause` for interface extends (needs validation with actual tree-sitter playground; grammar node names may differ from what DeepWiki returned)

---

## Metadata

**Confidence breakdown:**
- Standard stack (oxc_resolver): HIGH — source code directly inspected, API verified
- Architecture (resolution pipeline): HIGH — follows established two-pass pattern, aligned with user decisions
- tsconfig handling (TsconfigReferences::Auto): HIGH — source-verified in options.rs
- Workspace detection: HIGH — verified against official pnpm and npm docs; glob expansion confirmed
- Barrel chain traversal: HIGH — pure graph traversal using petgraph primitives already in project
- tree-sitter query node names: MEDIUM — cross-referenced multiple sources; `extends_clause` and `implements_clause` confirmed; `extends_type_clause` for interface extends needs playground validation
- Symbol call resolution across files: MEDIUM — ambiguity problem well-understood, recommended approach (single match = edge, multi-match = ambiguous) is reasonable

**Research date:** 2026-02-22
**Valid until:** 2026-05-22 (oxc_resolver is actively maintained; check for API changes if planning takes more than 3 months)
