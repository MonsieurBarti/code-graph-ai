# Pitfalls Research

**Domain:** Code intelligence engine / dependency graph tool
**Researched:** 2026-02-22
**Confidence:** HIGH

## Critical Pitfalls

### Pitfall 1: Barrel File Explosion

**What goes wrong:**
TypeScript projects use `index.ts` barrel files that re-export everything from a module. A single `import { X } from './module'` resolves to `module/index.ts`, which re-exports from 20 files. Naive resolution creates O(n^2) edges and makes every file appear to depend on every other file in the barrel.

**Why it happens:**
Import resolution follows the chain: `import spec → index.ts → actual file`. Without detecting barrel patterns, the graph connects the importing file to ALL files re-exported by the barrel, not just the one symbol it actually uses.

**How to avoid:**
Track which specific symbols are imported, not just which files. `import { UserService } from './services'` should create an edge to the file that defines `UserService`, not to every file the `services/index.ts` re-exports. This requires symbol-level import resolution.

**Warning signs:**
Fan-out metrics: if a single file has >50 import edges, barrel files are likely being over-connected.

**Phase to address:**
Phase 2 (Import Resolution) — must handle barrel files before building the rest of the graph.

---

### Pitfall 2: TypeScript Path Alias Resolution

**What goes wrong:**
TypeScript projects use `tsconfig.json` path aliases (`@/components/*`, `~/utils/*`). The tool resolves imports literally instead of following path mappings, resulting in broken import edges (unresolved imports) or incorrect connections.

**Why it happens:**
Path aliases are defined in `tsconfig.json` under `compilerOptions.paths` with `baseUrl`. They can also be inherited from `extends` in nested tsconfigs (monorepos). Many tools skip inheritance or handle `*` wildcards incorrectly.

**How to avoid:**
1. Parse `tsconfig.json` fully, including `extends` chains
2. Build a path alias resolver that handles wildcards
3. Test against real projects with complex tsconfig setups (Next.js, NX workspaces)

**Warning signs:**
High "unresolved import" count during indexing. If >5% of imports can't be resolved, path alias handling is broken.

**Phase to address:**
Phase 2 (Import Resolution) — path aliases are foundational to correct import graphs.

---

### Pitfall 3: Dynamic Imports and Require Expressions

**What goes wrong:**
`import('./module')` dynamic imports and `require(variable)` expressions can't be statically resolved. The tool either ignores them entirely (missing edges) or tries to resolve them and produces garbage.

**Why it happens:**
Dynamic imports take arbitrary expressions as arguments. `import(\`./pages/${name}\`)` or `require(config.module)` cannot be resolved at parse time without runtime information.

**How to avoid:**
1. Handle the common static case: `import('./module')` with a string literal — this IS resolvable
2. For dynamic expressions: record them as "unresolved dynamic import" nodes with the raw expression
3. Never guess — an honest "I can't resolve this" is better than a wrong edge
4. Report unresolved dynamic imports in stats so the user knows the graph has gaps

**Warning signs:**
Silent drops — if the tool silently ignores dynamic imports without reporting them, the graph appears complete but isn't.

**Phase to address:**
Phase 2 (Parsing) — detection. Phase 3 (Resolution) — static dynamic imports resolved, truly dynamic ones flagged.

---

### Pitfall 4: Memory Leaks in Watch Mode

**What goes wrong:**
The daemon runs for hours/days. Each file change adds new graph nodes but doesn't fully clean up old ones. After a day of active development, memory usage grows from 50MB to 500MB+.

**Why it happens:**
Graph libraries (including petgraph) don't compact memory on node removal. Removing a node leaves a "hole" in the internal storage. Old string allocations for symbol names aren't freed if references exist elsewhere (e.g., in the symbol index).

**How to avoid:**
1. Use string interning (lasso crate) — strings are stored once, referenced by index
2. On file change: remove ALL nodes/edges for that file, then re-add. Don't try to diff.
3. Periodic compaction: every N re-indexes, rebuild the graph from the current source of truth
4. Monitor RSS in tests — add a benchmark that watches memory after 1000 incremental re-indexes

