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
