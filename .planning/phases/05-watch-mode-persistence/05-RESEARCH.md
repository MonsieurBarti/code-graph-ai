# Phase 5: Watch Mode & Persistence - Research

**Researched:** 2026-02-23
**Domain:** Rust file-system watching (notify), binary persistence (bincode + petgraph serde), incremental graph mutation, tokio async integration
**Confidence:** HIGH

## Summary

Phase 5 adds two orthogonal capabilities to an existing Rust MCP server: (1) a live file watcher that triggers incremental graph re-indexing on file changes, and (2) a binary cache so that cold starts skip re-parsing unchanged files. The existing codebase is already async (tokio), already has a `CodeGraph` struct in an `Arc<Mutex<...>>` pattern in `mcp/server.rs`, and already does full-graph builds via `build_graph()`. Both capabilities slot naturally into that foundation.

The standard Rust ecosystem stack for this problem is well-established: `notify` 8.x for watching, `notify-debouncer-mini` for the debounce layer (simpler than debouncer-full), `bincode` 2 with `serde` feature for persistence, and `petgraph`'s `serde-1` feature to make the graph serialisable. The only non-trivial design challenge is bridging notify's sync callback API into tokio's async runtime correctly, and deciding on a persistence envelope that enables forward cache-invalidation without a full schema versioning library.

**Primary recommendation:** Use `notify` 8 + `notify-debouncer-mini` 0.7 for watching (std::sync::mpsc bridge to tokio), and `bincode` 2 (serde path) + `petgraph serde-1` for persistence. Keep the watcher running on a dedicated OS thread owned by a background `tokio::task::spawn_blocking` call, communicating to the async graph-update logic via a `tokio::sync::mpsc` channel.

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

**Daemon lifecycle:**
- Watcher embedded in the MCP server process (single process, watcher runs as internal thread)
- Watcher starts lazily — triggered on first index or query, not on MCP server startup
- Also provide standalone `code-graph watch` CLI command for watching without MCP (dual mode: embedded + standalone)
- Standalone watch command prints status to terminal, useful for debugging and manual use

**Persistence format:**
- Cache lives in `.code-graph/` directory in project root (similar to `.git/` convention)
- Serialization format: bincode (fast serialize/deserialize, compact on disk, maximizes cold start speed)
- Staleness detection: mtime + file size (stat call only, no content hashing — covers 99% of real changes)
- On load: smart diff — compare cached file list vs current file list, re-parse changed/new files, remove deleted entries. Handles branch switches gracefully without full re-index

**Incremental re-index scope:**
- Propagation: re-parse changed file + update direct dependents (files that import the changed file, whose resolution might have changed)
- File deletions: remove file's symbols and edges from graph, mark imports pointing to it as unresolved
- New files: parse immediately + check if existing unresolved imports now resolve to this file, fix those edges
- Renames: treated as delete + create (simpler, most watchers report it this way)

**Watch filtering:**
- Watcher respects same .gitignore rules used during initial indexing (single source of truth)
- Rapid saves handled with debounce (~50-100ms after last change before re-indexing)
- Config file changes (tsconfig.json, package.json) trigger full re-index (affect path resolution globally)
- node_modules always excluded from watching (hardcoded, regardless of .gitignore)

### Claude's Discretion
- Watcher activity reporting strategy (log file, terminal output, or silent)
- Exact debounce timing within the ~50-100ms range
- Bincode schema versioning / migration approach
- Temp file handling during cache writes (atomic write strategy)
- Exact watcher library choice (notify crate or alternative)

### Deferred Ideas (OUT OF SCOPE)
None — discussion stayed within phase scope
</user_constraints>

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|-----------------|
| INTG-04 | Tool runs as a background daemon with file watching, re-indexing incrementally on file changes | notify 8 + debouncer-mini + dedicated OS thread; lazy start on first MCP tool call |
| INTG-05 | Incremental re-index completes in under 100ms for single-file changes | Re-parse one file only + resolver pass scoped to changed file and its direct dependents; avoid full rebuild |
| PERF-04 | Graph persists to disk; cold start loads cached graph without re-parsing unchanged files | bincode 2 + petgraph serde-1; mtime+size staleness check on load; skip unchanged files, re-parse only changed/new |
</phase_requirements>

