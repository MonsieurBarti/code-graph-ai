use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::CodeGraphConfig;
use crate::language::LanguageKind;

/// Workspace field from package.json — can be either a flat list of glob patterns
/// or an object with a `packages` key.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WorkspacesField {
    Patterns(Vec<String>),
    Config { packages: Vec<String> },
}

impl WorkspacesField {
    fn patterns(&self) -> &[String] {
        match self {
            Self::Patterns(p) => p,
            Self::Config { packages } => packages,
        }
    }
}

/// Minimal package.json representation for monorepo workspace detection.
#[derive(Debug, Deserialize)]
struct PackageJson {
    workspaces: Option<WorkspacesField>,
}

/// Source file extensions that code-graph discovers.
/// .rs files are discovered and counted but not parsed until Phase 8.
const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "rs"];

/// Walk a project directory and collect source files.
///
/// Respects `.gitignore` rules, always excludes `node_modules`, applies any
/// additional exclusions from `config.exclude`, and detects monorepo workspaces
/// from `package.json`.
///
/// When `verbose` is true, each discovered file path is printed to stderr.
///
/// When `allowed_languages` is `Some(set)`, only files whose extension matches
/// one of the languages in the set are included. When `None`, all source
/// extensions are included.
pub fn walk_project(
    root: &Path,
    config: &CodeGraphConfig,
    verbose: bool,
    allowed_languages: Option<&HashSet<LanguageKind>>,
) -> anyhow::Result<Vec<PathBuf>> {
    // Always walk from the project root — this covers all files including workspace packages
    // (since workspace dirs are sub-directories of the root).
    //
    // We detect workspaces primarily so future plans can scope per-package operations,
    // but for file discovery the root walk is sufficient and avoids duplicates.
    let _ = detect_workspace_roots(root);

    let mut files = Vec::new();
    collect_files(root, config, verbose, allowed_languages, &mut files);

    Ok(files)
}

/// Walk a project directory and collect non-parsed files (everything that is not a source file).
///
/// Respects `.gitignore` rules, always excludes `node_modules`, applies any
/// additional exclusions from `config.exclude`. Returns files that are NOT
/// source code (not in SOURCE_EXTENSIONS).
///
/// These files will be added to the graph as File nodes with a kind tag but
/// will NOT have symbol extraction or import resolution.
pub fn walk_non_parsed_files(
    root: &Path,
    config: &CodeGraphConfig,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    let walker = ignore::WalkBuilder::new(root)
        .standard_filters(true)
        .require_git(false)
        .build();

    for result in walker {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();

        // Skip directories
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }

        // Exclude node_modules (hard exclusion)
        if path_contains_node_modules(path) {
            continue;
        }

        // Apply config exclusions (IDX-03: reuses same logic as source file walking)
        if is_excluded_by_config(path, config) {
            continue;
        }

        // INVERT the source extension filter: collect files that are NOT source files
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if SOURCE_EXTENSIONS.contains(&ext) {
            continue; // skip source files -- they are handled by walk_project
        }

        files.push(path.to_path_buf());
    }

    Ok(files)
}

/// Resolve workspace glob patterns from package.json to concrete directory paths.
fn detect_workspace_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![root.to_path_buf()];

    let pkg_path = root.join("package.json");
    if !pkg_path.exists() {
        return roots;
    }

    let contents = match std::fs::read_to_string(&pkg_path) {
        Ok(c) => c,
        Err(_) => return roots,
    };

    let pkg: PackageJson = match serde_json::from_str(&contents) {
        Ok(p) => p,
        Err(_) => return roots,
    };

    let workspaces = match pkg.workspaces {
        Some(w) => w,
        None => return roots,
    };

    // Expand each workspace glob pattern to concrete directories.
    for pattern in workspaces.patterns() {
        let glob_pattern = root.join(pattern).to_string_lossy().into_owned();
        if let Ok(entries) = glob::glob(&glob_pattern) {
            for entry in entries.flatten() {
                if entry.is_dir() && !roots.contains(&entry) {
                    roots.push(entry);
                }
            }
        }
    }

    roots
}

