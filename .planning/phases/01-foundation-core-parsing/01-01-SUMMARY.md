---
phase: 01-foundation-core-parsing
plan: 01
subsystem: cli
tags: [rust, clap, serde, toml, ignore, tree-sitter, petgraph, glob]

# Dependency graph
requires: []
provides:
  - "code-graph binary that compiles and runs"
  - "Clap-based CLI with index subcommand (path, --verbose/-v, --json)"
  - "CodeGraphConfig loading from code-graph.toml with serde/toml"
  - "walk_project function: .ts/.tsx/.js/.jsx discovery, gitignore, node_modules exclusion, monorepo workspace detection"
affects: [01-02, 01-03, all-subsequent-phases]

# Tech tracking
tech-stack:
  added:
    - "tree-sitter 0.26 — AST parsing (used in 01-02)"
    - "tree-sitter-typescript 0.23 — TypeScript grammar"
    - "tree-sitter-javascript 0.25 — JavaScript grammar"
    - "ignore 0.4 — gitignore-aware file walking"
    - "petgraph 0.6 (stable_graph) — dependency graph (used in 01-03)"
    - "clap 4 (derive) — CLI argument parsing"
    - "serde 1 (derive) + serde_json 1 — serialization"
    - "toml 0.8 — config file parsing"
    - "anyhow 1 — error handling"
    - "glob 0.3 — workspace pattern expansion"
  patterns:
    - "anyhow::Result as main return type for error propagation"
    - "Clap derive macros for CLI definition"
    - "serde Deserialize with Default for config structs"
    - "ignore::WalkBuilder with require_git(false) for gitignore in any directory"

key-files:
  created:
    - Cargo.toml
    - src/main.rs
    - src/cli.rs
    - src/config.rs
    - src/walker.rs
  modified: []

key-decisions:
  - "Use require_git(false) on WalkBuilder so .gitignore is respected even in non-git directories"
  - "Walk from project root only (not workspace subdirs separately) to avoid duplicate file discovery"
  - "Hard-exclude node_modules via path component check — not relying on .gitignore"
  - "Verbose output goes to stderr (not stdout) so stdout is clean for piping --json output"

patterns-established:
  - "Verbose/diagnostic output -> stderr; structured output -> stdout"
  - "Config defaults when file missing — never fail on absent optional config"
  - "Warn and continue on parse errors rather than failing the whole operation"

requirements-completed: [PARS-01]

# Metrics
duration: 4min
completed: 2026-02-22
---

# Phase 1 Plan 01: Project Scaffold, CLI, Config, and File Walker Summary

**Rust binary with Clap CLI, serde/toml config loading, and gitignore-aware .ts/.tsx/.js/.jsx file walker with node_modules exclusion and monorepo workspace detection**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-22T13:13:53Z
- **Completed:** 2026-02-22T13:17:00Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments

- Scaffolded Cargo project with all Phase 1 dependencies (tree-sitter, petgraph, clap, serde, ignore, glob, anyhow, toml)
- Implemented Clap derive CLI with `index` subcommand supporting positional `path`, `-v/--verbose`, and `--json` flags
- Implemented `CodeGraphConfig` loading `code-graph.toml` with serde/toml, defaults when absent
- Implemented `walk_project` using `ignore::WalkBuilder` with standard gitignore rules, hard node_modules exclusion, config-based exclusions, and monorepo workspace detection from `package.json`

## Task Commits

Each task was committed atomically:

1. **Task 1: Scaffold Cargo project with CLI and config** - `e09cc5b` (feat)
2. **Task 2: Implement file walker with gitignore, node_modules exclusion, monorepo detection** - `12e0a0e` (feat)

**Plan metadata:** (docs commit — see below)

## Files Created/Modified

- `Cargo.toml` — Project manifest with all Phase 1 dependencies
- `src/main.rs` — Entry point: parse CLI, load config, run walker, output results
- `src/cli.rs` — Clap derive structs: `Cli` (Parser) and `Commands::Index` with path/verbose/json
- `src/config.rs` — `CodeGraphConfig` struct with `exclude` field; `load()` reads code-graph.toml
- `src/walker.rs` — `walk_project()`, `detect_workspace_roots()`, monorepo package.json parsing, gitignore-aware walking

## Decisions Made

- Used `require_git(false)` on `WalkBuilder` so `.gitignore` files are respected even in non-git directories (important for testing and non-git projects)
- Walk from project root once rather than walking workspace subdirs separately — avoids duplicate file reporting since workspace dirs are always subdirs of root
- Hard-exclude `node_modules` by checking path components, not relying on `.gitignore` entry
- Verbose output to `stderr`, structured/JSON to `stdout` — clean for shell piping

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed duplicate file discovery in monorepo projects**
- **Found during:** Task 2 (file walker verification)
- **Issue:** Initial implementation walked each workspace dir separately in addition to root walk, causing files to appear twice in the result
- **Fix:** Simplified to walk only from project root (which already covers all workspace subdirs); `detect_workspace_roots` preserved for future plan use
- **Files modified:** `src/walker.rs`
- **Verification:** Monorepo test with `packages/*` workspaces reports correct unique file count
- **Committed in:** `12e0a0e` (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (Rule 1 - bug)
**Impact on plan:** Fix required for correct file count. No scope creep.

## Issues Encountered

- `ignore` crate's `standard_filters(true)` only reads `.gitignore` inside git repositories by default. Fixed by adding `.require_git(false)` to `WalkBuilder` so gitignore rules apply universally.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Binary builds and CLI is fully operational
- File discovery is correct and verified against all success criteria
- All Phase 1 dependencies are in Cargo.toml — ready for Plan 02 (tree-sitter parsing) and Plan 03 (graph construction)
- No blockers

---
*Phase: 01-foundation-core-parsing*
*Completed: 2026-02-22*

## Self-Check: PASSED

All artifacts verified:
- FOUND: Cargo.toml
- FOUND: src/main.rs
- FOUND: src/cli.rs
- FOUND: src/config.rs
- FOUND: src/walker.rs
- FOUND: .planning/phases/01-foundation-core-parsing/01-01-SUMMARY.md
- FOUND commit e09cc5b (Task 1: scaffold)
- FOUND commit 12e0a0e (Task 2: walker)
- FOUND commit 7a33e21 (docs: metadata)