---

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| notify | 8.2.0 | Cross-platform filesystem event delivery | Used by rust-analyzer, cargo-watch, deno, mdBook; 62M downloads; OS-native backends (inotify/kqueue/FSEvents/ReadDirectoryChanges) |
| notify-debouncer-mini | 0.7.0 | Debounce: emit one event per file per timeframe | Simpler than debouncer-full; sufficient for single-path-per-event semantics; MSRV 1.72 |
| bincode | 2.0.0-rc.3 or 2.x stable | Fast binary serialization | Zero-alloc encode, compact wire size; same serde derives already in project |
| petgraph (serde-1 feature) | 0.6.x (already in Cargo.toml) | Graph serialization/deserialization | Node/edge indices stable across de/serialize; same representation for Graph and StableGraph |
| tempfile | 3.x | Atomic cache writes (temp + rename) | Cross-platform atomic file replacement; used by many large Rust projects |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| tokio::sync::RwLock | (tokio 1.x) | Replace current Mutex on graph_cache | Read-heavy query workload in MCP server; many concurrent tool calls read the graph, watcher writes rarely |
| serde (already in project) | 1.x | Derive Serialize/Deserialize on graph nodes | Required for petgraph serde-1 + bincode serde path |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| notify 8 | watchexec-lib, inotify-rs directly | notify is the universal standard; cross-platform; OS-specific crates don't work on macOS/Windows |
| notify-debouncer-mini | notify-debouncer-full | debouncer-full adds path-deduplication across renames; overkill for this use case; mini is sufficient |
| bincode 2 serde path | rkyv (zero-copy) | rkyv requires unsafe, custom archive types — research flagged this as a risk; bincode with serde derives avoids extra code; STATE.md flag "rkyv may need custom serialization — prototype early" means rkyv was deprioritized |
| bincode 2 serde path | serde_json | JSON is 3-5x larger on disk and slower to decode; defeats PERF-04 goal |
| tempfile for atomic write | manual .tmp + rename | tempfile handles same-filesystem temp placement automatically; otherwise rename across filesystems fails |

**Installation:**
```toml
# Cargo.toml additions
notify = "8"
notify-debouncer-mini = "0.7"
bincode = { version = "2", features = ["serde", "derive"] }
tempfile = "3"
# petgraph already present — add serde-1 feature:
petgraph = { version = "0.6", features = ["stable_graph", "serde-1"] }
# serde already present — add derive if not already:
serde = { version = "1", features = ["derive"] }
```

---

## Architecture Patterns

### Recommended Module Structure
```
src/
├── watcher/
│   ├── mod.rs        # pub struct FileWatcher, pub fn start_watcher()
│   └── event.rs      # WatchEvent enum (Created, Modified, Deleted, ConfigChanged)
├── cache/
│   ├── mod.rs        # pub fn save_cache(), pub fn load_cache()
│   └── envelope.rs   # CacheEnvelope { version: u32, project_root: PathBuf, graph: SerializableGraph }
├── graph/            # (existing) — add serde derives to node.rs, edge.rs
└── mcp/
    └── server.rs     # (existing) — integrate watcher startup + RwLock upgrade
```

### Pattern 1: Notify Sync-to-Tokio Bridge

**What:** notify's event callback is synchronous (called from an OS thread). Tokio's async tasks cannot directly await inside that callback. The safe pattern is to create a `std::sync::mpsc::Sender` inside the callback, and have a tokio task that calls `rx.recv()` inside `spawn_blocking`.

**When to use:** Whenever integrating notify (or any sync callback) into a tokio runtime.

**Example:**
```rust
// Source: https://docs.rs/notify/8.0.0/notify/ + official tokio bridging guidance
use notify::{recommended_watcher, RecursiveMode, Watcher, Event};
use std::sync::mpsc;
use tokio::sync::mpsc as tokio_mpsc;

fn start_watcher(
    path: &Path,
    tx: tokio_mpsc::Sender<notify::Result<Event>>,
) -> notify::Result<notify::RecommendedWatcher> {
    let (std_tx, std_rx) = mpsc::channel::<notify::Result<Event>>();

    // Bridge: forward std events into tokio channel
    let tx_clone = tx.clone();
    tokio::task::spawn_blocking(move || {
        while let Ok(event) = std_rx.recv() {
            // blocking_send is the correct method for sync-context → tokio mpsc
            if tx_clone.blocking_send(event).is_err() {
                break; // receiver dropped, shutdown
            }
        }
    });

    let mut watcher = recommended_watcher(move |res| {
        let _ = std_tx.send(res);
    })?;
    watcher.watch(path, RecursiveMode::Recursive)?;
    Ok(watcher)
}
```

