use std::path::{Path, PathBuf};

/// Marker files/directories that indicate a project root.
///
/// Checked in priority order: project-specific markers first (`.code-graph/`,
/// `code-graph.toml`), then language ecosystem markers (`Cargo.toml`,
/// `package.json`, `go.mod`, `pyproject.toml`).
const PROJECT_MARKERS: &[&str] = &[
    ".code-graph",
    "code-graph.toml",
    "Cargo.toml",
    "package.json",
    "go.mod",
    "pyproject.toml",
];

/// Walk parent directories from `cwd` looking for a project root.
///
/// Returns the first directory that contains any of the [`PROJECT_MARKERS`].
/// Returns `None` if no marker is found all the way up to the filesystem root.
pub fn detect_project_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    detect_project_root_from(&cwd)
}

/// Walk parent directories from `start` looking for a project root.
///
/// Factored out of [`detect_project_root`] for testability.
fn detect_project_root_from(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        for marker in PROJECT_MARKERS {
            if dir.join(marker).exists() {
                return Some(dir);
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Resolve a project root from an optional user-provided path.
///
/// Priority:
/// 1. If `path` is `Some`, canonicalize and return it.
/// 2. Try [`detect_project_root`] (walk parents from cwd).
/// 3. Fall back to the current working directory.
pub fn resolve_project_root(path: Option<PathBuf>) -> PathBuf {
    if let Some(p) = path {
        std::fs::canonicalize(&p).unwrap_or(p)
    } else {
        detect_project_root()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_project_root_with_cargo_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();

        let result = detect_project_root_from(&sub);
        assert_eq!(result, Some(tmp.path().to_path_buf()));
    }

    #[test]
    fn test_detect_project_root_prefers_code_graph_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let inner = tmp.path().join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        // Both markers exist, but .code-graph is checked first
        std::fs::create_dir_all(inner.join(".code-graph")).unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "").unwrap();

        let result = detect_project_root_from(&inner);
        // Should find .code-graph in inner/ before Cargo.toml in parent
        assert_eq!(result, Some(inner));
    }

    #[test]
    fn test_detect_project_root_no_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("empty");
        std::fs::create_dir_all(&sub).unwrap();

        // This walks up to the actual filesystem root, which may have markers.
        // We just verify it doesn't panic.
        let _result = detect_project_root_from(&sub);
    }

    #[test]
    fn test_resolve_project_root_with_some_path() {
        let tmp = tempfile::tempdir().unwrap();
        let result = resolve_project_root(Some(tmp.path().to_path_buf()));
        // canonicalize should succeed for an existing dir
        assert!(result.exists());
    }

    #[test]
    fn test_resolve_project_root_with_none_falls_back() {
        // When None is passed, it tries detect_project_root() then cwd.
        // This test just verifies it doesn't panic and returns something.
        let result = resolve_project_root(None);
        assert!(!result.as_os_str().is_empty());
    }
}
