use std::path::PathBuf;

/// Internal watch event types after classification.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    /// A source file was modified (content changed).
    Modified(PathBuf),
    /// A source file was deleted.
    Deleted(PathBuf),
    /// A config file changed (tsconfig.json, package.json) — triggers full re-index.
    ConfigChanged,
    /// A crate root or Cargo.toml changed — triggers full re-index.
    /// Carries the path for log line display.
    CrateRootChanged(PathBuf),
}