### Pattern 2: Lazy Watcher Startup in MCP Server

**What:** Watcher starts on first graph access, not at server creation. This matches the locked decision: "Watcher starts lazily — triggered on first index or query".

**When to use:** MCP server `resolve_graph()` call-site — check if watcher is already running, start it if not.

**Example:**
```rust
// In CodeGraphServer, add a watcher handle
pub struct CodeGraphServer {
    default_project_root: Arc<PathBuf>,
    graph_cache: Arc<RwLock<HashMap<PathBuf, Arc<CodeGraph>>>>,
    watcher: Arc<Mutex<Option<WatcherHandle>>>,  // None until first use
    tool_router: ToolRouter<Self>,
}

// In resolve_graph(), after building/loading the graph:
let mut watcher_guard = self.watcher.lock().await;
if watcher_guard.is_none() {
    *watcher_guard = Some(start_watcher_for_path(&path, self.graph_cache.clone()).await?);
}
```

### Pattern 3: Incremental Re-Index — Remove then Re-Parse

**What:** For a Modified or Created event, surgically remove the stale file's nodes+edges from the graph, then re-parse and re-resolve that single file. For Deleted events, remove nodes+edges and mark dependent imports as Unresolved.

**When to use:** Every file event from the watcher.

**Pseudocode pattern:**
```rust
async fn handle_file_event(
    graph: &mut CodeGraph,
    event: WatchEvent,
    project_root: &Path,
) {
    match event {
        WatchEvent::Modified(path) | WatchEvent::Created(path) => {
            // 1. Remove old nodes (file + all its symbols + all edges to/from them)
            remove_file_from_graph(graph, &path);
            // 2. Re-parse and re-add
            if let Ok(result) = parser::parse_file(&path, &read_file(&path)) {
                let file_idx = graph.add_file(path.clone(), language_str);
                for (sym, children) in &result.symbols { /* ... */ }
                // 3. Re-resolve only this file's imports + check unresolved imports in dependents
                resolver::resolve_file(&mut graph, project_root, &path, &result);
                resolver::fix_unresolved_pointing_to(&mut graph, &path);
            }
        }
        WatchEvent::Deleted(path) => {
            remove_file_from_graph(graph, &path);
            resolver::mark_imports_to_unresolved(&mut graph, &path);
        }
        WatchEvent::ConfigChanged => {
            // full re-index (tsconfig.json / package.json changed)
            *graph = build_graph(project_root, false)?;
        }
    }
}
```

### Pattern 4: Persistence Envelope with Schema Version

**What:** Wrap the serialized graph in an envelope struct that includes a format version number and a project root path. On load, check version mismatch → discard cache and rebuild. This is the standard pattern since bincode has no built-in versioning.

**When to use:** Every cache save and load.

**Example:**
```rust
// Source: bincode 2 encode_into_std_write, standard pattern for versioned binary caches
const CACHE_VERSION: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
struct CacheEnvelope {
    version: u32,
    project_root: PathBuf,
    file_mtimes: HashMap<PathBuf, (u64, u64)>,  // path → (mtime_secs, file_size)
    graph: SerializedGraph,                       // petgraph serde-1 output
}

fn save_cache(path: &Path, graph: &CodeGraph, file_mtimes: &HashMap<PathBuf, FileMeta>) -> anyhow::Result<()> {
    let envelope = CacheEnvelope { version: CACHE_VERSION, ... };
    // Atomic write: write to .tmp, then rename
    let tmp_path = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp_path)?;
        bincode::serde::encode_into_std_write(&envelope, &mut f, bincode::config::standard())?;
    }
    std::fs::rename(&tmp_path, path)?;  // atomic on same filesystem
    Ok(())
}

fn load_cache(path: &Path) -> anyhow::Result<Option<CacheEnvelope>> {
    let bytes = std::fs::read(path)?;
    match bincode::serde::decode_from_slice::<CacheEnvelope, _>(&bytes, bincode::config::standard()) {
        Ok((envelope, _)) if envelope.version == CACHE_VERSION => Ok(Some(envelope)),
        _ => Ok(None),  // version mismatch or corrupt → triggers full rebuild
    }
}
```