/// Collect source files from a single directory tree using the `ignore` crate.
fn collect_files(
    root: &Path,
    config: &CodeGraphConfig,
    verbose: bool,
    allowed_languages: Option<&HashSet<LanguageKind>>,
    out: &mut Vec<PathBuf>,
) {
    let walker = ignore::WalkBuilder::new(root)
        .standard_filters(true)
        // Read .gitignore files even when the directory is not inside a git repository.
        // This ensures exclusions work for standalone directories and testing scenarios.
        .require_git(false)
        .build();

    for result in walker {
        let entry = match result {
            Ok(e) => e,
            Err(err) => {
                eprintln!("warning: {err}");
                continue;
            }
        };

        let path = entry.path();

        // Skip directories (we only want files); directory filtering for node_modules
        // and config.exclude is applied during the walk via filter_entry-equivalent logic below.
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }

        // Check that no component of the path is `node_modules` — hard exclusion.
        if path_contains_node_modules(path) {
            continue;
        }

        // Apply additional config exclusions.
        if is_excluded_by_config(path, config) {
            continue;
        }

        // Filter by source extension.
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !SOURCE_EXTENSIONS.contains(&ext) {
            continue;
        }

        // Apply language filter if specified.
        if let Some(langs) = allowed_languages
            && !langs.iter().any(|lk| lk.matches_extension(ext))
        {
            continue;
        }

        if verbose {
            eprintln!("{}", path.display());
        }

        out.push(path.to_path_buf());
    }
}

/// Returns true if any component of `path` is named `node_modules`.
fn path_contains_node_modules(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| s == "node_modules")
            .unwrap_or(false)
    })
}

/// Returns true if `path` matches any exclusion pattern from config.
fn is_excluded_by_config(path: &Path, config: &CodeGraphConfig) -> bool {
    let patterns = match &config.exclude {
        Some(p) => p,
        None => return false,
    };

    let path_str = path.to_string_lossy();

    for pattern in patterns {
        if let Ok(matched) = glob::Pattern::new(pattern)
            && matched.matches(&path_str)
        {
            return true;
        }
        // Also check if any component matches the pattern directly.
        for component in path.components() {
            if let Some(s) = component.as_os_str().to_str()
                && let Ok(matched) = glob::Pattern::new(pattern)
                && matched.matches(s)
            {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn test_walk_non_parsed_finds_non_source_files() {
        let dir = tmp();
        // Create source files (should NOT be returned)
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("app.ts"), "export {}").unwrap();
        // Create non-source files (SHOULD be returned)
        fs::write(dir.path().join("README.md"), "# Hello").unwrap();
        fs::write(dir.path().join("config.toml"), "[settings]").unwrap();
        fs::write(dir.path().join("Makefile"), "all:").unwrap();

        let config = CodeGraphConfig::default();
        let files = walk_non_parsed_files(dir.path(), &config).unwrap();

        let names: Vec<String> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_str().unwrap().to_string())
            .collect();

        assert!(
            names.contains(&"README.md".to_string()),
            "should find README.md"
        );
        assert!(
            names.contains(&"config.toml".to_string()),
            "should find config.toml"
        );
        assert!(
            names.contains(&"Makefile".to_string()),
            "should find Makefile"
        );
        assert!(
            !names.contains(&"main.rs".to_string()),
            "should NOT find source files"
        );
        assert!(
            !names.contains(&"app.ts".to_string()),
            "should NOT find source files"
        );
    }

    #[test]
    fn test_walk_non_parsed_respects_exclude_patterns() {
        let dir = tmp();
        fs::write(dir.path().join("README.md"), "# Hello").unwrap();
        fs::write(dir.path().join("config.toml"), "[settings]").unwrap();

        // Create a code-graph.toml with exclude patterns
        let config = CodeGraphConfig {
            exclude: Some(vec!["*.toml".to_string()]),
            mcp: Default::default(),
        };

        let files = walk_non_parsed_files(dir.path(), &config).unwrap();

        let names: Vec<String> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_str().unwrap().to_string())
            .collect();

        assert!(
            names.contains(&"README.md".to_string()),
            "should find non-excluded files"
        );
        assert!(
            !names.contains(&"config.toml".to_string()),
            "should exclude *.toml files"
        );
    }

    #[test]
    fn test_walk_non_parsed_excludes_node_modules() {
        let dir = tmp();
        let nm = dir.path().join("node_modules").join("pkg");
        fs::create_dir_all(&nm).unwrap();
        fs::write(nm.join("package.json"), "{}").unwrap();
        fs::write(dir.path().join("README.md"), "# Hello").unwrap();

        let config = CodeGraphConfig::default();
        let files = walk_non_parsed_files(dir.path(), &config).unwrap();

        let names: Vec<String> = files
            .iter()
            .map(|f| f.to_str().unwrap().to_string())
            .collect();

        assert!(
            !names.iter().any(|n| n.contains("node_modules")),
            "should not include node_modules files"
        );
    }

    #[test]
    fn test_walk_project_returns_only_source_files() {
        let dir = tmp();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("README.md"), "# Hello").unwrap();

        let config = CodeGraphConfig::default();
        let files = walk_project(dir.path(), &config, false, None).unwrap();

        let names: Vec<String> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_str().unwrap().to_string())
            .collect();

        assert!(
            names.contains(&"main.rs".to_string()),
            "should find source files"
        );
        assert!(
            !names.contains(&"README.md".to_string()),
            "should NOT find non-source files"
        );
    }
}
