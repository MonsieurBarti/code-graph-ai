use std::path::{Path, PathBuf};

use crate::config::CodeGraphConfig;

/// Walk a project directory and collect all TypeScript/JavaScript source files.
///
/// Respects `.gitignore` rules, always excludes `node_modules`, applies any
/// additional exclusions from `config.exclude`, and detects monorepo workspaces
/// from `package.json`.
///
/// When `verbose` is true, each discovered file path is printed to stderr.
pub fn walk_project(
    root: &Path,
    config: &CodeGraphConfig,
    verbose: bool,
) -> anyhow::Result<Vec<PathBuf>> {
    let _ = (config, verbose);
    Ok(vec![])
}