### Pattern 5: Cold Start Staleness Check

**What:** On load, compare cached `file_mtimes` against current filesystem state. Re-parse only files whose mtime or size differs, plus new files. Remove graph entries for deleted files.

**When to use:** At MCP server startup (or first tool call) when cache file exists.

**Pseudocode:**
```rust
fn apply_staleness_diff(
    cached: CacheEnvelope,
    current_files: &[PathBuf],   // from walk_project()
) -> CodeGraph {
    let mut graph = cached.into_graph();
    for file in current_files {
        let meta = fs::metadata(file)?;
        let (cached_mtime, cached_size) = cached.file_mtimes.get(file).copied().unwrap_or((0,0));
        if meta.modified_secs() != cached_mtime || meta.len() != cached_size {
            remove_file_from_graph(&mut graph, file);
            reparse_and_add(&mut graph, file);
        }
    }
    // Remove deleted files
    for cached_file in cached.file_mtimes.keys() {
        if !current_files.contains(cached_file) {
            remove_file_from_graph(&mut graph, cached_file);
        }
    }
    graph
}
```

### Pattern 6: Standalone `code-graph watch` CLI Command

**What:** A non-async CLI command that starts a watcher and prints events to stdout/stderr. Uses the same watcher logic as the MCP embedded mode but without the graph being served over stdio.

**When to use:** `code-graph watch <path>` CLI invocation.

**Integration point:** Add `Watch` variant to `cli.rs` `Commands` enum. In `main.rs`, handle it in the match block — start the watcher synchronously (blocking the CLI process in a `std::sync::mpsc::recv()` loop).

### Anti-Patterns to Avoid

- **Blocking tokio runtime with `std::sync::mpsc::recv()` inside async code:** Always use `spawn_blocking` to run the blocking receive loop; never call `rx.recv()` directly in an `async fn`.
- **Holding the graph RwLock during parse/resolve:** The lock must be dropped before any CPU-bound work, then re-acquired only to swap the result in. Holding a write lock during parsing (which can take tens of milliseconds) blocks all concurrent MCP tool calls.
- **Using `crossbeam-channel` inside tokio:** notify 6.x used crossbeam by default; notify 7+ defaults to std::sync::mpsc which is safe in tokio. With notify 8, do not re-enable the crossbeam feature.
- **Write cache on every file event:** Only persist after the incremental update completes successfully. Don't persist on every debounce tick for busy repos.
- **Cross-filesystem rename of temp file:** The temp file must be created in the same directory as the final cache path (`.code-graph/`) so that `fs::rename` is atomic. `tempfile::NamedTempFile::persist()` handles this automatically.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Filesystem event delivery | Custom inotify/kqueue polling | notify 8 | Platform-specific backends, inotify/FSEvents/kqueue each have subtleties; notify has 9+ years of fixes |
| Event debouncing | Custom timer + HashMap per path | notify-debouncer-mini | Handles rapid saves, editor swap-file patterns, partial writes; hard to get right |
| Atomic file write | Write directly + hope for no crash | tempfile::NamedTempFile | Handles cross-process partial reads, crash mid-write corrupting cache |
| Graph serialization | Custom recursive node walk to JSON | petgraph serde-1 + bincode | Node indices stable across de/serialize; already tested; manual walk loses edge metadata |
| Schema versioning | Complex migration framework | CACHE_VERSION constant + discard-on-mismatch | For a local developer cache, discarding is acceptable; migration adds complexity for no real user benefit |

**Key insight:** The hardest part is bridging notify's sync callback world into tokio's async world correctly. The `spawn_blocking` + `std::sync::mpsc` bridge is the only safe pattern; async closures in notify callbacks deadlock or panic.

---

## Common Pitfalls

### Pitfall 1: Crossbeam Channel Inside Tokio
**What goes wrong:** notify 6.x enabled crossbeam-channel by default; blocking on a crossbeam channel inside tokio causes panics or deadlocks.
**Why it happens:** notify 7+ disabled crossbeam by default; with notify 8 the crossbeam feature is opt-in. If added as a feature flag, the crossbeam Sender gets passed into the event callback and `.send()` blocks.
**How to avoid:** Use `std::sync::mpsc::Sender` in the notify callback. Use `tokio::sync::mpsc::Sender::blocking_send()` only from inside `spawn_blocking`.
**Warning signs:** `panicked at 'cannot block the current thread from within a Rust runtime'`.