**Warning signs:**
Memory growth that doesn't plateau. Run a soak test: auto-save a file 1000 times, measure RSS.

**Phase to address:**
Phase 4 (Watch Mode) — implement with memory discipline from the start, not as a fix later.

---

### Pitfall 5: MCP Tool Description Token Overhead

**What goes wrong:**
Each MCP tool has a description that Claude reads on every interaction. 6 tools with verbose descriptions = hundreds of tokens consumed before any actual work. Research (arxiv 2602.14878v1) confirms tool descriptions are a significant token sink.

**Why it happens:**
Developers write tool descriptions like documentation — full explanations, examples, parameter details. Claude reads ALL tool descriptions on every turn, even when it won't use most tools.

**How to avoid:**
1. Keep tool descriptions under 100 tokens each
2. Use parameter descriptions instead of putting everything in the tool description
3. Use a single "help" tool that returns detailed docs on demand
4. Benchmark: count tokens consumed by tool metadata vs actual responses

**Warning signs:**
If tool descriptions total >500 tokens, they're too verbose. Measure by serializing the MCP tool list.

**Phase to address:**
Phase 3 (MCP Server) — design tool descriptions with token budget from day one.

---

### Pitfall 6: Incorrect Call Graph from Aliased References

**What goes wrong:**
`const fn = someModule.someFunction; fn()` — the call graph doesn't connect `fn()` to `someFunction` because the alias breaks the reference chain. Similarly, destructured imports: `const { handler } = require('./module')` followed by `handler()`.

**Why it happens:**
Static analysis of call graphs is fundamentally limited. Following aliases requires data-flow analysis, which tree-sitter doesn't provide (it's a parser, not an analyzer).

**How to avoid:**
1. Start with direct calls only: `someFunction()`, `module.someFunction()` — these are reliable
2. Handle destructured imports: `import { x } from` + `x()` → link to x's definition
3. Assign confidence scores to call edges: direct call = HIGH, aliased = MEDIUM, computed = LOW
4. Don't pretend the call graph is complete — document its limitations

**Warning signs:**
False negatives in impact analysis (missed callers). Validate against known call chains in test projects.

**Phase to address:**
Phase 2 (Call graph linking) — set correct expectations, implement confidence scoring.

---

### Pitfall 7: Monorepo Package Resolution

**What goes wrong:**
In monorepos (NX, Turborepo, Lerna), `import { X } from '@my-org/shared'` should resolve to `packages/shared/src/index.ts`, not to `node_modules/@my-org/shared`. The tool resolves to node_modules (or fails entirely) because it doesn't understand workspace package resolution.

**Why it happens:**
Workspace resolution requires reading `package.json` workspaces, `tsconfig.json` references, and potentially `nx.json` or `turbo.json`. Each tool has its own resolution strategy.

**How to avoid:**
1. Read root `package.json` workspaces field
2. Build a workspace package map: package name → local directory
3. Prefer local resolution over node_modules when the package exists in the workspace
4. Test against real monorepo structures

**Warning signs:**
Internal package imports resolving to `node_modules/` instead of local workspace paths.

**Phase to address:**
Phase 2 (Import Resolution) — must handle workspaces for monorepo support.

---

## Technical Debt Patterns

| Shortcut | Immediate Benefit | Long-term Cost | When Acceptable |
|----------|-------------------|----------------|-----------------|
| Skip barrel file resolution | Simpler import resolver | Wrong dependency counts, noisy impact analysis | Never — barrel files are too common in TS |
| Store file paths as Strings instead of interned | Simpler code | 3-10x memory for paths (repeated across nodes) | Early prototype only |
| Full re-index instead of incremental | Simpler watcher logic | Unusable on large codebases (>5s on 10K files) | v0 prototype only |
| Single-threaded parsing | Simpler concurrency | 4-8x slower on multi-core (most machines) | Early phases, parallelize before release |
| Skip tsconfig extends | Simpler config parsing | Breaks monorepos and many real projects | Never after Phase 2 |

