# Phase 2: Import Resolution & Graph Completion - Context

**Gathered:** 2026-02-22
**Status:** Ready for planning

<domain>
## Phase Boundary

Resolve every import in a TypeScript/JavaScript codebase to its actual defining file and symbol. Handle tsconfig path aliases, barrel file re-exports, and monorepo workspace packages. Build a complete dependency graph with both file-level and symbol-level edges.

</domain>

<decisions>
## Implementation Decisions

### Resolution strategy
- Always chase through re-export chains to the original defining file — never stop at barrel files
- Wildcard re-exports (`export * from './foo'`) resolved lazily at query time, not eagerly at index time — faster indexing
- External packages (node_modules) appear as nodes in the graph (name + version) but their internals are not indexed — imports from externals are terminal edges with package metadata

### Unresolved imports
- Claude's Discretion: choose the best approach for handling unresolvable imports (missing files, dynamic paths, external packages without node_modules)

### Symbol relationships
- 'calls' edges track direct function/method calls only (`foo()`, `obj.method()`) — no callback passing or assignment tracking
- Type references (`const x: SomeType`, function parameter types) treated the same as value references — no separate edge type for type-only relationships
- Full inheritance hierarchy: class extends class, class implements interface, interface extends interface — all captured
- 'contains' relationship tracks all nesting levels: file > function > nested function, class > method > inner function — full containment tree

### Workspace & monorepo scope
- Support all three major package managers: npm, yarn, and pnpm workspace protocols
- Index from a user-specified root — follow cross-package imports into other workspace packages as needed (not automatic full-monorepo indexing)
- Cross-package imports resolve directly to source files, not dist/build output — map package name to its source directory
- Detect turbo.json/nx.json to confirm monorepo structure, but resolve packages from package.json workspaces only — no special Turbo/Nx integration

### tsconfig handling
- Claude's Discretion: handling of multiple tsconfig files, extends chains, and project references — researcher/planner determine best approach

</decisions>

<specifics>
## Specific Ideas

No specific requirements — open to standard approaches

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 02-import-resolution-graph-completion*
*Context gathered: 2026-02-22*