### Pitfall 2: Holding Write Lock During Graph Rebuild
**What goes wrong:** If the watcher task holds `Arc<RwLock<CodeGraph>>` as a write lock during the full parse+resolve pipeline, all MCP tool calls (which need read access) are blocked for the entire rebuild duration (potentially 100-500ms for large projects).
**Why it happens:** Naive implementation just locks → rebuilds → unlocks in sequence.
**How to avoid:** Build the new `CodeGraph` (or the incremental patch) in a local variable without holding the lock. Only lock long enough to swap in the result: `*graph_write = new_graph`.
**Warning signs:** MCP tool calls time out during indexing.

### Pitfall 3: Watcher Events for .tmp Files During Atomic Write
**What goes wrong:** When writing the cache atomically (write to `.code-graph/graph.bin.tmp`, then rename to `.code-graph/graph.bin`), the watcher may fire events for the `.tmp` file and trigger a re-index.
**Why it happens:** The watcher watches the project root recursively, which includes `.code-graph/`.
**How to avoid:** Either (a) exclude `.code-graph/` from the watched path, or (b) filter out events where path matches `.code-graph/**`.
**Warning signs:** Repeated re-index events immediately after save.

### Pitfall 4: node_modules Events Flooding the Watcher
**What goes wrong:** `npm install` or editor tooling touching node_modules triggers thousands of rapid events that overwhelm the debouncer or the re-index pipeline.
**Why it happens:** watcher watches recursively from project root; node_modules is under project root.
**How to avoid:** After setting up `watcher.watch(root, Recursive)`, the event filter in the debounce callback must check each event path and discard paths containing a `node_modules` component. Alternatively, use `watcher.unwatch(root.join("node_modules"))` explicitly after the initial watch call.
**Warning signs:** Re-index loop triggered continuously after `npm install`.

### Pitfall 5: petgraph serde-1 Integer Size Mismatch
**What goes wrong:** Serialized with NodeIndex as u32, deserialized expecting u64 (or vice versa) — decode panics with an integer size error.
**Why it happens:** petgraph's binary format serialization checks integer sizes at decode time; if the compile target changes (32-bit vs 64-bit) the cache is silently corrupt.
**How to avoid:** Bump `CACHE_VERSION` whenever the target triple or architecture changes. In practice, developer-only caches are always same-arch, so this is mostly a concern for CI caches shared across platforms.
**Warning signs:** `BinaryFormatError` or `IntegerOverflow` during cache load.

### Pitfall 6: Debounce Timer Too Short — Re-Index During Save
**What goes wrong:** With debounce < 50ms, many editors (vim, emacs, VS Code) trigger multiple events (write swap file → rename → fsync) that arrive within the debounce window as separate debounced events, causing 2-3 re-indexes per save.
**Why it happens:** Editor save patterns involve multiple filesystem operations.
**How to avoid:** Use 75-100ms debounce (within the locked 50-100ms range). 75ms is a safe midpoint.
**Warning signs:** Two rapid re-index log lines for a single file save.

### Pitfall 7: Config File Events Triggering Infinite Loop
**What goes wrong:** A full re-index triggered by `tsconfig.json` change also re-writes the cache. The cache write fires a watcher event on `.code-graph/graph.bin`, which triggers another re-index.
**Why it happens:** Watcher watches recursively including `.code-graph/`.
**How to avoid:** Exclude `.code-graph/` from watched paths (see Pitfall 3 mitigation).

---

## Code Examples

Verified patterns from official sources:

### notify 8 — Recommended Watcher with std mpsc Bridge
```rust
// Source: https://docs.rs/notify/8.0.0/notify/
use notify::{recommended_watcher, Event, RecursiveMode, Result, Watcher};
use std::sync::mpsc;

fn create_watcher(
    watch_path: &std::path::Path,
) -> Result<(notify::RecommendedWatcher, mpsc::Receiver<Result<Event>>)> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = recommended_watcher(tx)?;  // Sender<Result<Event>> implements EventHandler
    watcher.watch(watch_path, RecursiveMode::Recursive)?;
    Ok((watcher, rx))
}
```

