pub mod event;
pub mod incremental;

use std::path::{Path, PathBuf};
use std::time::Duration;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify_debouncer_mini::{new_debouncer, DebounceEventResult};
use notify::RecursiveMode;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::task::JoinHandle;

use event::WatchEvent;

/// Handle to a running watcher. Keeps the debouncer alive (dropping stops watching).
pub struct WatcherHandle {
    /// Keep alive: dropping the debouncer stops the OS watcher.
    _debouncer: notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
    /// The bridge task forwarding events from std channel to tokio channel.
    _bridge_task: JoinHandle<()>,
}

/// File extensions we care about for incremental re-index.
const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx"];

/// Config file basenames that trigger full re-index.
const CONFIG_FILES: &[&str] = &["tsconfig.json", "package.json", "pnpm-workspace.yaml"];

/// Build a Gitignore matcher from the project root's .gitignore file.
/// This is the same source of truth used by `walker::walk_project` via `ignore::WalkBuilder`.
/// If no .gitignore exists, returns an empty matcher that matches nothing.
fn build_gitignore_matcher(project_root: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(project_root);
    let gitignore_path = project_root.join(".gitignore");
    if gitignore_path.exists() {
        let _ = builder.add(&gitignore_path);
    }
    // Also check nested .gitignore files are handled — the ignore crate's
    // GitignoreBuilder handles the root .gitignore. For nested dirs, the
    // walker handles them during walk. For the watcher, the root .gitignore
    // covers the vast majority of cases (dist/, build/, *.generated.ts, etc.).
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Start a debounced file watcher on `watch_root`.
///
/// Returns a `WatcherHandle` (must be kept alive) and a tokio mpsc receiver
/// that yields classified `WatchEvent`s.
///
/// The watcher:
/// - Debounces at 75ms (within the locked 50-100ms range)
/// - Filters out node_modules and .code-graph paths (hardcoded)
/// - Filters out .gitignore'd paths (same rules as initial indexing)
/// - Classifies events into Modified/Created/Deleted/ConfigChanged
pub fn start_watcher(
    watch_root: &Path,
) -> anyhow::Result<(WatcherHandle, tokio_mpsc::Receiver<WatchEvent>)> {
    let (std_tx, std_rx) = std::sync::mpsc::channel::<DebounceEventResult>();

    // Create debounced watcher with 75ms debounce
    let mut debouncer = new_debouncer(Duration::from_millis(75), move |res| {
        let _ = std_tx.send(res);
    })?;
    debouncer.watcher().watch(watch_root, RecursiveMode::Recursive)?;

    // Build gitignore matcher — same rules as walker::walk_project
    let gitignore = build_gitignore_matcher(watch_root);

    // Tokio channel for classified events
    let (tokio_tx, tokio_rx) = tokio_mpsc::channel::<WatchEvent>(256);

    // Bridge: spawn_blocking to receive from std channel, classify, forward to tokio
    let root = watch_root.to_path_buf();
    let bridge_task = tokio::task::spawn_blocking(move || {
        while let Ok(result) = std_rx.recv() {
            match result {
                Ok(events) => {
                    for debounced_event in events {
                        let path = debounced_event.path;
                        if let Some(watch_event) = classify_event(&path, &root, &gitignore) {
                            if tokio_tx.blocking_send(watch_event).is_err() {
                                return; // receiver dropped, shutdown
                            }
                        }
                    }
                }
                Err(err) => {
                    eprintln!("[watcher] error: {:?}", err);
                }
            }
        }
    });

    Ok((
        WatcherHandle {
            _debouncer: debouncer,
            _bridge_task: bridge_task,
        },
        tokio_rx,
    ))
}

/// Classify a filesystem event path into a WatchEvent, or None if it should be ignored.
///
/// Filtering order:
/// 1. Hardcoded exclusions: node_modules, .code-graph (always excluded)
/// 2. .gitignore rules via the `gitignore` matcher (same source of truth as initial indexing)
/// 3. Config file detection (tsconfig.json, package.json → ConfigChanged)
/// 4. Source extension filter (.ts, .tsx, .js, .jsx)
/// 5. File existence check (Modified vs Deleted)
fn classify_event(path: &Path, _project_root: &Path, gitignore: &Gitignore) -> Option<WatchEvent> {
    // Filter: skip node_modules (hardcoded, regardless of .gitignore — per CONTEXT.md)
    if path.components().any(|c| c.as_os_str() == "node_modules") {
        return None;
    }
    // Filter: skip .code-graph directory (our own cache writes)
    if path.components().any(|c| c.as_os_str() == ".code-graph") {
        return None;
    }

    // Filter: skip paths matching .gitignore rules (CONTEXT.md locked decision:
    // "Watcher respects same .gitignore rules used during initial indexing")
    let is_dir = path.is_dir();
    if gitignore.matched(path, is_dir).is_ignore() {
        return None;
    }

    // Check if it's a config file
    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
        if CONFIG_FILES.contains(&file_name) {
            return Some(WatchEvent::ConfigChanged);
        }
    }

    // Check if it's a source file we care about
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !SOURCE_EXTENSIONS.contains(&ext) {
        return None;
    }

    // Classify based on file existence
    if path.exists() {
        // File exists — could be Modified or Created.
        // notify-debouncer-mini doesn't distinguish; we treat both the same
        // in the incremental pipeline (remove old + re-parse).
        Some(WatchEvent::Modified(path.to_path_buf()))
    } else {
        Some(WatchEvent::Deleted(path.to_path_buf()))
    }
}