## Performance Traps

| Trap | Symptoms | Prevention | When It Breaks |
|------|----------|------------|----------------|
| Parsing all files sequentially | Index takes >10s on 5K files | Use rayon for parallel parsing | >2K files |
| String cloning in graph operations | Memory 3-5x higher than expected | Use string interning (lasso) | >5K files |
| Naive glob for file walking | Walking takes seconds in deep node_modules | Use ignore crate (respects .gitignore) | Any project with node_modules |
| Unbounded BFS in impact analysis | Impact query hangs on circular deps | Depth limit + visited set | Any project with cycles |
| Serializing full graph on every change | Disk I/O stalls watcher | Debounce persistence (save max every 5s) | Active coding with autosave |

## "Looks Done But Isn't" Checklist

- [ ] **Import resolution:** Often missing `tsconfig.paths` support — verify with a real Next.js project
- [ ] **Watch mode:** Often misses file renames and deletions — verify rename triggers re-index
- [ ] **Barrel files:** Often creates too many edges — verify fan-out is reasonable
- [ ] **MCP responses:** Often too verbose — verify token count per response type
- [ ] **Empty project handling:** Often crashes on empty or invalid TS projects — verify graceful degradation
- [ ] **Symlinks:** Often infinite-loops on symlinked directories — verify symlink handling
- [ ] **Large files:** Often OOMs on generated files (>10K lines) — verify file size limit exists
- [ ] **Unicode paths:** Often breaks on non-ASCII file paths — verify with accented characters

## Recovery Strategies

| Pitfall | Recovery Cost | Recovery Steps |
|---------|---------------|----------------|
| Barrel file explosion | MEDIUM | Refactor resolver to track symbol-level imports, re-index |
| Path alias bugs | LOW | Fix tsconfig parser, re-index (graph rebuilds cleanly) |
| Memory leak in watch | HIGH | Requires architecture change if string interning not used from start |
| Wrong call graph edges | LOW | Add confidence scoring, filter low-confidence in queries |
| Monorepo resolution | MEDIUM | Add workspace awareness to resolver, re-index |
| MCP token overhead | LOW | Trim descriptions, no data migration needed |

## Pitfall-to-Phase Mapping

| Pitfall | Prevention Phase | Verification |
|---------|------------------|--------------|
| Barrel file explosion | Phase 2 (Import Resolution) | Fan-out <20 edges per file on test project |
| Path alias resolution | Phase 2 (Import Resolution) | 0 unresolved imports on project with path aliases |
| Dynamic imports | Phase 2 (Parsing + Resolution) | Dynamic imports flagged, static ones resolved |
| Memory leaks | Phase 4 (Watch Mode) | RSS stable after 1000 incremental re-indexes |
| MCP token overhead | Phase 3 (MCP Server) | Tool descriptions <100 tokens each |
| Aliased call references | Phase 2 (Call Graph) | Confidence scores on all call edges |
| Monorepo packages | Phase 2 (Import Resolution) | Workspace packages resolve to local paths |

## Sources

- TypeScript barrel file issues: common pattern in Next.js, NX, and Angular projects
- tsconfig path resolution: TypeScript handbook — Module Resolution
- tree-sitter limitations: tree-sitter.github.io/tree-sitter — explicitly a parser, not a type checker
- MCP tool description overhead: arxiv 2602.14878v1 — "MCP Tool Descriptions Are Smelly"
- petgraph memory behavior: docs.rs/petgraph — StableGraph vs Graph trade-offs
- Axon known limitations: github.com/harshkedia177/axon — Python performance, TS as secondary

---
*Pitfalls research for: code intelligence engine / dependency graph tool*
*Researched: 2026-02-22*