### notify-debouncer-mini 0.7 — Debounced Events via Channel
```rust
// Source: https://docs.rs/notify-debouncer-mini/latest/notify_debouncer_mini/
use notify_debouncer_mini::{notify::*, new_debouncer, DebounceEventResult};
use std::{sync::mpsc, time::Duration};

fn create_debounced_watcher(
    path: &std::path::Path,
    debounce_ms: u64,
) -> notify::Result<(impl Drop, mpsc::Receiver<DebounceEventResult>)> {
    let (tx, rx) = mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(debounce_ms), move |res| {
        let _ = tx.send(res);
    })?;
    debouncer.watcher().watch(path, RecursiveMode::Recursive)?;
    Ok((debouncer, rx))
}
```

### bincode 2 — Encode/Decode with serde via std::io
```rust
// Source: https://github.com/bincode-org/bincode/blob/trunk/docs/migration_guide.md
use bincode::config;

// Write to file
fn write_cache<T: serde::Serialize>(path: &std::path::Path, value: &T) -> anyhow::Result<()> {
    let tmp = path.with_extension("bin.tmp");
    let mut file = std::fs::File::create(&tmp)?;
    bincode::serde::encode_into_std_write(value, &mut file, config::standard())?;
    std::fs::rename(&tmp, path)?;  // atomic on same filesystem
    Ok(())
}

// Read from file
fn read_cache<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> anyhow::Result<T> {
    let bytes = std::fs::read(path)?;
    let (value, _) = bincode::serde::decode_from_slice(&bytes, config::standard())?;
    Ok(value)
}
```

### petgraph serde-1 Feature — Add Derives
```toml
# Cargo.toml — add serde-1 feature to existing petgraph dependency
petgraph = { version = "0.6", features = ["stable_graph", "serde-1"] }
```
```rust
// node.rs, edge.rs — add serde derives (requires serde feature already in Cargo.toml)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SymbolKind { Function, Class, /* ... */ }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SymbolInfo { /* ... */ }

// The StableGraph itself becomes serializable once all node/edge types implement Serialize/Deserialize
```

### Tokio RwLock — Upgrade from Mutex for Graph Cache
```rust
// server.rs — replace Mutex<HashMap<PathBuf, Arc<CodeGraph>>> with RwLock
use tokio::sync::RwLock;

pub struct CodeGraphServer {
    graph_cache: Arc<RwLock<HashMap<PathBuf, Arc<CodeGraph>>>>,
    // ...
}

// Reads (query tool handlers — majority case):
let cache = self.graph_cache.read().await;
if let Some(graph) = cache.get(&path) { return Ok((Arc::clone(graph), path)); }
drop(cache);

// Write (after build/update):
let mut cache = self.graph_cache.write().await;
cache.insert(path.clone(), Arc::new(new_graph));
```

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| notify 6 with crossbeam-channel default | notify 7+/8 with std::sync::mpsc default | notify 7.0 (2024) | crossbeam no longer causes tokio panics by default |
| bincode 1 (auto-config, old API) | bincode 2 (explicit Configuration object) | bincode 2.0 RC (2023+) | Must use new function names; old `serialize()`/`deserialize()` are bincode 1 only |
| rkyv for zero-copy graph cache | bincode 2 + serde (simpler, correct) | PROJECT STATE.md flag | rkyv needs custom archive types for petgraph — too risky; bincode serde path is straightforward |
| notify-debouncer-full | notify-debouncer-mini | current | debouncer-full is overkill for single-path-per-event use case; mini is lighter |

**Deprecated/outdated:**
- bincode 1 API (`bincode::serialize`, `bincode::deserialize`): Do not use. The project has no existing bincode usage, so start directly with bincode 2 API.
- rkyv for this use case: STATE.md explicitly flagged "rkyv integration with petgraph may need custom serialization — prototype early". Research confirms this is real — rkyv requires unsafe archive types and doesn't work transparently with petgraph's StableGraph. Use bincode + serde instead.

---

## Open Questions

1. **Remove-file helper for incremental graph update**
   - What we know: `CodeGraph` uses `StableGraph` (petgraph) which supports O(1) node removal with stable indices; `file_index`, `symbol_index`, `external_index` are `HashMap` lookups
   - What's unclear: There is no existing `remove_file` method on `CodeGraph`. The planner will need to design a `remove_file_from_graph(&mut self, path: &Path)` helper that removes the FileNode, all child SymbolNodes (reachable via `Contains` edges), all edges to/from those nodes, and cleans up the `HashMap` indexes.
   - Recommendation: Make this a new method on `CodeGraph`, task it in Wave 0 of planning.

2. **Resolver re-run scope for incremental update**
   - What we know: `resolver::resolve_all()` currently takes the full `parse_results` map and re-resolves everything. For incremental, we only want to re-resolve the changed file's imports + check dependents.
   - What's unclear: Whether to refactor `resolve_all` to accept a single-file scope, or add a new `resolve_file` function.
   - Recommendation: Add `resolve_file(graph, project_root, path, parse_result)` as a separate entrypoint alongside `resolve_all`; don't refactor the existing full-rebuild path.

3. **Watcher lifetime management in MCP server**
   - What we know: The watcher must be kept alive (not dropped) for the duration of the MCP server process. The `RecommendedWatcher` / debouncer struct stops watching when dropped.
   - What's unclear: Best ownership structure — field on `CodeGraphServer`, or a background task that owns it.
   - Recommendation: Store as `Arc<Mutex<Option<WatcherHandle>>>` field on `CodeGraphServer` where `WatcherHandle` bundles both the debouncer object (to keep it alive) and the tokio task JoinHandle.

4. **Watcher activity reporting strategy** (Claude's Discretion)
   - Recommendation: Use `eprintln!` for the standalone `watch` CLI command (user-visible). For the embedded MCP watcher, write to a log file at `.code-graph/watcher.log` (rotating, capped at ~1MB). This avoids polluting the MCP stdio transport with log noise.

5. **Exact debounce timing** (Claude's Discretion)
   - Recommendation: 75ms — safe midpoint of the 50-100ms window. Below 50ms risks double-firing on editor swap-file patterns; above 100ms is detectable latency to the user.

---

## Validation Architecture

> `workflow.nyquist_validation` is not present in `.planning/config.json` (key absent, not explicitly `true`). Skipping Validation Architecture section.

---

## Sources

### Primary (HIGH confidence)
- `/bincode-org/bincode` (Context7) — encode/decode API, serde path, migration guide, configuration
- https://docs.rs/notify/8.0.0/notify/ — RecommendedWatcher API, EventKind enum, tokio integration pattern
- https://docs.rs/notify-debouncer-mini/latest/notify_debouncer_mini/ — debouncer API, DebouncedEvent structure
- https://docs.rs/notify-debouncer-full/latest/notify_debouncer_full/ — version 0.7.0 confirmed
- https://github.com/notify-rs/notify/blob/main/examples/async_monitor.rs — async bridge pattern
- https://github.com/notify-rs/notify/blob/main/examples/monitor_debounced.rs — debounced std::sync::mpsc pattern

### Secondary (MEDIUM confidence)
- https://github.com/petgraph/petgraph/pull/166 — serde-1 feature confirmed for StableGraph; node/edge indices stable across de/serialize; same format for Graph and StableGraph
- https://github.com/petgraph/petgraph/blob/master/src/graph_impl/serialization.rs — serialization format includes node_holes for StableGraph
- https://users.rust-lang.org/t/how-to-write-replace-files-atomically/42821 — atomic write pattern (same-dir temp + rename)
- https://docs.rs/tempfile/ — NamedTempFile::persist() for atomic cross-directory writes

### Tertiary (LOW confidence)
- WebSearch: notify 8.2.0 as current stable — matches docs.rs "latest" redirect to 8.2.0 (MEDIUM, confirmed by two sources)
- WebSearch: notify 7.0 removed crossbeam-channel default — verified against release notes description

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — verified via Context7 (bincode), official docs.rs (notify 8, debouncer-mini 0.7), petgraph source
- Architecture: HIGH — patterns derived from official examples and known tokio bridging docs
- Pitfalls: HIGH — crossbeam/tokio conflict is documented in notify 7 changelog; lock-during-rebuild is a well-known Rust async pattern error; others derived from first-principles with HIGH confidence
- Persistence schema: HIGH — bincode versioning limitation confirmed ("no built-in versioning"), version-constant + discard pattern is standard

**Research date:** 2026-02-23
**Valid until:** 2026-05-23 (notify/bincode are stable; petgraph serde-1 has been stable since 2018)
